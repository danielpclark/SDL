// Rust translation of src/base/ftglyph.c (and include/freetype/ftglyph.h)
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType convenience functions to handle glyphs (body).
//!
//! This file contains the definition of several convenience functions
//! that can be used by client applications to easily retrieve glyph
//! bitmaps and outlines from a given face.
//!
//! These functions should be optional if you are writing a font server
//! or text layout engine on top of FreeType.  However, they are pretty
//! handy for many other simple uses of the library.
//!
//! Translation notes: an `FT_Glyph` is the [`FtGlyph`] enum of the three
//! glyph classes (bitmap, outline, SVG), and the class functions are
//! dispatched on it. None of the renderers SDL_ttf's build has a glyph
//! class of its own, so `FT_New_Glyph` knows only these three formats.

use super::super::fttypes::*;
use super::ftbitmap::{ft_bitmap_copy, ft_bitmap_done, ft_bitmap_init};
use super::ftcalc::{ft_matrix_multiply, ft_mul_fix};
use super::ftobjs::*;
use super::ftoutln::{
    ft_outline_copy, ft_outline_get_cbox, ft_outline_new, ft_outline_transform,
    ft_outline_translate, ft_vector_transform,
};

/* ftglyph.h */

/// `FT_GlyphRec`: The root glyph structure contains a given glyph image
/// plus its advance width in 16.16 fixed-point format.
#[derive(Debug, Clone)]
pub struct FtGlyphRec {
    pub library: FtLibrary,
    pub format: FtGlyphFormat,
    pub advance: FtVector,
}

/// `FT_BitmapGlyphRec`
#[derive(Debug, Clone)]
pub struct FtBitmapGlyphRec {
    pub root: FtGlyphRec,
    pub left: FtInt,
    pub top: FtInt,
    pub bitmap: FtBitmap,
}

/// `FT_OutlineGlyphRec`
#[derive(Debug, Clone)]
pub struct FtOutlineGlyphRec {
    pub root: FtGlyphRec,
    pub outline: FtOutline,
}

/// `FT_SvgGlyphRec`
#[derive(Debug, Clone)]
#[allow(non_snake_case)]
pub struct FtSvgGlyphRec {
    pub root: FtGlyphRec,

    pub svg_document: Vec<u8>,
    pub svg_document_length: FtULong,

    pub glyph_index: FtUInt,

    pub metrics: FtSizeMetrics,
    pub units_per_EM: FtUShort,

    pub start_glyph_id: FtUShort,
    pub end_glyph_id: FtUShort,

    pub transform: FtMatrix,
    pub delta: FtVector,
}

/// `FT_Glyph`: a glyph of one of the glyph classes (C's `clazz`)
#[derive(Debug, Clone)]
pub enum FtGlyph {
    /// `ft_bitmap_glyph_class`
    Bitmap(FtBitmapGlyphRec),
    /// `ft_outline_glyph_class`
    Outline(FtOutlineGlyphRec),
    /// `ft_svg_glyph_class`
    Svg(FtSvgGlyphRec),
}

impl FtGlyph {
    /// The `FT_GlyphRec` root.
    pub fn root(&self) -> &FtGlyphRec {
        match self {
            FtGlyph::Bitmap(g) => &g.root,
            FtGlyph::Outline(g) => &g.root,
            FtGlyph::Svg(g) => &g.root,
        }
    }

    /// The `FT_GlyphRec` root, mutably.
    pub fn root_mut(&mut self) -> &mut FtGlyphRec {
        match self {
            FtGlyph::Bitmap(g) => &mut g.root,
            FtGlyph::Outline(g) => &mut g.root,
            FtGlyph::Svg(g) => &mut g.root,
        }
    }
}

/// `FT_Glyph_BBox_Mode`
pub type FtGlyphBBoxMode = FtUInt;
pub const FT_GLYPH_BBOX_UNSCALED: FtGlyphBBoxMode = 0;
pub const FT_GLYPH_BBOX_SUBPIXELS: FtGlyphBBoxMode = 0;
pub const FT_GLYPH_BBOX_GRIDFIT: FtGlyphBBoxMode = 1;
pub const FT_GLYPH_BBOX_TRUNCATE: FtGlyphBBoxMode = 2;
pub const FT_GLYPH_BBOX_PIXELS: FtGlyphBBoxMode = 3;

