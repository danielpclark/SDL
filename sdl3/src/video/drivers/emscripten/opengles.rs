// Rust translation of src/video/emscripten/SDL_emscriptenopengles.c and
// SDL_emscriptenopengles.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! OpenGL ES on Emscripten: WebGL contexts on the window's canvas, through
//! emscripten's WebGL API (`html5_webgl.h`), and the GL functions of
//! emscripten's GLES2-on-WebGL library (`emscripten_webgl_get_proc_address()`).
//! An OpenGL ES 3.0 context is WebGL 2 (a program linked with
//! `-sMAX_WEBGL_VERSION=2`; without it emscripten makes it WebGL 1).
//!
//! The swap interval is the main loop's timing (`requestAnimationFrame()`
//! for an interval of 1 or more, `setTimeout()` for 0): it can be set only
//! once a main loop runs, so until the first event pump it is kept and
//! reported (`should_set_swap_interval()`). Swapping is the
//! browser's, when the main loop's iteration returns; with Asyncify the
//! swap gives the browser a turn instead.

use std::ffi::{c_int, c_void, CString};
use std::ptr::NonNull;

use sdl3_emscripten::sys;

use super::{pending_swap_interval, should_set_swap_interval, with_data};
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::video::core::window_ids;
use crate::video::gl::{gl_config, RawGlContext};

/// The context's emscripten handle.
fn handle(context: Option<RawGlContext>) -> sys::EMSCRIPTEN_WEBGL_CONTEXT_HANDLE {
    context.map_or(0, |c| c.as_ptr() as sys::EMSCRIPTEN_WEBGL_CONTEXT_HANDLE)
}

/// Translation of `Emscripten_GLES_LoadLibrary()`.
pub(super) fn gles_load_library(_path: Option<&str>) -> Result<()> {
    Ok(())
}

/// Translation of `Emscripten_GLES_UnloadLibrary()`.
pub(super) fn gles_unload_library() {}

/// Translation of `Emscripten_GLES_GetProcAddress()`.
pub(super) fn gles_get_proc_address(proc_name: &str) -> Option<NonNull<c_void>> {
    // (a name with a NUL in it is no GL function)
    let proc_name = CString::new(proc_name).ok()?;
    // SAFETY: a NUL-terminated name.
    NonNull::new(unsafe { sys::emscripten_webgl_get_proc_address(proc_name.as_ptr()) })
}

/// Translation of `Emscripten_GLES_SetSwapInterval()`.
pub(super) fn gles_set_swap_interval(interval: i32) -> Result<()> {
    if interval < 0 {
        return Err(Error::new("Late swap tearing currently unsupported"));
    }

    if should_set_swap_interval(interval) {
        // don't change the mainloop timing if the app is also driving a main callback with this hint,
        //  as we assume that was the more deliberate action.
        if !crate::app::has_main_callbacks() || hints::get(hints::MAIN_CALLBACK_RATE).is_none() {
            // SAFETY: plain calls (with a main loop running: see
            // should_set_swap_interval()).
            unsafe {
                if interval == 0 {
                    sys::emscripten_set_main_loop_timing(sys::EM_TIMING_SETTIMEOUT, 0);
                } else {
                    sys::emscripten_set_main_loop_timing(sys::EM_TIMING_RAF, interval);
                }
            }
        }
    }

    Ok(())
}

/// Translation of `Emscripten_GLES_GetSwapInterval()`.
pub(super) fn gles_get_swap_interval() -> Result<i32> {
    let pending = pending_swap_interval();
    if pending >= 0 {
        return Ok(pending); // we're planning to set it to this once the system settles down.
    }

    let (mut mode, mut value): (c_int, c_int) = (0, 0);

    // SAFETY: two ints to write.
    unsafe { sys::emscripten_get_main_loop_timing(&mut mode, &mut value) };

    if mode == sys::EM_TIMING_RAF {
        Ok(value)
    } else {
        Ok(0)
    }
}

