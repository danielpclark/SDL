// Rust translation of include/SDL3_ttf/SDL_ttf.h from SDL_ttf.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-ttf — SDL_ttf, translated to Rust
//!
//! The pure-Rust translation of [SDL_ttf](https://github.com/libsdl-org/SDL_ttf)
//! 3 (release 3.2.2), the TrueType font rendering library for SDL3,
//! together with the FreeType (2.13.2), HarfBuzz (8.5.0), PlutoVG (0.0.12)
//! and PlutoSVG (0.0.6) it bundles.
//! No C or C++ code is built, linked or loaded.
//!
//! Translated: `SDL_ttf.c` as upstream builds it by default, with its
//! bundled FreeType, HarfBuzz and PlutoSVG: fonts from files and
//! streams (TrueType, OpenType with TrueType or CFF outlines, CFF2 and
//! bare CFF fonts, collections, WOFF, variable fonts, Type 1, CID-keyed
//! Type 1, Type 42 and PFR fonts, and Windows FNT/FON, PCF and BDF bitmap
//! fonts), sizes and DPI,
//! styles, outlines, hinting (the TrueType bytecode interpreter, Adobe's
//! CFF engine and the auto-hinter, light and LCD modes), kerning, metrics, fallback fonts, text measuring and wrapping,
//! rendering in every mode (solid, shaded, blended, LCD) and glyph images,
//! OpenType SVG color glyphs;
//! text objects ([`Text`]: layout, clusters, substrings, editing) with the
//! surface ([`SurfaceTextEngine`]), renderer ([`RendererTextEngine`],
//! with stb_rect_pack) and GPU ([`GpuTextEngine`]) text engines. Of
//! FreeType, the modules for those fonts: the base layer (with the glyph
//! and stroker APIs), `sfnt`, `truetype`, `cff`, `psaux` (its CFF parts
//! and the Adobe CFF engine), `pshinter` (its global hints), `psnames`,
//! `autofit`, `smooth`, `raster`, `sdf` (signed distance fields, from
//! outlines and bitmaps), `gzip` (with its zlib), `lzw`, the `bzip2`
//! stub SDL_ttf builds, and the drivers `type1`, `cid`, `type42`, `pfr`,
//! `winfonts`, `pcf` and `bdf` (with the Type 1 parts of `psaux`), and
//! the OT-SVG renderer `svg` with PlutoSVG's hooks. Of PlutoSVG, its SVG
//! parser and renderer and FreeType hooks; of PlutoVG, the vector graphics
//! PlutoSVG draws with (paths, stroking and dashing, the FreeType-derived
//! rasterizer, paints, gradients and textures, compositing; not its text
//! and font functions, which PlutoSVG does not use). Of
//! HarfBuzz, what
//! SDL_ttf's build compiles and uses for text shaping (the font's or
//! text's direction, script and language, [`glyph_script`]): the buffer,
//! its own Unicode functions (the UCD tables), the OpenType layout engine
//! (GSUB, GPOS, GDEF, the `kern` table, feature and language selection),
//! Apple Advanced Typography (`morx`, `mort`, `kerx`, `trak`, `ankr`,
//! `feat`, `ltag`, and the state-machine `kern` subtables), the OpenType
//! shaper with all its shapers (default, Arabic with its
//! fallback shaping, Hebrew, Indic, Khmer, Myanmar and Zawgyi, Thai and
//! Lao, Hangul, and the Universal Shaping Engine) and fallback mark
//! positioning and kerning, the FreeType integration (`hb-ft`), and the
//! OpenType font functions FreeType's auto-hinter uses (FreeType is built
//! with HarfBuzz, as SDL_ttf builds it: the auto-hinter finds the glyphs
//! of OpenType features such as small capitals and superscripts).
//!
//! Not translated yet: the hinter of `pshinter` (which only the old Type 1
//! and CFF interpreters SDL_ttf's build leaves out use). SVG `image`
//! elements are decoded with SDL's stb_image (PNG and JPEG; PlutoVG's own
//! copy of stb_image also reads its other formats, which are not decoded
//! here).

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

#[doc(hidden)]
pub mod freetype;
#[doc(hidden)]
pub mod harfbuzz;
#[doc(hidden)]
pub mod plutosvg;
#[doc(hidden)]
pub mod plutovg;

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
