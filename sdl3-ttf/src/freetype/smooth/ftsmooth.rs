// Rust translation of src/smooth/ftsmooth.c (and ftsmooth.h) from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2000-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Anti-aliasing renderer interface (body).
//!
//! The bundled build leaves `FT_CONFIG_OPTION_SUBPIXEL_RENDERING`
//! undefined, so LCD rendering is Harmony's: three coverage bitmaps of the
//! outline shifted by the LCD geometry.
//!
//! Translation note: `ft_smooth_raster_lcdv` renders through the direct
//! (span) mode of the raster into every third row instead of adjusting the
//! bitmap's buffer pointer, pitch and rows; the raster computes the same
//! coverage in both modes.

use super::super::base::ftmemory::ft_alloc_mult;
use super::super::base::ftobjs::*;
use super::super::base::ftoutln::{
    ft_outline_get_cbox, ft_outline_transform, ft_outline_translate,
};
use super::super::ftimage::FtRasterParams;
use super::super::fttypes::*;
use super::ftgrays::FT_GRAYS_RASTER;

/// `ft_smooth_set_mode`: sets render-specific mode
fn ft_smooth_set_mode(_render: &FtModuleRec, _mode_tag: FtULong) -> FtResult<()> {
    /* we simply pass it to the raster */
    /* (`gray_raster_set_mode' has nothing to do) */
    Ok(())
}

/// `ft_smooth_transform`: transform a given glyph image
fn ft_smooth_transform(
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

/// `ft_smooth_get_cbox`: return the glyph's control box
fn ft_smooth_get_cbox(slot: &FtGlyphSlotRec) -> FtBBox {
    if slot.format == FT_GLYPH_FORMAT_OUTLINE {
        ft_outline_get_cbox(&slot.outline)
    } else {
        FtBBox::default()
    }
}

/// `TOrigin`
#[derive(Debug, Clone, Copy)]
struct TOrigin {
    origin: isize, /* pixmap origin at the bottom-left */
    pitch: isize,  /* pitch to go down one row */
}

/* !FT_CONFIG_OPTION_SUBPIXEL_RENDERING */

/// `ft_smooth_init`: initialize renderer -- init its raster
fn ft_smooth_init(library: &mut FtLibraryRec, _module: usize) -> FtResult<()> {
    let mut sub = library
        .lcd_geometry
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    /* set up default subpixel geometry for striped RGB panels. */
    sub[0].x = -21;
    sub[0].y = 0;
    sub[1].x = 0;
    sub[1].y = 0;
    sub[2].x = 21;
    sub[2].y = 0;

    /* (`gray_raster_reset' has nothing to do) */

    Ok(())
}

/// `ft_smooth_lcd_spans`: This function writes every third byte in direct
/// rendering mode
fn ft_smooth_lcd_spans(buffer: &mut [u8], target: &TOrigin, y: i32, spans: &[FtSpan]) {
    let dst_line = target.origin - y as isize * target.pitch;

    for span in spans {
        let mut dst = dst_line + span.x as isize * 3;
        for _ in 0..span.len {
            buffer[dst as usize] = span.coverage;
            dst += 3;
        }
    }
}

/// `ft_smooth_raster_lcd`
fn ft_smooth_raster_lcd(
    library: &FtLibraryRec,
    render: usize,
    outline: &mut FtOutline,
    bitmap: &mut FtBitmap,
) -> FtResult<()> {
    let sub = library.lcd_geometry();

    /* Render 3 separate coverage bitmaps, shifting the outline.  */
    /* Set up direct rendering to record them on each third byte. */
    let clip_box = FtBBox {
        xMin: 0,
        yMin: 0,
        xMax: bitmap.width as FtPos,
        yMax: bitmap.rows as FtPos,
    };

    let mut target = TOrigin {
        origin: if bitmap.pitch < 0 {
            0
        } else {
            (bitmap.rows as isize - 1) * bitmap.pitch as isize
        },
        pitch: bitmap.pitch as isize,
    };

    let buffer = &mut bitmap.buffer;

    let mut raster_render = |outline: &FtOutline, target: &TOrigin| -> FtResult<()> {
        let mut spans = |y: i32, spans: &[FtSpan]| ft_smooth_lcd_spans(buffer, target, y, spans);
        let mut params = FtRasterParams::new(None, FT_RASTER_FLAG_AA | FT_RASTER_FLAG_DIRECT);
        params.gray_spans = Some(&mut spans);
        params.clip_box = clip_box;
        library.renderer_raster_render(render, outline, &mut params)
    };

    let (x, y);
    let error;

    'exit: {
        ft_outline_translate(outline, -sub[0].x, -sub[0].y);
        let e = raster_render(outline, &target);
        if e.is_err() {
            x = sub[0].x;
            y = sub[0].y;
            error = e;
            break 'exit;
        }

        target.origin += 1;
        ft_outline_translate(outline, sub[0].x - sub[1].x, sub[0].y - sub[1].y);
        let e = raster_render(outline, &target);
        if e.is_err() {
            x = sub[1].x;
            y = sub[1].y;
            error = e;
            break 'exit;
        }

        target.origin += 1;
        ft_outline_translate(outline, sub[1].x - sub[2].x, sub[1].y - sub[2].y);
        error = raster_render(outline, &target);
        x = sub[2].x;
        y = sub[2].y;
    }

    /* Exit: */
    ft_outline_translate(outline, x, y);

    error
}

