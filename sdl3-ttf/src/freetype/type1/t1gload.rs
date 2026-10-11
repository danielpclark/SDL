// Rust translation of src/type1/t1gload.c and src/type1/t1gload.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Type 1 Glyph Loader (body).
//!
//! The glyph slot and size are the face's; the decoder's parse callback
//! (`T1_Parse_Glyph`) is called directly. `FT_CONFIG_OPTION_INCREMENTAL`
//! is defined, but no incremental interface is ever set, so its branches
//! are left out; `T1_CONFIG_OPTION_OLD_ENGINE` is undefined.

use super::super::base::ftcalc::{fixed_to_int, ft_mul_fix};
use super::super::base::ftobjs::*;
use super::super::base::ftoutln::*;
use super::super::cfftypes::CffSubFontRec;
use super::super::fttypes::*;
use super::super::psaux::psft::cf2_decoder_parse_charstrings_at;
use super::super::psaux::psobjs::*;
use super::super::psaux::t1decode::*;
use super::super::psaux::PsDriverRec;
use super::super::t1types::*;

/// The face data the builders and the Adobe engine read through the face.
pub fn t1_builder_face(face: &FtFaceRec) -> CffBuilderFace {
    let driver = face
        .library
        .as_ref()
        .map(|l| {
            l.modules[face.driver]
                .with_props(|d: &mut PsDriverRec| *d)
                .unwrap_or_default()
        })
        .unwrap_or_default();

    CffBuilderFace {
        units_per_EM: face.units_per_EM,
        y_ppem: face.size.metrics.y_ppem,
        no_stem_darkening: face.internal.no_stem_darkening,
        is_cff2: false,
        driver,
        num_coords: 0,
        normalizedcoords: None,
    }
}

/// `T1_Parse_Glyph_And_Get_Char_String` (the face's records the decoder
/// does not hold are `type1` and `random_seed`; the charstring is
/// returned as its offset and length in the charstrings' block)
pub fn t1_parse_glyph_and_get_char_string(
    decoder: &mut T1DecoderRec<'_>,
    type1: &T1FontRec,
    random_seed: &mut FtInt32,
    glyph_index: FtUInt,
    force_scaling: &mut bool,
) -> FtResult<(usize, usize)> {
    decoder.font_matrix = type1.font_matrix;
    decoder.font_offset = type1.font_offset;

    /* (no incremental interface) */

    /* For ordinary fonts get the character data stored in the face record. */
    let charstrings = &type1.charstrings;
    let start = charstrings
        .elements
        .get(glyph_index as usize)
        .copied()
        .flatten()
        .unwrap_or(0);
    let length = charstrings.length(glyph_index as usize) as usize;

    /* choose which renderer to use */
    /* (T1_CONFIG_OPTION_OLD_ENGINE is undefined) */
    if decoder.builder.builder.metrics_only {
        let bytes = charstrings.block.get(start..).unwrap_or(&[]);
        t1_decoder_parse_metrics(decoder, bytes, length as FtUInt)?;
    } else {
        let mut subfont = CffSubFontRec::default();

        t1_make_subfont(random_seed, &type1.private_dict, &mut subfont);

        let mut psdecoder = ps_decoder_init_t1(decoder, &mut subfont);

        let mut error = cf2_decoder_parse_charstrings_at(
            &mut psdecoder,
            &charstrings.block,
            start,
            length as FtULong,
        );

        /* Adobe's engine uses 16.16 numbers everywhere;              */
        /* as a consequence, glyphs larger than 2000ppem get rejected */
        if error == Err(FT_ERR_GLYPH_TOO_BIG) {
            /* this time, we retry unhinted and scale up the glyph later on */
            /* (the engine uses and sets the hardcoded value 0x10000 / 64 = */
            /* 0x400 for both `x_scale' and `y_scale' in this case)         */
            if let Some(glyph) = psdecoder.builder.builder.glyph.as_mut() {
                glyph.cff.hint = false;
            }

            *force_scaling = true;

            error = cf2_decoder_parse_charstrings_at(
                &mut psdecoder,
                &charstrings.block,
                start,
                length as FtULong,
            );
        }
        error?;
    }

    /* (no incremental interface overrides the metrics) */

    Ok((start, length))
}

