// Rust translation of src/psaux/ and the PostScript driver record of
// include/freetype/internal/psaux.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// Copyright 2006-2014 Adobe Systems Incorporated (the `ps*` files of the
// Adobe CFF engine, which also carry Adobe's patent license grant; see
// their headers).
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The PostScript auxiliary module: the PostScript table and parser and
//! the Type 1, CFF and PS glyph builders and decoder set-up (`psobjs`,
//! `cffdecode`), the Type 1 decoder set-up (`t1decode`), the Type 1
//! charmaps (`t1cmap`), the AFM parser (`afmparse`), the PostScript
//! number conversions (`psconv`) and the Adobe CFF engine (`psft`,
//! `psfont`, `psintrp`, `pshints`, `psblues`, `psstack`, `psarrst`,
//! `psread`, `pserror`), which renders CFF, CFF2 and (in its Type 1 mode,
//! `font->isT1`) Type 1 charstrings.
//!
//! As SDL_ttf's bundled build configures it, `CFF_CONFIG_OPTION_OLD_ENGINE`
//! and `T1_CONFIG_OPTION_OLD_ENGINE` are undefined, so the old charstring
//! interpreters (`cff_decoder_parse_charstrings`,
//! `t1_decoder_parse_charstrings`) are not compiled.

pub mod afmparse;
pub mod cffdecode;
pub mod psarrst;
pub mod psauxmod;
pub mod psblues;
pub mod psconv;
pub mod pserror;
pub mod psfixed;
pub mod psfont;
pub mod psft;
pub mod psglue;
pub mod pshints;
pub mod psintrp;
pub mod psobjs;
pub mod psread;
pub mod psstack;
pub mod t1cmap;
pub mod t1decode;

use super::fttypes::*;

/// `PS_DriverRec`: PostScript modules driver class (the CFF driver's
/// module data, `FT_DriverRec` aside).
#[derive(Debug, Clone, Copy, Default)]
pub struct PsDriverRec {
    pub hinting_engine: FtUInt,
    pub no_stem_darkening: bool,
    pub darken_params: [FtInt; 8],
    pub random_seed: FtInt32,
}

/* ftdriver.h */

/// `FT_HINTING_FREETYPE`
pub const FT_HINTING_FREETYPE: FtUInt = 0;
/// `FT_HINTING_ADOBE`
pub const FT_HINTING_ADOBE: FtUInt = 1;

/* ftoption.h */

pub const CFF_CONFIG_OPTION_DARKENING_PARAMETER_X1: FtInt = 500;
pub const CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y1: FtInt = 400;

pub const CFF_CONFIG_OPTION_DARKENING_PARAMETER_X2: FtInt = 1000;
pub const CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y2: FtInt = 275;

pub const CFF_CONFIG_OPTION_DARKENING_PARAMETER_X3: FtInt = 1667;
pub const CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y3: FtInt = 275;

pub const CFF_CONFIG_OPTION_DARKENING_PARAMETER_X4: FtInt = 2333;
pub const CFF_CONFIG_OPTION_DARKENING_PARAMETER_Y4: FtInt = 0;
