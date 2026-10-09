// sdl3-rtf: the Rust translation of SDL_rtf.
// Copyright (C) 2003-2024 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-rtf — SDL_rtf, translated to Rust
//!
//! The pure-Rust translation of [SDL_rtf](https://github.com/libsdl-org/SDL_rtf)
//! 3 (its `main` branch, [`REVISION`]), "a companion library to SDL for
//! displaying RTF format text": it reads simple Rich Text Format
//! documents, lays them out at a given width and renders them with an
//! [`sdl3::render::Renderer`]. It is a separate crate over [`sdl3`], as
//! SDL_rtf is a separate library over SDL, and renders its text through
//! [`sdl3_ttf`] fonts. No C code is built, linked or loaded.
//!
//! Translated: `SDL_rtf.c` (the API), `SDL_rtfreadr.c` (the layout:
//! word wrapping, indents, tab stops, left, right and centered
//! paragraphs, and rendering), and the RTF reader that SDL_rtf adapted
//! from the sample code of Microsoft's RTF specification 1.6
//! (`rtfreadr.c`, `rtfactn.c`, `rtftype.h`, `rtfdecl.h`): groups, control
//! words and symbols, the font and color tables, the document's title,
//! subject and author, character properties (font, size, bold, italic,
//! underline, color), paragraph properties (indents, alignment), `\'`
//! hex escapes, `\bin` data and skipped destinations (pictures, headers,
//! footers, footnotes, style sheets and unknown `\*` destinations). As
//! upstream's, the reader has no Unicode escapes (`\u` is skipped and its
//! fallback character shown), and no tables or images.
//!
//! The text is measured and drawn through a [`FontEngine`], which the app
//! provides (upstream's `RTF_FontEngine`); [`TtfFontEngine`] is the one
//! of upstream's `showrtf` example, over SDL_ttf, and `examples/showrtf.rs`
//! is the example itself.
//!
//! ```no_run
//! # use std::{cell::RefCell, rc::Rc};
//! # fn main() -> sdl3::Result<()> {
//! # let surface = sdl3::video::Surface::new(640, 480, sdl3::video::PixelFormat::ARGB8888)?;
//! # let renderer = Rc::new(RefCell::new(sdl3::render::Renderer::software(surface)?));
//! use sdl3_rtf::{Context, FontSource, TtfFontEngine};
//!
//! sdl3_ttf::init()?;
//! let engine = TtfFontEngine::new(FontSource::File("DejaVuSans.ttf".into()));
//! let mut ctx = Context::new(renderer.clone(), engine)?;
//! ctx.load("document.rtf")?;
//! let height = ctx.height(640);
//! println!("{}: {height} pixels high", ctx.title());
//! ctx.render(None, 0);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

mod rtf;
mod rtfactn;
mod rtfdecl;
mod rtfreadr;
mod rtftype;
mod sdl_rtfreadr;
mod showrtf;

pub use rtf::*;
pub use rtftype::Context;
pub use showrtf::*;

#[cfg(test)]
mod tests;