/// `ft_smooth_raster_lcdv`
fn ft_smooth_raster_lcdv(
    library: &FtLibraryRec,
    render: usize,
    outline: &mut FtOutline,
    bitmap: &mut FtBitmap,
) -> FtResult<()> {
    let pitch = bitmap.pitch as isize;
    let sub = library.lcd_geometry();

    /* Render 3 separate coverage bitmaps, shifting the outline. */
    /* Notice that the subpixel geometry vectors are rotated.    */
    /* Triple the pitch to render on each third row.            */
    let rows = bitmap.rows / 3;
    let tpitch = pitch * 3;

    let clip_box = FtBBox {
        xMin: 0,
        yMin: 0,
        xMax: bitmap.width as FtPos,
        yMax: rows as FtPos,
    };
    let base_origin = if tpitch < 0 {
        0
    } else {
        (rows as isize - 1) * tpitch
    };

    let buffer = &mut bitmap.buffer;

    /* (the bitmap mode of the raster, as direct spans into the rows */
    /* starting at `offset'; see the module documentation)          */
    let mut raster_render = |outline: &FtOutline, offset: isize| -> FtResult<()> {
        let target = TOrigin {
            origin: base_origin + offset,
            pitch: tpitch,
        };
        let mut spans = |y: i32, spans: &[FtSpan]| {
            let line = target.origin - y as isize * target.pitch;
            for span in spans {
                let start = (line + span.x as isize) as usize;
                buffer[start..start + span.len as usize].fill(span.coverage);
            }
        };
        let mut params = FtRasterParams::new(None, FT_RASTER_FLAG_AA | FT_RASTER_FLAG_DIRECT);
        params.gray_spans = Some(&mut spans);
        params.clip_box = clip_box;
        library.renderer_raster_render(render, outline, &mut params)
    };

    let (x, y);
    let error;

    'exit: {
        ft_outline_translate(outline, -sub[0].y, sub[0].x);
        let e = raster_render(outline, 0);
        if e.is_err() {
            x = sub[0].y;
            y = -sub[0].x;
            error = e;
            break 'exit;
        }

        ft_outline_translate(outline, sub[0].y - sub[1].y, sub[1].x - sub[0].x);
        let e = raster_render(outline, pitch);
        if e.is_err() {
            x = sub[1].y;
            y = -sub[1].x;
            error = e;
            break 'exit;
        }

        ft_outline_translate(outline, sub[1].y - sub[2].y, sub[2].x - sub[1].x);
        error = raster_render(outline, 2 * pitch);
        x = sub[2].y;
        y = -sub[2].x;
    }

    /* Exit: */
    ft_outline_translate(outline, x, y);

    error
}

/* Oversampling scale to be used in rendering overlaps */
const SCALE: i32 = 1 << 2;

/// `ft_smooth_overlap_spans`: This function averages inflated spans in
/// direct rendering mode
fn ft_smooth_overlap_spans(buffer: &mut [u8], target: &TOrigin, y: i32, spans: &[FtSpan]) {
    let dst = target.origin - (y / SCALE) as isize * target.pitch;

    /* When accumulating the oversampled spans we need to assure that  */
    /* fully covered pixels are equal to 255 and do not overflow.      */
    /* It is important that the SCALE is a power of 2, each subpixel   */
    /* cover can also reach a power of 2 after rounding, and the total */
    /* is clamped to 255 when it adds up to 256.                       */
    for span in spans {
        let cover = (span.coverage as u32 + (SCALE * SCALE / 2) as u32) / (SCALE * SCALE) as u32;
        for x in 0..span.len {
            let i = (dst + ((span.x as i32 + x as i32) / SCALE) as isize) as usize;
            let sum = buffer[i] as u32 + cover;
            buffer[i] = (sum - (sum >> 8)) as u8;
        }
    }
}

