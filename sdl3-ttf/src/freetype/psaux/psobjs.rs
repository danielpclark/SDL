// Rust translation of the CFF and PS builder parts of src/psaux/psobjs.c
// and of the builder and decoder records of
// include/freetype/internal/psaux.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auxiliary functions for PostScript fonts (body): the CFF and PS glyph
//! builders, the PS decoder wrapper and `cff_random`.
//!
//! A builder holds the glyph slot it builds into (with the CFF driver's
//! slot additions); the slot's glyph loader is C's `loader`, whose
//! `base` and `current` outlines `base` and `current` point to. The PS
//! builder wraps a CFF builder, whose `pos_x`, `pos_y`, `left_bearing`,
//! `advance` and `bbox` C's PS builder points to. The face data the
//! builders and the Adobe engine read through `builder->face` (and the
//! face's driver and size) are copied into [`CffBuilderFace`].
//!
//! Not translated yet (only the Type 1, CID and Type 42 drivers use them):
//! the PostScript table (`ps_table_*`) and parser (`ps_parser_*`), the
//! Type 1 builder (`t1_builder_*`), `t1_make_subfont` and `t1_decrypt`.

use std::sync::Arc;

use super::super::base::ftgloadr::FtGlyphLoaderRec;
use super::super::base::ftobjs::FtGlyphSlotRec;
use super::super::base::ftstream::FtStreamRec;
use super::super::cfftypes::*;
use super::super::fttypes::*;
use super::cffdecode::CffDecoder;
use super::PsDriverRec;

/// The face data a builder reads through C's `builder->face`.
#[derive(Debug, Clone, Default)]
#[allow(non_snake_case)]
pub struct CffBuilderFace {
    /// `face->units_per_EM`
    pub units_per_EM: FtUShort,
    /// `face->size->metrics.y_ppem`
    pub y_ppem: FtUShort,
    /// `face->internal->no_stem_darkening`
    pub no_stem_darkening: FtChar,
    /// `((TT_Face)face)->is_cff2`
    pub is_cff2: bool,
    /// the face's driver (`FT_FACE_DRIVER( face )`, a `PS_Driver`)
    pub driver: PsDriverRec,
    /// the face's normalized design coordinates (`mm->get_var_blend`):
    /// their number and the coordinates (`None` without a blend)
    pub num_coords: FtUInt,
    pub normalizedcoords: Option<Vec<FtFixed>>,
}

/// A CFF glyph slot (`CFF_GlyphSlot`): the root slot and the CFF driver's
/// additions.
#[derive(Debug)]
pub struct CffGlyph<'a> {
    pub root: &'a mut FtGlyphSlotRec,
    pub cff: &'a mut CffGlyphSlotRec,
}

/// `CFF_Builder`: a structure used during glyph loading to store its
/// outline.
///
/// * `face`: the current face object (the data read from it).
/// * `glyph`: the current glyph slot.
/// * `pos_x`: the horizontal translation (if composite glyph).
/// * `pos_y`: the vertical translation (if composite glyph).
/// * `left_bearing`: the left side bearing point.
/// * `advance`: the horizontal advance vector.
/// * `bbox`: unused.
/// * `path_begun`: a flag which indicates that a new path has begun.
/// * `load_points`: if this flag is not set, no points are loaded.
/// * `no_recurse`: set but not used.
/// * `metrics_only`: a boolean indicating that we only want to compute the
///   metrics of a given glyph, not load all of its points.
/// * `hints_funcs`: auxiliary pointer for hinting (set: the PS hinter's
///   Type 2 hints functions).
/// * `hints_globals`: auxiliary pointer for hinting (the PS hinter globals
///   of the top font or a subfont).
#[derive(Debug)]
pub struct CffBuilder<'a> {
    pub face: CffBuilderFace,
    pub glyph: Option<CffGlyph<'a>>,

    pub pos_x: FtPos,
    pub pos_y: FtPos,

    pub left_bearing: FtVector,
    pub advance: FtVector,

    pub bbox: FtBBox, /* bounding box */
    pub path_begun: bool,
    pub load_points: bool,
    pub no_recurse: bool,

    pub metrics_only: bool,

    pub hints_funcs: bool,                   /* hinter-specific */
    pub hints_globals: Option<CffSubFontId>, /* hinter-specific */
}