/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****   FT_BitmapGlyph support                                        ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/

/// `ft_bitmap_glyph_init`
fn ft_bitmap_glyph_init(glyph: &mut FtBitmapGlyphRec, slot: &mut FtGlyphSlotRec) -> FtResult<()> {
    if slot.format != FT_GLYPH_FORMAT_BITMAP {
        return Err(FT_ERR_INVALID_GLYPH_FORMAT);
    }

    glyph.left = slot.bitmap_left;
    glyph.top = slot.bitmap_top;

    /* do lazy copying whenever possible */
    if slot.internal.flags & FT_GLYPH_OWN_BITMAP != 0 {
        glyph.bitmap = std::mem::take(&mut slot.bitmap);
        /* (the slot keeps the bitmap's description, as in C) */
        slot.bitmap.rows = glyph.bitmap.rows;
        slot.bitmap.width = glyph.bitmap.width;
        slot.bitmap.pitch = glyph.bitmap.pitch;
        slot.bitmap.num_grays = glyph.bitmap.num_grays;
        slot.bitmap.pixel_mode = glyph.bitmap.pixel_mode;
        slot.bitmap.palette_mode = glyph.bitmap.palette_mode;
        slot.internal.flags &= !FT_GLYPH_OWN_BITMAP;
        Ok(())
    } else {
        ft_bitmap_init(&mut glyph.bitmap);
        ft_bitmap_copy(&slot.bitmap, &mut glyph.bitmap)
    }
}

/// `ft_bitmap_glyph_copy`
fn ft_bitmap_glyph_copy(source: &FtBitmapGlyphRec, target: &mut FtBitmapGlyphRec) -> FtResult<()> {
    target.left = source.left;
    target.top = source.top;

    ft_bitmap_copy(&source.bitmap, &mut target.bitmap)
}

/// `ft_bitmap_glyph_done`
fn ft_bitmap_glyph_done(glyph: &mut FtBitmapGlyphRec) {
    ft_bitmap_done(&mut glyph.bitmap);
}

/// `ft_bitmap_glyph_bbox`
fn ft_bitmap_glyph_bbox(glyph: &FtBitmapGlyphRec) -> FtBBox {
    let mut cbox = FtBBox::default();

    cbox.xMin = glyph.left as FtPos * 64;
    cbox.xMax = cbox.xMin + (glyph.bitmap.width as FtPos * 64);
    cbox.yMax = glyph.top as FtPos * 64;
    cbox.yMin = cbox.yMax - (glyph.bitmap.rows as FtPos * 64);

    cbox
}

/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****   FT_OutlineGlyph support                                       ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/

/// `ft_outline_glyph_init`
fn ft_outline_glyph_init(glyph: &mut FtOutlineGlyphRec, slot: &mut FtGlyphSlotRec) -> FtResult<()> {
    let source = &slot.outline;

    /* check format in glyph slot */
    if slot.format != FT_GLYPH_FORMAT_OUTLINE {
        return Err(FT_ERR_INVALID_GLYPH_FORMAT);
    }

    /* allocate new outline */
    glyph.outline = ft_outline_new(source.n_points as FtUInt, source.n_contours as FtInt)?;

    let _ = ft_outline_copy(source, &mut glyph.outline);

    Ok(())
}

/// `ft_outline_glyph_done`
fn ft_outline_glyph_done(glyph: &mut FtOutlineGlyphRec) {
    glyph.outline = FtOutline::default();
}

/// `ft_outline_glyph_copy`
fn ft_outline_glyph_copy(
    source: &FtOutlineGlyphRec,
    target: &mut FtOutlineGlyphRec,
) -> FtResult<()> {
    target.outline = ft_outline_new(
        source.outline.n_points as FtUInt,
        source.outline.n_contours as FtInt,
    )?;

    let _ = ft_outline_copy(&source.outline, &mut target.outline);

    Ok(())
}