/// `ft_smooth_raster_overlap`
fn ft_smooth_raster_overlap(
    library: &FtLibraryRec,
    render: usize,
    outline: &mut FtOutline,
    bitmap: &mut FtBitmap,
) -> FtResult<()> {
    /* Reject outlines that are too wide for 16-bit FT_Span.       */
    /* Other limits are applied upstream with the same error code. */
    if bitmap.width.wrapping_mul(SCALE as u32) > 0x7FFF {
        return Err(FT_ERR_RASTER_OVERFLOW);
    }

    /* Set up direct rendering to average oversampled spans. */
    let clip_box = FtBBox {
        xMin: 0,
        yMin: 0,
        xMax: (bitmap.width * SCALE as u32) as FtPos,
        yMax: (bitmap.rows.wrapping_mul(SCALE as u32)) as FtPos,
    };

    let target = TOrigin {
        origin: if bitmap.pitch < 0 {
            0
        } else {
            (bitmap.rows as isize - 1) * bitmap.pitch as isize
        },
        pitch: bitmap.pitch as isize,
    };

    let n_points = outline.n_points as usize;

    /* inflate outline */
    for vec in outline.points[..n_points].iter_mut() {
        vec.x = vec.x.wrapping_mul(SCALE as FtPos);
        vec.y = vec.y.wrapping_mul(SCALE as FtPos);
    }

    /* render outline into the bitmap */
    let error = {
        let buffer = &mut bitmap.buffer;
        let mut spans =
            |y: i32, spans: &[FtSpan]| ft_smooth_overlap_spans(buffer, &target, y, spans);
        let mut params = FtRasterParams::new(None, FT_RASTER_FLAG_AA | FT_RASTER_FLAG_DIRECT);
        params.gray_spans = Some(&mut spans);
        params.clip_box = clip_box;
        library.renderer_raster_render(render, outline, &mut params)
    };

    /* deflate outline */
    for vec in outline.points[..n_points].iter_mut() {
        vec.x /= SCALE as FtPos;
        vec.y /= SCALE as FtPos;
    }

    error
}

/// `ft_smooth_render`
fn ft_smooth_render(
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

        /* check mode */
        if mode != FT_RENDER_MODE_NORMAL
            && mode != FT_RENDER_MODE_LIGHT
            && mode != FT_RENDER_MODE_LCD
            && mode != FT_RENDER_MODE_LCD_V
        {
            break 'exit Err(FT_ERR_CANNOT_RENDER_GLYPH);
        }

        /* release old bitmap buffer */
        if slot.internal.flags & FT_GLYPH_OWN_BITMAP != 0 {
            slot.bitmap.buffer = Vec::new();
            slot.internal.flags &= !FT_GLYPH_OWN_BITMAP;
        }

        if ft_glyphslot_preset_bitmap(slot, mode, origin) {
            break 'exit Err(FT_ERR_RASTER_OVERFLOW);
        }

        if slot.bitmap.rows == 0 || slot.bitmap.pitch == 0 {
            break 'exit Ok(());
        }

        /* allocate new one */
        match ft_alloc_mult(slot.bitmap.rows as FtLong, slot.bitmap.pitch as FtLong) {
            Ok(b) => slot.bitmap.buffer = b,
            Err(e) => break 'exit Err(e),
        }

        slot.internal.flags |= FT_GLYPH_OWN_BITMAP;

        x_shift = 64 * -(slot.bitmap_left as FtPos);
        y_shift = 64 * -(slot.bitmap_top as FtPos);
        if slot.bitmap.pixel_mode == FT_PIXEL_MODE_LCD_V {
            y_shift += 64 * (slot.bitmap.rows as FtInt / 3) as FtPos;
        } else {
            y_shift += 64 * slot.bitmap.rows as FtInt as FtPos;
        }

        if let Some(origin) = origin {
            x_shift += origin.x;
            y_shift += origin.y;
        }

        /* translate outline to render it into the bitmap */
        if x_shift != 0 || y_shift != 0 {
            ft_outline_translate(&mut slot.outline, x_shift, y_shift);
        }

        let outline = &mut slot.outline;
        let bitmap = &mut slot.bitmap;

        if mode == FT_RENDER_MODE_NORMAL || mode == FT_RENDER_MODE_LIGHT {
            if outline.flags & FT_OUTLINE_OVERLAP != 0 {
                ft_smooth_raster_overlap(library, render, outline, bitmap)
            } else {
                let mut params = FtRasterParams::new(Some(bitmap), FT_RASTER_FLAG_AA);
                library.renderer_raster_render(render, outline, &mut params)
            }
        } else if mode == FT_RENDER_MODE_LCD {
            ft_smooth_raster_lcd(library, render, outline, bitmap)
        } else if mode == FT_RENDER_MODE_LCD_V {
            ft_smooth_raster_lcdv(library, render, outline, bitmap)
        } else {
            Ok(())
        }
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

/// `ft_smooth_renderer_class`
pub static FT_SMOOTH_RENDERER_CLASS: FtRendererClass = FtRendererClass {
    root: FtModuleClass {
        module_flags: FT_MODULE_RENDERER,

        module_name: "smooth",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::None, /* module specific interface */

        module_init: Some(ft_smooth_init), /* module_init   */
        module_done: None,                 /* module_done   */
        get_interface: None,               /* get_interface */
    },

    glyph_format: FT_GLYPH_FORMAT_OUTLINE,

    render_glyph: Some(ft_smooth_render), /* render_glyph    */
    transform_glyph: Some(ft_smooth_transform), /* transform_glyph */
    get_glyph_cbox: Some(ft_smooth_get_cbox), /* get_glyph_cbox  */
    set_mode: Some(ft_smooth_set_mode),   /* set_mode        */

    raster_class: Some(&FT_GRAYS_RASTER), /* raster_class    */
};
