// Rust translation of src/video/emscripten/SDL_emscriptenmouse.c and
// SDL_emscriptenmouse.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The mouse: cursors are CSS `cursor` values of `Module['canvas']` (a
//! system cursor's CSS name, or a custom cursor's PNG data URL), relative
//! mode is the Pointer Lock API, and the global mouse state is what
//! listeners on the document saw last.

use std::ffi::CString;

use sdl3_emscripten::sys;
use sdl3_emscripten::video::mouse as js;

use crate::error::{Error, Result};
use crate::events::mouse::{self, Cursor, MouseButtonFlags, SystemCursor};
use crate::video::core::css_cursor_name;
use crate::video::{PixelFormat, Surface};

/// A cursor's data. Translation of `struct SDL_CursorData` (a custom
/// cursor's string is Rust's: nothing to free by hand).
pub(super) struct CursorData {
    /// The CSS `cursor` value (`system_cursor`), if any.
    system_cursor: Option<CString>,
    /// (whether the string is a custom cursor's, not a CSS name)
    #[allow(dead_code)]
    is_custom: bool,
}

/// Translation of `Emscripten_CreateCursorFromString()`.
fn create_cursor_from_string(cursor_str: Option<&str>, is_custom: bool) -> Cursor {
    Cursor::with_internal(CursorData {
        system_cursor: cursor_str.map(super::c_string),
        is_custom,
    })
}

/// Translation of `Emscripten_CreateDefaultCursor()`.
fn create_default_cursor() -> Cursor {
    let id = mouse::default_system_cursor();
    let (cursor_name, _) = css_cursor_name(id);
    create_cursor_from_string(Some(cursor_name), false)
}

/// Translation of `Emscripten_CreateCursor()`.
pub(super) fn create_cursor(surface: &Surface<'_>, hot_x: i32, hot_y: i32) -> Result<Cursor> {
    let conv_surf = surface.convert(PixelFormat::RGBA32)?;
    let Some(pixels) = conv_surf.pixels() else {
        return Err(Error::new("Couldn't convert the cursor's pixels"));
    };

    // SAFETY: the surface's w*h RGBA32 pixels (packed rows), which the
    // JavaScript copies; it returns a string from SDL_malloc().
    let cursor_url = unsafe {
        js::sdl3_create_cursor(
            surface.width(),
            surface.height(),
            hot_x,
            hot_y,
            pixels.as_ptr().cast(),
        )
    };
    let cursor_url = super::take_sdl_string(cursor_url);

    Ok(create_cursor_from_string(cursor_url.as_deref(), true))
}

/// Translation of `Emscripten_CreateSystemCursor()`.
pub(super) fn create_system_cursor(id: SystemCursor) -> Cursor {
    let (cursor_name, _) = css_cursor_name(id);

    create_cursor_from_string(Some(cursor_name), false)
}

/// Translation of `Emscripten_ShowCursor()` (`Emscripten_FreeCursor()` is
/// the cursor's drop).
pub(super) fn show_cursor(cursor: Option<&Cursor>) {
    if mouse::mouse_focus().is_some() {
        if let Some(curdata) = cursor.and_then(|c| c.internal::<CursorData>()) {
            if let Some(system_cursor) = &curdata.system_cursor {
                // SAFETY: a NUL-terminated string.
                unsafe { js::sdl3_show_cursor(system_cursor.as_ptr()) };
            }
        } else {
            // SAFETY: plain JavaScript.
            unsafe { js::sdl3_hide_cursor() };
        }
    }
}

/// Translation of `Emscripten_SetRelativeMouseMode()`.
pub(super) fn set_relative_mouse_mode(enabled: bool) -> Result<()> {
    // TODO: pointer lock isn't actually enabled yet
    if enabled {
        let Some(window) = mouse::mouse_focus() else {
            return Err(Error::new("No window has the mouse focus"));
        };

        let Some(canvas_id) = super::canvas_id(window) else {
            return Err(Error::new("Invalid window"));
        };

        // SAFETY: a NUL-terminated selector.
        if unsafe { sys::emscripten_request_pointerlock(canvas_id.as_ptr(), true) }
            >= sys::EMSCRIPTEN_RESULT_SUCCESS
        {
            return Ok(());
        }
    } else {
        // SAFETY: a plain call.
        if unsafe { sys::emscripten_exit_pointerlock() } >= sys::EMSCRIPTEN_RESULT_SUCCESS {
            return Ok(());
        }
    }
    // (upstream returns false without an error message)
    Err(Error::new(if enabled {
        "Couldn't request the pointer lock"
    } else {
        "Couldn't exit the pointer lock"
    }))
}

/// Translation of `Emscripten_GetGlobalMouseState()`.
pub(super) fn global_mouse_state() -> (f32, f32, MouseButtonFlags) {
    // SAFETY: plain JavaScript (on what init_mouse() made).
    let (x, y) = unsafe { (js::sdl3_global_mouse_x(), js::sdl3_global_mouse_y()) };
    let mut flags = MouseButtonFlags::NONE;
    for i in 0..5 {
        // SAFETY: as above.
        let button_down = unsafe { js::sdl3_global_mouse_button(i) } != 0;
        if button_down {
            flags |= MouseButtonFlags(1 << i);
        }
    }
    (x as f32, y as f32, flags)
}

/// Translation of `Emscripten_InitMouse()` (the entry points are the
/// driver's).
pub(super) fn init_mouse() {
    // Add event listeners to track mouse events on the document
    // SAFETY: plain JavaScript.
    unsafe { js::sdl3_init_mouse() };

    mouse::set_default_cursor(Some(create_default_cursor()));
}

/// Translation of `Emscripten_QuitMouse()`.
pub(super) fn quit_mouse() {}
