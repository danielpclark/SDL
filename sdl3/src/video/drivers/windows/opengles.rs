// Rust translation of src/video/windows/SDL_windowsopengles.c and
// SDL_windowsopengles.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! EGL implementation of SDL OpenGL support on Windows (OpenGL ES, through
//! ANGLE's libEGL.dll for example, or any GL with
//! [`hints::VIDEO_FORCE_EGL`]), on [`crate::video::egl`].

use std::ffi::c_void;

use super::opengl::GlBackend;
use super::window::window_data;
use super::VideoData;
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::video::egl::{self, EGL_DEFAULT_DISPLAY, EGL_NO_SURFACE};
use crate::video::gl::{self, gl_config, RawGlContext, GL_CONTEXT_PROFILE_ES};

/// Whether the requested profile is desktop GL without EGL being forced
/// (the switch to the `WIN_GL_*` functions).
fn wants_wgl() -> bool {
    gl_config().profile_mask != GL_CONTEXT_PROFILE_ES
        && !hints::get_bool(hints::VIDEO_FORCE_EGL, false)
}

impl VideoData {
    /// Translation of `WIN_GLES_LoadLibrary()`.
    pub(crate) fn win_gles_load_library(&self, path: Option<&str>) -> Result<()> {
        // If the profile requested is not GL ES, switch over to WIN_GL functions
        if wants_wgl() {
            egl::unload_library();
            self.set_gl_backend(GlBackend::Wgl);
            return self.win_gl_load_library(path);
        }

        if !egl::is_loaded() {
            // FIXME (upstream): the requested path is ignored; EGL is loaded
            // from SDL_HINT_EGL_LIBRARY or the default library.
            return egl::load_library(None, EGL_DEFAULT_DISPLAY);
        }
        Ok(())
    }

    /// The window's `egl_surface`.
    fn egl_surface_of(&self, window: WindowID) -> Result<*mut c_void> {
        window_data(window)
            .map(|d| d.state.with(|s| s.egl_surface))
            .ok_or_else(|| Error::new("Invalid window"))
    }

    /// Translation of `WIN_GLES_CreateContext()`.
    pub(crate) fn win_gles_create_context(&self, window: WindowID) -> Result<RawGlContext> {
        if wants_wgl() {
            // Switch to WGL based functions
            egl::unload_library();
            self.set_gl_backend(GlBackend::Wgl);
            self.win_gl_load_library(None)?;
            return self.win_gl_create_context(window);
        }

        egl::create_context(self.egl_surface_of(window)?)
    }

    /// Translation of `WIN_GLES_SwapWindow()` (`SDL_EGL_SwapWindow_impl(WIN)`).
    pub(crate) fn win_gles_swap_window(&self, window: WindowID) -> Result<()> {
        egl::swap_buffers(self.egl_surface_of(window)?)
    }

    /// Translation of `WIN_GLES_MakeCurrent()` (`SDL_EGL_MakeCurrent_impl(WIN)`).
    pub(crate) fn win_gles_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Result<()> {
        let surface = match window {
            Some(window) => self.egl_surface_of(window)?,
            None => EGL_NO_SURFACE,
        };
        egl::make_current(surface, context)
    }

    /// Translation of `WIN_GLES_SetupWindow()`.
    pub(crate) fn win_gles_setup_window(&self, window: WindowID) -> Result<()> {
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
        // FIXME (upstream): WIN_DestroyWindow() never destroys this surface
        // (SDL_EGL_DestroySurface), so it outlives its window.
        let surface = egl::create_surface(Some(window), windowdata.hwnd)
            .ok()
            .filter(|s| *s != EGL_NO_SURFACE);
        windowdata
            .state
            .with(|s| s.egl_surface = surface.unwrap_or(EGL_NO_SURFACE));
        if surface.is_none() {
            return Err(Error::new("Could not create GLES window surface"));
        }

        self.win_gles_make_current(current_win, current_ctx)
    }

    /// Translation of `WIN_GLES_GetEGLSurface()`.
    pub(crate) fn win_gles_get_egl_surface(&self, window: WindowID) -> *mut c_void {
        self.egl_surface_of(window).unwrap_or(EGL_NO_SURFACE)
    }
}
