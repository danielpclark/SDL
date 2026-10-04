// The video drivers that need no platform support.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Video backends: X11 (on Unix other than Apple platforms), and the dummy
//! and offscreen drivers, both only available when requested with
//! [`hints::VIDEO_DRIVER`](crate::hints::VIDEO_DRIVER).

pub(crate) mod dummy;
pub(crate) mod offscreen;
#[cfg(all(unix, not(target_vendor = "apple")))]
pub(crate) mod x11;

use std::sync::atomic::{AtomicI32, Ordering};

use crate::error::Result;
use crate::events::WindowID;
use crate::video::{PixelFormat, Surface};

/// A zeroed XRGB8888 framebuffer of `w`x`h` pixels (`SDL_CreateSurfaceZeroed()`
/// in the dummy and offscreen `CreateWindowFramebuffer`).
pub(crate) fn create_xrgb8888_framebuffer(w: i32, h: i32) -> Result<Surface<'static>> {
    Surface::new(w, h, PixelFormat::XRGB8888)
}

/// Save a framebuffer as `SDL_window<id>-<frame>.bmp` when `hint` is set
/// (the dummy and offscreen `UpdateWindowFramebuffer`).
pub(crate) fn maybe_save_frame(
    hint: &str,
    frame_number: &AtomicI32,
    window: WindowID,
    surface: &Surface<'static>,
) -> Result<()> {
    // Send the data to the display
    if crate::hints::get_bool(hint, false) {
        let n = frame_number.fetch_add(1, Ordering::Relaxed) + 1;
        let file = format!("SDL_window{window}-{n:08}.bmp");
        surface.duplicate()?.save_bmp(file)?;
    }
    Ok(())
}
