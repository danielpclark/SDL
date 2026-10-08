// Rust translation of include/SDL3_ttf/SDL_ttf.h from SDL_ttf.
// Copyright (C) 2001-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-ttf — SDL_ttf, translated to Rust
//!
//! The pure-Rust translation of [SDL_ttf](https://github.com/libsdl-org/SDL_ttf)
//! 3, the TrueType font rendering library for SDL3, together with the
//! FreeType it bundles.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

#[doc(hidden)]
pub mod freetype;