/// `ft_outline_glyph_transform`
fn ft_outline_glyph_transform(
    glyph: &mut FtOutlineGlyphRec,
    matrix: Option<&FtMatrix>,
    delta: Option<&FtVector>,
) {
    if let Some(matrix) = matrix {
        ft_outline_transform(&mut glyph.outline, matrix);
    }

    if let Some(delta) = delta {
        ft_outline_translate(&mut glyph.outline, delta.x, delta.y);
    }
}

/// `ft_outline_glyph_bbox`
fn ft_outline_glyph_bbox(glyph: &FtOutlineGlyphRec) -> FtBBox {
    ft_outline_get_cbox(&glyph.outline)
}

/// `ft_outline_glyph_prepare` (the slot borrows the outline, which C
/// shares without the `FT_OUTLINE_OWNER` flag; [`ft_glyph_to_bitmap`]
/// gives it back)
fn ft_outline_glyph_prepare(
    glyph: &mut FtOutlineGlyphRec,
    slot: &mut FtGlyphSlotRec,
) -> FtResult<()> {
    slot.format = FT_GLYPH_FORMAT_OUTLINE;
    slot.outline = std::mem::take(&mut glyph.outline);
    slot.outline.flags &= !FT_OUTLINE_OWNER;

    Ok(())
}

/* FT_CONFIG_OPTION_SVG */

/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****   FT_SvgGlyph support                                           ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/

/// `ft_svg_glyph_init`
fn ft_svg_glyph_init(glyph: &mut FtSvgGlyphRec, slot: &mut FtGlyphSlotRec) -> FtResult<()> {
    if slot.format != FT_GLYPH_FORMAT_SVG {
        return Err(FT_ERR_INVALID_GLYPH_FORMAT);
    }

    let Some(document) = slot.other.as_ref() else {
        return Err(FT_ERR_INVALID_SLOT_HANDLE);
    };

    if document.svg_document_length == 0 {
        return Err(FT_ERR_INVALID_SLOT_HANDLE);
    }

    /* allocate a new document */
    let doc_length = document.svg_document_length;
    let mut svg_document = super::ftmemory::ft_qalloc(doc_length as FtLong)?;
    glyph.svg_document_length = doc_length;

    glyph.glyph_index = slot.glyph_index;

    glyph.metrics = document.metrics;
    glyph.units_per_EM = document.units_per_EM;

    glyph.start_glyph_id = document.start_glyph_id;
    glyph.end_glyph_id = document.end_glyph_id;

    glyph.transform = document.transform;
    glyph.delta = document.delta;

    /* copy the document into glyph */
    svg_document.copy_from_slice(&document.svg_document[..doc_length as usize]);
    glyph.svg_document = svg_document;

    Ok(())
}

/// `ft_svg_glyph_done`
fn ft_svg_glyph_done(glyph: &mut FtSvgGlyphRec) {
    /* just free the memory */
    glyph.svg_document = Vec::new();
}

/// `ft_svg_glyph_copy`
fn ft_svg_glyph_copy(source: &FtSvgGlyphRec, target: &mut FtSvgGlyphRec) -> FtResult<()> {
    if source.root.format != FT_GLYPH_FORMAT_SVG {
        return Err(FT_ERR_INVALID_GLYPH_FORMAT);
    }

    if source.svg_document_length == 0 {
        return Err(FT_ERR_INVALID_SLOT_HANDLE);
    }

    target.glyph_index = source.glyph_index;

    target.svg_document_length = source.svg_document_length;

    target.metrics = source.metrics;
    target.units_per_EM = source.units_per_EM;

    target.start_glyph_id = source.start_glyph_id;
    target.end_glyph_id = source.end_glyph_id;

    target.transform = source.transform;
    target.delta = source.delta;

    /* allocate space for the SVG document */
    let mut doc = super::ftmemory::ft_qalloc(target.svg_document_length as FtLong)?;

    /* copy the document */
    doc.copy_from_slice(&source.svg_document[..target.svg_document_length as usize]);
    target.svg_document = doc;

    Ok(())
}