/// `T1_Parse_Glyph`
pub fn t1_parse_glyph(
    decoder: &mut T1DecoderRec<'_>,
    type1: &T1FontRec,
    random_seed: &mut FtInt32,
    glyph_index: FtUInt,
) -> FtResult<()> {
    let mut force_scaling = false;
    t1_parse_glyph_and_get_char_string(
        decoder,
        type1,
        random_seed,
        glyph_index,
        &mut force_scaling,
    )?;

    /* (no incremental interface) */

    Ok(())
}

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/**********                                                      *********/
/**********            COMPUTE THE MAXIMUM ADVANCE WIDTH         *********/
/**********                                                      *********/
/**********    The following code is in charge of computing      *********/
/**********    the maximum advance width of the font.  It        *********/
/**********    quickly processes each glyph charstring to        *********/
/**********    extract the value from either a `sbw' or `seac'   *********/
/**********    operator.                                         *********/
/**********                                                      *********/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

/// `T1_Compute_Max_Advance`
pub fn t1_compute_max_advance(face: &mut T1FaceRec, max_advance: &mut FtPos) -> FtResult<()> {
    *max_advance = 0;

    let builder_face = t1_builder_face(&face.root);
    let T1FaceRec {
        root,
        type1,
        psnames,
        blend,
        buildchar,
        len_buildchar,
        ..
    } = face;

    /* initialize load decoder */
    let mut decoder = t1_decoder_init(
        *psnames,
        root.num_glyphs,
        builder_face,
        None, /* size       */
        None, /* glyph slot */
        Some(type1.glyph_names.clone()),
        blend.as_deref(),
        false,
        FT_RENDER_MODE_NORMAL,
        &mut buildchar[..],
    )?;

    decoder.builder.builder.metrics_only = true;
    decoder.builder.builder.load_points = false;

    decoder.num_subrs = type1.num_subrs;
    decoder.subrs = type1.subrs.clone();
    decoder.subrs_hash = type1.subrs_hash.as_ref();

    decoder.len_buildchar = *len_buildchar;

    *max_advance = 0;

    /* for each glyph, parse the glyph charstring and extract */
    /* the advance width                                      */
    for glyph_index in 0..type1.num_glyphs.max(0) {
        /* now get load the unscaled outline */
        let _ = t1_parse_glyph(
            &mut decoder,
            type1,
            &mut root.internal.random_seed,
            glyph_index as FtUInt,
        );
        if glyph_index == 0 || decoder.builder.builder.advance.x > *max_advance {
            *max_advance = decoder.builder.builder.advance.x;
        }

        /* ignore the error if one occurred - skip to next glyph */
    }

    t1_decoder_done(&mut decoder);

    Ok(())
}

/// `T1_Get_Advances`
pub fn t1_get_advances(
    t1face: &mut FtFace,
    first: FtUInt,
    count: FtUInt,
    load_flags: FtInt32,
    advances: &mut [FtFixed],
) -> FtResult<()> {
    let FtFace::T1(face) = t1face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    if load_flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
        for nn in 0..count as usize {
            advances[nn] = 0;
        }

        return Ok(());
    }

    let builder_face = t1_builder_face(&face.root);
    let T1FaceRec {
        root,
        type1,
        psnames,
        blend,
        buildchar,
        len_buildchar,
        ..
    } = &mut **face;

    let mut decoder = t1_decoder_init(
        *psnames,
        root.num_glyphs,
        builder_face,
        None, /* size       */
        None, /* glyph slot */
        Some(type1.glyph_names.clone()),
        blend.as_deref(),
        false,
        FT_RENDER_MODE_NORMAL,
        &mut buildchar[..],
    )?;

    decoder.builder.builder.metrics_only = true;
    decoder.builder.builder.load_points = false;

    decoder.num_subrs = type1.num_subrs;
    decoder.subrs = type1.subrs.clone();
    decoder.subrs_hash = type1.subrs_hash.as_ref();

    decoder.len_buildchar = *len_buildchar;

    for nn in 0..count as usize {
        let error = t1_parse_glyph(
            &mut decoder,
            type1,
            &mut root.internal.random_seed,
            first + nn as FtUInt,
        );
        if error.is_ok() {
            advances[nn] = fixed_to_int(decoder.builder.builder.advance.x);
        } else {
            advances[nn] = 0;
        }
    }

    /* (the decoder is not finalized, as in C) */

    Ok(())
}

