// Rust translation of src/video/cocoa/SDL_cocoaopengles.m and
// SDL_cocoaopengles.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! EGL implementation of SDL OpenGL support on macOS: OpenGL ES through
//! ANGLE's `libEGL.dylib` and `libGLESv2.dylib` (or another EGL named by
//! [`hints::EGL_LIBRARY`](crate::hints::EGL_LIBRARY)), on
//! [`crate::video::egl`]. Upstream's `Cocoa_GLES_GetProcAddress`,
//! `_UnloadLibrary`, `_SetSwapInterval` and `_GetSwapInterval` are macros
//! for the EGL functions, which the driver calls directly.

use std::ffi::c_void;

use super::opengl::{set_gl_backend, GlBackend};
use super::window::window_data;
use crate::core::apple::objc::{autoreleasepool, msg_send, Id};
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::video::egl::{self, EGL_DEFAULT_DISPLAY, EGL_NO_SURFACE};
use crate::video::gl::{self, gl_config, RawGlContext, GL_CONTEXT_PROFILE_ES};

/// The window's `egl_surface`.
fn egl_surface_of(window: WindowID) -> Result<*mut c_void> {
    window_data(window)
        .map(|d| d.state().egl_surface)
        .ok_or_else(|| Error::new("Invalid window"))
}

/// Translation of `Cocoa_GLES_LoadLibrary()`.
pub(crate) fn gles_load_library(path: Option<&str>) -> Result<()> {
    // If the profile requested is not GL ES, switch over to WIN_GL functions
    if gl_config().profile_mask != GL_CONTEXT_PROFILE_ES {
        egl::unload_library();
        set_gl_backend(GlBackend::Cgl);
        return super::opengl::cgl_load_library(path);
    }

    if !egl::is_loaded() {
        return egl::load_library(None, EGL_DEFAULT_DISPLAY);
    }

    Ok(())
}

/// Translation of `Cocoa_GLES_CreateContext()`.
pub(crate) fn gles_create_context(window: WindowID) -> Result<RawGlContext> {
    autoreleasepool(|| {
        if gl_config().profile_mask != GL_CONTEXT_PROFILE_ES {
            // Switch to CGL based functions
            egl::unload_library();
            set_gl_backend(GlBackend::Cgl);

            super::opengl::cgl_load_library(None)?;

            return super::opengl::cgl_create_context(window);
        }

        egl::create_context(egl_surface_of(window)?)
    })
}

/// Translation of `Cocoa_GLES_DestroyContext()`.
pub(crate) fn gles_destroy_context(context: RawGlContext) {
    autoreleasepool(|| egl::destroy_context(context))
}

/// Translation of `Cocoa_GLES_SwapWindow()`.
pub(crate) fn gles_swap_window(window: WindowID) -> Result<()> {
    autoreleasepool(|| egl::swap_buffers(egl_surface_of(window)?))
}

/// Translation of `Cocoa_GLES_MakeCurrent()`.
pub(crate) fn gles_make_current(
    window: Option<WindowID>,
    context: Option<RawGlContext>,
) -> Result<()> {
    autoreleasepool(|| {
        let surface = match window {
            Some(window) => egl_surface_of(window)?,
            None => EGL_NO_SURFACE,
        };
        egl::make_current(surface, context)
    })
}

/// Translation of `Cocoa_GLES_SetupWindow()`.
pub(crate) fn gles_setup_window(window: WindowID) -> Result<()> {
    autoreleasepool(|| {
        // The current context is lost in here; save it and reset it.
        let Some(windowdata) = window_data(window) else {
            return Err(Error::new("Invalid window"));
        };
        let current_win = gl::current_thread_window();
        let current_ctx = gl::current_thread_context();

        if !egl::is_loaded() {
            // !!! FIXME: commenting out this assertion is (I think) incorrect; figure out why driver_loaded is wrong for ANGLE instead. --ryan.
            // (upstream: When hint SDL_HINT_OPENGL_ES_DRIVER is set to "1"
            // (e.g. for ANGLE support), _this->gl_config.driver_loaded can be
            // 1, while the below lines function.)
            if let Err(e) = egl::load_library(None, EGL_DEFAULT_DISPLAY) {
                egl::unload_library();
                return Err(e);
            }
            gl::set_driver_loaded(|_| 1);
        }

        // Create the GLES window surface
        // (destroyed with the window: see destroy_window())
        // SAFETY: the window's NSWindow and its content view's layer.
        let layer: Id = unsafe {
            let v: Id = msg_send![windowdata.nswindow(), contentView];
            msg_send![v, layer]
        };
        let surface = egl::create_surface(Some(window), layer as egl::NativeWindowType)
            .ok()
            .filter(|s| *s != EGL_NO_SURFACE);
        windowdata.state().egl_surface = surface.unwrap_or(EGL_NO_SURFACE);
        if surface.is_none() {
            return Err(Error::new("Could not create GLES window surface"));
        }

        gles_make_current(current_win, current_ctx)
    })
}

/// Translation of `Cocoa_GLES_GetEGLSurface()`.
pub(crate) fn gles_get_egl_surface(window: WindowID) -> *mut c_void {
    autoreleasepool(|| egl_surface_of(window).unwrap_or(EGL_NO_SURFACE))
}
