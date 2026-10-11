// Rust translation of src/lzw/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2004-2023 by Albert Chin-A-Young and David Turner.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The `lzw` component: the .Z stream (`ftlzw`) and its LZW decompressor
//! (`ftzopen`), which the PCF driver reads compressed fonts with.

pub mod ftlzw;
pub mod ftzopen;
