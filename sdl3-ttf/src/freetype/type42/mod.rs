// Rust translation of src/type42/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2002-2023 by Roberto Alameda.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The Type 42 font driver (`t42drivr`), its face, size and slot objects
//! (`t42objs`, with `t42types.h`) and parser (`t42parse`): Type 42 fonts
//! are TrueType fonts in a PostScript wrapper, whose glyphs the TrueType
//! driver loads.

pub mod t42drivr;
pub mod t42objs;
pub mod t42parse;
