// Rust translation of src/autofit/afloader.c and afloader.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2003-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter glyph loading routines.
//!
//! Translation notes: the face's globals (`face->autohint.data`) are taken
//! out of the face while a glyph is loaded and put back afterwards, and
//! the style metrics out of the globals, so that the face can be used
//! (FreeType loads the unscaled glyph through `FT_Load_Glyph`) while they
//! are.

use super::super::base::ftcalc::{ft_div_fix, ft_matrix_invert, ft_msb, ft_mul_div, ft_mul_fix};
use super::super::base::ftobjs::*;
use super::super::base::ftoutln::*;
use super::super::fttypes::*;
use super::afglobal::*;
use super::afhints::*;
use super::afmodule::AfModuleRec;
use super::afstyles::AF_STYLE_NONE_DFLT;
use super::aftypes::*;

/*
 * The autofitter module's (global) data structure to communicate with
 * actual fonts.  If necessary, `local' data like the current face, the
 * current face's auto-hint data, or the current glyph's parameters
 * relevant to auto-hinting are `swapped in'.  Cf. functions like
 * `af_loader_reset' and `af_loader_load_g'.
 */

/// `AF_LoaderRec`
#[derive(Debug, Default)]
pub struct AfLoaderRec {
    /* current face data */
    /* (`face' and `globals' are passed and taken out of the face) */

    /* current glyph data */
    pub hints: AfGlyphHintsRec,
    /* (`metrics' is taken out of the globals) */
    pub transformed: bool,
    pub trans_matrix: FtMatrix,
    pub trans_delta: FtVector,
    pub pp1: FtVector,
    pub pp2: FtVector,
    /* we don't handle vertical phantom points */
}

/* Initialize glyph loader. */

/// `af_loader_init`
pub fn af_loader_init(hints: AfGlyphHintsRec) -> AfLoaderRec {
    AfLoaderRec {
        hints,
        ..Default::default()
    }
}

/* Reset glyph loader and compute globals if necessary. */

/// `af_loader_reset`: takes the face globals out of the face (creating
/// them if necessary)
pub fn af_loader_reset(module: &AfModuleRec, face: &mut FtFace) -> FtResult<Box<AfFaceGlobalsRec>> {
    match face.autohint.take() {
        Some(data) => match data.downcast::<AfFaceGlobalsRec>() {
            Ok(globals) => Ok(globals),
            Err(_) => Err(FT_ERR_INVALID_ARGUMENT),
        },
        None => af_face_globals_new(face, module),
    }
}

/* Finalize glyph loader. */

/// `af_loader_done`
pub fn af_loader_done(loader: &mut AfLoaderRec) {
    let _ = loader;
}

/// `af_intToFixed`
const fn af_int_to_fixed(i: FtInt) -> FtFixed {
    ((i as FtUInt32) << 16) as FtFixed
}
/// `af_fixedToInt`
const fn af_fixed_to_int(x: FtFixed) -> FtShort {
    (((x as FtUInt32).wrapping_add(0x8000)) >> 16) as FtShort
}
/// `af_floatToFixed`
const fn af_float_to_fixed(f: f64) -> FtFixed {
    (f * 65536.0 + 0.5) as FtFixed
}

