// Rust translation of src/cid/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The CID-keyed Type 1 font driver (`cidriver`), its face, size and slot
//! objects (`cidobjs`), loader (`cidload`, with the keywords of
//! `cidtoken.h`), parser (`cidparse`) and glyph loader (`cidgload`).

pub mod cidgload;
pub mod cidload;
pub mod cidobjs;
pub mod cidparse;
pub mod cidriver;
