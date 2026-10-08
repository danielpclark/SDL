// Rust translation of src/psnames/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The `psnames` module: PostScript glyph names and the Unicode charmap
//! synthesized from them.

pub mod psmodule;
pub mod pstables;
