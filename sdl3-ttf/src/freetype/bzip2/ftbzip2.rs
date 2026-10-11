// Rust translation of src/bzip2/ftbzip2.c from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2010-2023 by Joel Klinghed.
// Based on `src/gzip/ftgzip.c'.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType support for .bz2 compressed files.
//!
//! This optional component relies on libbz2.  It should mainly be used to
//! parse compressed PCF fonts, as found with many X11 server
//! distributions.
//!
//! SDL_ttf's bundled build disables bzip2 (`FT_DISABLE_BZIP2`), so
//! `FT_CONFIG_OPTION_USE_BZIP2` is undefined and only the stub of
//! `FT_Stream_OpenBzip2` is compiled (which the PCF driver, built without
//! the option too, never calls).

use super::super::base::ftstream::{FtSharedStream, FtStream};
use super::super::fttypes::*;

/// `FT_Stream_OpenBzip2` (`!FT_CONFIG_OPTION_USE_BZIP2`)
pub fn ft_stream_open_bzip2(_source: &FtSharedStream) -> FtResult<FtStream> {
    Err(FT_ERR_UNIMPLEMENTED_FEATURE)
}
