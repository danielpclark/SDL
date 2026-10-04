// Rust translation of src/video/x11/SDL_x11opengles.c and
// SDL_x11opengles.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! EGL implementation of SDL OpenGL support on X11 (OpenGL ES, or any GL
//! with [`hints::VIDEO_FORCE_EGL`]), on [`crate::video::egl`].

use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

use super::modes::x11_get_pixel_format_from_visual_info;
use super::opengl::GlBackend;
use super::sys::*;
use super::video::X11Video;
use super::window::with_x11_window;
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::hints;
use crate::video::egl::{self, EGLint, EGL_FALSE, EGL_NATIVE_VISUAL_ID, EGL_NO_SURFACE};
use crate::video::gl::{gl_config, GlConfig, RawGlContext, GL_CONTEXT_PROFILE_ES};

const EGL_PLATFORM_X11_KHR: c_int = 0x31D5;

/// Translation of `X11_GLES_SetDefaultProfileConfig()`.
pub(crate) fn x11_gles_set_default_profile_config(config: &mut GlConfig) {
    config.egl_platform = EGL_PLATFORM_X11_KHR;
}

impl X11Video {
    /// Translation of `X11_GLES_LoadLibrary()`.
    pub(crate) fn x11_gles_load_library(&self, path: Option<&str>) -> Result<()> {
        // If the profile requested is not GL ES, switch over to X11_GL functions
        if gl_config().profile_mask != GL_CONTEXT_PROFILE_ES
            && !hints::get_bool(hints::VIDEO_FORCE_EGL, false)
        {
            egl::unload_library();
            self.set_gl_backend(GlBackend::Glx);
            return self.x11_gl_load_library(path);
        }

        egl::load_library(path, self.display.cast())
    }

    /// Translation of `X11_GLES_GetVisual()`: a visual info to free with
    /// `XFree()`.
    pub(crate) fn x11_gles_get_visual(
        &self,
        display: *mut Display,
        screen: c_int,
        transparent: bool,
    ) -> Result<NonNull<XVisualInfo>> {
        let x = &self.x;
        let mut egl_visualinfo: *mut XVisualInfo = std::ptr::null_mut();
        let mut visual_id: EGLint = 0;
        let mut out_count: c_int = 0;

        let Some(e) = egl::egl() else {
            // The EGL library wasn't loaded, SDL_GetError() should have info
            return Err(Error::new("EGL not initialized"));
        };

        if e.get_config_attrib(e.egl_config, EGL_NATIVE_VISUAL_ID, &mut visual_id) == EGL_FALSE {
            visual_id = 0;
        }
        // SAFETY: the display is open; the visual infos are freed with
        // XFree by the caller (or here).
        unsafe {
            if visual_id != 0 {
                let mut vi_in = XVisualInfo {
                    screen,
                    visualid: visual_id as VisualID,
                    ..XVisualInfo::default()
                };
                egl_visualinfo = (x.XGetVisualInfo)(
                    display,
                    VisualScreenMask | VisualIDMask,
                    &mut vi_in,
                    &mut out_count,
                );
                if transparent && !egl_visualinfo.is_null() {
                    let format =
                        x11_get_pixel_format_from_visual_info(x, display, &*egl_visualinfo);
                    if !format.has_alpha() {
                        // not transparent!
                        (x.XFree)(egl_visualinfo.cast());
                        egl_visualinfo = std::ptr::null_mut();
                    }
                }
            }

            if egl_visualinfo.is_null() {
                // Use the default visual when all else fails
                let mut vi_in = XVisualInfo {
                    screen,
                    ..XVisualInfo::default()
                };
                egl_visualinfo =
                    (x.XGetVisualInfo)(display, VisualScreenMask, &mut vi_in, &mut out_count);

                // Return the first transparent Visual
                if transparent && !egl_visualinfo.is_null() {
                    let list =
                        std::slice::from_raw_parts(egl_visualinfo, out_count.max(0) as usize);
                    for v in list {
                        let format = x11_get_pixel_format_from_visual_info(x, display, v);
                        if format.has_alpha() {
                            // found!
                            // re-request it to have a copy that can be X11_XFree'ed later
                            vi_in.screen = screen;
                            vi_in.visualid = v.visualid;
                            (x.XFree)(egl_visualinfo.cast());
                            egl_visualinfo = (x.XGetVisualInfo)(
                                display,
                                VisualScreenMask | VisualIDMask,
                                &mut vi_in,
                                &mut out_count,
                            );
                            break;
                        }
                    }
                }
            }
        }
        // (a NULL result fails window creation; upstream sets no error)
        NonNull::new(egl_visualinfo).ok_or_else(|| Error::new("Couldn't find matching EGL visual"))
    }

    /// The window's `egl_surface`.
    fn egl_surface_of(&self, window: WindowID) -> Result<*mut c_void> {
        with_x11_window(window, |_, d| d.egl_surface)
    }

    /// Translation of `X11_GLES_CreateContext()`.
    pub(crate) fn x11_gles_create_context(&self, window: WindowID) -> Result<RawGlContext> {
        let display = self.display;
        let egl_surface = self.egl_surface_of(window)?;

        // SAFETY: the display is open.
        unsafe { (self.x.XSync)(display, False) };
        let context = egl::create_context(egl_surface);
        // SAFETY: as above.
        unsafe { (self.x.XSync)(display, False) };

        context
    }

    /// Translation of `X11_GLES_GetEGLSurface()`.
    pub(crate) fn x11_gles_get_egl_surface(&self, window: WindowID) -> *mut c_void {
        self.egl_surface_of(window).unwrap_or(EGL_NO_SURFACE)
    }

    /// Translation of `X11_GLES_SwapWindow()`.
    pub(crate) fn x11_gles_swap_window(&self, window: WindowID) -> Result<()> {
        let ret = egl::swap_buffers(self.egl_surface_of(window)?);

        self.x11_handle_present(window);

        ret
    }

    /// Translation of `X11_GLES_MakeCurrent()` (`SDL_EGL_MakeCurrent_impl(X11)`).
    pub(crate) fn x11_gles_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Result<()> {
        let surface = match window {
            Some(window) => self.egl_surface_of(window)?,
            Option::None => EGL_NO_SURFACE,
        };
        egl::make_current(surface, context)
    }
}