/// `af_loader_embolden_glyph_in_slot`
fn af_loader_embolden_glyph_in_slot(
    module: &AfModuleRec,
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
    style_metrics: &AfStyleMetricsRec,
) -> FtResult<()> {
    let size_metrics = face.size.internal.autohint_metrics;

    let mut std_vw: FtPos = 0;
    let mut std_hw: FtPos = 0;

    let size_changed = size_metrics.x_ppem != globals.stem_darkening_for_ppem;

    let em_size = af_int_to_fixed(face.units_per_EM as FtInt);

    let mut scale_down_matrix = FtMatrix {
        xx: 0x10000,
        xy: 0,
        yx: 0,
        yy: 0x10000,
    };

    /* Skip stem darkening for broken fonts. */
    if face.units_per_EM == 0 {
        return Err(FT_ERR_CORRUPTED_FONT_HEADER);
    }

    /*
     * We depend on the writing system (script analyzers) to supply
     * standard widths for the script of the glyph we are looking at.  If
     * it can't deliver, stem darkening is disabled.
     */
    let writing_system_class =
        AF_WRITING_SYSTEM_CLASSES[style_metrics.style_class.writing_system as usize];

    if let Some(getstdw) = writing_system_class.style_metrics_getstdw {
        (std_hw, std_vw) = getstdw(style_metrics);
    } else {
        return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
    }

    if size_changed || (std_vw > 0 && std_vw != globals.standard_vertical_width) {
        let darken_by_font_units_x = af_loader_compute_darkening(module, face, std_vw);
        let darken_x = ft_mul_fix(darken_by_font_units_x, size_metrics.x_scale);

        globals.standard_vertical_width = std_vw;
        globals.stem_darkening_for_ppem = size_metrics.x_ppem;
        globals.darken_x = af_fixed_to_int(darken_x) as FtPos;
    }

    if size_changed || (std_hw > 0 && std_hw != globals.standard_horizontal_width) {
        let darken_by_font_units_y = af_loader_compute_darkening(module, face, std_hw);
        let darken_y = ft_mul_fix(darken_by_font_units_y, size_metrics.y_scale);

        globals.standard_horizontal_width = std_hw;
        globals.stem_darkening_for_ppem = size_metrics.x_ppem;
        globals.darken_y = af_fixed_to_int(darken_y) as FtPos;

        /*
         * Scale outlines down on the Y-axis to keep them inside their blue
         * zones.  The stronger the emboldening, the stronger the downscaling
         * (plus heuristical padding to prevent outlines still falling out
         * their zones due to rounding).
         *
         * Reason: `FT_Outline_Embolden' works by shifting the rightmost
         * points of stems farther to the right, and topmost points farther
         * up.  This positions points on the Y-axis outside their
         * pre-computed blue zones and leads to distortion when applying the
         * hints in the code further below.  Code outside this emboldening
         * block doesn't know we are presenting it with modified outlines the
         * analyzer didn't see!
         *
         * An unfortunate side effect of downscaling is that the emboldening
         * effect is slightly decreased.  The loss becomes more pronounced
         * versus the CFF driver at smaller sizes, e.g., at 9ppem and below.
         */
        globals.scale_down_factor = ft_div_fix(
            em_size - (darken_by_font_units_y + af_int_to_fixed(8)),
            em_size,
        );
    }

    let _ = ft_outline_embolden_xy(&mut face.glyph.outline, globals.darken_x, globals.darken_y);

    scale_down_matrix.yy = globals.scale_down_factor;
    ft_outline_transform(&mut face.glyph.outline, &scale_down_matrix);

    Ok(())
}

/* Load the glyph at index into the current slot of a face and hint it. */

/// `af_loader_load_glyph`
pub fn af_loader_load_glyph(
    loader: &mut AfLoaderRec,
    module: &AfModuleRec,
    face: &mut FtFace,
    glyph_index: FtUInt,
    mut load_flags: FtInt32,
) -> FtResult<()> {
    let style_options: FtUInt = AF_STYLE_NONE_DFLT;

    if face.size.internal.autohint_metrics.x_scale == 0
        || face.size.internal.autohint_mode != ft_load_target_mode(load_flags)
    {
        /* switching between hinting modes usually means different scaling */
        /* values; this later on enforces recomputation of everything      */
        /* related to the current size                                     */

        face.size.internal.autohint_mode = ft_load_target_mode(load_flags);
        face.size.internal.autohint_metrics = face.size.metrics;

        /* (AF_CONFIG_OPTION_TT_SIZE_METRICS is undefined) */
    }

    /*
     * TODO: This code currently doesn't support fractional advance widths,
     * i.e., placing hinted glyphs at anything other than integer
     * x-positions.  This is only relevant for the warper code, which
     * scales and shifts glyphs to optimize blackness of stems (hinting on
     * the x-axis by nature places things on pixel integers, hinting on the
     * y-axis only, i.e., LIGHT mode, doesn't touch the x-axis).  The delta
     * values of the scaler would need to be adjusted.
     */
    let scaler = AfScalerRec {
        face: AfScalerFace::of(face),
        x_scale: face.size.internal.autohint_metrics.x_scale,
        x_delta: 0,
        y_scale: face.size.internal.autohint_metrics.y_scale,
        y_delta: 0,

        render_mode: ft_load_target_mode(load_flags),
        flags: 0,
    };

    /* note that the fallback style can't be changed anymore */
    /* after the first call of `af_loader_load_glyph'        */
    let mut globals = af_loader_reset(module, face)?;

    let result = af_loader_load_glyph_globals(
        loader,
        module,
        face,
        &mut globals,
        glyph_index,
        &mut load_flags,
        scaler,
        style_options,
    );

    /* the globals stay with the face (`face->autohint.data') */
    face.autohint = Some(globals);

    result
}

