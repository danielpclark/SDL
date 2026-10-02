// Rust translation of src/video/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Video subsystem: geometry primitives and pixel formats so far.
//!
//! Upstream `src/video/` is the largest directory in SDL (window management,
//! ~20 platform backends, software blitters, YUV conversion, surfaces...).
//! This first phase translates the platform-independent building blocks that
//! everything else is expressed in terms of: [`rect`] and [`pixels`].

pub mod blendmode;
pub(crate) mod blit;
pub mod pixels;
pub mod rect;
pub(crate) mod rle;
mod stretch;
pub mod surface;

pub use blendmode::*;
pub use pixels::*;
pub use rect::*;
pub use surface::*;
