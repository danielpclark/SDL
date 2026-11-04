// Rust translation of src/video/kmsdrm/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The KMS/DRM video driver (`kmsdrm`): fullscreen windows on a DRM card's
//! connectors, without a display server, drawn with OpenGL (ES) through
//! EGL on GBM, or Vulkan on a display plane (`VK_KHR_display`), with the
//! console input reader (evdev) for input.
//!
//! libdrm and libgbm are loaded at run time ([`kmsdrmdyn`]), as upstream's
//! `SDL_KMSDRM_SHARED` build does; their declarations are [`sys`]'s.
//!
//! | Upstream                     | Here            |
//! |------------------------------|-----------------|
//! | `SDL_kmsdrmvideo.c`/`.h`     | [`video`], this module (the device's function table) |
//! | `SDL_kmsdrmopengles.c`/`.h`  | [`opengles`]    |
//! | `SDL_kmsdrmmouse.c`/`.h`     | [`mouse`]       |
//! | `SDL_kmsdrmevents.c`/`.h`    | [`events`]      |
//! | `SDL_kmsdrmdyn.c`/`.h`, `SDL_kmsdrmsym.h` | [`kmsdrmdyn`], [`sys`] |
//! | `SDL_kmsdrmvulkan.c`/`.h`    | [`vulkan`]      |

pub(crate) mod events;
#[path = "dyn.rs"]
pub(crate) mod kmsdrmdyn;
pub(crate) mod mouse;
pub(crate) mod opengles;
pub(crate) mod sys;
pub(crate) mod video;
pub(crate) mod vulkan;

use std::ffi::c_void;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::events::mouse::{Cursor, MouseFeature};
use crate::events::{DisplayID, WindowID};
use crate::properties::Properties;
use crate::video::gl::{GlConfig, RawGlContext};
use crate::video::sysvideo::{
    DisplayMode, FullscreenOp, FullscreenResult, VideoBootStrap, VideoDriver,
};
use crate::video::window::WindowOp;
use crate::video::Surface;

use video::KmsdrmVideo;

impl VideoDriver for KmsdrmVideo {
    // Setup all functions which we can handle

    fn video_init(&self) -> Result<()> {
        self.kmsdrm_video_init()
    }

    fn video_quit(&self) {
        self.kmsdrm_video_quit();
    }

    fn display_modes(&self, display: DisplayID) -> Option<()> {
        self.kmsdrm_get_display_modes(display);
        Some(())
    }

    fn set_display_mode(&self, display: DisplayID, mode: &DisplayMode) -> Option<Result<()>> {
        Some(self.kmsdrm_set_display_mode(display, mode))
    }

    fn create_window(&self, window: WindowID, _create_props: &Properties) -> Option<Result<()>> {
        Some(self.kmsdrm_create_window(window))
    }

    /// Translation of `KMSDRM_SetWindowTitle()`.
    fn set_window_title(&self, _window: WindowID) -> Option<()> {
        Some(())
    }

    /// Translation of `KMSDRM_SetWindowPosition()`.
    fn set_window_position(&self, _window: WindowID) -> Option<Result<()>> {
        Some(Err(Error::unsupported()))
    }

    fn set_window_size(&self, window: WindowID) -> Option<()> {
        self.kmsdrm_set_window_size(window);
        Some(())
    }

    fn set_window_fullscreen(
        &self,
        window: WindowID,
        _display: DisplayID,
        _fullscreen: FullscreenOp,
    ) -> Option<FullscreenResult> {
        self.kmsdrm_set_window_fullscreen(window);
        Some(FullscreenResult::Succeeded)
    }

    /// Translation of `KMSDRM_ShowWindow()`.
    fn show_window(&self, _window: WindowID) -> Option<()> {
        Some(())
    }

    /// Translation of `KMSDRM_HideWindow()`.
    fn hide_window(&self, _window: WindowID) -> Option<()> {
        Some(())
    }

    /// Translation of `KMSDRM_RaiseWindow()`.
    fn raise_window(&self, _window: WindowID) -> Option<()> {
        Some(())
    }

    /// Translation of `KMSDRM_MaximizeWindow()`.
    fn maximize_window(&self, _window: WindowID) -> Option<()> {
        Some(())
    }

    /// Translation of `KMSDRM_MinimizeWindow()`.
    fn minimize_window(&self, _window: WindowID) -> Option<()> {
        Some(())
    }

    /// Translation of `KMSDRM_RestoreWindow()`.
    fn restore_window(&self, _window: WindowID) -> Option<()> {
        Some(())
    }

    fn destroy_window(&self, window: WindowID) -> Option<()> {
        self.kmsdrm_destroy_window(window);
        Some(())
    }

    /// Translation of `KMSDRM_SetWindowFocusable()`.
    fn set_window_focusable(&self, _window: WindowID, _focusable: bool) -> Option<Result<()>> {
        Some(Ok(())) // this just has to exist or SDL_SetWindowFocusable() will refuse to change the window flag.
    }

    fn implements_window_op(&self, op: WindowOp) -> bool {
        matches!(
            op,
            WindowOp::Maximize
                | WindowOp::Minimize
                | WindowOp::Restore
                | WindowOp::SetFocusable
                | WindowOp::SetFullscreen
        )
    }