/// The part of `af_loader_load_glyph` that runs with the face globals.
fn af_loader_load_glyph_globals(
    loader: &mut AfLoaderRec,
    module: &AfModuleRec,
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
    glyph_index: FtUInt,
    load_flags: &mut FtInt32,
    scaler: AfScalerRec,
    style_options: FtUInt,
) -> FtResult<()> {
    /*
     * Glyphs (really code points) are assigned to scripts.  Script
     * analysis is done lazily: For each glyph that passes through here,
     * the corresponding script analyzer is called, but returns immediately
     * if it has been run already.
     */
    let style = af_face_globals_get_metrics(globals, face, glyph_index, style_options)?;

    let mut style_metrics = globals.metrics[style as usize]
        .take()
        .expect("style metrics");
    let result = af_loader_load_glyph_metrics(
        loader,
        module,
        face,
        globals,
        &mut style_metrics,
        glyph_index,
        load_flags,
        scaler,
    );
    globals.metrics[style as usize] = Some(style_metrics);

    result
}

/// The part of `af_loader_load_glyph` that runs with the style metrics.
fn af_loader_load_glyph_metrics(
    loader: &mut AfLoaderRec,
    module: &AfModuleRec,
    face: &mut FtFace,
    globals: &mut AfFaceGlobalsRec,
    style_metrics: &mut AfStyleMetricsRec,
    glyph_index: FtUInt,
    load_flags: &mut FtInt32,
    scaler: AfScalerRec,
) -> FtResult<()> {
    let style_class = style_metrics.style_class;
    let writing_system_class = AF_WRITING_SYSTEM_CLASSES[style_class.writing_system as usize];

    /* loader->metrics = style_metrics; */

    if let Some(scale) = writing_system_class.style_metrics_scale {
        scale(style_metrics, &scaler, globals);
    } else {
        style_metrics.scaler = scaler;
    }

    if let Some(init) = writing_system_class.style_hints_init {
        init(&mut loader.hints, style_metrics)?;
    }

    /*
     * Do the main work of `af_loader_load_glyph'.  Note that we never have
     * to deal with composite glyphs as those get loaded into
     * FT_GLYPH_FORMAT_OUTLINE by the recursed `FT_Load_Glyph' function.
     * In the rare cases where FT_LOAD_NO_RECURSE is set, it implies
     * FT_LOAD_NO_SCALE and as such the auto-hinter is never called.
     */
    *load_flags |= FT_LOAD_NO_SCALE | FT_LOAD_IGNORE_TRANSFORM | FT_LOAD_LINEAR_DESIGN;
    *load_flags &= !FT_LOAD_RENDER;

    ft_load_glyph(face, glyph_index, *load_flags)?;

    /*
     * Apply stem darkening (emboldening) here before hints are applied to
     * the outline.  Glyphs are scaled down proportionally to the
     * emboldening so that curve points don't fall outside their
     * precomputed blue zones.
     *
     * Any emboldening done by the font driver (e.g., the CFF driver)
     * doesn't reach here because the autohinter loads the unprocessed
     * glyphs in font units for analysis (functions `af_*_metrics_init_*')
     * and then above to prepare it for the rasterizers by itself,
     * independently of the font driver.  So emboldening must be done here,
     * within the autohinter.
     *
     * All glyphs to be autohinted pass through here one by one.  The
     * standard widths can therefore change from one glyph to the next,
     * depending on what script a glyph is assigned to (each script has its
     * own set of standard widths and other metrics).  The darkening amount
     * must therefore be recomputed for each size and
     * `standard_{vertical,horizontal}_width' change.
     *
     * Ignore errors and carry on without emboldening.
     *
     */

    /* stem darkening only works well in `light' mode */
    if scaler.render_mode == FT_RENDER_MODE_LIGHT
        && (face.internal.no_stem_darkening == 0
            || (face.internal.no_stem_darkening < 0 && !module.no_stem_darkening))
    {
        let _ = af_loader_embolden_glyph_in_slot(module, face, globals, style_metrics);
    }

    loader.transformed = face.glyph.internal.glyph_transformed;
    if loader.transformed {
        loader.trans_matrix = face.glyph.internal.glyph_matrix;
        loader.trans_delta = face.glyph.internal.glyph_delta;

        let mut inverse = loader.trans_matrix;
        if ft_matrix_invert(&mut inverse).is_ok() {
            ft_vector_transform(&mut loader.trans_delta, &inverse);
        }
    }

    let mut error: FtResult<()> = Ok(());
    let hints = &mut loader.hints;

    'hint_metrics: {
        match face.glyph.format {
            FT_GLYPH_FORMAT_OUTLINE => {
                /* translate the loaded glyph when an internal transform is needed */
                if loader.transformed {
                    ft_outline_translate(
                        &mut face.glyph.outline,
                        loader.trans_delta.x,
                        loader.trans_delta.y,
                    );
                }

                /* compute original horizontal phantom points */
                /* (and ignore vertical ones)                 */
                loader.pp1.x = hints.x_delta;
                loader.pp1.y = hints.y_delta;
                loader.pp2.x =
                    ft_mul_fix(face.glyph.metrics.horiAdvance, hints.x_scale) + hints.x_delta;
                loader.pp2.y = hints.y_delta;

                /* be sure to check for spacing glyphs */
                if face.glyph.outline.n_points == 0 {
                    break 'hint_metrics;
                }

                /* now load the slot image into the auto-outline */
                /* and run the automatic hinting process         */
                if let Some(apply) = writing_system_class.style_hints_apply {
                    apply(
                        glyph_index,
                        hints,
                        &mut face.glyph.outline,
                        style_metrics,
                        globals,
                    )?;
                }

                /* we now need to adjust the metrics according to the change in */
                /* width/positioning that occurred during the hinting process   */
                if scaler.render_mode != FT_RENDER_MODE_LIGHT {
                    let axis = &hints.axis[AF_DIMENSION_HORZ];

                    if axis.num_edges > 1 && af_hints_do_advance(hints) {
                        let edge1 = &axis.edges[0]; /* leftmost edge  */
                        let edge2 = &axis.edges[axis.num_edges as usize - 1]; /* rightmost edge */

                        let old_rsb = loader.pp2.x - edge2.opos;
                        /* loader->pp1.x is always zero at this point of time */
                        let old_lsb = edge1.opos; /* - loader->pp1.x */
                        let new_lsb = edge1.pos;

                        /* remember unhinted values to later account */
                        /* for rounding errors                       */
                        let mut pp1x_uh = new_lsb - old_lsb;
                        let mut pp2x_uh = edge2.pos + old_rsb;

                        /* prefer too much space over too little space */
                        /* for very small sizes                        */

                        if old_lsb < 24 {
                            pp1x_uh -= 8;
                        }

                        if old_rsb < 24 {
                            pp2x_uh += 8;
                        }

                        loader.pp1.x = ft_pix_round(pp1x_uh);
                        loader.pp2.x = ft_pix_round(pp2x_uh);

                        if loader.pp1.x >= new_lsb && old_lsb > 0 {
                            loader.pp1.x -= 64;
                        }

                        if loader.pp2.x <= edge2.pos && old_rsb > 0 {
                            loader.pp2.x += 64;
                        }

                        face.glyph.lsb_delta = loader.pp1.x - pp1x_uh;
                        face.glyph.rsb_delta = loader.pp2.x - pp2x_uh;
                    } else {
                        let pp1x = loader.pp1.x;
                        let pp2x = loader.pp2.x;

                        loader.pp1.x = ft_pix_round(pp1x);
                        loader.pp2.x = ft_pix_round(pp2x);

                        face.glyph.lsb_delta = loader.pp1.x - pp1x;
                        face.glyph.rsb_delta = loader.pp2.x - pp2x;
                    }
                }
                /* `light' mode uses integer advance widths */
                /* but sets `lsb_delta' and `rsb_delta'     */
                else {
                    let pp1x = loader.pp1.x;
                    let pp2x = loader.pp2.x;

                    loader.pp1.x = ft_pix_round(pp1x);
                    loader.pp2.x = ft_pix_round(pp2x);

                    face.glyph.lsb_delta = loader.pp1.x - pp1x;
                    face.glyph.rsb_delta = loader.pp2.x - pp2x;
                }
            }

            _ => {
                /* we don't support other formats (yet?) */
                error = Err(FT_ERR_UNIMPLEMENTED_FEATURE);
            }
        }
    }

    /* Hint_Metrics: */
    {
        let slot = &mut face.glyph;

        let mut vvector = FtVector {
            x: slot.metrics.vertBearingX - slot.metrics.horiBearingX,
            y: slot.metrics.vertBearingY - slot.metrics.horiBearingY,
        };
        vvector.x = ft_mul_fix(vvector.x, style_metrics.scaler.x_scale);
        vvector.y = ft_mul_fix(vvector.y, style_metrics.scaler.y_scale);

        /* transform the hinted outline if needed */
        if loader.transformed {
            ft_outline_transform(&mut slot.outline, &loader.trans_matrix);
            ft_vector_transform(&mut vvector, &loader.trans_matrix);
        }

        /* we must translate our final outline by -pp1.x and compute */
        /* the new metrics                                           */
        if loader.pp1.x != 0 {
            ft_outline_translate(&mut slot.outline, -loader.pp1.x, 0);
        }

        let mut bbox = ft_outline_get_cbox(&slot.outline);

        bbox.xMin = ft_pix_floor(bbox.xMin);
        bbox.yMin = ft_pix_floor(bbox.yMin);
        bbox.xMax = ft_pix_ceil(bbox.xMax);
        bbox.yMax = ft_pix_ceil(bbox.yMax);

        slot.metrics.width = bbox.xMax - bbox.xMin;
        slot.metrics.height = bbox.yMax - bbox.yMin;
        slot.metrics.horiBearingX = bbox.xMin;
        slot.metrics.horiBearingY = bbox.yMax;

        slot.metrics.vertBearingX = ft_pix_floor(bbox.xMin + vvector.x);
        slot.metrics.vertBearingY = ft_pix_floor(bbox.yMax + vvector.y);

        /* for mono-width fonts (like Andale, Courier, etc.) we need */
        /* to keep the original rounded advance width; ditto for     */
        /* digits if all have the same advance width                 */
        if scaler.render_mode != FT_RENDER_MODE_LIGHT
            && (slot.face_flags & FT_FACE_FLAG_FIXED_WIDTH != 0
                || (af_face_globals_is_digit(globals, glyph_index)
                    && style_metrics.digits_have_same_width))
        {
            slot.metrics.horiAdvance =
                ft_mul_fix(slot.metrics.horiAdvance, style_metrics.scaler.x_scale);

            /* Set delta values to 0.  Otherwise code that uses them is */
            /* going to ruin the fixed advance width.                   */
            slot.lsb_delta = 0;
            slot.rsb_delta = 0;
        } else {
            /* non-spacing glyphs must stay as-is */
            if slot.metrics.horiAdvance != 0 {
                slot.metrics.horiAdvance = loader.pp2.x - loader.pp1.x;
            }
        }

        slot.metrics.vertAdvance =
            ft_mul_fix(slot.metrics.vertAdvance, style_metrics.scaler.y_scale);

        slot.metrics.horiAdvance = ft_pix_round(slot.metrics.horiAdvance);
        slot.metrics.vertAdvance = ft_pix_round(slot.metrics.vertAdvance);

        slot.format = FT_GLYPH_FORMAT_OUTLINE;
    }

    error
}

