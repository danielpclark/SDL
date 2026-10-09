// Rust translation of src/sdf/ftsdfrend.c (and ftsdfrend.h) from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2020-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// Written by Anuj Verma.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Signed Distance Field renderer interface (body).
//!
//! Translation note: the renderer's properties (`SDF_Renderer_Module`'s
//! fields) are the module's `props`.

use super::super::base::ftmemory::ft_alloc_mult;
use super::super::base::ftobjs::*;
use super::super::base::ftoutln::{
    ft_outline_get_cbox, ft_outline_transform, ft_outline_translate,
};
use super::super::ftimage::{FtRasterParams, FtRasterSource};
use super::super::fttypes::*;
use super::ftbsdf::{bsdf_raster_set_mode, FT_BITMAP_SDF_RASTER};
use super::ftsdf::{sdf_raster_set_mode, FT_SDF_RASTER};
use super::ftsdfcommon::*;

/**************************************************************************
 *
 * ftsdfrend.h
 *
 */

/// `SDF_Renderer_Module`: This struct extends the native renderer struct
/// `FT_RendererRec`.  It is basically used to store various parameters
/// required by the renderer and some additional parameters that can be
/// used to tweak the output of the renderer.
#[derive(Debug, Clone, Copy)]
pub struct SdfRendererModule {
    /// This is an essential parameter/property required by the renderer.
    /// `spread` defines the maximum unsigned value that is present in the
    /// final SDF output.  For the default value check file
    /// `ftsdfcommon.h`.
    pub spread: FtUInt,
    /// By default positive values indicate positions inside of contours,
    /// i.e., filled by a contour.  If this property is true then that
    /// output will be the opposite of the default, i.e., negative values
    /// indicate positions inside of contours.
    pub flip_sign: u8, /* FT_Bool */
    /// Setting this parameter to true makes the output image flipped
    /// along the y-axis.
    pub flip_y: u8, /* FT_Bool */
    /// Set this to true to generate SDF for glyphs having overlapping
    /// contours.  The overlapping support is limited to glyphs that do not
    /// have self-intersecting contours.  Also, removing overlaps require a
    /// considerable amount of extra memory; additionally, it will not work
    /// if generating SDF from bitmap.
    pub overlaps: u8, /* FT_Bool */
}

/**************************************************************************
 *
 * macros and default property values
 *
 */

/// `SDF_RENDERER( rend )`: the renderer's properties
fn sdf_renderer(module: &FtModuleRec) -> SdfRendererModule {
    module
        .with_props(|r: &mut SdfRendererModule| *r)
        .unwrap_or(SdfRendererModule {
            spread: DEFAULT_SPREAD,
            flip_sign: 0,
            flip_y: 0,
            overlaps: 0,
        })
}

/**************************************************************************
 *
 * for setting properties
 *
 */

/// The `FT_Int` a property value points to (`*(const FT_Int*)value`).
///
/// FIXME (upstream): C reads the string of a property given as a string
/// (through `FREETYPE_PROPERTIES`) as an integer too; it is parsed as a
/// decimal number here.
fn property_int(value: &FtPropertyValue) -> FtResult<FtInt> {
    match value {
        FtPropertyValue::Int(v) => Ok(*v),
        FtPropertyValue::UInt(v) => Ok(*v as FtInt),
        FtPropertyValue::Bool(b) => Ok(*b as FtInt),
        FtPropertyValue::IntArray(a) => a.first().copied().ok_or(FT_ERR_INVALID_ARGUMENT),
        FtPropertyValue::Str(s) => s
            .trim()
            .parse::<FtInt>()
            .map_err(|_| FT_ERR_INVALID_ARGUMENT),
    }
}

