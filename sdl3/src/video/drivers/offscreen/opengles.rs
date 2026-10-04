// Rust translation of src/video/offscreen/SDL_offscreenopengles.c and
// SDL_offscreenopengles.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! EGL implementation of SDL OpenGL support for the offscreen driver: a
//! device display (`EGL_EXT_platform_device`, through
//! [`egl::initialize_offscreen`]) and pbuffer window surfaces.

use std::ffi::c_void;

use super::OffscreenWindow;
use crate::error::Result;
use crate::events::WindowID;
use crate::video::core::with_window;
use crate::video::egl::{self, EGL_NO_SURFACE};
use crate::video::gl::{self, RawGlContext};

/// The window's `egl_surface` (`EGL_NO_SURFACE` without one).
fn egl_surface_of(window: WindowID) -> *mut c_void {
    with_window(window, |w| {
        w.internal
            .as_ref()
            .and_then(|i| i.downcast_ref::<OffscreenWindow>())
            .and_then(|d| d.egl_surface)
    })
    .ok()
    .flatten()
    .map_or(EGL_NO_SURFACE, |s| s.as_ptr())
}

/// Translation of `OFFSCREEN_GLES_LoadLibrary()`.
pub(super) fn offscreen_gles_load_library(path: Option<&str>) -> Result<()> {
    egl::load_library_only(path)?;

    /* driver_loaded gets incremented by SDL_GL_LoadLibrary when we return,
    but SDL_EGL_InitializeOffscreen checks that we're loaded before then,
    so temporarily bump it since we know that LoadLibraryOnly succeeded. */
    gl::set_driver_loaded(|n| n + 1);
    let result = egl::initialize_offscreen(0);
    gl::set_driver_loaded(|n| n - 1);
    result?;

    egl::choose_config()?;

    Ok(())
}

/// Translation of `OFFSCREEN_GLES_CreateContext()`.
pub(super) fn offscreen_gles_create_context(window: WindowID) -> Result<RawGlContext> {
    egl::create_context(egl_surface_of(window))
}

/// Translation of `OFFSCREEN_GLES_MakeCurrent()`.
pub(super) fn offscreen_gles_make_current(
    window: Option<WindowID>,
    context: Option<RawGlContext>,
) -> Result<()> {
    match window {
        Some(window) => egl::make_current(egl_surface_of(window), context),
        None => egl::make_current(EGL_NO_SURFACE, None),
    }
}

/// Translation of `OFFSCREEN_GLES_SwapWindow()`.
pub(super) fn offscreen_gles_swap_window(window: WindowID) -> Result<()> {
    egl::swap_buffers(egl_surface_of(window))
}

#[cfg(test)]
mod tests {
    use crate::events::window::WindowFlags;
    use crate::hints;
    use crate::init::{self, InitFlags};
    use crate::video::gl::{self, GlAttr, GlContext};
    use crate::video::window::Window;

    /// Clear the current context's framebuffer to a color and read a pixel
    /// back.
    fn clear_and_read(r: f32, g: f32, b: f32) -> [u8; 4] {
        type ClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
        type Clear = unsafe extern "C" fn(u32);
        type Finish = unsafe extern "C" fn();
        type ReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut std::ffi::c_void);
        let get = |name: &str| gl::gl_get_proc_address(name).unwrap_or_else(|| panic!("{name}"));
        let mut pixel = [0u8; 4];
        // SAFETY: the GL functions have these types; a context is current;
        // the buffer holds one RGBA pixel.
        unsafe {
            let clear_color: ClearColor = std::mem::transmute(get("glClearColor"));
            let clear: Clear = std::mem::transmute(get("glClear"));
            let finish: Finish = std::mem::transmute(get("glFinish"));
            let read_pixels: ReadPixels = std::mem::transmute(get("glReadPixels"));
            clear_color(r, g, b, 1.0);
            clear(0x4000); // GL_COLOR_BUFFER_BIT
            finish();
            // GL_RGBA, GL_UNSIGNED_BYTE
            read_pixels(1, 1, 1, 1, 0x1908, 0x1401, pixel.as_mut_ptr().cast());
        }
        pixel
    }

    /// Video up on the offscreen driver; quit when dropped.
    struct Video;

    impl Video {
        fn init() -> Video {
            init::quit();
            hints::set(hints::VIDEO_DRIVER, "offscreen").unwrap();
            init::init(InitFlags::VIDEO).unwrap();
            Video
        }
    }

    impl Drop for Video {
        fn drop(&mut self) {
            init::quit();
            hints::reset(hints::VIDEO_DRIVER);
        }
    }

    /// OpenGL and OpenGL ES contexts on pbuffer windows, through Mesa's EGL
    /// device platform (skipped without a libEGL that has one).
    #[test]
    fn offscreen_egl_contexts() {
        let _l = crate::test_support::test_lock();
        for es in [false, true] {
            let _video = Video::init();
            if es {
                gl::gl_set_attribute(GlAttr::ContextProfileMask, gl::GL_CONTEXT_PROFILE_ES)
                    .unwrap();
                gl::gl_set_attribute(GlAttr::ContextMajorVersion, 2).unwrap();
                gl::gl_set_attribute(GlAttr::ContextMinorVersion, 0).unwrap();
            }
            let what = if es { "OpenGL ES" } else { "OpenGL" };
            if let Err(e) = gl::gl_load_library(None) {
                eprintln!(
                    "note: no offscreen EGL for {what} ({}); skipping",
                    e.message()
                );
                return;
            }

            let window = Window::create("offscreen GL", 32, 24, WindowFlags::OPENGL).unwrap();
            assert!(gl::egl_window_surface(&window).unwrap().is_none()); // (no GL_GetEGLSurface)
            assert!(gl::egl_current_display().is_ok());
            assert!(gl::egl_current_config().unwrap().is_some());
            let context = match GlContext::new(&window) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "note: no offscreen {what} context ({}); skipping",
                        e.message()
                    );
                    window.destroy();
                    gl::gl_unload_library();
                    continue;
                }
            };
            assert!(context.is_current());
            assert_eq!(&clear_and_read(1.0, 0.0, 1.0)[..3], &[255, 0, 255]);
            gl::gl_swap_window(&window).unwrap();
            assert!(gl::gl_get_attribute(GlAttr::RedSize).unwrap() >= 8);

            gl::gl_set_swap_interval(0).unwrap();
            assert_eq!(gl::gl_get_swap_interval().unwrap(), 0);
            // (late swap tearing isn't supported through EGL)
            assert!(gl::gl_set_swap_interval(-1).is_err());

            gl::gl_release_current().unwrap();
            assert!(!context.is_current());
            context.make_current(Some(&window)).unwrap();
            assert_eq!(&clear_and_read(0.0, 1.0, 0.0)[..3], &[0, 255, 0]);

            drop(context);
            window.destroy();
            gl::gl_unload_library();
        }
    }
}
