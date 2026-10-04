// Rust translation of src/video/wayland/SDL_waylandopengles.c and
// SDL_waylandopengles.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! EGL implementation of SDL OpenGL ES support on Wayland, on
//! [`crate::video::egl`]: the windows' EGL surfaces are on their
//! `wl_egl_window`s (see [`super::window`]), and the swap interval is paced
//! here with the window's frame callback rather than by EGL.

use std::ffi::c_void;
use std::sync::atomic::Ordering;

use super::video::WaylandVideo;
use super::window::ShellSurfaceStatus;
use crate::core::unix::{io_ready, IoReadyFlags};
use crate::error::{Error, Result};
use crate::events::WindowID;
use crate::timer::{ticks_ns, NS_PER_SECOND};
use crate::video::egl::{self, EGL_NO_SURFACE};
use crate::video::gl::{GlConfig, RawGlContext};

const EGL_PLATFORM_WAYLAND_KHR: i32 = 0x31D8;

// EGL implementation of SDL OpenGL ES support

/// Translation of `Wayland_GLES_SetDefaultProfileConfig()`.
pub(crate) fn wayland_gles_set_default_profile_config(config: &mut GlConfig) {
    config.egl_platform = EGL_PLATFORM_WAYLAND_KHR;
}

impl WaylandVideo {
    /// The window's `egl_surface` (`EGL_NO_SURFACE` without one).
    fn egl_surface_of(&self, window: WindowID) -> *mut c_void {
        self.with_data(|d| d.window(window).and_then(|w| w.egl_surface))
            .map_or(EGL_NO_SURFACE, |s| s.as_ptr())
    }

    /// Translation of `Wayland_GLES_LoadLibrary()`.
    pub(crate) fn wayland_gles_load_library(&self, path: Option<&str>) -> Result<()> {
        let result = egl::load_library(path, self.conn.raw().cast());

        self.wayland_pump_events();
        let _ = self.conn.flush();

        result
    }

    /// Translation of `Wayland_GLES_CreateContext()`.
    pub(crate) fn wayland_gles_create_context(&self, window: WindowID) -> Result<RawGlContext> {
        let context = egl::create_context(self.egl_surface_of(window));
        let _ = self.conn.flush();

        context
    }

    /* Wayland wants to tell you when to provide new frames, and if you have a non-zero
    swap interval, Mesa will block until a callback tells it to do so. On some
    compositors, they might decide that a minimized window _never_ gets a callback,
    which causes apps to hang during swapping forever. So we always set the official
    eglSwapInterval to zero to avoid blocking inside EGL, and manage this ourselves.
    If a swap blocks for too long waiting on a callback, we just go on, under the
    assumption the frame will be wasted, but this is better than freezing the app.
    I frown upon platforms that dictate this sort of control inversion (the callback
    is intended for _rendering_, not stalling until vsync), but we can work around
    this for now.  --ryan. */
    /* Addendum: several recent APIs demand this sort of control inversion: Emscripten,
    libretro, Wayland, probably others...it feels like we're eventually going to have
    to give in with a future SDL API revision, since we can bend the other APIs to
    this style, but this style is much harder to bend the other way.  :/ */
    /// Translation of `Wayland_GLES_SetSwapInterval()`.
    pub(crate) fn wayland_gles_set_swap_interval(&self, interval: i32) -> Result<()> {
        if !egl::is_loaded() {
            return Err(Error::new("EGL not initialized"));
        }

        /* technically, this is _all_ adaptive vsync (-1), because we can't
        actually wait for the _next_ vsync if you set 1, but things that
        request 1 probably won't care _that_ much. I hope. No matter what
        you do, though, you never see tearing on Wayland. */
        let interval = interval.clamp(-1, 1);

        // !!! FIXME: technically, this should be per-context, right?
        egl::set_stored_swap_interval(interval);
        egl::call_swap_interval(0);
        Ok(())
    }

    /// Translation of `Wayland_GLES_GetSwapInterval()`.
    pub(crate) fn wayland_gles_get_swap_interval(&self) -> Result<i32> {
        egl::get_swap_interval()
    }

