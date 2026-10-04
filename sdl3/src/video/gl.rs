// Rust translation of the OpenGL and EGL parts of src/video/SDL_video.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OpenGL front end: library loading, context attributes, contexts.
//!
//! No OpenGL loader is translated yet, so this behaves as an upstream build
//! without `SDL_VIDEO_OPENGL*` and `SDL_VIDEO_OPENGL_EGL`: attributes can't
//! be set or queried, and no video driver creates GL contexts (so OpenGL
//! windows can't be created). The entry points that reach a driver are
//! translated in full for the drivers that will implement them.

use std::cell::Cell;
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::WindowID;

use super::core::{self, dll_not_supported, driver, uninitialized_video, with_device, with_window};
use super::window::Window;

/// An OpenGL context: an opaque handle from the driver. Translation of
/// `SDL_GLContext`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlContext(pub std::num::NonZeroUsize);

/// An OpenGL configuration attribute. Translation of `SDL_GLAttr`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GlAttr {
    /// the minimum number of bits for the red channel of the color buffer; defaults to 8.
    RedSize,
    /// the minimum number of bits for the green channel of the color buffer; defaults to 8.
    GreenSize,
    /// the minimum number of bits for the blue channel of the color buffer; defaults to 8.
    BlueSize,
    /// the minimum number of bits for the alpha channel of the color buffer; defaults to 8.
    AlphaSize,
    /// the minimum number of bits for frame buffer size; defaults to 0.
    BufferSize,
    /// whether the output is single or double buffered; defaults to double buffering on.
    DoubleBuffer,
    /// the minimum number of bits in the depth buffer; defaults to 16.
    DepthSize,
    /// the minimum number of bits in the stencil buffer; defaults to 0.
    StencilSize,
    /// the minimum number of bits for the red channel of the accumulation buffer; defaults to 0.
    AccumRedSize,
    /// the minimum number of bits for the green channel of the accumulation buffer; defaults to 0.
    AccumGreenSize,
    /// the minimum number of bits for the blue channel of the accumulation buffer; defaults to 0.
    AccumBlueSize,
    /// the minimum number of bits for the alpha channel of the accumulation buffer; defaults to 0.
    AccumAlphaSize,
    /// whether the output is stereo 3D; defaults to off.
    Stereo,
    /// the number of buffers used for multisample anti-aliasing; defaults to 0.
    MultisampleBuffers,
    /// the number of samples used around the current pixel used for multisample anti-aliasing.
    MultisampleSamples,
    /// set to 1 to require hardware acceleration, set to 0 to force software rendering; defaults to allow either.
    AcceleratedVisual,
    /// not used (deprecated).
    RetainedBacking,
    /// OpenGL context major version.
    ContextMajorVersion,
    /// OpenGL context minor version.
    ContextMinorVersion,
    /// some combination of 0 or more of the `GL_CONTEXT_*_FLAG`s; defaults to 0.
    ContextFlags,
    /// type of GL context (Core, Compatibility, ES); default value depends on platform.
    ContextProfileMask,
    /// OpenGL context sharing; defaults to 0.
    ShareWithCurrentContext,
    /// requests sRGB capable visual; defaults to 0.
    FramebufferSrgbCapable,
    /// sets context the release behavior; defaults to FLUSH.
    ContextReleaseBehavior,
    /// set context reset notification; defaults to NO_NOTIFICATION.
    ContextResetNotification,
    ContextNoError,
    FloatBuffers,
    EglPlatform,
}

/// OpenGL Core Profile context. Translation of `SDL_GL_CONTEXT_PROFILE_CORE`.
pub const GL_CONTEXT_PROFILE_CORE: i32 = 0x0001;
/// OpenGL Compatibility Profile context. Translation of `SDL_GL_CONTEXT_PROFILE_COMPATIBILITY`.
pub const GL_CONTEXT_PROFILE_COMPATIBILITY: i32 = 0x0002;
/// GLX_CONTEXT_ES2_PROFILE_BIT_EXT. Translation of `SDL_GL_CONTEXT_PROFILE_ES`.
pub const GL_CONTEXT_PROFILE_ES: i32 = 0x0004;
pub const GL_CONTEXT_DEBUG_FLAG: i32 = 0x0001;
pub const GL_CONTEXT_FORWARD_COMPATIBLE_FLAG: i32 = 0x0002;
pub const GL_CONTEXT_ROBUST_ACCESS_FLAG: i32 = 0x0004;
pub const GL_CONTEXT_RESET_ISOLATION_FLAG: i32 = 0x0008;
pub const GL_CONTEXT_RELEASE_BEHAVIOR_NONE: i32 = 0x0000;
pub const GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH: i32 = 0x0001;
pub const GL_CONTEXT_RESET_NO_NOTIFICATION: i32 = 0x0000;
pub const GL_CONTEXT_RESET_LOSE_CONTEXT: i32 = 0x0001;

