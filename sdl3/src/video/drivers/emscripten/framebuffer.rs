// Rust translation of src/video/emscripten/SDL_emscriptenframebuffer.c and
// SDL_emscriptenframebuffer.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The window framebuffer: an RGBA32 surface whose pixels go to the canvas
//! with `putImageData()` on its 2D context.

use sdl3_emscripten::sys;
use sdl3_emscripten::video::framebuffer as js;

use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::video::{PixelFormat, Rect, Surface};

/// Translation of `Emscripten_CreateWindowFramebuffer()`: a zeroed RGBA32
/// surface of the window's size in pixels, `w`x`h` (the video core
/// passes `SDL_GetWindowSizeInPixels()`'s, and owns the surface: the old
/// one is gone when it asks for a new one).
pub(super) fn create_window_framebuffer(
    _window: WindowID,
    w: i32,
    h: i32,
) -> Result<Surface<'static>> {
    let surface_format = PixelFormat::RGBA32;

    // Create a new one
    Surface::new(w, h, surface_format)
}

/// Translation of `Emscripten_UpdateWindowFramebuffer()` (the whole
/// surface, whatever the rectangles).
pub(super) fn update_window_framebuffer(
    window: WindowID,
    surface: &Surface<'static>,
    _rects: &[Rect],
) -> Result<()> {
    let Some(canvas_id) = super::canvas_id(window) else {
        return Err(Error::new("Couldn't find framebuffer surface for window"));
    };
    let Some(pixels) = surface.pixels() else {
        return Err(Error::new("Couldn't find framebuffer surface for window"));
    };

    // Send the data to the display

    // SAFETY: the surface's w*h RGBA32 pixels (its rows are packed: the
    // pitch of a 4-byte format is w*4), which the JavaScript copies, and
    // a NUL-terminated selector.
    let updated = unsafe {
        js::sdl3_update_window_framebuffer(
            surface.width(),
            surface.height(),
            pixels.as_ptr().cast(),
            canvas_id.as_ptr(),
        )
    };

    if updated == 0 {
        return Err(Error::new("Couldn't create context for canvas update"));
    }

    // SAFETY: plain calls (emscripten_sleep() only with Asyncify).
    unsafe {
        if sys::emscripten_has_asyncify() != 0 && hints::get_bool(hints::EMSCRIPTEN_ASYNCIFY, true)
        {
            // give back control to browser for screen refresh
            sys::emscripten_sleep(0);
        }
    }

    Ok(())
}

/// Translation of `Emscripten_DestroyWindowFramebuffer()`: the video core
/// drops the surface.
pub(super) fn destroy_window_framebuffer(_window: WindowID) {}