/// `T1_Load_Glyph` (into the face's glyph slot, at its size)
pub fn t1_load_glyph(
    t1face: &mut FtFace,
    glyph_index: FtUInt,
    mut load_flags: FtInt32,
) -> FtResult<()> {
    let FtFace::T1(face) = t1face else {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    };

    /* (the face's slot and size exist together) */
    if !face.root.has_slot_and_size {
        return Err(FT_ERR_INVALID_SLOT_HANDLE);
    }

    /* (no incremental interface) */
    if glyph_index >= face.root.num_glyphs as FtUInt {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    if load_flags & FT_LOAD_NO_RECURSE != 0 {
        load_flags |= FT_LOAD_NO_SCALE | FT_LOAD_NO_HINTING;
    }

    /* (the face's size is always there: C's `t1size' is non-NULL) */
    face.glyph.x_scale = face.root.size.metrics.x_scale;
    face.glyph.y_scale = face.root.size.metrics.y_scale;

    face.root.glyph.outline.n_points = 0;
    face.root.glyph.outline.n_contours = 0;

    let mut hinting =
        (load_flags & FT_LOAD_NO_SCALE) == 0 && (load_flags & FT_LOAD_NO_HINTING) == 0;
    let scaled = (load_flags & FT_LOAD_NO_SCALE) == 0;

    face.glyph.hint = hinting;
    face.glyph.scaled = scaled;
    face.root.glyph.format = FT_GLYPH_FORMAT_OUTLINE;

    let mut force_scaling = false;

    let builder_face = t1_builder_face(&face.root);
    let size_internal = Some(face.size_module_data.is_some());

    let left_bearing: FtVector;
    let advance: FtVector;
    let font_matrix: FtMatrix;
    let font_offset: FtVector;
    let hints_funcs: bool;
    let glyph_data: (usize, usize);
    {
        let T1FaceRec {
            root,
            type1,
            psnames,
            blend,
            buildchar,
            len_buildchar,
            glyph,
            ..
        } = &mut **face;
        let FtFaceRec {
            glyph: root_glyph,
            internal,
            num_glyphs,
            ..
        } = root;

        let mut decoder = t1_decoder_init(
            *psnames,
            *num_glyphs,
            builder_face,
            size_internal,
            Some(CffGlyph {
                root: root_glyph,
                cff: glyph,
            }),
            Some(type1.glyph_names.clone()),
            blend.as_deref(),
            hinting,
            ((load_flags >> 16) & 15) as FtRenderMode, /* FT_LOAD_TARGET_MODE */
            &mut buildchar[..],
        )?;

        decoder.builder.builder.no_recurse = load_flags & FT_LOAD_NO_RECURSE != 0;

        decoder.num_subrs = type1.num_subrs;
        decoder.subrs = type1.subrs.clone();
        decoder.subrs_hash = type1.subrs_hash.as_ref();
        decoder.charstrings = Some(type1.charstrings.clone());

        decoder.len_buildchar = *len_buildchar;

        /* now load the unscaled outline */
        let error = t1_parse_glyph_and_get_char_string(
            &mut decoder,
            type1,
            &mut internal.random_seed,
            glyph_index,
            &mut force_scaling,
        );

        /* save new glyph tables (on an error, too: `Exit') */
        t1_decoder_done(&mut decoder);

        glyph_data = error?;

        hinting = decoder
            .builder
            .builder
            .glyph
            .as_ref()
            .map_or(hinting, |g| g.cff.hint);
        font_matrix = decoder.font_matrix;
        font_offset = decoder.font_offset;

        left_bearing = decoder.builder.builder.left_bearing;
        advance = decoder.builder.builder.advance;
        hints_funcs = decoder.builder.builder.hints_funcs;
    }

    /* now, set the metrics -- this is rather simple, as   */
    /* the left side bearing is the xMin, and the top side */
    /* bearing the yMax                                    */
    let font_bbox = face.type1.font_bbox;
    let (x_scale, y_scale) = (face.glyph.x_scale, face.glyph.y_scale);
    let y_ppem = face.root.size.metrics.y_ppem;
    let t1glyph = &mut face.root.glyph;

    t1glyph.outline.flags &= FT_OUTLINE_OWNER;
    t1glyph.outline.flags |= FT_OUTLINE_REVERSE_FILL;

    /* for composite glyphs, return only left side bearing and */
    /* advance width                                           */
    if load_flags & FT_LOAD_NO_RECURSE != 0 {
        let internal = &mut t1glyph.internal;

        t1glyph.metrics.horiBearingX = fixed_to_int(left_bearing.x);
        t1glyph.metrics.horiAdvance = fixed_to_int(advance.x);

        internal.glyph_matrix = font_matrix;
        internal.glyph_delta = font_offset;
        internal.glyph_transformed = true;
    } else {
        /* copy the _unscaled_ advance width */
        t1glyph.metrics.horiAdvance = fixed_to_int(advance.x);
        t1glyph.linearHoriAdvance = fixed_to_int(advance.x);
        t1glyph.internal.glyph_transformed = false;

        if load_flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
            /* make up vertical ones */
            t1glyph.metrics.vertAdvance = (font_bbox.yMax - font_bbox.yMin) >> 16;
            t1glyph.linearVertAdvance = t1glyph.metrics.vertAdvance;
        } else {
            t1glyph.metrics.vertAdvance = fixed_to_int(advance.y);
            t1glyph.linearVertAdvance = fixed_to_int(advance.y);
        }

        t1glyph.format = FT_GLYPH_FORMAT_OUTLINE;

        if y_ppem < 24 {
            t1glyph.outline.flags |= FT_OUTLINE_HIGH_PRECISION;
        }

        /* (`#if 1') */
        /* apply the font matrix, if any */
        if font_matrix.xx != 0x10000
            || font_matrix.yy != 0x10000
            || font_matrix.xy != 0
            || font_matrix.yx != 0
        {
            ft_outline_transform(&mut t1glyph.outline, &font_matrix);

            t1glyph.metrics.horiAdvance = ft_mul_fix(t1glyph.metrics.horiAdvance, font_matrix.xx);
            t1glyph.metrics.vertAdvance = ft_mul_fix(t1glyph.metrics.vertAdvance, font_matrix.yy);
        }

        if font_offset.x != 0 || font_offset.y != 0 {
            ft_outline_translate(&mut t1glyph.outline, font_offset.x, font_offset.y);

            t1glyph.metrics.horiAdvance += font_offset.x;
            t1glyph.metrics.vertAdvance += font_offset.y;
        }

        if (load_flags & FT_LOAD_NO_SCALE) == 0 || force_scaling {
            /* scale the outline and the metrics */
            /* (the builder's base outline is the slot's) */
            let cur = &mut t1glyph.outline;

            /* First of all, scale the points, if we are not hinting */
            if !hinting || !hints_funcs {
                let n = (cur.n_points.max(0) as usize).min(cur.points.len());
                for vec in cur.points[..n].iter_mut() {
                    vec.x = ft_mul_fix(vec.x, x_scale);
                    vec.y = ft_mul_fix(vec.y, y_scale);
                }
            }

            /* Then scale the metrics */
            t1glyph.metrics.horiAdvance = ft_mul_fix(t1glyph.metrics.horiAdvance, x_scale);
            t1glyph.metrics.vertAdvance = ft_mul_fix(t1glyph.metrics.vertAdvance, y_scale);
        }

        /* compute the other metrics */
        let cbox = ft_outline_get_cbox(&t1glyph.outline);

        let metrics = &mut t1glyph.metrics;
        metrics.width = cbox.xMax - cbox.xMin;
        metrics.height = cbox.yMax - cbox.yMin;

        metrics.horiBearingX = cbox.xMin;
        metrics.horiBearingY = cbox.yMax;

        if load_flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
            /* make up vertical ones */
            let va = metrics.vertAdvance;
            ft_synthesize_vertical_metrics(metrics, va);
        }
    }

    /* Set control data to the glyph charstrings.  Note that this is */
    /* _not_ zero-terminated.                                        */
    let (start, length) = glyph_data;
    t1glyph.control_data = face
        .type1
        .charstrings
        .block
        .get(start..start + length)
        .map(|b| b.to_vec())
        .unwrap_or_default();
    t1glyph.control_len = length as i64;

    /* Exit: */
    /* (no incremental interface) */

    Ok(())
}
