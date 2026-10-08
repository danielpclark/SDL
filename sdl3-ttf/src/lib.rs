// Rust translation of include/SDL3_ttf/SDL_ttf.h from SDL_ttf.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-ttf — SDL_ttf, translated to Rust
//!
//! The pure-Rust translation of [SDL_ttf](https://github.com/libsdl-org/SDL_ttf)
//! 3 (release 3.2.2), the TrueType font rendering library for SDL3,
//! together with the FreeType it bundles (2.13.2). No C code is built,
//! linked or loaded.
//!
//! Translated: `SDL_ttf.c` as upstream builds it with its bundled
//! FreeType and without HarfBuzz and PlutoSVG: fonts from files and
//! streams (TrueType and OpenType with TrueType outlines, collections,
//! WOFF, variable fonts), sizes and DPI, styles, outlines, hinting (the
//! TrueType bytecode interpreter and the auto-hinter, light and LCD
//! modes), kerning, metrics, fallback fonts, text measuring and wrapping,
//! rendering in every mode (solid, shaded, blended, LCD) and glyph images;
//! text objects ([`Text`]: layout, clusters, substrings, editing) with the
//! surface ([`SurfaceTextEngine`]), renderer ([`RendererTextEngine`],
//! with stb_rect_pack) and GPU ([`GpuTextEngine`]) text engines. Of
//! FreeType, the modules for those fonts: the base layer (with the glyph
//! and stroker APIs), `sfnt`, `truetype`, `psnames`, `autofit`, `smooth`,
//! `raster` and `gzip` (with its zlib).
//!
//! Not translated yet: the HarfBuzz paths (part 2: HarfBuzz; without them
//! text is laid out left to right, and setting a script or language is
//! unsupported, as in such a C build), and FreeType's other modules: the
//! drivers of the other font formats (`cff`, `type1`, `cid`, `type42`,
//! `pfr`, `winfnt`, `pcf`, `bdf`, with `psaux`, `pshinter` and `lzw`), the
//! signed distance field renderers (`sdf`; SDF rendering fails) and the
//! OT-SVG renderer (`svg`, which needs PlutoSVG's hooks anyway).

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

#[doc(hidden)]
pub mod freetype;

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