/// `ft_svg_glyph_transform`
fn ft_svg_glyph_transform(
    glyph: &mut FtSvgGlyphRec,
    matrix: Option<&FtMatrix>,
    delta: Option<&FtVector>,
) {
    let tmp_matrix = FtMatrix {
        xx: 0x10000,
        xy: 0,
        yx: 0,
        yy: 0x10000,
    };
    let tmp_delta = FtVector { x: 0, y: 0 };

    let matrix = matrix.unwrap_or(&tmp_matrix);
    let delta = delta.unwrap_or(&tmp_delta);

    let mut a = glyph.transform;
    let b = *matrix;
    ft_matrix_multiply(&b, &mut a);

    let x = ft_mul_fix(matrix.xx, glyph.delta.x)
        .wrapping_add(ft_mul_fix(matrix.xy, glyph.delta.y))
        .wrapping_add(delta.x);
    let y = ft_mul_fix(matrix.yx, glyph.delta.x)
        .wrapping_add(ft_mul_fix(matrix.yy, glyph.delta.y))
        .wrapping_add(delta.y);

    glyph.delta.x = x;
    glyph.delta.y = y;

    glyph.transform = a;
}

/// `ft_svg_glyph_prepare`
fn ft_svg_glyph_prepare(glyph: &FtSvgGlyphRec, slot: &mut FtGlyphSlotRec) -> FtResult<()> {
    let document = FtSvgDocumentRec {
        svg_document: glyph.svg_document.clone(),
        svg_document_length: glyph.svg_document_length,

        metrics: glyph.metrics,
        units_per_EM: glyph.units_per_EM,

        start_glyph_id: glyph.start_glyph_id,
        end_glyph_id: glyph.end_glyph_id,

        transform: glyph.transform,
        delta: glyph.delta,
    };

    slot.format = FT_GLYPH_FORMAT_SVG;
    slot.glyph_index = glyph.glyph_index;
    slot.other = Some(Box::new(document));

    Ok(())
}

/*************************************************************************/
/*************************************************************************/
/****                                                                 ****/
/****   FT_Glyph class and API                                        ****/
/****                                                                 ****/
/*************************************************************************/
/*************************************************************************/

/// `ft_new_glyph`
fn ft_new_glyph(library: &FtLibrary, format: FtGlyphFormat) -> FtGlyph {
    let root = FtGlyphRec {
        library: library.clone(),
        format,
        advance: FtVector::default(),
    };

    match format {
        FT_GLYPH_FORMAT_BITMAP => FtGlyph::Bitmap(FtBitmapGlyphRec {
            root,
            left: 0,
            top: 0,
            bitmap: FtBitmap::default(),
        }),
        FT_GLYPH_FORMAT_OUTLINE => FtGlyph::Outline(FtOutlineGlyphRec {
            root,
            outline: FtOutline::default(),
        }),
        _ => FtGlyph::Svg(FtSvgGlyphRec {
            root,
            svg_document: Vec::new(),
            svg_document_length: 0,
            glyph_index: 0,
            metrics: FtSizeMetrics::default(),
            units_per_EM: 0,
            start_glyph_id: 0,
            end_glyph_id: 0,
            transform: FtMatrix::default(),
            delta: FtVector::default(),
        }),
    }
}

/// `FT_Glyph_Copy`
pub fn ft_glyph_copy(source: &FtGlyph) -> FtResult<FtGlyph> {
    let mut copy = ft_new_glyph(&source.root().library, source.root().format);

    copy.root_mut().advance = source.root().advance;
    copy.root_mut().format = source.root().format;

    let error = match (source, &mut copy) {
        (FtGlyph::Bitmap(s), FtGlyph::Bitmap(t)) => ft_bitmap_glyph_copy(s, t),
        (FtGlyph::Outline(s), FtGlyph::Outline(t)) => ft_outline_glyph_copy(s, t),
        (FtGlyph::Svg(s), FtGlyph::Svg(t)) => ft_svg_glyph_copy(s, t),
        _ => Ok(()),
    };

    match error {
        Err(e) => {
            ft_done_glyph(copy);
            Err(e)
        }
        Ok(()) => Ok(copy),
    }
}