    /// Translation of `Wayland_GLES_SwapWindow()`.
    pub(crate) fn wayland_gles_swap_window(&self, window: WindowID) -> Result<()> {
        // Note (upstream): egl_data is read without checking it; this fails
        // instead.
        let swap_interval = egl::get_swap_interval()?;
        let Some((status, double_buffer, egl_surface, gles_swap_frame, swap_interval_ready)) = self
            .with_data(|d| {
                d.window(window).map(|data| {
                    (
                        data.shell_surface_status,
                        data.double_buffer,
                        data.egl_surface.map_or(EGL_NO_SURFACE, |s| s.as_ptr()),
                        data.gles_swap_frame.clone(),
                        data.swap_interval_ready.clone(),
                    )
                })
            })
        else {
            return Err(Error::new("Invalid window"));
        };

        /* For windows that we know are hidden, skip swaps entirely, if we don't do
         * this compositors will intentionally stall us indefinitely and there's no
         * way for an end user to show the window, unlike other situations (i.e.
         * the window is minimized, behind another window, etc.).
         *
         * FIXME: Request EGL_WAYLAND_swap_buffers_with_timeout.
         * -flibit
         */
        if status != ShellSurfaceStatus::Shown && status != ShellSurfaceStatus::WaitingForFrame {
            return Ok(());
        }

        /* By default, we wait for the Wayland frame callback and then issue the pageflip (eglSwapBuffers),
         * but if we want low latency (double buffer scheme), we issue the pageflip and then wait
         * immediately for the Wayland frame callback.
         */
        if double_buffer {
            // Feed the frame to Wayland. This will set it so the wl_surface_frame callback can fire again.
            egl::swap_buffers(egl_surface)?;

            let _ = self.conn.flush();
        }

        // Control swap interval ourselves. See comments on Wayland_GLES_SetSwapInterval
        if swap_interval != 0 && status == ShellSurfaceStatus::Shown {
            let display = &self.conn;
            // (an OpenGL window always has its swap frame queue)
            let queue = gles_swap_frame.as_ref().map(|f| &f.event_queue);
            // 20hz, so we'll progress even if throttled to zero.
            let max_wait = ticks_ns() + (NS_PER_SECOND as u64 / 20);
            while let (0, Some(queue)) = (swap_interval_ready.load(Ordering::SeqCst), queue) {
                let _ = display.flush();

                /* wl_display_prepare_read_queue() will return false if the event queue is not empty.
                 * If the event queue is empty, it will prepare us for our SDL_IOReady() call. */
                if !display.prepare_read_queue(queue) {
                    // We have some pending events. Check if the frame callback happened.
                    let _ = display.dispatch_queue_pending(queue);
                    continue;
                }

                // Beyond this point, we must either call wl_display_cancel_read() or wl_display_read_events()

                let now = ticks_ns();
                if now >= max_wait {
                    // Timeout expired. Cancel the read.
                    display.cancel_read();
                    break;
                }

                if io_ready(display.fd(), IoReadyFlags::READ, (max_wait - now) as i64) <= 0 {
                    // Error or timeout expired without any events for us. Cancel the read.
                    display.cancel_read();
                    break;
                }

                // We have events. Read and dispatch them.
                let _ = display.read_events();
                let _ = display.dispatch_queue_pending(queue);
            }
            swap_interval_ready.store(0, Ordering::SeqCst);
        }

        if !double_buffer {
            // Feed the frame to Wayland. This will set it so the wl_surface_frame callback can fire again.
            egl::swap_buffers(egl_surface)?;

            let _ = self.conn.flush();
        }

        Ok(())
    }

    /// Translation of `Wayland_GLES_MakeCurrent()`.
    pub(crate) fn wayland_gles_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Result<()> {
        let result = match (window, context) {
            (Some(window), Some(context)) => {
                egl::make_current(self.egl_surface_of(window), Some(context))
            }
            _ => egl::make_current(EGL_NO_SURFACE, None),
        };

        let _ = self.conn.flush();

        // Note (upstream): egl_data is used without checking it here; this
        // does nothing without EGL instead.
        egl::call_swap_interval(0); // see comments on Wayland_GLES_SetSwapInterval.

        result
    }

    /// Translation of `Wayland_GLES_DestroyContext()`.
    pub(crate) fn wayland_gles_destroy_context(&self, context: RawGlContext) {
        egl::destroy_context(context);
        let _ = self.conn.flush();
    }

    /// Translation of `Wayland_GLES_GetEGLSurface()`.
    pub(crate) fn wayland_gles_get_egl_surface(&self, window: WindowID) -> *mut c_void {
        self.egl_surface_of(window)
    }
}
