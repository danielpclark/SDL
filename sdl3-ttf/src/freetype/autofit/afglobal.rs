// Rust translation of src/autofit/afglobal.c and afglobal.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it), with
// `AF_CONFIG_OPTION_CJK` defined.
// Copyright (C) 2003-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter routines to compute global hinting values.
//!
//! The script and style classes that afglobal.c instantiates from
//! `afscript.h` and `afstyles.h` are in `afscript.rs` and `afstyles.rs`.

use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::afcjk::AF_CJK_WRITING_SYSTEM_CLASS;
use super::afdummy::AF_DUMMY_WRITING_SYSTEM_CLASS;
use super::afindic::AF_INDIC_WRITING_SYSTEM_CLASS;
use super::aflatin::AF_LATIN_WRITING_SYSTEM_CLASS;
use super::afmodule::AfModuleRec;
use super::afscript::*;
use super::afshaper::af_shaper_get_coverage;
use super::afstyles::*;
use super::aftypes::*;
use super::ft_hb::hb_ft_font_create_;
use crate::harfbuzz::hb_buffer::HbBuffer;
use crate::harfbuzz::hb_font::HbFontData;

/// `af_writing_system_classes`
pub static AF_WRITING_SYSTEM_CLASSES: [&AfWritingSystemClassRec; AF_WRITING_SYSTEM_MAX as usize] = [
    &AF_DUMMY_WRITING_SYSTEM_CLASS,
    &AF_LATIN_WRITING_SYSTEM_CLASS,
    &AF_CJK_WRITING_SYSTEM_CLASS,
    &AF_INDIC_WRITING_SYSTEM_CLASS,
];

/*
 * Default values and flags for both autofitter globals (found in
 * AF_ModuleRec) and face globals (in AF_FaceGlobalsRec).
 */

/* index of fallback style in `af_style_classes' */
/* (AF_CONFIG_OPTION_CJK) */
pub const AF_STYLE_FALLBACK: AfStyle = AF_STYLE_HANI_DFLT;

/* default script for OpenType; ignored if HarfBuzz isn't used */
pub const AF_SCRIPT_DEFAULT: AfScript = AF_SCRIPT_LATN;

/* a bit mask for AF_DIGIT and AF_NONBASE */
pub const AF_STYLE_MASK: FtUShort = 0x3FFF;
/* an uncovered glyph      */
pub const AF_STYLE_UNASSIGNED: FtUShort = AF_STYLE_MASK;

/* if this flag is set, we have an ASCII digit   */
pub const AF_DIGIT: FtUShort = 0x8000;
/* if this flag is set, we have a non-base character */
pub const AF_NONBASE: FtUShort = 0x4000;

/* `increase-x-height' property */
pub const AF_PROP_INCREASE_X_HEIGHT_MIN: FtUInt = 6;
pub const AF_PROP_INCREASE_X_HEIGHT_MAX: FtUInt = 0;

/************************************************************************/
/************************************************************************/
/*****                                                              *****/
/*****                  F A C E   G L O B A L S                     *****/
/*****                                                              *****/
/************************************************************************/
/************************************************************************/

/*
 * Note that glyph_styles[] maps each glyph to an index into the
 * `af_style_classes' array.
 *
 */

/// `AF_FaceGlobalsRec` (the face and the module are passed to the
/// functions that need them)
#[derive(Debug)]
pub struct AfFaceGlobalsRec {
    pub glyph_count: FtUInt, /* unsigned face->num_glyphs */
    pub glyph_styles: Vec<FtUShort>,

    /* (FT_CONFIG_OPTION_USE_HARFBUZZ) */
    pub hb_font: HbFontData,
    pub hb_buf: HbBuffer, /* for feature comparison */

    /* per-face auto-hinter properties */
    pub increase_x_height: FtUInt,

    pub metrics: Vec<Option<Box<AfStyleMetricsRec>>>,

    /* Compute darkening amount once per size.  Use this to check whether */
    /* darken_{x,y} needs to be recomputed.                               */
    pub stem_darkening_for_ppem: FtUShort,
    /* Copy from e.g. AF_LatinMetrics.axis[AF_DIMENSION_HORZ] */
    /* to compute the darkening amount.                       */
    pub standard_vertical_width: FtPos,
    /* Copy from e.g. AF_LatinMetrics.axis[AF_DIMENSION_VERT] */
    /* to compute the darkening amount.                       */
    pub standard_horizontal_width: FtPos,
    /* The actual amount to darken a glyph along the X axis. */
    pub darken_x: FtPos,
    /* The actual amount to darken a glyph along the Y axis. */
    pub darken_y: FtPos,
    /* Amount to scale down by to keep emboldened points */
    /* on the Y-axis in pre-computed blue zones.         */
    pub scale_down_factor: FtFixed,
}

/* Compute the style index of each glyph within a given face. */

