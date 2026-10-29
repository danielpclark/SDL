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
// (SDL_VIDEO_OPENGL_EGL: not on Emscripten or UIKit)
#[cfg(any(
    windows,
    target_os = "macos",
    all(unix, not(target_vendor = "apple"), not(target_os = "emscripten"))
))]
pub(crate) mod egl;
pub mod gl;
pub(crate) mod image;
pub mod messagebox;
pub mod pixels;
pub mod rect;
pub(crate) mod rle;
pub(crate) mod rotate;
pub(crate) mod stb;

/// The stb_image decoder SDL bundles (PNG and JPEG, scalar kernels), shared
/// with the satellite libraries: SDL_image builds its own copy of
/// `stb_image.h` with the same configuration (`IMG_stb.c`), and the
/// `sdl3-image` crate uses this one instead of a second translation. With
/// [`load_from_memory_all_formats`](stb_image::load_from_memory_all_formats),
/// stb_image's other formats too (BMP, GIF, HDR, PIC, PNM, PSD, TGA), as
/// PlutoVG builds its copy of the same release (for the `sdl3-ttf` crate's
/// SVG images).
pub mod stb_image {
    pub use super::image::stb_image::{
        load_from_callbacks, load_from_callbacks_with_palette, load_from_memory_all_formats,
        Callbacks, Image,
    };
}
mod stretch;
pub mod surface;
pub mod sysvideo;
pub mod textinput;
pub(crate) mod vk;
pub mod vulkan;
// (no video driver has Vulkan on Emscripten)
#[cfg_attr(target_os = "emscripten", allow(dead_code))]
pub(crate) mod vulkan_utils;
pub mod window;
mod yuv;

#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android"), not(target_os = "emscripten")))]
pub use drivers::x11::events::{set_x11_event_hook, X11EventHook};

pub use self::core::{
    current_video_driver, disable_screen_saver, enable_screen_saver, num_video_drivers,
    on_application_did_enter_background, on_application_did_enter_foreground,
    on_application_did_receive_memory_warning, on_application_will_enter_background,
    on_application_will_enter_foreground, on_application_will_terminate, screen_saver_enabled,
    system_theme, video_driver,
};
pub use display::*;
pub use sysvideo::{
    windowpos_centered_display, windowpos_is_centered, windowpos_is_undefined,
    windowpos_undefined_display, DisplayMode, DisplayOrientation, FlashOperation, HitTest,
    HitTestResult, ProgressState, SystemTheme, WINDOWPOS_CENTERED, WINDOWPOS_CENTERED_MASK,
    WINDOWPOS_UNDEFINED, WINDOWPOS_UNDEFINED_MASK,
};
pub use window::{grabbed_window, windows, Window, WindowBuilder, WindowSurface};

// The Windows-specific entry points of SDL_system.h
#[cfg(windows)]
pub use drivers::windows::events::{
    register_app, set_windows_message_hook, unregister_app, WindowsMessageHook,
};
#[cfg(windows)]
pub use drivers::windows::{get_direct3d9_adapter_index, get_dxgi_output_info};

// The iOS-specific entry points of SDL_system.h (on every UIKit platform)
#[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
pub use drivers::uikit::events::set_ios_event_pump;

/// Report that the interface orientation changed: the primary display's
/// desktop mode (and its fullscreen modes) are turned to match, and a
/// [`DISPLAY_ORIENTATION`](crate::events::EventType::DISPLAY_ORIENTATION)
/// event is sent. For iOS apps that have their own event handling; SDL
/// does this itself on UIKit's notification. Unsupported on tvOS and
/// visionOS. Translation of
/// `SDL_OnApplicationDidChangeStatusBarOrientation()`.
///
/// It is safe to call this function from any thread.
#[cfg(all(target_vendor = "apple", not(target_os = "macos")))]
pub fn on_application_did_change_status_bar_orientation() -> crate::error::Result<()> {
    #[cfg(not(any(target_os = "tvos", target_os = "visionos")))]
    {
        drivers::uikit::modes::on_application_did_change_status_bar_orientation();
        Ok(())
    }
    #[cfg(any(target_os = "tvos", target_os = "visionos"))]
    {
        Err(crate::error::Error::unsupported())
    }
}

pub use blendmode::*;
pub use bmp::is_bmp;
pub use pixels::*;
pub use rect::*;
pub use stb::{is_jpg, is_png};
pub use surface::*;

#[cfg(test)]
mod tests;
