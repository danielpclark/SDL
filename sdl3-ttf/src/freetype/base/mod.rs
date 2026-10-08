// Rust translation of src/base/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The FreeType base layer (`ftbase` and the base extensions SDL_ttf
//! uses).

pub mod ftbitmap;
pub mod ftcalc;
pub mod ftgloadr;
pub mod ftinit;
pub mod ftlcdfil;
pub mod ftmemory;
pub mod ftobjs;
pub mod ftoutln;
pub mod ftrfork;
pub mod ftstream;
pub mod fttrigon;