/// `sdf_property_set`: property setter function
fn sdf_property_set(
    module: &FtModuleRec,
    property_name: &str,
    value: &FtPropertyValue,
    _value_is_string: bool,
) -> FtResult<()> {
    if property_name == "spread" {
        let val = property_int(value)?;

        if val > MAX_SPREAD as FtInt || val < MIN_SPREAD as FtInt {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        module.with_props(|render: &mut SdfRendererModule| render.spread = val as FtUInt);
    } else if property_name == "flip_sign" {
        let val = property_int(value)?;

        module.with_props(|render: &mut SdfRendererModule| {
            render.flip_sign = if val != 0 { 1 } else { 0 }
        });
    } else if property_name == "flip_y" {
        let val = property_int(value)?;

        module.with_props(|render: &mut SdfRendererModule| {
            render.flip_y = if val != 0 { 1 } else { 0 }
        });
    } else if property_name == "overlaps" {
        /* (`*(const FT_Bool*)value': the value's first byte) */
        let val = property_int(value)? as u8;

        module.with_props(|render: &mut SdfRendererModule| render.overlaps = val);
    } else {
        return Err(FT_ERR_MISSING_PROPERTY);
    }

    Ok(())
}

/// `sdf_property_get`: property getter function
fn sdf_property_get(module: &FtModuleRec, property_name: &str) -> FtResult<FtPropertyValue> {
    let render = sdf_renderer(module);

    if property_name == "spread" {
        Ok(FtPropertyValue::UInt(render.spread))
    } else if property_name == "flip_sign" {
        Ok(FtPropertyValue::Int(render.flip_sign as FtInt))
    } else if property_name == "flip_y" {
        Ok(FtPropertyValue::Int(render.flip_y as FtInt))
    } else if property_name == "overlaps" {
        Ok(FtPropertyValue::Int(render.overlaps as FtInt))
    } else {
        Err(FT_ERR_MISSING_PROPERTY)
    }
}

/// `sdf_service_properties`
static SDF_SERVICE_PROPERTIES: FtServicePropertiesRec = FtServicePropertiesRec {
    set_property: sdf_property_set, /* set_property */
    get_property: sdf_property_get, /* get_property */
};

/// `ft_sdf_requester` (with `sdf_services`)
fn ft_sdf_requester(_module: &FtModuleRec, module_interface: &str) -> Option<FtService> {
    if module_interface == FT_SERVICE_ID_PROPERTIES {
        return Some(FtService::Properties(&SDF_SERVICE_PROPERTIES));
    }
    None
}

/*************************************************************************/
/*************************************************************************/
/*                                                                      */
/*   OUTLINE TO SDF CONVERTER                                           */
/*                                                                      */
/*************************************************************************/
/*************************************************************************/

/**************************************************************************
 *
 * interface functions
 *
 */

/// `ft_sdf_init`
fn ft_sdf_init(library: &mut FtLibraryRec, module: usize) -> FtResult<()> {
    let sdf_render = SdfRendererModule {
        spread: DEFAULT_SPREAD,
        flip_sign: 0,
        flip_y: 0,
        overlaps: 0,
    };

    let m = &library.modules[module];
    *m.props.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(sdf_render));

    Ok(())
}

/// `ft_sdf_done`
fn ft_sdf_done(_module: &FtModuleRec) {}