/*
 * Compute amount of font units the face should be emboldened by, in
 * analogy to the CFF driver's `cf2_computeDarkening' function.  See there
 * for details of the algorithm.
 *
 * XXX: Currently a crude adaption of the original algorithm.  Do better?
 */

/// `af_loader_compute_darkening`
pub fn af_loader_compute_darkening(
    module: &AfModuleRec,
    face: &FtFace,
    standard_width: FtPos,
) -> FtFixed {
    let stem_width: FtFixed;
    let stem_width_per_1000: FtFixed;
    let scaled_stem: FtFixed;
    let mut darken_amount: FtFixed;

    let ppem = af_int_to_fixed(4).max(af_int_to_fixed(face.size.metrics.x_ppem as FtInt));
    let units_per_em = face.units_per_EM;

    let em_ratio = ft_div_fix(
        af_int_to_fixed(1000),
        af_int_to_fixed(units_per_em as FtInt),
    );
    if em_ratio < af_float_to_fixed(0.01) {
        /* If something goes wrong, don't embolden. */
        return 0;
    }

    let x1 = module.darken_params[0];
    let y1 = module.darken_params[1];
    let x2 = module.darken_params[2];
    let y2 = module.darken_params[3];
    let x3 = module.darken_params[4];
    let y3 = module.darken_params[5];
    let x4 = module.darken_params[6];
    let y4 = module.darken_params[7];

    if standard_width <= 0 {
        stem_width = af_int_to_fixed(75); /* taken from cf2font.c */
        stem_width_per_1000 = stem_width;
    } else {
        stem_width = af_int_to_fixed(standard_width as FtInt);
        stem_width_per_1000 = ft_mul_fix(stem_width, em_ratio);
    }

    let log_base_2 = ft_msb(stem_width_per_1000 as FtUInt32) + ft_msb(ppem as FtUInt32);

    if log_base_2 >= 46 {
        /* possible overflow */
        scaled_stem = af_int_to_fixed(x4);
    } else {
        scaled_stem = ft_mul_fix(stem_width_per_1000, ppem);
    }

    /* now apply the darkening parameters */
    enum Next {
        TryX3,
        TryX4,
        UseY4,
    }
    let mut next: Option<Next> = None;

    if scaled_stem < af_int_to_fixed(x1) {
        darken_amount = ft_div_fix(af_int_to_fixed(y1), ppem);
    } else if scaled_stem < af_int_to_fixed(x2) {
        let xdelta = x2 - x1;
        let ydelta = y2 - y1;
        let x = (stem_width_per_1000 - ft_div_fix(af_int_to_fixed(x1), ppem)) as FtInt;

        if xdelta == 0 {
            next = Some(Next::TryX3);
            darken_amount = 0;
        } else {
            darken_amount = ft_mul_div(x as FtLong, ydelta as FtLong, xdelta as FtLong)
                + ft_div_fix(af_int_to_fixed(y1), ppem);
        }
    } else if scaled_stem < af_int_to_fixed(x3) {
        next = Some(Next::TryX3);
        darken_amount = 0;
    } else if scaled_stem < af_int_to_fixed(x4) {
        next = Some(Next::TryX4);
        darken_amount = 0;
    } else {
        next = Some(Next::UseY4);
        darken_amount = 0;
    }

    if let Some(Next::TryX3) = next {
        /* Try_x3: */
        let xdelta = x3 - x2;
        let ydelta = y3 - y2;
        let x = (stem_width_per_1000 - ft_div_fix(af_int_to_fixed(x2), ppem)) as FtInt;

        if xdelta == 0 {
            next = Some(Next::TryX4);
        } else {
            darken_amount = ft_mul_div(x as FtLong, ydelta as FtLong, xdelta as FtLong)
                + ft_div_fix(af_int_to_fixed(y2), ppem);
            next = None;
        }
    }

    if let Some(Next::TryX4) = next {
        /* Try_x4: */
        let xdelta = x4 - x3;
        let ydelta = y4 - y3;
        let x = (stem_width_per_1000 - ft_div_fix(af_int_to_fixed(x3), ppem)) as FtInt;

        if xdelta == 0 {
            next = Some(Next::UseY4);
        } else {
            darken_amount = ft_mul_div(x as FtLong, ydelta as FtLong, xdelta as FtLong)
                + ft_div_fix(af_int_to_fixed(y3), ppem);
            next = None;
        }
    }

    if let Some(Next::UseY4) = next {
        /* Use_y4: */
        darken_amount = ft_div_fix(af_int_to_fixed(y4), ppem);
    }

    let _ = stem_width;

    /* Convert darken_amount from per 1000 em to true character space. */
    ft_div_fix(darken_amount, em_ratio)
}