/// `af_face_globals_compute_style_coverage`
fn af_face_globals_compute_style_coverage(
    globals: &mut AfFaceGlobalsRec,
    face: &mut FtFace,
    module: &AfModuleRec,
) -> FtResult<()> {
    let old_charmap = face.charmap;
    let mut dflt: FtUShort = 0xFFFF; /* a non-valid value */

    /* the value AF_STYLE_UNASSIGNED means `uncovered glyph' */
    for i in 0..globals.glyph_count as usize {
        globals.glyph_styles[i] = AF_STYLE_UNASSIGNED;
    }

    'exit: {
        if ft_select_charmap(face, FT_ENCODING_UNICODE).is_err() {
            /*
             * Ignore this error; we simply use the fallback style.
             * XXX: Shouldn't we rather disable hinting?
             */
            break 'exit;
        }

        let glyph_count = globals.glyph_count;

        /* scan each style in a Unicode charmap */
        for ss in 0..AF_STYLE_CLASSES.len() {
            let style_class = &AF_STYLE_CLASSES[ss];
            let script_class = &AF_SCRIPT_CLASSES[style_class.script as usize];

            if script_class.script_uni_ranges.is_empty() {
                continue;
            }

            /*
             * Scan all Unicode points in the range and set the corresponding
             * glyph style index.
             */
            if style_class.coverage == AF_COVERAGE_DEFAULT {
                let gstyles = &mut globals.glyph_styles;

                if style_class.script == module.default_script {
                    dflt = ss as FtUShort;
                }

                for range in script_class.script_uni_ranges {
                    if range.first == 0 {
                        break;
                    }

                    let mut charcode = range.first as FtULong;
                    let mut gindex = ft_get_char_index(face, charcode);

                    if gindex != 0
                        && gindex < glyph_count
                        && (gstyles[gindex as usize] & AF_STYLE_MASK) == AF_STYLE_UNASSIGNED
                    {
                        gstyles[gindex as usize] = ss as FtUShort;
                    }

                    loop {
                        charcode = ft_get_next_char(face, charcode, &mut gindex);

                        if gindex == 0 || charcode > range.last as FtULong {
                            break;
                        }

                        if gindex < glyph_count
                            && (gstyles[gindex as usize] & AF_STYLE_MASK) == AF_STYLE_UNASSIGNED
                        {
                            gstyles[gindex as usize] = ss as FtUShort;
                        }
                    }
                }

                /* do the same for the script's non-base characters */
                for range in script_class.script_uni_nonbase_ranges {
                    if range.first == 0 {
                        break;
                    }

                    let mut charcode = range.first as FtULong;
                    let mut gindex = ft_get_char_index(face, charcode);

                    if gindex != 0
                        && gindex < glyph_count
                        && (gstyles[gindex as usize] & AF_STYLE_MASK) == ss as FtUShort
                    {
                        gstyles[gindex as usize] |= AF_NONBASE;
                    }

                    loop {
                        charcode = ft_get_next_char(face, charcode, &mut gindex);

                        if gindex == 0 || charcode > range.last as FtULong {
                            break;
                        }

                        if gindex < glyph_count
                            && (gstyles[gindex as usize] & AF_STYLE_MASK) == ss as FtUShort
                        {
                            gstyles[gindex as usize] |= AF_NONBASE;
                        }
                    }
                }
            } else {
                /* get glyphs not directly addressable by cmap */
                let mut gstyles = std::mem::take(&mut globals.glyph_styles);
                let _ = af_shaper_get_coverage(globals, face, style_class, &mut gstyles, false);
                globals.glyph_styles = gstyles;
            }
        }

        /* handle the remaining default OpenType features ... */
        for style_class in &AF_STYLE_CLASSES {
            if style_class.coverage == AF_COVERAGE_DEFAULT {
                let mut gstyles = std::mem::take(&mut globals.glyph_styles);
                let _ = af_shaper_get_coverage(globals, face, style_class, &mut gstyles, false);
                globals.glyph_styles = gstyles;
            }
        }

        /* ... and finally the default OpenType features of the default script */
        /* (FIXME (upstream): `dflt' stays 0xFFFF, past the style classes, if */
        /* the default script has no default style; this build's shaper     */
        /* doesn't look at the style class)                                 */
        if let Some(style_class) = AF_STYLE_CLASSES.get(dflt as usize) {
            let mut gstyles = std::mem::take(&mut globals.glyph_styles);
            let _ = af_shaper_get_coverage(globals, face, style_class, &mut gstyles, true);
            globals.glyph_styles = gstyles;
        }

        /* mark ASCII digits */
        for i in 0x30..=0x39 {
            let gindex = ft_get_char_index(face, i);

            if gindex != 0 && gindex < globals.glyph_count {
                globals.glyph_styles[gindex as usize] |= AF_DIGIT;
            }
        }
    }

    /* Exit: */
    /*
     * By default, all uncovered glyphs are set to the fallback style.
     * XXX: Shouldn't we disable hinting or do something similar?
     */
    if module.fallback_style != AF_STYLE_UNASSIGNED as FtUInt {
        for nn in 0..globals.glyph_count as usize {
            if (globals.glyph_styles[nn] & AF_STYLE_MASK) == AF_STYLE_UNASSIGNED {
                globals.glyph_styles[nn] &= !AF_STYLE_MASK;
                globals.glyph_styles[nn] |= module.fallback_style as FtUShort;
            }
        }
    }

    face.charmap = old_charmap;
    Ok(())
}