impl<'a> CffBuilder<'a> {
    /// The glyph's loader (`builder->loader`).
    pub fn loader(&mut self) -> Option<&mut FtGlyphLoaderRec> {
        self.glyph
            .as_mut()
            .and_then(|g| g.root.internal.loader.as_mut())
    }
}

/// `cff_builder_init`: initializes a given glyph builder.
///
/// * `face`: the current face object.
/// * `size`: whether there is a size, and whether its
///   `internal->module_data` (`CFF_Internal`) is set.
/// * `glyph`: the current glyph object.
/// * `hinting`: whether hinting is active.
pub fn cff_builder_init<'a>(
    face: CffBuilderFace,
    size: Option<bool>,
    glyph: Option<CffGlyph<'a>>,
    hinting: bool,
) -> CffBuilder<'a> {
    let mut builder = CffBuilder {
        face,
        glyph,
        pos_x: 0,
        pos_y: 0,
        left_bearing: FtVector::default(),
        advance: FtVector::default(),
        bbox: FtBBox::default(),
        path_begun: false,
        load_points: true,
        no_recurse: false,
        metrics_only: false,
        hints_funcs: false,
        hints_globals: None,
    };

    if let Some(glyph) = builder.glyph.as_mut() {
        let glyph_hints = glyph.root.internal.glyph_hints.is_some();
        let loader = glyph
            .root
            .internal
            .loader
            .get_or_insert_with(FtGlyphLoaderRec::new);

        loader.rewind();

        builder.hints_globals = None;
        builder.hints_funcs = false;

        if hinting {
            if let Some(internal) = size {
                if internal {
                    builder.hints_globals = Some(CffSubFontId::Top);
                    builder.hints_funcs = glyph_hints;
                }
            }
        }
    }

    builder.pos_x = 0;
    builder.pos_y = 0;

    builder.left_bearing.x = 0;
    builder.left_bearing.y = 0;
    builder.advance.x = 0;
    builder.advance.y = 0;

    builder
}

/// Copies the loader's base outline into the glyph slot (C's structure
/// copy `glyph->root.outline = *builder->base`).
fn copy_base_outline(glyph: &mut CffGlyph<'_>) {
    let Some(loader) = glyph.root.internal.loader.as_ref() else {
        return;
    };
    let base = &loader.base.outline;
    let n = (base.n_points.max(0) as usize).min(base.points.len());
    let nc = (base.n_contours.max(0) as usize).min(base.contours.len());

    glyph.root.outline = FtOutline {
        n_contours: base.n_contours,
        n_points: base.n_points,
        points: base.points[..n].to_vec(),
        tags: base.tags[..n].to_vec(),
        contours: base.contours[..nc].to_vec(),
        flags: base.flags,
    };
}

/// `cff_builder_done`: finalizes a given glyph builder.  Its contents can
/// still be used after the call, but the function saves important
/// information within the corresponding glyph slot.
pub fn cff_builder_done(builder: &mut CffBuilder<'_>) {
    if let Some(glyph) = builder.glyph.as_mut() {
        copy_base_outline(glyph);
    }
}

/// `cff_check_points`: check that there is enough space for `count' more
/// points
pub fn cff_check_points(builder: &mut CffBuilder<'_>, count: FtInt) -> FtResult<()> {
    match builder.loader() {
        Some(loader) => loader.check_points_macro(count as FtUInt, 0),
        None => Ok(()),
    }
}

/// `cff_builder_add_point`: add a new point, do not check space
pub fn cff_builder_add_point(builder: &mut CffBuilder<'_>, x: FtPos, y: FtPos, flag: FtByte) {
    let load_points = builder.load_points;
    let Some(loader) = builder.loader() else {
        return;
    };

    if load_points {
        let n = loader.current.n_points as u16 as usize;

        /* cf2_decoder_parse_charstrings uses 16.16 coordinates */
        if let Some(point) = loader.current_points().get_mut(n) {
            point.x = x >> 10;
            point.y = y >> 10;
        }
        if let Some(control) = loader.current_tags().get_mut(n) {
            *control = if flag != 0 {
                FT_CURVE_TAG_ON
            } else {
                FT_CURVE_TAG_CUBIC
            };
        }
    }
    loader.current.n_points = loader.current.n_points.wrapping_add(1);
}