/// The OpenGL attributes for new contexts. Translation of `gl_config`.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // (read by the GL drivers)
pub(crate) struct GlConfig {
    pub(crate) red_size: i32,
    pub(crate) green_size: i32,
    pub(crate) blue_size: i32,
    pub(crate) alpha_size: i32,
    pub(crate) depth_size: i32,
    pub(crate) buffer_size: i32,
    pub(crate) stencil_size: i32,
    pub(crate) double_buffer: i32,
    pub(crate) accum_red_size: i32,
    pub(crate) accum_green_size: i32,
    pub(crate) accum_blue_size: i32,
    pub(crate) accum_alpha_size: i32,
    pub(crate) stereo: i32,
    pub(crate) multisamplebuffers: i32,
    pub(crate) multisamplesamples: i32,
    pub(crate) floatbuffers: i32,
    pub(crate) accelerated: i32,
    pub(crate) major_version: i32,
    pub(crate) minor_version: i32,
    pub(crate) flags: i32,
    pub(crate) profile_mask: i32,
    pub(crate) share_with_current_context: i32,
    pub(crate) release_behavior: i32,
    pub(crate) reset_notification: i32,
    pub(crate) framebuffer_srgb_capable: i32,
    pub(crate) no_error: i32,
    pub(crate) retained_backing: i32,
    pub(crate) egl_platform: i32,
}

/// The attributes, and the current window and context (`current_glwin`,
/// `current_glctx`) of the video device.
struct GlState {
    config: GlConfig,
    current: Option<(Option<WindowID>, GlContext)>,
}

static GL_STATE: Mutex<GlState> = Mutex::new(GlState {
    config: GlConfig {
        red_size: 0,
        green_size: 0,
        blue_size: 0,
        alpha_size: 0,
        depth_size: 0,
        buffer_size: 0,
        stencil_size: 0,
        double_buffer: 0,
        accum_red_size: 0,
        accum_green_size: 0,
        accum_blue_size: 0,
        accum_alpha_size: 0,
        stereo: 0,
        multisamplebuffers: 0,
        multisamplesamples: 0,
        floatbuffers: 0,
        accelerated: 0,
        major_version: 0,
        minor_version: 0,
        flags: 0,
        profile_mask: 0,
        share_with_current_context: 0,
        release_behavior: 0,
        reset_notification: 0,
        framebuffer_srgb_capable: 0,
        no_error: 0,
        retained_backing: 0,
        egl_platform: 0,
    },
    current: None,
});

fn gl_state() -> std::sync::MutexGuard<'static, GlState> {
    GL_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

thread_local! {
    /// Translation of `current_glwin_tls`.
    static CURRENT_GLWIN: Cell<Option<WindowID>> = const { Cell::new(None) };
    /// Translation of `current_glctx_tls`.
    static CURRENT_GLCTX: Cell<Option<GlContext>> = const { Cell::new(None) };
}

/// The attributes new contexts are created with (for the GL drivers).
#[allow(dead_code)] // (used by the video drivers)
pub(crate) fn gl_config() -> GlConfig {
    gl_state().config.clone()
}

/// The window of the device's current context (`_this->current_glwin`).
pub(crate) fn current_window_id() -> Option<WindowID> {
    gl_state().current.and_then(|(w, _)| w)
}

/// Forget a destroyed window as the current context window.
pub(crate) fn forget_window(window: WindowID) {
    let mut state = gl_state();
    if let Some((w, _)) = &mut state.current {
        if *w == Some(window) {
            *w = None;
        }
    }
}

/// Load an OpenGL library (`None`: the default one); calls are reference
/// counted. Translation of `SDL_GL_LoadLibrary()`.
pub fn gl_load_library(path: Option<&str>) -> Result<()> {
    let (loaded, loaded_path) = with_device(|v| (v.gl_driver_loaded, v.gl_driver_path.clone()))?;
    let driver = driver()?;
    let result = if loaded != 0 {
        if let Some(path) = path {
            if Some(path) != loaded_path.as_deref() {
                return Err(Error::new("OpenGL library already loaded"));
            }
        }
        Ok(())
    } else {
        match driver.gl_load_library(path) {
            None => return Err(dll_not_supported("OpenGL")),
            Some(result) => result,
        }
    };
    match result {
        Ok(()) => {
            with_device(|v| {
                v.gl_driver_loaded += 1;
                if v.gl_driver_path.is_none() {
                    v.gl_driver_path = path.map(str::to_owned);
                }
            })?;
            Ok(())
        }
        Err(e) => {
            let _ = driver.gl_unload_library();
            Err(e)
        }
    }
}

