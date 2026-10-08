// Rust translation of the raster interface of include/freetype/ftimage.h
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! `FT_Raster_Params`: the target bitmap, the flags, the direct-rendering
//! span callback (with its `user` pointer folded into the closure) and the
//! clip box, plus the extra parameters of the SDF rasters
//! (`SDF_Raster_Params`, which extends it in ftsdfrend.h). The `source`
//! (an outline, or a bitmap for the bitmap SDF raster) is passed to the
//! raster functions with the parameters.

use super::fttypes::*;

/// `SDF_Raster_Params` extension fields
#[derive(Debug, Clone, Copy, Default)]
pub struct SdfRasterExt {
    pub spread: FtUInt,
    pub flip_sign: bool,
    pub flip_y: bool,
    pub overlaps: bool,
}

/// `FT_Raster_Params`
pub struct FtRasterParams<'a> {
    pub target: Option<&'a mut FtBitmap>,
    pub flags: i32,
    /// `gray_spans` (and `user`)
    pub gray_spans: Option<&'a mut dyn FnMut(i32, &[FtSpan])>,
    pub clip_box: FtBBox,
    /// the SDF rasters' parameters
    pub sdf: SdfRasterExt,
}

impl std::fmt::Debug for FtRasterParams<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtRasterParams")
            .field("flags", &self.flags)
            .field("clip_box", &self.clip_box)
            .finish()
    }
}

impl<'a> FtRasterParams<'a> {
    /// Parameters with a target and flags, the rest zeroed (as C's
    /// callers `FT_ZERO` the structure first).
    pub fn new(target: Option<&'a mut FtBitmap>, flags: i32) -> FtRasterParams<'a> {
        FtRasterParams {
            target,
            flags,
            gray_spans: None,
            clip_box: FtBBox::default(),
            sdf: SdfRasterExt::default(),
        }
    }
}

/// The source of a raster operation (`params->source`).
#[derive(Debug, Clone, Copy)]
pub enum FtRasterSource<'a> {
    Outline(&'a FtOutline),
    Bitmap(&'a FtBitmap),
}

/// `FT_Raster_Funcs` (the raster objects of the rasters translated here
/// hold no state of their own, so `raster_new`, `raster_reset` and
/// `raster_done` have nothing to do).
#[derive(Debug)]
pub struct FtRasterFuncs {
    pub glyph_format: FtGlyphFormat,
    /// `raster_render`
    pub raster_render: fn(source: FtRasterSource, params: &mut FtRasterParams) -> FtResult<()>,
}
