// Rust translation of src/cff/cffobjs.c and src/cff/cffobjs.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! OpenType objects manager (body).
//!
//! The `sfnt`, `psnames`, `pshinter` and `psaux` modules are called
//! directly; the face records whether it found them, as C keeps their
//! interfaces. CFF strings are byte strings; the face's names are made
//! from them as `String`s (bytes that are not UTF-8 become replacement
//! characters).

use super::super::base::ftcalc::*;
use super::super::base::ftobjs::*;
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::super::psaux::*;
use super::super::pshinter::pshglob::{PsPrivateRec, PshGlobalsFuncsRec};
use super::super::pshinter::pshmod::T2HintsFuncs;
use super::super::sfnt::sfobjs::{sfnt_done_face, sfnt_init_face, sfnt_load_face};
use super::super::sfnt::ttload::{tt_face_goto_table, tt_face_load_cmap, with_stream};
use super::super::sfnt::ttsbit::tt_face_set_sbit_strike;
use super::super::tttables::*;
use super::super::tttypes::TtFaceRec;
use super::cffcmap::*;
use super::cffload::*;

/*************************************************************************
 *
 *                           SIZE FUNCTIONS
 *
 */

fn cff_size_get_globals_funcs(face: &TtFaceRec) -> Option<&'static PshGlobalsFuncsRec> {
    let font = face.cff.as_ref()?;
    let library = face.root.library.as_ref()?;

    let module = library.get_module("pshinter")?;

    if !font.pshinter {
        return None;
    }

    match library.modules[module].clazz.root().module_interface {
        FtModuleInterface::PsHinter(pshinter) => {
            Some((pshinter.get_globals_funcs)(&library.modules[module]))
        }
        _ => None,
    }
}

/// `cff_size_done`
pub fn cff_size_done(face: &mut TtFaceRec) {
    if let Some(mut internal) = face.cff_size.module_data.take() {
        if let Some(funcs) = cff_size_get_globals_funcs(face) {
            (funcs.destroy)(internal.topfont.take());

            for sub in internal.subfonts.iter_mut().rev() {
                (funcs.destroy)(sub.take());
            }
        }
    }
}

/* CFF and Type 1 private dictionaries have slightly different      */
/* structures; we need to synthesize a Type 1 dictionary on the fly */

fn cff_make_private_dict(subfont: &CffSubFontRec, priv_: &mut PsPrivateRec) {
    let cpriv = &subfont.private_dict;

    *priv_ = PsPrivateRec::default();

    let count = cpriv.num_blue_values as usize;
    priv_.num_blue_values = cpriv.num_blue_values;
    for n in 0..count.min(14) {
        priv_.blue_values[n] = cpriv.blue_values[n] as FtShort;
    }

    let count = cpriv.num_other_blues as usize;
    priv_.num_other_blues = cpriv.num_other_blues;
    for n in 0..count.min(10) {
        priv_.other_blues[n] = cpriv.other_blues[n] as FtShort;
    }

    let count = cpriv.num_family_blues as usize;
    priv_.num_family_blues = cpriv.num_family_blues;
    for n in 0..count.min(14) {
        priv_.family_blues[n] = cpriv.family_blues[n] as FtShort;
    }

    let count = cpriv.num_family_other_blues as usize;
    priv_.num_family_other_blues = cpriv.num_family_other_blues;
    for n in 0..count.min(10) {
        priv_.family_other_blues[n] = cpriv.family_other_blues[n] as FtShort;
    }

    priv_.blue_scale = cpriv.blue_scale;
    priv_.blue_shift = cpriv.blue_shift as FtInt;
    priv_.blue_fuzz = cpriv.blue_fuzz as FtInt;

    priv_.standard_width[0] = cpriv.standard_width as FtUShort;
    priv_.standard_height[0] = cpriv.standard_height as FtUShort;

    let count = cpriv.num_snap_widths as usize;
    priv_.num_snap_widths = cpriv.num_snap_widths;
    for n in 0..count.min(13) {
        priv_.snap_widths[n] = cpriv.snap_widths[n] as FtShort;
    }

    let count = cpriv.num_snap_heights as usize;
    priv_.num_snap_heights = cpriv.num_snap_heights;
    for n in 0..count.min(13) {
        priv_.snap_heights[n] = cpriv.snap_heights[n] as FtShort;
    }

    priv_.force_bold = cpriv.force_bold;
    priv_.language_group = cpriv.language_group as FtLong;
    priv_.lenIV = cpriv.lenIV;
}

