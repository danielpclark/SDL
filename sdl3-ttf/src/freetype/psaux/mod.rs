// Rust translation of src/psaux/ and the PostScript driver record of
// include/freetype/internal/psaux.h from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// Copyright 2006-2014 Adobe Systems Incorporated (the `ps*` files of the
// Adobe CFF engine, which also carry Adobe's patent license grant; see
// their headers).
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The PostScript auxiliary module: the CFF glyph builder and decoder
//! set-up (`psobjs`, `cffdecode`) and the Adobe CFF engine (`psft`,
//! `psfont`, `psintrp`, `pshints`, `psblues`, `psstack`, `psarrst`,
//! `psread`, `pserror`), which renders CFF and CFF2 charstrings.
//!
//! As SDL_ttf's bundled build configures it, `CFF_CONFIG_OPTION_OLD_ENGINE`
//! and `T1_CONFIG_OPTION_OLD_ENGINE` are undefined, so the old charstring
//! interpreters are not compiled. Not translated yet: the parts only the
//! Type 1, CID and Type 42 drivers use (the PostScript table and parser of
//! `psobjs`, `t1decode`, `t1cmap`, `afmparse`, `psconv`), and with them the
//! Adobe engine's Type 1 mode (`font->isT1`, which only those drivers
//! set); its branches are noted where C has them.

pub mod cffdecode;
pub mod psarrst;
pub mod psauxmod;
pub mod psblues;
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