    // * * * GL pointers (`KMSDRM_GLES_*`; SDL_EGL_* where upstream defines
    // them so)

    fn implements_gl_contexts(&self) -> bool {
        true
    }

    fn gl_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(opengles::kmsdrm_gles_load_library(path))
    }

    fn gl_get_proc_address(&self, proc_name: &str) -> Option<Option<std::ptr::NonNull<c_void>>> {
        Some(crate::video::egl::get_proc_address_internal(proc_name))
    }

    fn gl_unload_library(&self) -> Option<()> {
        opengles::kmsdrm_gles_unload_library();
        Some(())
    }

    fn gl_create_context(&self, window: WindowID) -> Option<Result<RawGlContext>> {
        Some(self.kmsdrm_gles_create_context(window))
    }

    fn gl_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Option<Result<()>> {
        Some(self.kmsdrm_gles_make_current(window, context))
    }

    fn gl_set_swap_interval(&self, interval: i32) -> Option<Result<()>> {
        Some(opengles::kmsdrm_gles_set_swap_interval(interval))
    }

    fn gl_get_swap_interval(&self) -> Option<Result<i32>> {
        Some(crate::video::egl::get_swap_interval())
    }

    fn gl_swap_window(&self, window: WindowID) -> Option<Result<()>> {
        Some(self.kmsdrm_gles_swap_window(window))
    }

    fn gl_destroy_context(&self, context: RawGlContext) -> Option<Result<()>> {
        crate::video::egl::destroy_context(context);
        Some(Ok(()))
    }

    fn gl_set_default_profile_config(&self, config: &mut GlConfig) -> Option<()> {
        opengles::kmsdrm_gles_set_default_profile_config(config);
        Some(())
    }

    // * * * Vulkan (`KMSDRM_Vulkan_*`: upstream built with
    // SDL_VIDEO_VULKAN; no Vulkan_GetPresentationSupport)

    fn implements_vulkan_surfaces(&self) -> bool {
        true
    }

    fn vulkan_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(vulkan::kmsdrm_vulkan_load_library(&self.vulkan, path))
    }

    fn vulkan_unload_library(&self) -> Option<()> {
        vulkan::kmsdrm_vulkan_unload_library(&self.vulkan);
        Some(())
    }

    fn vulkan_loader_path(&self) -> Option<Option<String>> {
        Some(vulkan::kmsdrm_vulkan_loader_path(&self.vulkan))
    }

    fn vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        vulkan::kmsdrm_vulkan_get_instance_proc_addr(&self.vulkan)
    }

    fn vulkan_instance_extensions(&self) -> Option<Vec<&'static str>> {
        Some(vulkan::kmsdrm_vulkan_get_instance_extensions())
    }

    fn vulkan_create_surface(
        &self,
        window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Option<Result<u64>> {
        Some(vulkan::kmsdrm_vulkan_create_surface(
            &self.vulkan,
            window,
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
        vulkan::kmsdrm_vulkan_destroy_surface(&self.vulkan, instance, surface, allocator);
        Some(())
    }

    fn pump_events(&self) -> Option<()> {
        events::kmsdrm_pump_events();
        Some(())
    }

    // * * * The mouse (`KMSDRM_InitMouse()`'s, set with the first window;
    // NULL before). The relative mode is the evdev reader's hook.

    fn warp_mouse(&self, window: WindowID, x: f32, y: f32) -> Option<Result<()>> {
        self.mouse_init()
            .then(|| self.kmsdrm_warp_mouse(window, x, y))
    }

    fn warp_mouse_global(&self, x: f32, y: f32) -> Option<Result<()>> {
        self.mouse_init()
            .then(|| self.kmsdrm_warp_mouse_global(x, y))
    }

    fn create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        self.mouse_init()
            .then(|| mouse::kmsdrm_create_cursor(surface, hot_x, hot_y))
    }

    fn show_cursor(&self, cursor: Option<&Cursor>) -> Option<Result<()>> {
        self.mouse_init().then(|| self.kmsdrm_show_cursor(cursor))
    }

    fn move_cursor(&self, cursor: &Cursor) -> Option<Result<()>> {
        self.mouse_init().then(|| self.kmsdrm_move_cursor(cursor))
    }

    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        matches!(
            feature,
            MouseFeature::WarpMouse | MouseFeature::WarpMouseGlobal | MouseFeature::MoveCursor
        ) && self.mouse_init()
    }
}

impl KmsdrmVideo {
    /// Whether `KMSDRM_InitMouse()` has set the mouse's entry points.
    fn mouse_init(&self) -> bool {
        self.with_state(|s| s.vid.mouse_init)
    }
}

/// Translation of `KMSDRM_CreateDevice()` (as a bootstrap's).
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    let device: Arc<dyn VideoDriver> = video::kmsdrm_create_device()?;
    Some(device)
}

/// Translation of `KMSDRM_bootstrap`.
pub(crate) static KMSDRM_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: "kmsdrm",
    desc: "KMS/DRM Video Driver",
    create: create_device,
    show_message_box: None, // no ShowMessageBox implementation
    is_preferred: false,
};
