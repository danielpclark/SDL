// Rust translation of src/cff/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The OpenType/CFF font driver: bare CFF fonts and CFF or CFF2
//! outlines in OpenType (`OTTO') wrappers, whose charstrings the Adobe CFF
//! engine of `psaux` renders and whose global hints the `pshinter` module
//! scales. The face is the `sfnt` module's `TT_FaceRec`, as in C.

pub mod cffcmap;
pub mod cffdrivr;
pub mod cffgload;
pub mod cffload;
pub mod cffobjs;
pub mod cffparse;
