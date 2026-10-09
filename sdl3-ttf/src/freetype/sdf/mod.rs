// Rust translation of src/sdf/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2020-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The `sdf' module: the signed distance field renderers, 'sdf' (from
//! outlines) and 'bsdf' (from bitmaps).

pub mod ftbsdf;
pub mod ftsdf;
pub mod ftsdfcommon;
pub mod ftsdfrend;
