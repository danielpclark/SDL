// Rust translation of src/video/offscreen/SDL_offscreenvideo.c,
// SDL_offscreenwindow.c, SDL_offscreenevents.c and
// SDL_offscreenframebuffer.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The offscreen video driver: like the dummy driver, but meant to let
//! applications use some of the video functionality (notably context
//! creation) without a display, say for automated testing on a headless
//! machine. Its OpenGL contexts are EGL's, on a device display with pbuffer
//! window surfaces ([`opengles`], where EGL is built: on Unix other than
//! Apple platforms). Its Vulkan surfaces are `VK_EXT_headless_surface`'s,
//! from the Vulkan loader loaded at run time ([`vulkan`]).

#[cfg(all(unix, not(target_vendor = "apple")))]
mod opengles;
mod vulkan;

use std::sync::atomic::AtomicI32;
use std::sync::{Arc, Mutex};

#[cfg(all(unix, not(target_vendor = "apple")))]
use std::ffi::c_void;

#[cfg(all(unix, not(target_vendor = "apple")))]
use crate::error::Error;
use crate::error::Result;
use crate::events::window::{send_window_event, WindowFlags};
use crate::events::{DisplayID, EventType, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::video::core::with_window;
use crate::video::display::add_basic_video_display;
#[cfg(all(unix, not(target_vendor = "apple")))]
use crate::video::gl::RawGlContext;
use crate::video::sysvideo::{
    DeviceCaps, DisplayMode, VideoBootStrap, VideoDriver, WINDOWPOS_UNDEFINED,
};
use crate::video::{PixelFormat, Rect, Surface};

const OFFSCREENVID_DRIVER_NAME: &str = "offscreen";

/// The offscreen window data. Translation of `SDL_WindowData`.
struct OffscreenWindow {
    #[allow(dead_code)]
    sdl_window: WindowID,
    #[cfg(all(unix, not(target_vendor = "apple")))]
    egl_surface: Option<crate::video::gl::EglSurface>,
}

#[derive(Default)]
struct OffscreenVideo {
    /// The Vulkan loader (`_this->vulkan_config`).
    vulkan: Mutex<vulkan::VulkanConfig>,
}

static FRAME_NUMBER: AtomicI32 = AtomicI32::new(0);

impl VideoDriver for OffscreenVideo {
    fn caps(&self) -> DeviceCaps {
        // TODO: Is this needed?
        DeviceCaps::SLOW_FRAMEBUFFER
    }

    fn video_init(&self) -> Result<()> {
        // Use a fake 32-bpp desktop mode
        let mode = DisplayMode {
            format: PixelFormat::XRGB8888,
            w: 1024,
            h: 768,
            ..Default::default()
        };
        if add_basic_video_display(Some(&mode)) == 0 {
            return Err(crate::video::core::uninitialized_video());
        }

        // We're done!
        Ok(())
    }

    fn video_quit(&self) {}

    fn set_display_mode(&self, _display: DisplayID, _mode: &DisplayMode) -> Option<Result<()>> {
        Some(Ok(()))
    }

    fn pump_events(&self) -> Option<()> {
        // do nothing.
        Some(())
    }

    fn create_window(&self, window: WindowID, _create_props: &Properties) -> Option<Result<()>> {
        let created = with_window(window, |w| {
            w.internal = Some(Box::new(OffscreenWindow {
                sdl_window: window,
                #[cfg(all(unix, not(target_vendor = "apple")))]
                egl_surface: None,
            }));

            if w.core.x == WINDOWPOS_UNDEFINED {
                w.core.x = 0;
            }

            if w.core.y == WINDOWPOS_UNDEFINED {
                w.core.y = 0;
            }

            (w.flags().contains(WindowFlags::OPENGL), w.core.w, w.core.h)
        });
        #[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(unused_variables))]
        let (opengl, width, height) = match created {
            Ok(created) => created,
            Err(e) => return Some(Err(e)),
        };

        #[cfg(all(unix, not(target_vendor = "apple")))]
        if opengl {
            use crate::video::egl;

            let Some(e) = egl::egl() else {
                return Some(Err(Error::new(
                    "Cannot create an OPENGL window invalid egl_data",
                )));
            };

            let egl_surface = egl::create_offscreen_surface(width, height)
                .ok()
                .and_then(crate::video::gl::EglSurface::from_ptr);

            let Some(egl_surface) = egl_surface else {
                return Some(Err(Error::new(format!(
                    "Failed to created an offscreen surface (EGL display: {:p})",
                    e.egl_display
                ))));
            };
            let _ = with_window(window, |w| {
                if let Some(d) = w
                    .internal
                    .as_mut()
                    .and_then(|i| i.downcast_mut::<OffscreenWindow>())
                {
                    d.egl_surface = Some(egl_surface);
                }
            });
        }

        Some(Ok(()))
    }

    fn destroy_window(&self, window: WindowID) -> Option<()> {
        let _internal = with_window(window, |w| w.internal.take());
        #[cfg(all(unix, not(target_vendor = "apple")))]
        if let Some(d) = _internal
            .ok()
            .flatten()
            .and_then(|i| i.downcast::<OffscreenWindow>().ok())
        {
            crate::video::egl::destroy_surface(
                d.egl_surface
                    .map_or(crate::video::egl::EGL_NO_SURFACE, |s| s.as_ptr()),
            );
        }
        Some(())
    }

    fn set_window_size(&self, window: WindowID) -> Option<()> {
        let pending = with_window(window, |w| w.pending).ok()?;
        send_window_event(window, EventType::WINDOW_RESIZED, pending.w, pending.h);
        Some(())
    }

    fn implements_window_framebuffer(&self) -> bool {
        true
    }

    fn create_window_framebuffer(
        &self,
        _window: WindowID,
        w: i32,
        h: i32,
    ) -> Option<Result<Surface<'static>>> {
        // Create a new framebuffer
        Some(super::create_xrgb8888_framebuffer(w, h))
    }

    fn update_window_framebuffer(
        &self,
        window: WindowID,
        surface: &Surface<'static>,
        _rects: &[Rect],
    ) -> Option<Result<()>> {
        Some(super::maybe_save_frame(
            hints::VIDEO_OFFSCREEN_SAVE_FRAMES,
            &FRAME_NUMBER,
            window,
            surface,
        ))
    }

    fn destroy_window_framebuffer(&self, _window: WindowID) -> Option<()> {
        Some(())
    }

    // * * * GL context (`OFFSCREEN_GLES_*`, where EGL is built)

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn implements_gl_contexts(&self) -> bool {
        true
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn gl_swap_window(&self, window: WindowID) -> Option<Result<()>> {
        Some(opengles::offscreen_gles_swap_window(window))
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn gl_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Option<Result<()>> {
        Some(opengles::offscreen_gles_make_current(window, context))
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn gl_create_context(&self, window: WindowID) -> Option<Result<RawGlContext>> {
        Some(opengles::offscreen_gles_create_context(window))
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn gl_destroy_context(&self, context: RawGlContext) -> Option<Result<()>> {
        crate::video::egl::destroy_context(context);
        Some(Ok(()))
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn gl_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(opengles::offscreen_gles_load_library(path))
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn gl_unload_library(&self) -> Option<()> {
        crate::video::egl::unload_library();
        Some(())
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn gl_get_proc_address(&self, proc_name: &str) -> Option<Option<std::ptr::NonNull<c_void>>> {
        Some(crate::video::egl::get_proc_address_internal(proc_name))
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn gl_get_swap_interval(&self) -> Option<Result<i32>> {
        Some(crate::video::egl::get_swap_interval())
    }

    #[cfg(all(unix, not(target_vendor = "apple")))]
    fn gl_set_swap_interval(&self, interval: i32) -> Option<Result<()>> {
        Some(crate::video::egl::set_swap_interval(interval))
    }

    // * * * Vulkan (`OFFSCREEN_Vulkan_*`)

    fn implements_vulkan_surfaces(&self) -> bool {
        true
    }

    fn vulkan_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(vulkan::offscreen_vulkan_load_library(&self.vulkan, path))
    }

    fn vulkan_unload_library(&self) -> Option<()> {
        vulkan::offscreen_vulkan_unload_library(&self.vulkan);
        Some(())
    }

    fn vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        vulkan::offscreen_vulkan_get_instance_proc_addr(&self.vulkan)
    }

    fn vulkan_instance_extensions(&self) -> Option<Vec<&'static str>> {
        Some(vulkan::offscreen_vulkan_get_instance_extensions(
            &self.vulkan,
        ))
    }

    fn vulkan_create_surface(
        &self,
        _window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Option<Result<u64>> {
        Some(vulkan::offscreen_vulkan_create_surface(
            &self.vulkan,
            instance,
            allocator,
        ))
    }

    fn vulkan_destroy_surface(
        &self,
        instance: usize,
        surface: u64,
        allocator: usize,
    ) -> Option<()> {
        vulkan::offscreen_vulkan_destroy_surface(&self.vulkan, instance, surface, allocator);
        Some(())
    }
}

/// Translation of `OFFSCREEN_CreateDevice()`.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    if !super::dummy::available(OFFSCREENVID_DRIVER_NAME) {
        return None;
    }
    Some(Arc::new(OffscreenVideo::default()))
}

/// Translation of `OFFSCREEN_bootstrap`.
pub(crate) static OFFSCREEN_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: OFFSCREENVID_DRIVER_NAME,
    desc: "SDL offscreen video driver",
    create: create_device,
    show_message_box: None, // no ShowMessageBox implementation
    is_preferred: false,
};
