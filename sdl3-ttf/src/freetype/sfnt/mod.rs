// Rust translation of src/sfnt/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The `sfnt` module: SFNT (TrueType/OpenType container) table loading.
//!
//! SDL_ttf's bundled build leaves out WOFF2 (`sfwoff2.c`, `woff2tags.c`:
//! no Brotli) and PNG bitmaps (`pngshim.c`: no libpng), so those files have
//! no counterpart here.

pub mod sfdriver;
pub mod sfobjs;
pub mod sfwoff;
pub mod ttbdf;
pub mod ttcmap;
pub mod ttcolr;
pub mod ttcpal;
pub mod ttkern;
pub mod ttload;
pub mod ttmtx;
pub mod ttpost;
pub mod ttsbit;
pub mod ttsvg;