/// Get an OpenGL function by name (once a library is loaded).
/// Translation of `SDL_GL_GetProcAddress()`.
pub fn gl_get_proc_address(proc_name: &str) -> Result<usize> {
    let loaded = with_device(|v| v.gl_driver_loaded)?;
    match driver()?.gl_get_proc_address(proc_name) {
        Some(func) => {
            if loaded != 0 {
                func.ok_or_else(|| Error::new(format!("Couldn't find OpenGL function {proc_name}")))
            } else {
                Err(Error::new("No GL driver has been loaded"))
            }
        }
        None => Err(Error::new(format!(
            "No dynamic GL support in current SDL video driver ({})",
            core::current_video_driver().unwrap_or("")
        ))),
    }
}

/// Get an EGL library function by name. Translation of
/// `SDL_EGL_GetProcAddress()` in a build without EGL.
pub fn egl_get_proc_address(_proc_name: &str) -> Result<usize> {
    Err(Error::new("SDL was not built with EGL support"))
}

/// Unload the OpenGL library loaded by [`gl_load_library`] (when the last
/// reference goes). Translation of `SDL_GL_UnloadLibrary()`.
pub fn gl_unload_library() {
    let Ok(Some(driver)) = with_device(|v| {
        if v.gl_driver_loaded > 0 {
            v.gl_driver_loaded -= 1;
            if v.gl_driver_loaded > 0 {
                return None;
            }
            v.gl_driver_path = None;
            Some(v.driver.clone())
        } else {
            None
        }
    }) else {
        return;
    };
    let _ = driver.gl_unload_library();
}

/// Whether an OpenGL extension is supported by the current context.
/// Translation of `SDL_GL_ExtensionSupported()` in a build without OpenGL.
pub fn gl_extension_supported(_extension: &str) -> bool {
    false
}

/// Reset all previously set OpenGL context attributes to their defaults.
/// Translation of `SDL_GL_ResetAttributes()` (with no OpenGL flavor
/// configured, so the version and profile stay 0).
pub fn gl_reset_attributes() {
    let Ok(driver) = driver() else { return };

    let mut config = GlConfig {
        red_size: 8,
        green_size: 8,
        blue_size: 8,
        alpha_size: 8,
        buffer_size: 0,
        depth_size: 16,
        stencil_size: 0,
        double_buffer: 1,
        accum_red_size: 0,
        accum_green_size: 0,
        accum_blue_size: 0,
        accum_alpha_size: 0,
        stereo: 0,
        multisamplebuffers: 0,
        multisamplesamples: 0,
        floatbuffers: 0,
        retained_backing: 1,
        accelerated: -1, // accelerated or not, both are fine
        major_version: 0,
        minor_version: 0,
        profile_mask: 0,
        flags: 0,
        framebuffer_srgb_capable: 0,
        no_error: 0,
        release_behavior: GL_CONTEXT_RELEASE_BEHAVIOR_FLUSH,
        reset_notification: GL_CONTEXT_RESET_NO_NOTIFICATION,
        share_with_current_context: 0,
        egl_platform: 0,
    };

    let _ = driver.gl_set_default_profile_config(&mut config);
    gl_state().config = config;
}

/// Set an OpenGL window attribute before window creation. Translation of
/// `SDL_GL_SetAttribute()` in a build without OpenGL.
pub fn gl_set_attribute(_attr: GlAttr, _value: i32) -> Result<()> {
    Err(Error::unsupported())
}

/// The actual value of an attribute for the current context. Translation
/// of `SDL_GL_GetAttribute()` in a build without OpenGL.
pub fn gl_get_attribute(_attr: GlAttr) -> Result<i32> {
    Err(Error::unsupported())
}

/// The error for a window without [`WindowFlags::OPENGL`].
fn not_an_opengl_window() -> Error {
    Error::new("The specified window isn't an OpenGL window")
}

/// Create an OpenGL context for an OpenGL window, and make it current.
/// Translation of `SDL_GL_CreateContext()`.
pub fn gl_create_context(window: &Window) -> Result<GlContext> {
    let flags = window.flags()?;
    if !flags.contains(WindowFlags::OPENGL) {
        return Err(not_an_opengl_window());
    }

    let ctx = driver()?
        .gl_create_context(window.id())
        .unwrap_or_else(|| Err(core::context_not_supported("OpenGL")))?;

    // Creating a context is assumed to make it current in the SDL driver.
    gl_state().current = Some((Some(window.id()), ctx));
    CURRENT_GLWIN.with(|c| c.set(Some(window.id())));
    CURRENT_GLCTX.with(|c| c.set(Some(ctx)));

    Ok(ctx)
}