/// `cff_size_init`
pub fn cff_size_init(face: &mut TtFaceRec) -> FtResult<()> {
    let Some(funcs) = cff_size_get_globals_funcs(face) else {
        return Ok(());
    };
    let Some(font) = face.cff.as_ref() else {
        return Ok(());
    };

    let mut priv_ = PsPrivateRec::default();
    let mut internal = Box::new(CffInternalRec::default());

    cff_make_private_dict(&font.top_font, &mut priv_);
    internal.topfont = Some((funcs.create)(&priv_)?);

    let n = font.num_subfonts as usize;
    let mut subfonts: Vec<Option<Box<_>>> = Vec::new();
    if subfonts.try_reserve_exact(n).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    subfonts.resize_with(n, || None);

    for i in (0..n).rev() {
        let sub = &font.subfonts[i];

        cff_make_private_dict(sub, &mut priv_);
        subfonts[i] = Some((funcs.create)(&priv_)?);
    }
    internal.subfonts = subfonts;

    face.cff_size.module_data = Some(internal);

    face.cff_size.strike_index = 0xFFFFFFFF;

    Ok(())
}

/// Sets the scale of the size's PS hinter globals (`funcs->set_scale` on
/// the top font's and each subfont's).
fn cff_size_set_globals_scale(face: &mut TtFaceRec) {
    let Some(funcs) = cff_size_get_globals_funcs(face) else {
        return;
    };

    let metrics = face.root.size.metrics;
    let (Some(font), Some(internal)) = (face.cff.as_ref(), face.cff_size.module_data.as_mut())
    else {
        return;
    };

    let top_upm: FtLong = font.top_font.font_dict.units_per_em as FtLong;

    if let Some(top) = internal.topfont.as_mut() {
        (funcs.set_scale)(top, metrics.x_scale, metrics.y_scale, 0, 0);
    }

    for i in (0..font.num_subfonts as usize).rev() {
        let sub = &font.subfonts[i];
        let sub_upm: FtLong = sub.font_dict.units_per_em as FtLong;
        let (x_scale, y_scale);

        if top_upm != sub_upm {
            x_scale = ft_mul_div(metrics.x_scale, top_upm, sub_upm);
            y_scale = ft_mul_div(metrics.y_scale, top_upm, sub_upm);
        } else {
            x_scale = metrics.x_scale;
            y_scale = metrics.y_scale;
        }

        if let Some(Some(g)) = internal.subfonts.get_mut(i) {
            (funcs.set_scale)(g, x_scale, y_scale, 0, 0);
        }
    }
}

/* TT_CONFIG_OPTION_EMBEDDED_BITMAPS */

/// `cff_size_select`
pub fn cff_size_select(face: &mut TtFaceRec, strike_index: FtULong) -> FtResult<()> {
    face.cff_size.strike_index = strike_index;

    ft_select_metrics(&mut face.root, strike_index);

    cff_size_set_globals_scale(face);

    Ok(())
}

/// `cff_size_request`
pub fn cff_size_request(face: &mut TtFaceRec, req: &FtSizeRequestRec) -> FtResult<()> {
    /* TT_CONFIG_OPTION_EMBEDDED_BITMAPS */
    if ft_has_fixed_sizes(&face.root) {
        match tt_face_set_sbit_strike(face, req) {
            Err(_) => face.cff_size.strike_index = 0xFFFFFFFF,
            Ok(strike_index) => return cff_size_select(face, strike_index),
        }
    }

    ft_request_metrics(&mut face.root, req)?;

    cff_size_set_globals_scale(face);

    Ok(())
}

/*************************************************************************
 *
 *                           SLOT  FUNCTIONS
 *
 */

/// `cff_slot_done`
pub fn cff_slot_done(slot: &mut FtGlyphSlotRec) {
    slot.internal.glyph_hints = None;
}

