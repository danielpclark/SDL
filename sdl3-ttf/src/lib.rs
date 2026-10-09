// Rust translation of include/SDL3_ttf/SDL_ttf.h from SDL_ttf.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-ttf — SDL_ttf, translated to Rust
//!
//! The pure-Rust translation of [SDL_ttf](https://github.com/libsdl-org/SDL_ttf)
//! 3 (release 3.2.2), the TrueType font rendering library for SDL3,
//! together with the FreeType (2.13.2) and HarfBuzz (8.5.0) it bundles.
//! No C or C++ code is built, linked or loaded.
//!
//! Translated: `SDL_ttf.c` as upstream builds it by default, with its
//! bundled FreeType and HarfBuzz and without PlutoSVG: fonts from files and
//! streams (TrueType, OpenType with TrueType or CFF outlines, CFF2 and
//! bare CFF fonts, collections, WOFF, variable fonts), sizes and DPI,
//! styles, outlines, hinting (the TrueType bytecode interpreter, Adobe's
//! CFF engine and the auto-hinter, light and LCD modes), kerning, metrics, fallback fonts, text measuring and wrapping,
//! rendering in every mode (solid, shaded, blended, LCD) and glyph images;
//! text objects ([`Text`]: layout, clusters, substrings, editing) with the
//! surface ([`SurfaceTextEngine`]), renderer ([`RendererTextEngine`],
//! with stb_rect_pack) and GPU ([`GpuTextEngine`]) text engines. Of
//! FreeType, the modules for those fonts: the base layer (with the glyph
//! and stroker APIs), `sfnt`, `truetype`, `cff`, `psaux` (its CFF parts
//! and the Adobe CFF engine), `pshinter` (its global hints), `psnames`,
//! `autofit`, `smooth`, `raster`, `sdf` (signed distance fields, from
//! outlines and bitmaps) and `gzip` (with its zlib). Of HarfBuzz, what
//! SDL_ttf's build compiles and uses for text shaping (the font's or
//! text's direction, script and language, [`glyph_script`]): the buffer,
//! its own Unicode functions (the UCD tables), the OpenType layout engine
//! (GSUB, GPOS, GDEF, the `kern` table, feature and language selection),
//! the OpenType shaper with all its shapers (default, Arabic with its
//! fallback shaping, Hebrew, Indic, Khmer, Myanmar and Zawgyi, Thai and
//! Lao, Hangul, and the Universal Shaping Engine) and fallback mark
//! positioning and kerning, and the FreeType integration (`hb-ft`).
//!
//! Not translated yet: HarfBuzz's AAT layout (`morx`, `kerx`, `trak`
//! tables, treated as absent) and the state-machine `kern` subtables
//! (format 1, which are skipped), and FreeType's other modules: the
//! drivers of the other font formats (`type1`, `cid`, `type42`, `pfr`,
//! `winfnt`, `pcf`, `bdf`, with the Type 1 parts of `psaux`, the hinter of
//! `pshinter`, which only their old interpreters use, and `lzw`) and the
//! OT-SVG renderer (`svg`, which needs PlutoSVG's hooks anyway).

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

#[doc(hidden)]
pub mod freetype;
#[doc(hidden)]
pub mod harfbuzz;

mod gpu_textengine;
mod qsort;
mod renderer_textengine;
mod stb_rect_pack;
mod surface_textengine;
mod text;
mod ttf;

pub use gpu_textengine::*;
pub use renderer_textengine::*;
pub use surface_textengine::*;
pub use text::*;
pub use ttf::*;

#[cfg(test)]
mod tests;
