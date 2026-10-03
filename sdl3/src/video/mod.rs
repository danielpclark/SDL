// Rust translation of src/video/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Video: geometry, pixel formats and software surfaces.
//!
//! Upstream `src/video/` is the largest directory in SDL (window management,
//! ~20 platform backends, software blitters, YUV conversion, surfaces...).
//! Translated so far are the platform-independent parts: [`rect`],
//! [`pixels`], [`blendmode`] and [`surface`] (blitting, conversion including
//! YUV, fill, stretch, RLE, rotation, BMP files). Windows, displays and the
//! backends come with the platform layer.

pub mod blendmode;
pub(crate) mod blit;
mod bmp;
pub mod pixels;
pub mod rect;
pub(crate) mod rle;
pub(crate) mod rotate;
mod stb;
mod stretch;
pub mod surface;
mod yuv;

pub use blendmode::*;
pub use bmp::is_bmp;
pub use pixels::*;
pub use rect::*;
pub use stb::{is_jpg, is_png};
pub use surface::*;