/// `FT_New_Glyph`
pub fn ft_new_glyph_public(library: &FtLibrary, format: FtGlyphFormat) -> FtResult<FtGlyph> {
    /* if it is a bitmap, that's easy :-) */
    /* if it is an outline */
    /* if it is an SVG glyph */
    if format == FT_GLYPH_FORMAT_BITMAP
        || format == FT_GLYPH_FORMAT_OUTLINE
        || format == FT_GLYPH_FORMAT_SVG
    {
        /* create FT_Glyph object */
        return Ok(ft_new_glyph(library, format));
    }

    /* try to find a renderer that supports the glyph image format */
    /* (see the module documentation: none has a glyph class) */
    Err(FT_ERR_INVALID_GLYPH_FORMAT)
}

/// `FT_Get_Glyph`
pub fn ft_get_glyph(slot: &mut FtGlyphSlotRec) -> FtResult<FtGlyph> {
    let library = slot.library.clone().ok_or(FT_ERR_INVALID_SLOT_HANDLE)?;

    /* create FT_Glyph object */
    let mut glyph = ft_new_glyph_public(&library, slot.format)?;

    let error: FtResult<()> = 'exit2: {
        /* copy advance while converting 26.6 to 16.16 format */
        if slot.advance.x >= 0x8000 * 64 || slot.advance.x <= -0x8000 * 64 {
            break 'exit2 Err(FT_ERR_INVALID_ARGUMENT);
        }
        if slot.advance.y >= 0x8000 * 64 || slot.advance.y <= -0x8000 * 64 {
            break 'exit2 Err(FT_ERR_INVALID_ARGUMENT);
        }

        glyph.root_mut().advance.x = slot.advance.x * 1024;
        glyph.root_mut().advance.y = slot.advance.y * 1024;

        /* now import the image from the glyph slot */
        match &mut glyph {
            FtGlyph::Bitmap(g) => ft_bitmap_glyph_init(g, slot),
            FtGlyph::Outline(g) => ft_outline_glyph_init(g, slot),
            FtGlyph::Svg(g) => ft_svg_glyph_init(g, slot),
        }
    };

    /* Exit2: */
    /* if an error occurred, destroy the glyph */
    match error {
        Err(e) => {
            ft_done_glyph(glyph);
            Err(e)
        }
        Ok(()) => Ok(glyph),
    }
}

/// `FT_Glyph_Transform`
pub fn ft_glyph_transform(
    glyph: &mut FtGlyph,
    matrix: Option<&FtMatrix>,
    delta: Option<&FtVector>,
) -> FtResult<()> {
    /* transform glyph image */
    match glyph {
        FtGlyph::Bitmap(_) => return Err(FT_ERR_INVALID_GLYPH_FORMAT),
        FtGlyph::Outline(g) => ft_outline_glyph_transform(g, matrix, delta),
        FtGlyph::Svg(g) => ft_svg_glyph_transform(g, matrix, delta),
    }

    /* transform advance vector */
    if let Some(matrix) = matrix {
        ft_vector_transform(&mut glyph.root_mut().advance, matrix);
    }

    Ok(())
}

/// `FT_Glyph_Get_CBox`
pub fn ft_glyph_get_cbox(glyph: &FtGlyph, bbox_mode: FtGlyphBBoxMode) -> FtBBox {
    /* retrieve bbox in 26.6 coordinates */
    let mut acbox = match glyph {
        FtGlyph::Bitmap(g) => ft_bitmap_glyph_bbox(g),
        FtGlyph::Outline(g) => ft_outline_glyph_bbox(g),
        FtGlyph::Svg(_) => return FtBBox::default(),
    };

    /* perform grid fitting if needed */
    if bbox_mode == FT_GLYPH_BBOX_GRIDFIT || bbox_mode == FT_GLYPH_BBOX_PIXELS {
        acbox.xMin = ft_pix_floor(acbox.xMin);
        acbox.yMin = ft_pix_floor(acbox.yMin);
        acbox.xMax = ft_pix_ceil_long(acbox.xMax);
        acbox.yMax = ft_pix_ceil_long(acbox.yMax);
    }

    /* convert to integer pixels if needed */
    if bbox_mode == FT_GLYPH_BBOX_TRUNCATE || bbox_mode == FT_GLYPH_BBOX_PIXELS {
        acbox.xMin >>= 6;
        acbox.yMin >>= 6;
        acbox.xMax >>= 6;
        acbox.yMax >>= 6;
    }

    acbox
}