/// `cff_builder_add_point1`: check space for a new on-curve point, then
/// add it
pub fn cff_builder_add_point1(builder: &mut CffBuilder<'_>, x: FtPos, y: FtPos) -> FtResult<()> {
    cff_check_points(builder, 1)?;
    cff_builder_add_point(builder, x, y, 1);
    Ok(())
}

/// `cff_builder_add_contour`: check space for a new contour, then add it
pub fn cff_builder_add_contour(builder: &mut CffBuilder<'_>) -> FtResult<()> {
    let load_points = builder.load_points;
    let Some(loader) = builder.loader() else {
        return Ok(());
    };

    if !load_points {
        loader.current.n_contours = loader.current.n_contours.wrapping_add(1);
        return Ok(());
    }

    loader.check_points_macro(0, 1)?;

    if loader.current.n_contours > 0 {
        let c = loader.current.n_contours as usize - 1;
        let v = (loader.current.n_points as i32 - 1) as i16;
        loader.current_contours()[c] = v;
    }
    loader.current.n_contours = loader.current.n_contours.wrapping_add(1);

    Ok(())
}

/// `cff_builder_start_point`: if a path was begun, add its first on-curve
/// point
pub fn cff_builder_start_point(builder: &mut CffBuilder<'_>, x: FtPos, y: FtPos) -> FtResult<()> {
    /* test whether we are building a new contour */
    if !builder.path_begun {
        builder.path_begun = true;
        cff_builder_add_contour(builder)?;
        cff_builder_add_point1(builder, x, y)?;
    }

    Ok(())
}

/// The body of `cff_builder_close_contour` and `ps_builder_close_contour`
/// (they are the same) on a loader's current outline.
fn close_contour(loader: &mut FtGlyphLoaderRec) {
    let n_contours = loader.current.n_contours as i32;
    let n_points = loader.current.n_points as i32;

    let first: i32 = if n_contours <= 1 {
        0
    } else {
        loader.current_contours()[n_contours as usize - 2] as i32 + 1
    };

    /* in malformed fonts it can happen that a contour was started */
    /* but no points were added                                    */
    if n_contours != 0 && first == n_points {
        loader.current.n_contours -= 1;
        return;
    }

    /* We must not include the last point in the path if it */
    /* is located on the first point.                       */
    if n_points > 1 {
        let points = loader.current_points();
        let p1 = points
            .get(first.max(0) as usize)
            .copied()
            .unwrap_or_default();
        let p2 = points[n_points as usize - 1];
        let control = loader.current_tags()[n_points as usize - 1];

        /* `delete' last point only if it coincides with the first */
        /* point and it is not a control point (which can happen). */
        if p1.x == p2.x && p1.y == p2.y && control == FT_CURVE_TAG_ON {
            loader.current.n_points -= 1;
        }
    }

    if loader.current.n_contours > 0 {
        let n_points = loader.current.n_points as i32;

        /* Don't add contours only consisting of one point, i.e.,  */
        /* check whether the first and the last point is the same. */
        if first == n_points - 1 {
            loader.current.n_contours -= 1;
            loader.current.n_points -= 1;
        } else {
            let c = loader.current.n_contours as usize - 1;
            loader.current_contours()[c] = (n_points - 1) as i16;
        }
    }
}

