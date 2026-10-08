// Rust translation of src/raster/ftrend1.c (and ftrend1.h) from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The FreeType glyph rasterizer interface (body).

use super::super::base::ftmemory::ft_alloc_mult;
use super::super::base::ftobjs::*;
use super::super::base::ftoutln::{
    ft_outline_get_cbox, ft_outline_transform, ft_outline_translate,
};
use super::super::ftimage::FtRasterParams;
use super::super::fttypes::*;
use super::ftraster::FT_STANDARD_RASTER;

/// `ft_raster1_init`: initialize renderer -- init its raster
fn ft_raster1_init(_library: &mut FtLibraryRec, _module: usize) -> FtResult<()> {
    /* (`ft_black_reset' has nothing to do) */
    Ok(())
}

/// `ft_raster1_set_mode`: set render-specific mode
fn ft_raster1_set_mode(_render: &FtModuleRec, _mode_tag: FtULong) -> FtResult<()> {
    /* we simply pass it to the raster */
    /* (`ft_black_set_mode' has nothing to do) */
    Ok(())
}

/// `ft_raster1_transform`: transform a given glyph image
fn ft_raster1_transform(
    slot: &mut FtGlyphSlotRec,
    matrix: Option<&FtMatrix>,
    delta: Option<&FtVector>,
) -> FtResult<()> {
    if slot.format != FT_GLYPH_FORMAT_OUTLINE {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    if let Some(matrix) = matrix {
        ft_outline_transform(&mut slot.outline, matrix);
    }

    if let Some(delta) = delta {
        ft_outline_translate(&mut slot.outline, delta.x, delta.y);
    }

    Ok(())
}

/// `ft_raster1_get_cbox`: return the glyph's control box
fn ft_raster1_get_cbox(slot: &FtGlyphSlotRec) -> FtBBox {
    if slot.format == FT_GLYPH_FORMAT_OUTLINE {
        ft_outline_get_cbox(&slot.outline)
    } else {
        FtBBox::default()
    }
}

/// `ft_raster1_render`: convert a slot's glyph image into a bitmap
fn ft_raster1_render(
    library: &FtLibraryRec,
    render: usize,
    slot: &mut FtGlyphSlotRec,
    mode: FtRenderMode,
    origin: Option<&FtVector>,
) -> FtResult<()> {
    let mut x_shift: FtPos = 0;
    let mut y_shift: FtPos = 0;

    let error: FtResult<()> = 'exit: {
        /* check glyph image format */
        if slot.format != FT_GLYPH_FORMAT_OUTLINE {
            break 'exit Err(FT_ERR_INVALID_ARGUMENT);
        }

        /* check rendering mode */
        if mode != FT_RENDER_MODE_MONO {
            /* raster1 is only capable of producing monochrome bitmaps */
            return Err(FT_ERR_CANNOT_RENDER_GLYPH);
        }

        /* release old bitmap buffer */
        if slot.internal.flags & FT_GLYPH_OWN_BITMAP != 0 {
            slot.bitmap.buffer = Vec::new();
            slot.internal.flags &= !FT_GLYPH_OWN_BITMAP;
        }

        if ft_glyphslot_preset_bitmap(slot, mode, origin) {
            break 'exit Err(FT_ERR_RASTER_OVERFLOW);
        }

        /* allocate new one */
        match ft_alloc_mult(slot.bitmap.rows as FtLong, slot.bitmap.pitch as FtLong) {
            Ok(b) => slot.bitmap.buffer = b,
            Err(e) => break 'exit Err(e),
        }

        slot.internal.flags |= FT_GLYPH_OWN_BITMAP;

        x_shift = -(slot.bitmap_left as FtPos) * 64;
        y_shift = (slot.bitmap.rows as FtInt as FtPos - slot.bitmap_top as FtPos) * 64;

        if let Some(origin) = origin {
            x_shift += origin.x;
            y_shift += origin.y;
        }

        /* translate outline to render it into the bitmap */
        if x_shift != 0 || y_shift != 0 {
            ft_outline_translate(&mut slot.outline, x_shift, y_shift);
        }

        /* set up parameters */
        let mut params = FtRasterParams::new(Some(&mut slot.bitmap), FT_RASTER_FLAG_DEFAULT);

        /* render outline into the bitmap */
        library.renderer_raster_render(render, &slot.outline, &mut params)
    };

    /* Exit: */
    if error.is_ok() {
        /* everything is fine; the glyph is now officially a bitmap */
        slot.format = FT_GLYPH_FORMAT_BITMAP;
    } else if slot.internal.flags & FT_GLYPH_OWN_BITMAP != 0 {
        slot.bitmap.buffer = Vec::new();
        slot.internal.flags &= !FT_GLYPH_OWN_BITMAP;
    }

    if x_shift != 0 || y_shift != 0 {
        ft_outline_translate(&mut slot.outline, -x_shift, -y_shift);
    }

    error
}

/// `ft_raster1_renderer_class`
pub static FT_RASTER1_RENDERER_CLASS: FtRendererClass = FtRendererClass {
    root: FtModuleClass {
        module_flags: FT_MODULE_RENDERER,

        module_name: "raster1",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None, /* module specific interface */

        module_init: Some(ft_raster1_init), /* FT_Module_Constructor module_init   */
        module_done: None,                  /* FT_Module_Destructor  module_done   */
        get_interface: None,                /* FT_Module_Requester   get_interface */
    },

    glyph_format: FT_GLYPH_FORMAT_OUTLINE,

    render_glyph: Some(ft_raster1_render), /* FT_Renderer_RenderFunc    render_glyph    */
    transform_glyph: Some(ft_raster1_transform), /* FT_Renderer_TransformFunc transform_glyph */
    get_glyph_cbox: Some(ft_raster1_get_cbox), /* FT_Renderer_GetCBoxFunc   get_glyph_cbox  */
    set_mode: Some(ft_raster1_set_mode),   /* FT_Renderer_SetModeFunc   set_mode        */

    raster_class: Some(&FT_STANDARD_RASTER), /* FT_Raster_Funcs*          raster_class    */
};
