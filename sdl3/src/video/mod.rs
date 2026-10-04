// Rust translation of src/video/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Video: displays and windows, geometry, pixel formats and software
//! surfaces.
//!
//! Upstream `src/video/` is the largest directory in SDL (window management,
//! ~20 platform backends, software blitters, YUV conversion, surfaces...).
//! Translated so far are the platform-independent parts: the video core
//! ([`window`], [`display`], [`clipboard`], [`messagebox`], text input, the
//! OpenGL/Vulkan/Metal front ends) with the dummy and offscreen drivers;
//! [`rect`], [`pixels`], [`blendmode`] and [`surface`] (blitting,
//! conversion including YUV, fill, stretch, RLE, rotation, BMP files). The
//! platform backends (X11, Wayland, Windows, Cocoa...) come with the
//! platform layer.

pub mod blendmode;
pub(crate) mod blit;
mod bmp;
pub mod clipboard;
pub(crate) mod core;
pub mod display;
pub(crate) mod drivers;
pub mod gl;
pub(crate) mod image;
pub mod messagebox;
pub mod pixels;
pub mod rect;
pub(crate) mod rle;
pub(crate) mod rotate;
pub(crate) mod stb;
mod stretch;
pub mod surface;
pub mod sysvideo;
pub mod textinput;
pub mod vulkan;
pub mod window;
mod yuv;

pub use self::core::{
    current_video_driver, disable_screen_saver, enable_screen_saver, num_video_drivers,
    screen_saver_enabled, system_theme, video_driver,
};
pub use display::*;
pub use sysvideo::{
    windowpos_centered_display, windowpos_is_centered, windowpos_is_undefined,
    windowpos_undefined_display, DisplayMode, DisplayOrientation, FlashOperation, HitTest,
    HitTestResult, ProgressState, SystemTheme, WINDOWPOS_CENTERED, WINDOWPOS_CENTERED_MASK,
    WINDOWPOS_UNDEFINED, WINDOWPOS_UNDEFINED_MASK,
};
pub use window::{grabbed_window, windows, Window, WindowBuilder, WindowSurface};

pub use blendmode::*;
pub use bmp::is_bmp;
pub use pixels::*;
pub use rect::*;
pub use stb::{is_jpg, is_png};
pub use surface::*;

#[cfg(test)]
mod tests;