/// `cff_builder_close_contour`: close the current contour
pub fn cff_builder_close_contour(builder: &mut CffBuilder<'_>) {
    if let Some(loader) = builder.loader() {
        close_contour(loader);
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                            PS BUILDER                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

/// `PS_Builder`: a structure used during glyph loading to store its
/// outline (a wrapper around the CFF builder `builder`, whose glyph,
/// loader, positions, bearing, advance and bounding box it uses).
///
/// * `path_begun`: a flag which indicates that a new path has begun.
/// * `load_points`: if this flag is not set, no points are loaded.
/// * `no_recurse`: set but not used.
/// * `metrics_only`: a boolean indicating that we only want to compute the
///   metrics of a given glyph, not load all of its points.
/// * `is_t1`: set if current font type is Type 1.
#[derive(Debug)]
pub struct PsBuilder<'a, 'b> {
    pub builder: &'a mut CffBuilder<'b>,

    pub path_begun: bool,
    pub load_points: bool,
    pub no_recurse: bool,

    pub metrics_only: bool,
    pub is_t1: bool,
}

/// `ps_builder_init`: initializes a given glyph builder (the CFF builder
/// wrapper; the Type 1 one is not translated yet).
pub fn ps_builder_init<'a, 'b>(
    cffbuilder: &'a mut CffBuilder<'b>,
    is_t1: bool,
) -> PsBuilder<'a, 'b> {
    let path_begun = cffbuilder.path_begun;
    let load_points = cffbuilder.load_points;
    let no_recurse = cffbuilder.no_recurse;
    let metrics_only = cffbuilder.metrics_only;

    PsBuilder {
        builder: cffbuilder,
        path_begun,
        load_points,
        no_recurse,
        metrics_only,
        is_t1,
    }
}

/// `ps_builder_done`: finalizes a given glyph builder.  Its contents can
/// still be used after the call, but the function saves important
/// information within the corresponding glyph slot.
pub fn ps_builder_done(builder: &mut PsBuilder<'_, '_>) {
    if let Some(glyph) = builder.builder.glyph.as_mut() {
        copy_base_outline(glyph);
    }
}

/// `ps_builder_check_points`: check that there is enough space for
/// `count' more points
pub fn ps_builder_check_points(builder: &mut PsBuilder<'_, '_>, count: FtInt) -> FtResult<()> {
    match builder.builder.loader() {
        Some(loader) => loader.check_points_macro(count as FtUInt, 0),
        None => Ok(()),
    }
}

/// `ps_builder_add_point`: add a new point, do not check space
pub fn ps_builder_add_point(builder: &mut PsBuilder<'_, '_>, x: FtPos, y: FtPos, flag: FtByte) {
    let load_points = builder.load_points;
    let Some(loader) = builder.builder.loader() else {
        return;
    };

    if load_points {
        let n = loader.current.n_points as u16 as usize;

        /* (the old engines are not compiled) */
        /* cf2_decoder_parse_charstrings uses 16.16 coordinates */
        if let Some(point) = loader.current_points().get_mut(n) {
            point.x = x >> 10;
            point.y = y >> 10;
        }
        if let Some(control) = loader.current_tags().get_mut(n) {
            *control = if flag != 0 {
                FT_CURVE_TAG_ON
            } else {
                FT_CURVE_TAG_CUBIC
            };
        }
    }
    loader.current.n_points = loader.current.n_points.wrapping_add(1);
}

/// `ps_builder_add_point1`: check space for a new on-curve point, then add
/// it
pub fn ps_builder_add_point1(builder: &mut PsBuilder<'_, '_>, x: FtPos, y: FtPos) -> FtResult<()> {
    ps_builder_check_points(builder, 1)?;
    ps_builder_add_point(builder, x, y, 1);
    Ok(())
}

/// `ps_builder_add_contour`: check space for a new contour, then add it
pub fn ps_builder_add_contour(builder: &mut PsBuilder<'_, '_>) -> FtResult<()> {
    let load_points = builder.load_points;

    /* this might happen in invalid fonts */
    let Some(loader) = builder.builder.loader() else {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    };

    if !load_points {
        loader.current.n_contours = loader.current.n_contours.wrapping_add(1);
        return Ok(());
    }

    loader.check_points_macro(0, 1)?;

    if loader.current.n_contours > 0 {
        let c = loader.current.n_contours as usize - 1;
        let v = (loader.current.n_points as i32 - 1) as i16;
        loader.current_contours()[c] = v;
    }
    loader.current.n_contours = loader.current.n_contours.wrapping_add(1);

    Ok(())
}

/// `ps_builder_start_point`: if a path was begun, add its first on-curve
/// point
pub fn ps_builder_start_point(builder: &mut PsBuilder<'_, '_>, x: FtPos, y: FtPos) -> FtResult<()> {
    /* test whether we are building a new contour */
    if !builder.path_begun {
        builder.path_begun = true;
        ps_builder_add_contour(builder)?;
        ps_builder_add_point1(builder, x, y)?;
    }

    Ok(())
}

/// `ps_builder_close_contour`: close the current contour
pub fn ps_builder_close_contour(builder: &mut PsBuilder<'_, '_>) {
    if let Some(loader) = builder.builder.loader() {
        close_contour(loader);
    }
}

/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                            PS DECODER                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/

pub const PS_MAX_OPERANDS: usize = 48;
pub const PS_MAX_SUBRS_CALLS: usize = 16; /* maximum subroutine nesting;         */
/* only 10 are allowed but there exist */
/* fonts like `HiraKakuProN-W3.ttf'    */
/* (Hiragino Kaku Gothic ProN W3;      */
/* 8.2d6e1; 2014-12-19) that exceed    */
/* this limit                          */

/// `PS_Decoder` (in CFF mode: the decoder wraps a CFF decoder, whose font,
/// stream and glyph width it uses; the Type 1 fields are left out with the
/// Type 1 mode).
///
/// The subroutine tables are offsets into their index's bytes
/// (`locals_bytes`, `globals_bytes`).
#[derive(Debug)]
pub struct PsDecoder<'a, 'b> {
    pub builder: PsBuilder<'a, 'b>,

    pub flex_state: FtInt,
    pub num_flex_vectors: FtInt,
    pub flex_vectors: [FtVector; 7],

    pub cff: &'a mut CffFontRec,
    pub current_subfont: CffSubFontId, /* for current glyph_index */
    /// the face's stream (for the charstrings of seac components)
    pub stream: &'a mut FtStreamRec,

    pub glyph_width: &'a mut FtPos,
    pub width_only: bool,
    pub num_hints: FtInt,

    pub num_locals: FtUInt,
    pub num_globals: FtUInt,

    pub locals_bias: FtInt,
    pub globals_bias: FtInt,

    pub locals: Option<Arc<[usize]>>,
    pub locals_bytes: Option<Arc<[u8]>>,
    pub globals: Option<Arc<[usize]>>,
    pub globals_bytes: Option<Arc<[u8]>>,

    pub num_glyphs: FtUInt, /* number of glyphs in font */

    pub hint_mode: FtRenderMode,

    pub seac: bool,
}

/// `ps_decoder_init`: creates a wrapper decoder for use in the combined
/// Type 1 / CFF interpreter (the CFF decoder's; Type 1 decoders are not
/// translated yet).
pub fn ps_decoder_init<'a, 'b>(decoder: &'a mut CffDecoder<'b>, is_t1: bool) -> PsDecoder<'a, 'b> {
    let CffDecoder {
        builder,
        cff,
        stream,
        glyph_width,
        width_only,
        num_locals,
        num_globals,
        locals_bias,
        globals_bias,
        locals,
        locals_bytes,
        globals,
        globals_bytes,
        hint_mode,
        current_subfont,
        ..
    } = decoder;

    PsDecoder {
        builder: ps_builder_init(builder, is_t1),

        flex_state: 0,
        num_flex_vectors: 0,
        flex_vectors: [FtVector::default(); 7],

        cff,
        current_subfont: *current_subfont,
        stream,

        num_globals: *num_globals,
        globals: globals.clone(),
        globals_bytes: globals_bytes.clone(),
        globals_bias: *globals_bias,
        num_locals: *num_locals,
        locals: locals.clone(),
        locals_bytes: locals_bytes.clone(),
        locals_bias: *locals_bias,

        glyph_width,
        width_only: *width_only,
        num_hints: 0,

        num_glyphs: 0,

        hint_mode: *hint_mode,

        seac: false,
    }
}

/// `cff_random`
pub fn cff_random(mut r: FtUInt32) -> FtUInt32 {
    /* a 32bit version of the `xorshift' algorithm */
    r ^= r << 13;
    r ^= r >> 17;
    r ^= r << 5;

    r
}
