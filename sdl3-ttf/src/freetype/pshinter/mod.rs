// Rust translation of src/pshinter/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2001-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The PostScript hinter module: its global hints (`pshglob`), which the
//! CFF driver creates and scales with each size, and the module
//! (`pshmod`). The hints recorder (`pshrec`) and hinting algorithm
//! (`pshalgo`) are only used by the old Type 1 and CFF interpreters, which
//! SDL_ttf's bundled build does not compile, and are not translated.

pub mod pshglob;
pub mod pshmod;
