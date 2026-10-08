// Rust translation of the FreeType library (the modules SDL_ttf's
// bundled build compiles) from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType, translated from the copy SDL_ttf bundles (external/freetype,
//! FreeType 2.13.2), configured as SDL_ttf's CMake build configures it.
//!
//! Portions of this software are copyright © 2023 The FreeType Project
//! (www.freetype.org). All rights reserved. FreeType is used here under
//! the FreeType License (`FTL.TXT`).
//!
//! The translation keeps FreeType's file layout (one Rust module per C
//! file, under the C component's directory), function order, and
//! comments; the identifiers of the C structures' fields are kept as
//! well, which is why non-snake-case names are allowed in this module.

#![allow(non_snake_case, non_upper_case_globals, dead_code, unused_assignments)]
// (the translation follows the C code's loops, branches, and arithmetic)
#![allow(
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::explicit_counter_loop,
    clippy::if_same_then_else,
    clippy::implicit_saturating_sub,
    clippy::manual_checked_ops,
    clippy::manual_is_multiple_of,
    clippy::manual_range_contains,
    clippy::manual_range_patterns,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unnecessary_unwrap
)]

pub mod base;
pub mod ftimage;
pub mod fttypes;
pub mod gzip;
pub mod psnames;
pub mod sfnt;
pub mod smooth;
pub mod truetype;
pub mod tttables;
pub mod tttypes;
