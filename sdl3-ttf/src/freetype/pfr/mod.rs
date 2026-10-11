// Rust translation of src/pfr/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The PFR (Portable Font Resource) font driver (`pfrdrivr`), its face
//! and slot objects (`pfrobjs`), loader (`pfrload`), glyph loader
//! (`pfrgload`), bitmap loader (`pfrsbit`), charmap (`pfrcmap`), and
//! records (`pfrtypes`).

pub mod pfrcmap;
pub mod pfrdrivr;
pub mod pfrgload;
pub mod pfrload;
pub mod pfrobjs;
pub mod pfrsbit;
pub mod pfrtypes;
