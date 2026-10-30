// Rust translation of src/video/android/SDL_androidgl.c and SDL_androidgl.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! OpenGL ES through EGL on the activity's surface: the default display
//! (`EGL_DEFAULT_DISPLAY`), and the window's EGL window surface, which a
//! context is made current with; creating a context waits for the app to
//! be active, and a swap holds the activity mutex (the surface's natives
//! replace the EGL surface under it).

use crate::core::android as core_android;
use crate::error::Result;
use crate::events::WindowID;
use crate::video::egl::{self, EGL_DEFAULT_DISPLAY, EGL_NO_SURFACE};
use crate::video::gl::RawGlContext;

/// Translation of `Android_GLES_MakeCurrent()`.
pub(crate) fn gles_make_current(
    window: Option<WindowID>,
    context: Option<RawGlContext>,
) -> Result<()> {
    match (window, context) {
        (Some(window), Some(context)) => {
            egl::make_current(super::window::egl_surface(window), Some(context))
        }
        _ => egl::make_current(EGL_NO_SURFACE, None),
    }
}

/// Translation of `Android_GLES_CreateContext()`.
pub(crate) fn gles_create_context(window: WindowID) -> Result<RawGlContext> {
    super::events::wait_active_and_lock_activity()?;

    let result = egl::create_context(super::window::egl_surface(window));

    core_android::unlock_activity_mutex();

    result
}

/// Translation of `Android_GLES_SwapWindow()`.
pub(crate) fn gles_swap_window(window: WindowID) -> Result<()> {
    core_android::lock_activity_mutex();

    /* The following two calls existed in the original Java code
     * If you happen to have a device that's affected by their removal,
     * please report to our bug tracker. -- Gabriel
     */

    /*_this->egl_data->eglWaitNative(EGL_CORE_NATIVE_ENGINE);
    _this->egl_data->eglWaitGL();*/
    let result = egl::swap_buffers(super::window::egl_surface(window));

    core_android::unlock_activity_mutex();

    result
}

/// Translation of `Android_GLES_LoadLibrary()`.
pub(crate) fn gles_load_library(path: Option<&str>) -> Result<()> {
    egl::load_library(path, EGL_DEFAULT_DISPLAY)
}
