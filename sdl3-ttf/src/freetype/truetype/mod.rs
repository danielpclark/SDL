// Rust translation of src/truetype/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The TrueType font driver: `glyf' outlines, the bytecode interpreter
//! (v35 and v40), and TrueType GX/OpenType font variations.

pub mod ttdriver;
pub mod ttgload;
pub mod ttgxvar;
pub mod ttinterp;
pub mod ttobjs;
pub mod ttpload;
