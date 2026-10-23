// Rust translation of the parts of src/video/emscripten/SDL_emscriptenevents.c
// and SDL_emscriptenevents.h that the video driver needs, from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The DOM events the video driver itself needs: the window's `resize`
//! (fill-document, fullscreen and resizable canvases follow the page),
//! the document's `fullscreenchange` (the end of a fullscreen request) and
//! `pointerlockchange`, and the canvas resize callback of a fullscreen
//! request ([`handle_canvas_resize`]).
//!
//! The rest of `SDL_emscriptenevents.c` comes with the Emscripten port's
//! event handling (PR 8): the keyboard (with the lock-key check and the
//! DOM-code scancodes), pointer (mouse, touch and pen), wheel, focus and
//! blur, the global `mouseup`, visibility, `beforeunload`, orientation and
//! drag-and-drop handlers. Until then [`register_event_handlers`] and
//! [`unregister_event_handlers`] set up nothing, and the global ones only
//! what is here.
//!
//! Upstream passes its handlers the `SDL_VideoDevice *` (the global ones,
//! which walk its windows) or the `SDL_WindowData *` as their user data;
//! here the global ones walk the video core's windows and the canvas
//! resize callback gets the window's ID.

use std::ffi::{c_int, c_void, CStr};
use std::ptr;

use sdl3_emscripten::sys::{
    self, EmscriptenFullscreenChangeEvent, EmscriptenPointerlockChangeEvent, EmscriptenUiEvent,
    EMSCRIPTEN_EVENT_TARGET_DOCUMENT, EMSCRIPTEN_EVENT_TARGET_WINDOW,
    EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
};

use super::{fill_document_window, with_data, EmscriptenWindow};
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{EventType, WindowID};
use crate::stdlib::math::lroundf;
use crate::video::core::{update_fullscreen_mode, window_ids, with_window};
use crate::video::sysvideo::FullscreenOp;

/// Translation of `Emscripten_HandlePointerLockChange()`.
fn handle_pointer_lock_change(
    window: WindowID,
    change_event: &EmscriptenPointerlockChangeEvent,
) -> bool {
    // keep track of lock losses, so we can regrab if/when appropriate.
    with_data(window, |d| d.has_pointer_lock = change_event.isActive);
    false
}

/// Translation of `Emscripten_HandlePointerLockChangeGlobal()`.
unsafe extern "C" fn handle_pointer_lock_change_global(
    _event_type: c_int,
    change_event: *const EmscriptenPointerlockChangeEvent,
    _user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten's event, for the call.
    let change_event = unsafe { &*change_event };
    let mut prevent_default = false;

    for window in window_ids() {
        prevent_default |= handle_pointer_lock_change(window, change_event);
    }

    prevent_default
}

/// Translation of `Emscripten_HandleFullscreenChange()`.
fn handle_fullscreen_change(
    window: WindowID,
    fullscreen_change_event: &EmscriptenFullscreenChangeEvent,
) -> bool {
    with_data(window, |d| d.fullscreen_change_in_progress = false);

    if fullscreen_change_event.isFullscreen {
        send_window_event(window, EventType::WINDOW_ENTER_FULLSCREEN, 0, 0);
        with_data(window, |d| d.fullscreen_mode_flags = WindowFlags::NONE);
    } else {
        send_window_event(window, EventType::WINDOW_LEAVE_FULLSCREEN, 0, 0);
    }

    let _ = update_fullscreen_mode(
        window,
        if fullscreen_change_event.isFullscreen {
            FullscreenOp::Enter
        } else {
            FullscreenOp::Leave
        },
        false,
    );

    false
}

/// Translation of `Emscripten_HandleFullscreenChangeGlobal()`.
unsafe extern "C" fn handle_fullscreen_change_global(
    _event_type: c_int,
    fullscreen_change_event: *const EmscriptenFullscreenChangeEvent,
    _user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten's event, for the call.
    let fullscreen_change_event = unsafe { &*fullscreen_change_event };
    // SAFETY: emscripten writes a NUL-terminated id (cut to the array).
    let id = unsafe { CStr::from_ptr(fullscreen_change_event.id.as_ptr()) };

    let window = window_ids().into_iter().find(|&window| {
        with_data(window, |d| {
            let canvas_id = d.canvas_id.to_bytes();
            let canvas_id = canvas_id.strip_prefix(b"#").unwrap_or(canvas_id);
            canvas_id == id.to_bytes() // this is the window.
        })
        .unwrap_or(false)
    });

    if let Some(window) = window {
        return handle_fullscreen_change(window, fullscreen_change_event);
    }

    false
}

