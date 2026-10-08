// Rust translation of src/autofit/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it), with `AF_CONFIG_OPTION_CJK` and
// `AF_CONFIG_OPTION_INDIC` defined and without HarfBuzz, as SDL_ttf builds
// it.
// Copyright (C) 2003-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The `autofit' module: the auto-hinter (FreeType's light hinting, and
//! the hinting of fonts without hinting instructions).

pub mod afblue;
pub mod afcjk;
pub mod afdummy;
pub mod afglobal;
pub mod afhints;
pub mod afindic;
pub mod aflatin;
pub mod afloader;
pub mod afmodule;
pub mod afranges;
pub mod afscript;
pub mod afshaper;
pub mod afstyles;
pub mod aftypes;
