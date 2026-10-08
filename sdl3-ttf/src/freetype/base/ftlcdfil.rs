// Rust translation of src/base/ftlcdfil.c from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2006-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType API for color filtering of subpixel bitmap glyphs (body).
//!
//! `FT_CONFIG_OPTION_SUBPIXEL_RENDERING` is undefined in upstream's build,
//! so this is the `!FT_CONFIG_OPTION_SUBPIXEL_RENDERING` half: Harmony LCD
//! rendering with a subpixel geometry, and the filter functions report
//! `Unimplemented_Feature`.

use super::super::fttypes::*;
use super::ftobjs::{FtGlyphSlotRec, FtLibraryRec};

/// `FT_LcdFilter`
pub type FtLcdFilter = u32;
pub const FT_LCD_FILTER_NONE: FtLcdFilter = 0;
pub const FT_LCD_FILTER_DEFAULT: FtLcdFilter = 1;
pub const FT_LCD_FILTER_LIGHT: FtLcdFilter = 2;
pub const FT_LCD_FILTER_LEGACY1: FtLcdFilter = 3;
pub const FT_LCD_FILTER_LEGACY: FtLcdFilter = 16;

/// `ft_lcd_padding`: add padding to accommodate outline shifts
pub fn ft_lcd_padding(cbox: &mut FtBBox, slot: &FtGlyphSlotRec, mode: FtRenderMode) {
    let sub = match slot.library.as_ref() {
        Some(l) => l.lcd_geometry(),
        None => return,
    };

    if mode == FT_RENDER_MODE_LCD {
        cbox.xMin -= sub[0].x.max(sub[1].x).max(sub[2].x);
        cbox.xMax -= sub[0].x.min(sub[1].x).min(sub[2].x);
        cbox.yMin -= sub[0].y.max(sub[1].y).max(sub[2].y);
        cbox.yMax -= sub[0].y.min(sub[1].y).min(sub[2].y);
    } else if mode == FT_RENDER_MODE_LCD_V {
        cbox.xMin -= sub[0].y.max(sub[1].y).max(sub[2].y);
        cbox.xMax -= sub[0].y.min(sub[1].y).min(sub[2].y);
        cbox.yMin += sub[0].x.min(sub[1].x).min(sub[2].x);
        cbox.yMax += sub[0].x.max(sub[1].x).max(sub[2].x);
    }
}

/// `FT_Library_SetLcdFilterWeights`
pub fn ft_library_set_lcd_filter_weights(
    _library: &FtLibraryRec,
    _weights: &[u8; 5],
) -> FtResult<()> {
    Err(FT_ERR_UNIMPLEMENTED_FEATURE)
}

/// `FT_Library_SetLcdFilter`
pub fn ft_library_set_lcd_filter(_library: &FtLibraryRec, _filter: FtLcdFilter) -> FtResult<()> {
    Err(FT_ERR_UNIMPLEMENTED_FEATURE)
}

/// `FT_Library_SetLcdGeometry` (documentation in ftlcdfil.h)
pub fn ft_library_set_lcd_geometry(library: &FtLibraryRec, sub: [FtVector; 3]) -> FtResult<()> {
    *library
        .lcd_geometry
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = sub;

    Ok(())
}