/// `cff_slot_init` (the font's `pshinter` is the library's PS hinter
/// interface, which the face init looks up)
pub fn cff_slot_init(slot: &mut FtGlyphSlotRec) -> FtResult<()> {
    let Some(library) = slot.library.as_ref() else {
        return Ok(());
    };

    if let Some(module) = library.get_module("pshinter") {
        if let FtModuleInterface::PsHinter(pshinter) =
            library.modules[module].clazz.root().module_interface
        {
            let funcs: T2HintsFuncs = (pshinter.get_t2_funcs)(&library.modules[module]);
            slot.internal.glyph_hints = Some(Box::new(funcs));
        }
    }

    Ok(())
}

/*************************************************************************
 *
 *                          FACE  FUNCTIONS
 *
 */

fn cff_strcpy(source: &[u8]) -> Vec<u8> {
    source.to_vec()
}

/* Strip all subset prefixes of the form `ABCDEF+'.  Usually, there */
/* is only one, but font names like `APCOOG+JFABTD+FuturaBQ-Bold'   */
/* have been seen in the wild.                                      */

fn remove_subset_prefix(name: &mut Vec<u8>) {
    let mut continue_search = true;

    while continue_search {
        if name.len() + 1 >= 7 && name.get(6) == Some(&b'+') {
            for idx in 0..6 {
                /* ASCII uppercase letters */
                if !name[idx].is_ascii_uppercase() {
                    continue_search = false;
                }
            }

            if continue_search {
                name.drain(..7);
            }
        } else {
            continue_search = false;
        }
    }
}

/* Remove the style part from the family name (if present). */

fn remove_style(family_name: &mut Vec<u8>, style_name: &[u8]) {
    let family_name_length = family_name.len() as FtInt32;
    let style_name_length = style_name.len() as FtInt32;

    if family_name_length > style_name_length {
        let mut idx: FtInt = 1;

        while idx <= style_name_length {
            if family_name[(family_name_length - idx) as usize]
                != style_name[(style_name_length - idx) as usize]
            {
                break;
            }
            idx += 1;
        }

        if idx > style_name_length {
            /* family_name ends with style_name; remove it */
            idx = family_name_length - style_name_length - 1;

            /* also remove special characters     */
            /* between real family name and style */
            while idx > 0
                && (family_name[idx as usize] == b'-'
                    || family_name[idx as usize] == b' '
                    || family_name[idx as usize] == b'_'
                    || family_name[idx as usize] == b'+')
            {
                idx -= 1;
            }

            if idx > 0 {
                family_name.truncate(idx as usize + 1);
            }
        }
    }
}

/// A face name from C string bytes.
fn name_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Whether the library has the module `name` with the interface `f`
/// checks (`FT_Get_Module_Interface`).
fn has_module_interface(face: &TtFaceRec, name: &str, f: fn(&FtModuleInterface) -> bool) -> bool {
    let Some(library) = face.root.library.as_ref() else {
        return false;
    };
    match library.get_module(name) {
        Some(m) => f(&library.modules[m].clazz.root().module_interface),
        None => false,
    }
}