/// Translation of `Emscripten_GLES_CreateContext()`.
pub(super) fn gles_create_context(window: WindowID) -> Result<RawGlContext> {
    let mut attribs = std::mem::MaybeUninit::<sys::EmscriptenWebGLContextAttributes>::uninit();
    // SAFETY: emscripten fills in the whole structure.
    let mut attribs = unsafe {
        sys::emscripten_webgl_init_context_attributes(attribs.as_mut_ptr());
        attribs.assume_init()
    };

    let config = gl_config();
    attribs.alpha = config.alpha_size > 0;
    attribs.depth = config.depth_size > 0;
    attribs.stencil = config.stencil_size > 0;
    attribs.antialias = config.multisamplebuffers == 1;

    if config.major_version == 3 {
        attribs.majorVersion = 2; // WebGL 2.0 ~= GLES 3.0
    }

    let Some((canvas_id, gl_context)) = with_data(window, |d| (d.canvas_id.clone(), d.gl_context))
    else {
        return Err(Error::new("Invalid window"));
    };

    if gl_context.is_some() {
        return Err(Error::new(
            "Cannot create multiple webgl contexts per window",
        ));
    }

    // SAFETY: a NUL-terminated selector and initialized attributes.
    let context = unsafe { sys::emscripten_webgl_create_context(canvas_id.as_ptr(), &attribs) };

    let Some(context) = RawGlContext::from_ptr(context as *mut c_void) else {
        return Err(Error::new("Could not create webgl context"));
    };

    // SAFETY: the context just made.
    if unsafe { sys::emscripten_webgl_make_context_current(handle(Some(context))) }
        != sys::EMSCRIPTEN_RESULT_SUCCESS
    {
        // SAFETY: as above; nothing else has it.
        unsafe { sys::emscripten_webgl_destroy_context(handle(Some(context))) };
        // Note (upstream): it returns NULL without setting an error (the
        // caller sees whatever error was set before); here it's this one.
        return Err(Error::new("Unable to make context current"));
    }

    with_data(window, |d| d.gl_context = Some(context));

    Ok(context)
}

/// Translation of `Emscripten_GLES_DestroyContext()`.
pub(super) fn gles_destroy_context(context: RawGlContext) {
    // remove the context from its window
    for window in window_ids() {
        with_data(window, |d| {
            if d.gl_context == Some(context) {
                d.gl_context = None;
            }
        });
    }

    // SAFETY: a context emscripten made (the front end passes only those).
    unsafe { sys::emscripten_webgl_destroy_context(handle(Some(context))) };
}

/// Translation of `Emscripten_GLES_SwapWindow()`.
pub(super) fn gles_swap_window(_window: WindowID) {
    // SAFETY: a plain call.
    if unsafe { sys::emscripten_has_asyncify() } != 0
        && hints::get_bool(hints::EMSCRIPTEN_ASYNCIFY, true)
    {
        // give back control to browser for screen refresh
        // SAFETY: with Asyncify, a plain call.
        unsafe { sys::emscripten_sleep(0) };
    }
}

/// Translation of `Emscripten_GLES_MakeCurrent()`.
pub(super) fn gles_make_current(
    window: Option<WindowID>,
    context: Option<RawGlContext>,
) -> Result<()> {
    // it isn't possible to reuse contexts across canvases
    if let (Some(window), Some(context)) = (window, context) {
        if with_data(window, |d| d.gl_context).flatten() != Some(context) {
            return Err(Error::new("Cannot make context current to another window"));
        }
    }

    // SAFETY: a context emscripten made, or none (0).
    if unsafe { sys::emscripten_webgl_make_context_current(handle(context)) }
        != sys::EMSCRIPTEN_RESULT_SUCCESS
    {
        return Err(Error::new("Unable to make context current"));
    }
    Ok(())
}