/// `FT_Glyph_To_Bitmap`: on success `the_glyph` becomes the bitmap glyph,
/// and the old glyph is returned unless `destroy` is set; on failure the
/// glyph is left in `the_glyph`.
pub fn ft_glyph_to_bitmap(
    the_glyph: &mut FtGlyph,
    render_mode: FtRenderMode,
    origin: Option<&FtVector>,
    destroy: bool,
) -> FtResult<Option<FtGlyph>> {
    /* when called with a bitmap glyph, do nothing and return successfully */
    if matches!(the_glyph, FtGlyph::Bitmap(_)) {
        return Ok(None);
    }

    let library = the_glyph.root().library.clone();

    /* we render the glyph into a glyph bitmap using a `dummy' glyph slot */
    /* then calling FT_Render_Glyph_Internal()                            */

    let mut dummy = FtGlyphSlotRec {
        library: Some(library.clone()),
        format: the_glyph.root().format,
        ..Default::default()
    };

    /* create result bitmap glyph */
    let FtGlyph::Bitmap(mut bitmap) = ft_new_glyph(&library, FT_GLYPH_FORMAT_BITMAP) else {
        unreachable!()
    };

    /* if `origin' is set, translate the glyph image */
    if let Some(origin) = origin {
        let _ = ft_glyph_transform(the_glyph, None, Some(origin));
    }

    /* prepare dummy slot for rendering */
    let outline_flags = match the_glyph {
        FtGlyph::Outline(g) => g.outline.flags,
        _ => 0,
    };
    let mut error = match the_glyph {
        FtGlyph::Outline(g) => ft_outline_glyph_prepare(g, &mut dummy),
        FtGlyph::Svg(g) => ft_svg_glyph_prepare(g, &mut dummy),
        FtGlyph::Bitmap(_) => Ok(()),
    };
    if error.is_ok() {
        error = ft_render_glyph_internal(&library, &mut dummy, render_mode);
    }

    /* (give the outline back) */
    if let FtGlyph::Outline(g) = the_glyph {
        g.outline = std::mem::take(&mut dummy.outline);
        g.outline.flags = outline_flags;
    }

    /* FT_CONFIG_OPTION_SVG */
    if let FtGlyph::Svg(_) = the_glyph {
        dummy.other = None;
    }

    if !destroy {
        if let Some(origin) = origin {
            let v = FtVector {
                x: -origin.x,
                y: -origin.y,
            };
            let _ = ft_glyph_transform(the_glyph, None, Some(&v));
        }
    }

    error?;

    /* in case of success, copy the bitmap to the glyph bitmap */
    if let Err(e) = ft_bitmap_glyph_init(&mut bitmap, &mut dummy) {
        ft_done_glyph(FtGlyph::Bitmap(bitmap));
        return Err(e);
    }

    /* copy advance */
    bitmap.root.advance = the_glyph.root().advance;

    let old = std::mem::replace(the_glyph, FtGlyph::Bitmap(bitmap));
    if destroy {
        ft_done_glyph(old);
        return Ok(None);
    }

    Ok(Some(old))
}

/// `FT_Done_Glyph`
pub fn ft_done_glyph(mut glyph: FtGlyph) {
    match &mut glyph {
        FtGlyph::Bitmap(g) => ft_bitmap_glyph_done(g),
        FtGlyph::Outline(g) => ft_outline_glyph_done(g),
        FtGlyph::Svg(g) => ft_svg_glyph_done(g),
    }
}