/// `cff_face_init`
pub fn cff_face_init(
    face: &mut TtFaceRec,
    face_index: FtInt,
    params: &[FtParameter],
) -> FtResult<()> {
    let mut pure_cff = true;
    let mut cff2 = false;
    let mut sfnt_format = false;

    if !has_module_interface(face, "sfnt", |_| true) {
        return Err(FT_ERR_MISSING_MODULE);
    }

    /* FT_FACE_FIND_GLOBAL_SERVICE( face, psnames, POSTSCRIPT_CMAPS ) */
    let psnames = has_module_interface(face, "psnames", |_| true);

    let pshinter = has_module_interface(face, "pshinter", |i| {
        matches!(i, FtModuleInterface::PsHinter(_))
    });

    let psaux = has_module_interface(face, "psaux", |i| matches!(i, FtModuleInterface::PsAux));
    if !psaux {
        return Err(FT_ERR_MISSING_MODULE);
    }
    face.psaux = psaux;

    /* FT_FACE_FIND_GLOBAL_SERVICE( face, cffload, CFF_LOAD ): the face's */
    /* driver is this one                                                 */
    let cffload = true;

    /* create input stream from resource */
    face.root.stream().seek(0)?;

    /* check whether we have a valid OpenType file */
    let error = sfnt_init_face(face, face_index, params);
    if error.is_ok() {
        if face.format_tag != TTAG_OTTO as FtULong {
            /* `OTTO'; OpenType/CFF font */
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }

        /* if we are performing a simple font format check, exit immediately */
        if face_index < 0 {
            return Ok(());
        }

        sfnt_format = true;

        /* now, the font can be either an OpenType/CFF font, or an SVG CEF */
        /* font; in the latter case it doesn't have a `head' table         */
        let head = with_stream(face, |face, stream| {
            tt_face_goto_table(face, TTAG_head as FtULong, stream)
        });
        if head.is_ok() {
            pure_cff = false;

            /* load font directory */
            sfnt_load_face(face, face_index, params)?;
        } else {
            /* load the `cmap' table explicitly */
            with_stream(face, tt_face_load_cmap)?;
        }

        /* now load the CFF part of the file; */
        /* give priority to CFF2              */
        let mut error = with_stream(face, |face, stream| {
            tt_face_goto_table(face, TTAG_CFF2 as FtULong, stream)
        })
        .map(|_| ());
        if error.is_ok() {
            cff2 = true;
            face.is_cff2 = cff2;
        }

        if error == Err(FT_ERR_TABLE_MISSING) {
            error = with_stream(face, |face, stream| {
                tt_face_goto_table(face, TTAG_CFF as FtULong, stream)
            })
            .map(|_| ());
        }

        error?;
    } else {
        /* rewind to start of file; we are going to load a pure-CFF font */
        face.root.stream().seek(0)?;
    }

    /* now load and parse the CFF table in the file */
    {
        face.cff = Some(Box::default());

        let driver_index = face.root.driver;
        let library = face.root.library().clone();
        let driver = &library.modules[driver_index];

        with_stream(face, |face, stream| {
            let mut random_seed = face.root.internal.random_seed;
            let r = cff_font_load(
                stream,
                face_index,
                face.cff.as_mut().unwrap(),
                &mut random_seed,
                driver,
                pure_cff,
                cff2,
            );
            face.root.internal.random_seed = random_seed;
            r
        })?;

        let cff = face.cff.as_mut().unwrap();

        /* if we are performing a simple font format check, exit immediately */
        /* (this is here for pure CFF)                                       */
        if face_index < 0 {
            face.root.num_faces = cff.num_faces as FtLong;
            return Ok(());
        }

        cff.pshinter = pshinter;
        cff.psnames = psnames;
        cff.cffload = cffload;

        face.root.face_index = (face_index & 0xFFFF) as FtLong;

        /* Complement the root flags with some interesting information. */
        /* Note that this is only necessary for pure CFF and CEF fonts; */
        /* SFNT based fonts use the `name' table instead.               */

        face.root.num_glyphs = cff.num_glyphs as FtLong;

        /* we need the `psnames' module for CFF and CEF formats */
        /* which aren't CID-keyed                               */
        if cff.top_font.font_dict.cid_registry == 0xFFFF && !psnames {
            return Err(FT_ERR_MISSING_MODULE);
        }

        /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
        {
            let instance_index: FtUInt = (face_index as FtUInt) >> 16;

            if ft_has_multiple_masters(&face.root) && instance_index > 0 {
                super::super::truetype::ttgxvar::tt_set_named_instance_ftmm(face, instance_index)?;
            }
        }

        let units_per_em = face.root.units_per_EM;
        let cff = face.cff.as_mut().unwrap();
        let dict = &mut cff.top_font.font_dict;

        if !dict.has_font_matrix {
            dict.units_per_em = if pure_cff {
                1000
            } else {
                units_per_em as FtULong
            };
        }

        /* Normalize the font matrix so that `matrix->yy' is 1; if  */
        /* it is zero, we use `matrix->yx' instead.  The scaling is */
        /* done with `units_per_em' then (at this point, it already */
        /* contains the scaling factor, but without normalization   */
        /* of the matrix).                                          */
        /*                                                          */
        /* Note that the offsets must be expressed in integer font  */
        /* units.                                                   */

        cff_normalize_matrix(dict);

        let top = cff.top_font.font_dict;
        for i in (0..cff.num_subfonts as usize).rev() {
            let sub = &mut cff.subfonts[i].font_dict;

            if sub.has_font_matrix {
                /* if we have a top-level matrix, */
                /* concatenate the subfont matrix */

                if top.has_font_matrix {
                    let scaling: FtLong = if top.units_per_em > 1 && sub.units_per_em > 1 {
                        top.units_per_em.min(sub.units_per_em) as FtLong
                    } else {
                        1
                    };

                    ft_matrix_multiply_scaled(&top.font_matrix, &mut sub.font_matrix, scaling);
                    ft_vector_transform_scaled(&mut sub.font_offset, &top.font_matrix, scaling);

                    sub.units_per_em = ft_mul_div(
                        sub.units_per_em as FtLong,
                        top.units_per_em as FtLong,
                        scaling,
                    ) as FtULong;
                }
            } else {
                sub.font_matrix = top.font_matrix;
                sub.font_offset = top.font_offset;

                sub.units_per_em = top.units_per_em;
            }

            cff_normalize_matrix(sub);
        }

        if pure_cff {
            let mut style_name: Option<Vec<u8>> = None;
            let dict = top;

            /* set up num_faces */
            face.root.num_faces = cff.num_faces as FtLong;

            /* compute number of glyphs */
            if dict.cid_registry != 0xFFFF {
                face.root.num_glyphs = cff.charset.max_cid as FtLong + 1;
            } else {
                face.root.num_glyphs = cff.charstrings_index.count as FtLong;
            }

            /* set global bbox, as well as EM size */
            face.root.bbox.xMin = dict.font_bbox.xMin >> 16;
            face.root.bbox.yMin = dict.font_bbox.yMin >> 16;
            /* no `U' suffix here to 0xFFFF! */
            face.root.bbox.xMax = (dict.font_bbox.xMax + 0xFFFF) >> 16;
            face.root.bbox.yMax = (dict.font_bbox.yMax + 0xFFFF) >> 16;

            face.root.units_per_EM = dict.units_per_em as FtUShort;

            face.root.ascender = face.root.bbox.yMax as FtShort;
            face.root.descender = face.root.bbox.yMin as FtShort;

            face.root.height = ((face.root.units_per_EM as FtInt * 12) / 10) as FtShort;
            if (face.root.height as FtInt)
                < face.root.ascender as FtInt - face.root.descender as FtInt
            {
                face.root.height =
                    (face.root.ascender as FtInt - face.root.descender as FtInt) as FtShort;
            }

            face.root.underline_position = (dict.underline_position >> 16) as FtShort;
            face.root.underline_thickness = (dict.underline_thickness >> 16) as FtShort;

            /* retrieve font family & style name */
            let mut family_name: Option<Vec<u8>> = None;
            if dict.family_name != 0 {
                if let Some(f) = cff_index_get_sid_string(cff, dict.family_name) {
                    family_name = Some(cff_strcpy(f));
                }
            }

            if family_name.is_none() {
                family_name = with_stream(face, |face, stream| {
                    cff_index_get_name(
                        face.cff.as_ref().unwrap(),
                        stream,
                        (face_index & 0xFFFF) as FtUInt,
                    )
                });
                if let Some(f) = family_name.as_mut() {
                    remove_subset_prefix(f);
                }
            }

            let cff = face.cff.as_ref().unwrap();

            if let Some(family) = family_name.as_mut() {
                let full = cff_index_get_sid_string(cff, dict.full_name);

                /* We try to extract the style name from the full name.   */
                /* We need to ignore spaces and dashes during the search. */
                if let Some(full) = full {
                    let at = |s: &[u8], i: usize| s.get(i).copied().unwrap_or(0);
                    let mut fullp = 0usize;
                    let mut fam = 0usize;

                    while at(full, fullp) != 0 {
                        /* skip common characters at the start of both strings */
                        if at(full, fullp) == at(family, fam) {
                            fam += 1;
                            fullp += 1;
                            continue;
                        }

                        /* ignore spaces and dashes in full name during comparison */
                        if at(full, fullp) == b' ' || at(full, fullp) == b'-' {
                            fullp += 1;
                            continue;
                        }

                        /* ignore spaces and dashes in family name during comparison */
                        if at(family, fam) == b' ' || at(family, fam) == b'-' {
                            fam += 1;
                            continue;
                        }

                        if at(family, fam) == 0 && at(full, fullp) != 0 {
                            /* The full name begins with the same characters as the  */
                            /* family name, with spaces and dashes removed.  In this */
                            /* case, the remaining string in `fullp' will be used as */
                            /* the style name.                                       */
                            let s = cff_strcpy(&full[fullp..]);

                            /* remove the style part from the family name (if present) */
                            remove_style(family, &s);
                            style_name = Some(s);
                        }
                        break;
                    }
                }
            } else if let Some(cid_font_name) = cff_index_get_sid_string(cff, dict.cid_font_name) {
                /* do we have a `/FontName' for a CID-keyed font? */
                family_name = Some(cff_strcpy(cid_font_name));
            }

            face.root.family_name = family_name.map(|f| name_string(&f));

            if let Some(s) = style_name {
                face.root.style_name = Some(name_string(&s));
            } else {
                /* assume "Regular" style if we don't know better */
                face.root.style_name = Some("Regular".to_string());
            }

            /********************************************************************
             *
             * Compute face flags.
             */
            let mut flags: FtLong = FT_FACE_FLAG_SCALABLE | /* scalable outlines */
                FT_FACE_FLAG_HORIZONTAL | /* horizontal data   */
                FT_FACE_FLAG_HINTER; /* has native hinter */

            if sfnt_format {
                flags |= FT_FACE_FLAG_SFNT;
            }

            /* fixed width font? */
            if dict.is_fixed_pitch != 0 {
                flags |= FT_FACE_FLAG_FIXED_WIDTH;
            }

            /* XXX: WE DO NOT SUPPORT KERNING METRICS IN THE GPOS TABLE FOR NOW */

            face.root.face_flags |= flags;

            /********************************************************************
             *
             * Compute style flags.
             */
            let mut flags: FtLong = 0;

            if dict.italic_angle != 0 {
                flags |= FT_STYLE_FLAG_ITALIC;
            }

            {
                if let Some(weight) = cff_index_get_sid_string(cff, dict.weight) {
                    if weight == b"Bold" || weight == b"Black" {
                        flags |= FT_STYLE_FLAG_BOLD;
                    }
                }
            }

            /* double check */
            if flags & FT_STYLE_FLAG_BOLD == 0 {
                if let Some(style) = face.root.style_name.as_ref() {
                    if style.as_bytes().starts_with(b"Bold")
                        || style.as_bytes().starts_with(b"Black")
                    {
                        flags |= FT_STYLE_FLAG_BOLD;
                    }
                }
            }

            face.root.style_flags = flags;
        }

        let cff = face.cff.as_ref().unwrap();
        let dict = cff.top_font.font_dict;

        /* CID-keyed CFF or CFF2 fonts don't have glyph names -- the SFNT */
        /* loader has unset this flag because of the 3.0 `post' table.    */
        if dict.cid_registry == 0xFFFF && !cff2 {
            face.root.face_flags |= FT_FACE_FLAG_GLYPH_NAMES;
        }

        if dict.cid_registry != 0xFFFF && pure_cff {
            face.root.face_flags |= FT_FACE_FLAG_CID_KEYED;
        }

        /********************************************************************
         *
         * Compute char maps.
         */

        /* Try to synthesize a Unicode charmap if there is none available */
        /* already.  If an OpenType font contains a Unicode "cmap", we    */
        /* will use it, whatever be in the CFF part of the file.          */
        {
            let mut skip_unicode = false;

            for cmap in face.root.charmaps.iter() {
                /* Windows Unicode? */
                if cmap.charmap.platform_id == TT_PLATFORM_MICROSOFT
                    && cmap.charmap.encoding_id == TT_MS_ID_UNICODE_CS
                {
                    skip_unicode = true;
                    break;
                }

                /* Apple Unicode platform id? */
                if cmap.charmap.platform_id == TT_PLATFORM_APPLE_UNICODE {
                    skip_unicode = true; /* Apple Unicode */
                    break;
                }
            }

            if !skip_unicode {
                /* since CID-keyed fonts don't contain glyph names, we can't */
                /* construct a cmap                                          */
                if pure_cff && dict.cid_registry != 0xFFFF {
                    return Ok(());
                }

                /* we didn't find a Unicode charmap -- synthesize one */
                let cmaprec = FtCharMapRec {
                    platform_id: TT_PLATFORM_MICROSOFT,
                    encoding_id: TT_MS_ID_UNICODE_CS,
                    encoding: FT_ENCODING_UNICODE,
                };

                let nn = face.root.num_charmaps;

                match cff_cmap_unicode_init(face.cff.as_ref().unwrap()) {
                    Ok(unicodes) => {
                        ft_cmap_new(
                            &mut face.root,
                            &CFF_CMAP_UNICODE_CLASS_REC,
                            FtCMapData::PsUnicodes(unicodes),
                            cmaprec,
                        )?;
                    }
                    Err(e)
                        if ft_err_neq(e, FT_ERR_NO_UNICODE_GLYPH_NAME)
                            && ft_err_neq(e, FT_ERR_UNIMPLEMENTED_FEATURE) =>
                    {
                        return Err(e);
                    }
                    Err(_) => {}
                }

                /* if no Unicode charmap was previously selected, select this one */
                if face.root.charmap.is_none() && nn != face.root.num_charmaps {
                    face.root.charmap = Some(nn as usize);
                }
            }

            /* Skip_Unicode: */
            let cff = face.cff.as_ref().unwrap();
            let encoding = &cff.encoding;
            if encoding.count > 0 {
                let mut cmaprec = FtCharMapRec {
                    platform_id: TT_PLATFORM_ADOBE, /* Adobe platform id */
                    ..Default::default()
                };

                if encoding.offset == 0 {
                    cmaprec.encoding_id = TT_ADOBE_ID_STANDARD;
                    cmaprec.encoding = FT_ENCODING_ADOBE_STANDARD;
                } else if encoding.offset == 1 {
                    cmaprec.encoding_id = TT_ADOBE_ID_EXPERT;
                    cmaprec.encoding = FT_ENCODING_ADOBE_EXPERT;
                } else {
                    cmaprec.encoding_id = TT_ADOBE_ID_CUSTOM;
                    cmaprec.encoding = FT_ENCODING_ADOBE_CUSTOM;
                }

                let data = cff_cmap_encoding_init(cff);
                ft_cmap_new(&mut face.root, &CFF_CMAP_ENCODING_CLASS_REC, data, cmaprec)?;
            }
        }
    }

    /* Exit: */
    Ok(())
}