/// `ft_sdf_render`: generate signed distance field from a glyph's slot
/// image
fn ft_sdf_render(
    library: &FtLibraryRec,
    module: usize,
    slot: &mut FtGlyphSlotRec,
    mode: FtRenderMode,
    origin: Option<&FtVector>,
) -> FtResult<()> {
    let mut x_shift: FtPos = 0;
    let mut y_shift: FtPos = 0;

    let x_pad: FtPos;
    let y_pad: FtPos;

    let sdf_module = sdf_renderer(&library.modules[module]);
    let render = &library.modules[module];

    let error: FtResult<()> = 'exit: {
        /* check whether slot format is correct before rendering */
        if slot.format != render.renderer_class().glyph_format {
            break 'exit Err(FT_ERR_INVALID_GLYPH_FORMAT);
        }

        /* check whether render mode is correct */
        if mode != FT_RENDER_MODE_SDF {
            break 'exit Err(FT_ERR_CANNOT_RENDER_GLYPH);
        }

        /* deallocate the previously allocated bitmap */
        if slot.internal.flags & FT_GLYPH_OWN_BITMAP != 0 {
            slot.bitmap.buffer = Vec::new();
            slot.internal.flags &= !FT_GLYPH_OWN_BITMAP;
        }

        /* preset the bitmap using the glyph's outline;         */
        /* the sdf bitmap is similar to an anti-aliased bitmap  */
        /* with a slightly bigger size and different pixel mode */
        if ft_glyphslot_preset_bitmap(slot, FT_RENDER_MODE_NORMAL, origin) {
            break 'exit Err(FT_ERR_RASTER_OVERFLOW);
        }

        /* nothing to render */
        if slot.bitmap.rows == 0 || slot.bitmap.pitch == 0 {
            break 'exit Ok(());
        }

        /* the padding will simply be equal to the `spread' */
        x_pad = sdf_module.spread as FtPos;
        y_pad = sdf_module.spread as FtPos;

        let bitmap = &mut slot.bitmap;

        /* apply the padding; will be in all the directions */
        bitmap.rows = (bitmap.rows as FtPos).wrapping_add(y_pad * 2) as u32;
        bitmap.width = (bitmap.width as FtPos).wrapping_add(x_pad * 2) as u32;

        /* ignore the pitch, pixel mode and set custom */
        bitmap.pixel_mode = FT_PIXEL_MODE_GRAY;
        bitmap.pitch = bitmap.width as i32;
        bitmap.num_grays = 255;

        /* allocate new buffer */
        match ft_alloc_mult(bitmap.rows as FtLong, bitmap.pitch as FtLong) {
            Ok(b) => bitmap.buffer = b,
            Err(e) => break 'exit Err(e),
        }

        slot.internal.flags |= FT_GLYPH_OWN_BITMAP;

        slot.bitmap_top = (slot.bitmap_top as FtPos).wrapping_add(y_pad) as FtInt;
        slot.bitmap_left = (slot.bitmap_left as FtPos).wrapping_sub(x_pad) as FtInt;

        x_shift = 64 * -(slot.bitmap_left as FtPos);
        y_shift = 64 * -(slot.bitmap_top as FtPos);
        y_shift += 64 * slot.bitmap.rows as FtInt as FtPos;

        if let Some(origin) = origin {
            x_shift += origin.x;
            y_shift += origin.y;
        }

        /* translate outline to render it into the bitmap */
        if x_shift != 0 || y_shift != 0 {
            ft_outline_translate(&mut slot.outline, x_shift, y_shift);
        }

        /* set up parameters */
        let error = {
            let mut params = FtRasterParams::new(Some(&mut slot.bitmap), FT_RASTER_FLAG_SDF);
            params.sdf.spread = sdf_module.spread;
            params.sdf.flip_sign = sdf_module.flip_sign != 0;
            params.sdf.flip_y = sdf_module.flip_y != 0;
            params.sdf.overlaps = sdf_module.overlaps != 0;

            /* render the outline */
            library.renderer_raster_render(module, &slot.outline, &mut params)
        };

        /* transform the outline back to the original state */
        if x_shift != 0 || y_shift != 0 {
            ft_outline_translate(&mut slot.outline, -x_shift, -y_shift);
        }

        error
    };

    /* Exit: */
    if error.is_ok() {
        /* the glyph is successfully rendered to a bitmap */
        slot.format = FT_GLYPH_FORMAT_BITMAP;
    } else if slot.internal.flags & FT_GLYPH_OWN_BITMAP != 0 {
        slot.bitmap.buffer = Vec::new();
        slot.internal.flags &= !FT_GLYPH_OWN_BITMAP;
    }

    error
}