/// Make a context current for a window (`None`, `None`: release the
/// current context). Translation of `SDL_GL_MakeCurrent()`.
pub fn gl_make_current(window: Option<&Window>, context: Option<GlContext>) -> Result<()> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }

    let mut window = window.map(|w| w.id());
    if window == current_thread_window() && context == current_thread_context() {
        // We're already current.
        return Ok(());
    }

    let driver = driver()?;
    if context.is_none() {
        window = None;
    } else if let Some(w) = window {
        if !with_window(w, |w| w.flags().contains(WindowFlags::OPENGL))? {
            return Err(not_an_opengl_window());
        }
    } else if !driver.gl_allow_no_surface() {
        return Err(Error::new(
            "Use of OpenGL without a window is not supported on this platform",
        ));
    }

    driver
        .gl_make_current(window, context)
        .unwrap_or_else(|| Err(core::context_not_supported("OpenGL")))?;
    gl_state().current = context.map(|c| (window, c));
    CURRENT_GLWIN.with(|c| c.set(window));
    CURRENT_GLCTX.with(|c| c.set(context));
    Ok(())
}

fn current_thread_window() -> Option<WindowID> {
    CURRENT_GLWIN.with(|c| c.get())
}

fn current_thread_context() -> Option<GlContext> {
    CURRENT_GLCTX.with(|c| c.get())
}

/// The window of this thread's current context. Translation of
/// `SDL_GL_GetCurrentWindow()`.
pub fn gl_current_window() -> Result<Option<Window>> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }
    Ok(current_thread_window().map(Window::from_raw))
}

/// This thread's current context. Translation of `SDL_GL_GetCurrentContext()`.
pub fn gl_current_context() -> Result<Option<GlContext>> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }
    Ok(current_thread_context())
}

/// The current EGL display. Translation of `SDL_EGL_GetCurrentDisplay()`
/// in a build without EGL.
pub fn egl_current_display() -> Result<usize> {
    Err(Error::new("SDL was not built with EGL support"))
}

/// The current EGL config. Translation of `SDL_EGL_GetCurrentConfig()` in
/// a build without EGL.
pub fn egl_current_config() -> Result<usize> {
    Err(Error::new("SDL was not built with EGL support"))
}

/// The EGL surface of a window. Translation of `SDL_EGL_GetWindowSurface()`
/// in a build without EGL.
pub fn egl_window_surface(_window: &Window) -> Result<usize> {
    Err(Error::new("SDL was not built with EGL support"))
}

/// Set the swap interval for the current context (0 immediate, 1 vsync, -1
/// adaptive vsync). Translation of `SDL_GL_SetSwapInterval()`.
pub fn gl_set_swap_interval(interval: i32) -> Result<()> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }
    if current_thread_context().is_none() {
        return Err(Error::new("No OpenGL context has been made current"));
    }
    driver()?
        .gl_set_swap_interval(interval)
        .unwrap_or_else(|| Err(Error::new("Setting the swap interval is not supported")))
}

/// The swap interval of the current context. Translation of
/// `SDL_GL_GetSwapInterval()`.
pub fn gl_get_swap_interval() -> Result<i32> {
    if !core::initialized() {
        return Err(Error::new("no video driver"));
    }
    if current_thread_context().is_none() {
        return Err(Error::new("no current context"));
    }
    driver()?
        .gl_get_swap_interval()
        .unwrap_or_else(|| Err(Error::new("not implemented")))
}

/// Swap the buffers of an OpenGL window whose context is current.
/// Translation of `SDL_GL_SwapWindow()`.
pub fn gl_swap_window(window: &Window) -> Result<()> {
    if !window.flags()?.contains(WindowFlags::OPENGL) {
        return Err(not_an_opengl_window());
    }

    if current_thread_window() != Some(window.id()) {
        return Err(Error::new("The specified window has not been made current"));
    }

    driver()?
        .gl_swap_window(window.id())
        .unwrap_or_else(|| Err(core::context_not_supported("OpenGL")))
}

/// Delete an OpenGL context. Translation of `SDL_GL_DestroyContext()`.
pub fn gl_destroy_context(context: GlContext) -> Result<()> {
    if !core::initialized() {
        return Err(uninitialized_video());
    }

    if current_thread_context() == Some(context) {
        let _ = gl_make_current(None, None);
    }

    driver()?
        .gl_destroy_context(context)
        .unwrap_or_else(|| Err(core::context_not_supported("OpenGL")))
}