/// `af_face_globals_new`
pub fn af_face_globals_new(
    face: &mut FtFace,
    module: &AfModuleRec,
) -> FtResult<Box<AfFaceGlobalsRec>> {
    /* we allocate an AF_FaceGlobals structure together */
    /* with the glyph_styles array                      */
    let glyph_count = face.num_glyphs as FtUInt;
    let mut glyph_styles = Vec::new();
    if glyph_styles
        .try_reserve_exact(glyph_count as usize)
        .is_err()
    {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    glyph_styles.resize(glyph_count as usize, 0);

    let mut globals = Box::new(AfFaceGlobalsRec {
        glyph_count,
        /* right after the globals structure come the glyph styles */
        glyph_styles,
        increase_x_height: 0,
        metrics: vec![None; AF_STYLE_MAX as usize],
        stem_darkening_for_ppem: 0,
        darken_x: 0,
        darken_y: 0,
        standard_vertical_width: 0,
        standard_horizontal_width: 0,
        scale_down_factor: 0,

        /* (FT_CONFIG_OPTION_USE_HARFBUZZ) */
        hb_font: hb_ft_font_create_(face),
        hb_buf: HbBuffer::new(),
    });

    af_face_globals_compute_style_coverage(&mut globals, face, module)?;
    globals.increase_x_height = AF_PROP_INCREASE_X_HEIGHT_MAX;

    Ok(globals)
}

/// `af_face_globals_free`
pub fn af_face_globals_free(globals: &mut AfFaceGlobalsRec) {
    for nn in 0..AF_STYLE_MAX as usize {
        if let Some(metrics) = globals.metrics[nn].as_mut() {
            let style_class = &AF_STYLE_CLASSES[nn];
            let writing_system_class =
                AF_WRITING_SYSTEM_CLASSES[style_class.writing_system as usize];

            if let Some(done) = writing_system_class.style_metrics_done {
                done(metrics);
            }
        }
        globals.metrics[nn] = None;
    }

    /* no need to free `globals->glyph_styles'; */
    /* it is part of the `globals' array        */
}

/// `af_face_globals_get_metrics`: returns the style of the metrics, in
/// `globals.metrics`
pub fn af_face_globals_get_metrics(
    globals: &mut AfFaceGlobalsRec,
    face: &mut FtFace,
    gindex: FtUInt,
    options: FtUInt,
) -> FtResult<AfStyle> {
    let mut style: AfStyle = options;

    if gindex >= globals.glyph_count {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* if we have a forced style (via `options'), use it, */
    /* otherwise look into `glyph_styles' array           */
    if style == AF_STYLE_NONE_DFLT || style + 1 >= AF_STYLE_MAX {
        style = (globals.glyph_styles[gindex as usize] & AF_STYLE_UNASSIGNED) as AfStyle;
    }

    loop {
        /* Again: */
        let style_class = &AF_STYLE_CLASSES[style as usize];
        let writing_system_class = AF_WRITING_SYSTEM_CLASSES[style_class.writing_system as usize];

        if globals.metrics[style as usize].is_none() {
            /* create the global metrics object if necessary */
            let ws = match style_class.writing_system {
                AF_WRITING_SYSTEM_LATIN => AfWritingSystemMetrics::Latin(Box::default()),
                AF_WRITING_SYSTEM_CJK | AF_WRITING_SYSTEM_INDIC => {
                    AfWritingSystemMetrics::Cjk(Box::default())
                }
                _ => AfWritingSystemMetrics::Dummy,
            };
            let mut metrics = Box::new(AfStyleMetricsRec {
                style_class,
                scaler: AfScalerRec::default(),
                digits_have_same_width: false,
                ws,
            });

            if let Some(init) = writing_system_class.style_metrics_init {
                if let Err(error) = init(&mut metrics, face, globals) {
                    if let Some(done) = writing_system_class.style_metrics_done {
                        done(&mut metrics);
                    }

                    /* internal error code -1 indicates   */
                    /* that no blue zones have been found */
                    if error == -1 {
                        style = (globals.glyph_styles[gindex as usize] & AF_STYLE_UNASSIGNED)
                            as AfStyle;
                        /* IMPORTANT: Clear the error code, see
                         * https://gitlab.freedesktop.org/freetype/freetype/-/issues/1063
                         */
                        continue;
                    }
                    return Err(error);
                }
            }

            globals.metrics[style as usize] = Some(metrics);
        }

        return Ok(style);
    }
}

/// `af_face_globals_is_digit`
pub fn af_face_globals_is_digit(globals: &AfFaceGlobalsRec, gindex: FtUInt) -> bool {
    if gindex < globals.glyph_count {
        return globals.glyph_styles[gindex as usize] & AF_DIGIT != 0;
    }

    false
}