/// The font matrix normalization of `cff_face_init`: normalize the font
/// matrix so that `matrix->yy' is 1; if it is zero, we use `matrix->yx'
/// instead.
fn cff_normalize_matrix(dict: &mut CffFontRecDictRec) {
    let matrix = &mut dict.font_matrix;
    let offset = &mut dict.font_offset;
    let upm = &mut dict.units_per_em;

    let temp: FtFixed = if matrix.yy != 0 {
        matrix.yy.abs()
    } else {
        matrix.yx.abs()
    };

    if temp != 0x10000 {
        *upm = ft_div_fix(*upm as FtLong, temp) as FtULong;

        matrix.xx = ft_div_fix(matrix.xx, temp);
        matrix.yx = ft_div_fix(matrix.yx, temp);
        matrix.xy = ft_div_fix(matrix.xy, temp);
        matrix.yy = ft_div_fix(matrix.yy, temp);
        offset.x = ft_div_fix(offset.x, temp);
        offset.y = ft_div_fix(offset.y, temp);
    }

    offset.x >>= 16;
    offset.y >>= 16;
}

/// `cff_face_done`
pub fn cff_face_done(face: &mut TtFaceRec) {
    /* (the `sfnt' module is called directly) */
    sfnt_done_face(face);

    if let Some(mut cff) = face.cff.take() {
        cff_font_done(&mut cff);
    }

    /* TT_CONFIG_OPTION_GX_VAR_SUPPORT */
    cff_done_blend(face);
    face.blend = None;
}

/// `cff_driver_init`
pub fn cff_driver_init(library: &mut FtLibraryRec, module: usize) -> FtResult<()> {
    let mut driver = PsDriverRec {
        /* set default property values, cf. `ftcffdrv.h' */
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
        ^ (library as *const FtLibraryRec as usize)
        ^ (&library.modules[module] as *const FtModuleRec as usize))
        as FtUInt32;
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

/// `cff_driver_done`
pub fn cff_driver_done(_module: &FtModuleRec) {}
