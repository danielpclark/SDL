// Rust translation of src/svg/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2022-2023 by David Turner, Robert Wilhelm, Werner Lemberg,
// and Moazin Khatti.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The `svg' module: the OT-SVG renderer (`ot-svg`), which renders the
//! SVG documents of OpenType SVG glyphs through the hooks set with its
//! `svg-hooks` property (SDL_ttf sets PlutoSVG's).

pub mod ftsvg;