/// `ft_sdf_transform`: transform the glyph using matrix and/or delta (for
/// a renderer of `glyph_format`)
fn ft_sdf_transform_format(
    glyph_format: FtGlyphFormat,
    slot: &mut FtGlyphSlotRec,
    matrix: Option<&FtMatrix>,
    delta: Option<&FtVector>,
) -> FtResult<()> {
    if slot.format != glyph_format {
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

/// `ft_sdf_transform` of the 'sdf' renderer
fn ft_sdf_transform(
    slot: &mut FtGlyphSlotRec,
    matrix: Option<&FtMatrix>,
    delta: Option<&FtVector>,
) -> FtResult<()> {
    ft_sdf_transform_format(FT_GLYPH_FORMAT_OUTLINE, slot, matrix, delta)
}

/// `ft_sdf_transform` of the 'bsdf' renderer
fn ft_bsdf_transform(
    slot: &mut FtGlyphSlotRec,
    matrix: Option<&FtMatrix>,
    delta: Option<&FtVector>,
) -> FtResult<()> {
    ft_sdf_transform_format(FT_GLYPH_FORMAT_BITMAP, slot, matrix, delta)
}

/// `ft_sdf_get_cbox`: return the control box of a glyph's outline (for a
/// renderer of `glyph_format`)
fn ft_sdf_get_cbox_format(glyph_format: FtGlyphFormat, slot: &FtGlyphSlotRec) -> FtBBox {
    let mut cbox = FtBBox::default();

    if slot.format == glyph_format {
        cbox = ft_outline_get_cbox(&slot.outline);
    }

    cbox
}

/// `ft_sdf_get_cbox` of the 'sdf' renderer
fn ft_sdf_get_cbox(slot: &FtGlyphSlotRec) -> FtBBox {
    ft_sdf_get_cbox_format(FT_GLYPH_FORMAT_OUTLINE, slot)
}

/// `ft_sdf_get_cbox` of the 'bsdf' renderer
fn ft_bsdf_get_cbox(slot: &FtGlyphSlotRec) -> FtBBox {
    ft_sdf_get_cbox_format(FT_GLYPH_FORMAT_BITMAP, slot)
}

/// `ft_sdf_set_mode`: set render specific modes or attributes (passed to
/// the rasterizer) of the 'sdf' renderer
fn ft_sdf_set_mode(_render: &FtModuleRec, mode_tag: FtULong) -> FtResult<()> {
    /* pass it to the rasterizer */
    sdf_raster_set_mode(mode_tag)
}

/// `ft_sdf_set_mode` of the 'bsdf' renderer
fn ft_bsdf_set_mode(_render: &FtModuleRec, mode_tag: FtULong) -> FtResult<()> {
    /* pass it to the rasterizer */
    bsdf_raster_set_mode(mode_tag)
}

/// `ft_sdf_renderer_class`
pub static FT_SDF_RENDERER_CLASS: FtRendererClass = FtRendererClass {
    root: FtModuleClass {
        module_flags: FT_MODULE_RENDERER,

        module_name: "sdf",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None,

        module_init: Some(ft_sdf_init),
        module_done: Some(ft_sdf_done),
        get_interface: Some(ft_sdf_requester),
    },

    glyph_format: FT_GLYPH_FORMAT_OUTLINE,

    render_glyph: Some(ft_sdf_render),       /* render_glyph    */
    transform_glyph: Some(ft_sdf_transform), /* transform_glyph */
    get_glyph_cbox: Some(ft_sdf_get_cbox),   /* get_glyph_cbox  */
    set_mode: Some(ft_sdf_set_mode),         /* set_mode        */

    raster_class: Some(&FT_SDF_RASTER), /* raster_class    */
};

/*************************************************************************/
/*************************************************************************/
/*                                                                      */
/*   BITMAP TO SDF CONVERTER                                            */
/*                                                                      */
/*************************************************************************/
/*************************************************************************/

/// `ft_bsdf_render`: generate signed distance field from glyph's bitmap
fn ft_bsdf_render(
    library: &FtLibraryRec,
    module: usize,
    slot: &mut FtGlyphSlotRec,
    mode: FtRenderMode,
    origin: Option<&FtVector>,
) -> FtResult<()> {
    let mut x_pad: FtPos = 0;
    let mut y_pad: FtPos = 0;

    let sdf_module = sdf_renderer(&library.modules[module]);
    let render = &library.modules[module];

    /* initialize the bitmap in case any error occurs */
    let mut target = FtBitmap::default();

    let error: FtResult<()> = 'exit: {
        /* check whether slot format is correct before rendering */
        if slot.format != render.renderer_class().glyph_format {
            break 'exit Err(FT_ERR_INVALID_GLYPH_FORMAT);
        }

        /* check whether render mode is correct */
        if mode != FT_RENDER_MODE_SDF {
            break 'exit Err(FT_ERR_CANNOT_RENDER_GLYPH);
        }

        if origin.is_some() {
            break 'exit Err(FT_ERR_UNIMPLEMENTED_FEATURE);
        }

        /* nothing to render */
        if slot.bitmap.rows == 0 || slot.bitmap.pitch == 0 {
            break 'exit Ok(());
        }

        /* Do not generate SDF if the bitmap is not owned by the       */
        /* glyph: it might be that the source buffer is already freed. */
        if slot.internal.flags & FT_GLYPH_OWN_BITMAP == 0 {
            break 'exit Err(FT_ERR_INVALID_ARGUMENT);
        }

        /* (FT_Bitmap_New) */

        /* padding will simply be equal to `spread` */
        x_pad = sdf_module.spread as FtPos;
        y_pad = sdf_module.spread as FtPos;

        /* apply padding, which extends to all directions */
        target.rows = (slot.bitmap.rows as FtPos).wrapping_add(y_pad * 2) as u32;
        target.width = (slot.bitmap.width as FtPos).wrapping_add(x_pad * 2) as u32;

        /* set up the target bitmap */
        target.pixel_mode = FT_PIXEL_MODE_GRAY;
        target.pitch = target.width as i32;
        target.num_grays = 255;

        match ft_alloc_mult(target.rows as FtLong, target.pitch as FtLong) {
            Ok(b) => target.buffer = b,
            Err(e) => break 'exit Err(e),
        }

        /* set up parameters */
        let mut params = FtRasterParams::new(Some(&mut target), FT_RASTER_FLAG_SDF);
        params.sdf.spread = sdf_module.spread;
        params.sdf.flip_sign = sdf_module.flip_sign != 0;
        params.sdf.flip_y = sdf_module.flip_y != 0;
        /* (`overlaps' is left unset) */

        match render.renderer_class().raster_class {
            Some(rc) => (rc.raster_render)(FtRasterSource::Bitmap(&slot.bitmap), &mut params),
            None => Err(FT_ERR_CANNOT_RENDER_GLYPH),
        }
    };

    /* Exit: */
    if error.is_ok() {
        /* the glyph is successfully converted to a SDF */
        /* (the old buffer goes with the old bitmap)    */
        let has_buffer = !target.buffer.is_empty();
        slot.bitmap = target;
        slot.bitmap_top = (slot.bitmap_top as FtPos).wrapping_add(y_pad) as FtInt;
        slot.bitmap_left = (slot.bitmap_left as FtPos).wrapping_sub(x_pad) as FtInt;

        if has_buffer {
            slot.internal.flags |= FT_GLYPH_OWN_BITMAP;
        }
    }
    /* (else the target's buffer is dropped) */

    error
}

/// `ft_bitmap_sdf_renderer_class`
pub static FT_BITMAP_SDF_RENDERER_CLASS: FtRendererClass = FtRendererClass {
    root: FtModuleClass {
        module_flags: FT_MODULE_RENDERER,

        module_name: "bsdf",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None,

        module_init: Some(ft_sdf_init),
        module_done: Some(ft_sdf_done),
        get_interface: Some(ft_sdf_requester),
    },

    glyph_format: FT_GLYPH_FORMAT_BITMAP,

    render_glyph: Some(ft_bsdf_render),       /* render_glyph    */
    transform_glyph: Some(ft_bsdf_transform), /* transform_glyph */
    get_glyph_cbox: Some(ft_bsdf_get_cbox),   /* get_glyph_cbox  */
    set_mode: Some(ft_bsdf_set_mode),         /* set_mode        */

    raster_class: Some(&FT_BITMAP_SDF_RASTER), /* raster_class    */
};
