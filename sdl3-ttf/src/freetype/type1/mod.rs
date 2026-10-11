// Rust translation of src/type1/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The Type 1 font driver (`t1driver`), its face, size and slot objects
//! (`t1objs`), loader (`t1load`, with the keywords of `t1tokens.h`),
//! parser (`t1parse`), glyph loader (`t1gload`), and AFM and PFM metrics
//! support (`t1afm`).

pub mod t1afm;
pub mod t1driver;
pub mod t1gload;
pub mod t1load;
pub mod t1objs;
pub mod t1parse;