/// Translation of `Emscripten_HandleResize()`.
fn handle_resize(window: WindowID, ui_event: &EmscriptenUiEvent) -> bool {
    let Some((pixel_ratio, external_size, canvas_id)) = with_data(window, |d| {
        (d.pixel_ratio, d.external_size, d.canvas_id.clone())
    }) else {
        return false;
    };
    let Ok((flags, window_w, window_h)) = with_window(window, |w| (w.flags(), w.core.w, w.core.h))
    else {
        return false;
    };

    let mut force = false;
    let mut pixel_ratio = pixel_ratio;

    // update pixel ratio
    if flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
        // SAFETY: a plain call.
        let ratio = unsafe { sys::emscripten_get_device_pixel_ratio() };
        if f64::from(pixel_ratio) != ratio {
            pixel_ratio = ratio as f32;
            with_data(window, |d| d.pixel_ratio = pixel_ratio);
            force = true;
        }
    }

    let fill_document = fill_document_window() == Some(window);
    let fullscreen = flags.contains(WindowFlags::FULLSCREEN); // fullscreen windows can resize on Emscripten, and the canvas should fill it.
    let resizable = flags.contains(WindowFlags::RESIZABLE);
    if fill_document || fullscreen || resizable {
        let (w, h) = if fill_document || fullscreen {
            (
                f64::from(ui_event.windowInnerWidth),
                f64::from(ui_event.windowInnerHeight),
            )
        } else {
            debug_assert!(flags.contains(WindowFlags::RESIZABLE));
            // this will only work if the canvas size is set through css
            if external_size {
                super::element_css_size(&canvas_id)
            } else {
                (f64::from(window_w), f64::from(window_h))
            }
        };

        // (double times float: double arithmetic, as C's)
        super::set_canvas_element_size(
            &canvas_id,
            lroundf((w * f64::from(pixel_ratio)) as f32),
            lroundf((h * f64::from(pixel_ratio)) as f32),
        );

        // set_canvas_size unsets this
        if !external_size && pixel_ratio != 1.0 {
            super::set_element_css_size(&canvas_id, w, h);
        }

        if force {
            // force the event to trigger, so pixel ratio changes can be handled
            let _ = with_window(window, |w| {
                w.core.w = 0;
                w.core.h = 0;
            });
        }

        send_window_event(
            window,
            EventType::WINDOW_RESIZED,
            lroundf(w as f32) as i32,
            lroundf(h as f32) as i32,
        );
    }

    false
}

/// Translation of `Emscripten_HandleResizeGlobal()`.
unsafe extern "C" fn handle_resize_global(
    _event_type: c_int,
    ui_event: *const EmscriptenUiEvent,
    _user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten's event, for the call.
    let ui_event = unsafe { &*ui_event };
    let mut prevent_default = false;

    for window in window_ids() {
        prevent_default |= handle_resize(window, ui_event);
    }

    prevent_default
}

/// The canvas resize callback of a fullscreen request (the window's ID is
/// the user data). Translation of `Emscripten_HandleCanvasResize()`.
pub(super) unsafe extern "C" fn handle_canvas_resize(
    _event_type: c_int,
    _reserved: *const c_void,
    user_data: *mut c_void,
) -> bool {
    // this is used during fullscreen changes
    let window = user_data as usize as WindowID;

    if let Some((true, canvas_id)) =
        with_data(window, |d| (d.fullscreen_resize, d.canvas_id.clone()))
    {
        let (css_w, css_h) = super::element_css_size(&canvas_id);
        send_window_event(
            window,
            EventType::WINDOW_RESIZED,
            lroundf(css_w as f32) as i32,
            lroundf(css_h as f32) as i32,
        );
    }

    false
}

/// Translation of `Emscripten_RegisterGlobalEventHandlers()`: the
/// `pointerlockchange`, `fullscreenchange` and `resize` handlers (the
/// global `mouseup`, `focus` and `blur` ones come with the event handling,
/// PR 8).
pub(super) fn register_global_event_handlers() {
    // SAFETY: the special targets, and handlers with emscripten's
    // signatures, which use no user data.
    unsafe {
        sys::emscripten_set_pointerlockchange_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_DOCUMENT,
            ptr::null_mut(),
            false,
            Some(handle_pointer_lock_change_global),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_fullscreenchange_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_DOCUMENT,
            ptr::null_mut(),
            false,
            Some(handle_fullscreen_change_global),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_resize_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_WINDOW,
            ptr::null_mut(),
            false,
            Some(handle_resize_global),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );
    }
}

/// Translation of `Emscripten_UnregisterGlobalEventHandlers()` (what
/// [`register_global_event_handlers`] set up).
pub(super) fn unregister_global_event_handlers() {
    // SAFETY: the special targets; no handlers.
    unsafe {
        sys::emscripten_set_pointerlockchange_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_DOCUMENT,
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_fullscreenchange_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_DOCUMENT,
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_resize_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_WINDOW,
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );
    }
}

/// `Emscripten_RegisterEventHandlers()`: a stub until the event handling
/// (PR 8 of the Emscripten port), which registers the window's wheel,
/// orientation, keyboard, visibility, `beforeunload`, pointer and drag
/// handlers.
pub(super) fn register_event_handlers(_window: WindowID) {}

/// `Emscripten_UnregisterEventHandlers()`: a stub, as
/// [`register_event_handlers`].
pub(super) fn unregister_event_handlers(_data: &EmscriptenWindow) {}
