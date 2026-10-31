// Rust translation of src/video/android/SDL_androidvideo.c and
// SDL_androidvideo.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The android video driver: its one display, from the metrics the
//! activity's surface reports before `SDL_main` runs and whenever it
//! changes (`nativeSetScreenResolution()`: the surface's size, the
//! device's, its density and refresh rate; the pixel format of the EGL
//! window surface; the orientation the activity's rotation gives; the safe
//! area of the window insets), its one window ([`window`]: the activity's
//! `SDLSurface`, its `ANativeWindow` and EGL surface), OpenGL ES through
//! EGL on that window ([`gl`]), Vulkan surfaces on it ([`vulkan`]), the
//! activity's lifecycle ([`events`], which SDL's event pump uses whatever
//! the driver, with the EGL context's backup while the app is paused), and
//! the system's theme ([`set_dark_mode`]), and the input of the surface:
//! the touch screens and their fingers ([`touch`]), the mouse, its cursors
//! and its relative mode ([`mouse`]), the pens ([`pen`]), and the keys and
//! the screen keyboard ([`keyboard`]); the clipboard ([`clipboard`]) and
//! message boxes ([`messagebox`], the bootstrap's).

pub(crate) mod clipboard;
pub(crate) mod events;
pub(crate) mod gl;
pub(crate) mod keyboard;
pub(crate) mod messagebox;
pub(crate) mod mouse;
pub(crate) mod pen;
pub(crate) mod touch;
pub(crate) mod vulkan;
pub(crate) mod window;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::Result;
use crate::events::mouse::{Cursor, MouseFeature, SystemCursor};
use crate::events::{DisplayID, EventType, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::video::display::{add_basic_video_display, set_desktop_display_mode};
use crate::video::gl::RawGlContext;
use crate::video::sysvideo::{
    DeviceCaps, DisplayMode, DisplayOrientation, FullscreenOp, FullscreenResult, SystemTheme,
    VideoBootStrap, VideoDriver,
};
use crate::video::PixelFormat;

const ANDROID_VID_DRIVER_NAME: &str = "android";

/// What the activity reports about its screen: filled in with real values
/// in [`set_screen_resolution`] on init (before `SDL_main()`).
/// Translation of `Android_SurfaceWidth`, `Android_SurfaceHeight`,
/// `Android_DeviceWidth`, `Android_DeviceHeight`, `Android_ScreenFormat`,
/// `Android_ScreenDensity`, `Android_ScreenRate`,
/// `Android_ScreenOrientation` and the `Android_SafeInset*`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Screen {
    pub(crate) surface_width: i32,
    pub(crate) surface_height: i32,
    device_width: i32,
    device_height: i32,
    format: PixelFormat,
    pub(crate) density: f32,
    rate: f32,
    orientation: DisplayOrientation,
    pub(crate) safe_inset_left: i32,
    pub(crate) safe_inset_right: i32,
    pub(crate) safe_inset_top: i32,
    pub(crate) safe_inset_bottom: i32,
}

static SCREEN: Mutex<Screen> = Mutex::new(Screen {
    surface_width: 0,
    surface_height: 0,
    device_width: 0,
    device_height: 0,
    // Default SurfaceView format, in case this is queried before being filled
    format: PixelFormat::RGB565,
    density: 1.0,
    rate: 0.0,
    orientation: DisplayOrientation::Unknown,
    safe_inset_left: 0,
    safe_inset_right: 0,
    safe_inset_top: 0,
    safe_inset_bottom: 0,
});

fn screen_state() -> MutexGuard<'static, Screen> {
    SCREEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// The screen as the activity last reported it.
pub(crate) fn screen() -> Screen {
    *screen_state()
}

/// `Android_SystemTheme` (`SDL_SYSTEM_THEME_UNKNOWN` at first).
static SYSTEM_THEME: AtomicU8 = AtomicU8::new(0);

fn system_theme() -> SystemTheme {
    match SYSTEM_THEME.load(Ordering::Acquire) {
        1 => SystemTheme::Light,
        2 => SystemTheme::Dark,
        _ => SystemTheme::Unknown,
    }
}

/// `Android_Window`, the driver's window (currently only one).
pub(crate) fn window() -> Option<WindowID> {
    window::android_window()
}

/// The android video driver (the function pointers of the
/// `SDL_VideoDevice` that `Android_CreateDevice()` fills in), with its
/// data. (`SDL_VideoData`'s `isPaused` and `isPausing`, which
/// `Android_VideoInit()` clears and nothing reads, are left out.)
#[derive(Default)]
struct AndroidVideo {
    /// The Vulkan loader (`_this->vulkan_config`).
    vulkan: Mutex<vulkan::VulkanConfig>,
    /// `Android_Video_InternalHintCallback()`, while video is initialized.
    trap_back_button: Mutex<Option<hints::Callback>>,
}

/// Input is part of video for Android, so this goes here: the back
/// button's trap follows `SDL_HINT_ANDROID_TRAP_BACK_BUTTON`. Translation
/// of `Android_Video_InternalHintCallback()`.
fn video_internal_hint_callback(change: &hints::HintChange<'_>) {
    crate::core::android::set_back_button_trap_active(hints::string_to_bool(
        change.new_value,
        false,
    ));
}

impl VideoDriver for AndroidVideo {
    fn caps(&self) -> DeviceCaps {
        DeviceCaps::SENDS_FULLSCREEN_DIMENSIONS
    }

    /// Translation of `Android_VideoInit()` (with `Android_CreateDevice()`'s
    /// `device->system_theme`, which exists once the driver is chosen).
    fn video_init(&self) -> Result<()> {
        let _ = crate::video::core::with_device(|v| v.system_theme = system_theme());

        let s = screen();
        let mode = DisplayMode {
            format: s.format,
            w: s.device_width,
            h: s.device_height,
            refresh_rate: s.rate,
            ..Default::default()
        };

        let display_id = add_basic_video_display(Some(&mode));
        if display_id == 0 {
            return Err(crate::video::core::uninitialized_video());
        }
        let _ = crate::video::display::with_video_display(display_id, |display| {
            display.natural_orientation = crate::core::android::display_natural_orientation();
            display.current_orientation = crate::core::android::display_current_orientation();
            display.content_scale = s.density;
        });

        *self
            .trap_back_button
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = hints::watch(
            hints::ANDROID_TRAP_BACK_BUTTON,
            video_internal_hint_callback,
        )
        .ok();

        touch::init_touch();

        mouse::init_mouse();

        // We're done!
        Ok(())
    }

    /// Translation of `Android_VideoQuit()`.
    fn video_quit(&self) {
        drop(
            self.trap_back_button
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take(),
        );
        mouse::quit_mouse();
        touch::quit_touch();
    }

    fn create_window(&self, window: WindowID, create_props: &Properties) -> Option<Result<()>> {
        Some(window::create_window(window, create_props))
    }

    fn set_window_title(&self, window: WindowID) -> Option<()> {
        window::set_window_title(window);
        Some(())
    }

    fn set_window_fullscreen(
        &self,
        window: WindowID,
        display: DisplayID,
        fullscreen: FullscreenOp,
    ) -> Option<FullscreenResult> {
        Some(window::set_window_fullscreen(window, display, fullscreen))
    }

    fn minimize_window(&self, window: WindowID) -> Option<()> {
        window::minimize_window(window);
        Some(())
    }

    fn set_window_resizable(&self, window: WindowID, resizable: bool) -> Option<()> {
        window::set_window_resizable(window, resizable);
        Some(())
    }

    fn destroy_window(&self, window: WindowID) -> Option<()> {
        window::destroy_window(window);
        Some(())
    }

    // * * * GL pointers (`Android_GLES_*`; SDL_EGL_* where upstream defines
    // them so)

    fn implements_gl_contexts(&self) -> bool {
        true
    }

    fn gl_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(gl::gles_load_library(path))
    }

    fn gl_get_proc_address(
        &self,
        proc_name: &str,
    ) -> Option<Option<std::ptr::NonNull<std::ffi::c_void>>> {
        Some(crate::video::egl::get_proc_address_internal(proc_name))
    }

    fn gl_unload_library(&self) -> Option<()> {
        crate::video::egl::unload_library();
        Some(())
    }

    fn gl_create_context(&self, window: WindowID) -> Option<Result<RawGlContext>> {
        Some(gl::gles_create_context(window))
    }

    fn gl_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Option<Result<()>> {
        Some(gl::gles_make_current(window, context))
    }

    fn gl_set_swap_interval(&self, interval: i32) -> Option<Result<()>> {
        Some(crate::video::egl::set_swap_interval(interval))
    }

    fn gl_get_swap_interval(&self) -> Option<Result<i32>> {
        Some(crate::video::egl::get_swap_interval())
    }

    fn gl_swap_window(&self, window: WindowID) -> Option<Result<()>> {
        Some(gl::gles_swap_window(window))
    }

    fn gl_destroy_context(&self, context: RawGlContext) -> Option<Result<()>> {
        crate::video::egl::destroy_context(context);
        Some(Ok(()))
    }

    // Note (upstream): there is no GL_GetEGLSurface: SDL_EGL_GetWindowSurface()
    // gives NULL for the android window, whose EGL surface is its
    // SDL_PROP_WINDOW_ANDROID_SURFACE_POINTER only.

    // * * * Vulkan (`Android_Vulkan_*`)

    fn implements_vulkan_surfaces(&self) -> bool {
        true
    }

    fn vulkan_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(vulkan::vulkan_load_library(&self.vulkan, path))
    }

    fn vulkan_unload_library(&self) -> Option<()> {
        vulkan::vulkan_unload_library(&self.vulkan);
        Some(())
    }

    fn vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        vulkan::vulkan_get_instance_proc_addr(&self.vulkan)
    }

    fn vulkan_instance_extensions(&self) -> Option<Vec<&'static str>> {
        Some(vulkan::vulkan_get_instance_extensions())
    }

    fn vulkan_create_surface(
        &self,
        window: WindowID,
        instance: usize,
        allocator: usize,
    ) -> Option<Result<u64>> {
        Some(vulkan::vulkan_create_surface(
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
        vulkan::vulkan_destroy_surface(&self.vulkan, instance, surface, allocator);
        Some(())
    }

    // * * * The mouse (`Android_InitMouse()`'s)

    fn create_cursor(
        &self,
        surface: &crate::video::Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(mouse::create_cursor(surface, hot_x, hot_y))
    }

    fn create_system_cursor(&self, id: SystemCursor) -> Option<Result<Cursor>> {
        Some(Ok(mouse::create_system_cursor(id)))
    }

    fn show_cursor(&self, cursor: Option<&Cursor>) -> Option<Result<()>> {
        Some(mouse::show_cursor(cursor))
    }

    fn set_relative_mouse_mode(&self, enabled: bool) -> Option<Result<()>> {
        Some(mouse::set_relative_mouse_mode(enabled))
    }

    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        matches!(feature, MouseFeature::SetRelativeMouseMode)
    }

    // * * * Screensaver

    /// Translation of `Android_SuspendScreenSaver()`.
    fn suspend_screen_saver(&self, suspend: bool) -> Option<Result<()>> {
        // (a false result without an error: an empty message, as for the
        // other Android functions upstream returns false from without
        // setting one)
        Some(if crate::core::android::suspend_screen_saver(suspend) {
            Ok(())
        } else {
            Err(crate::error::Error::new(""))
        })
    }

    // * * * Screen keyboard (`Android_*ScreenKeyboard*()`)

    fn has_screen_keyboard_support(&self) -> Option<bool> {
        Some(keyboard::has_screen_keyboard_support())
    }

    fn show_screen_keyboard(&self, window: WindowID, props: Option<&Properties>) -> Option<()> {
        keyboard::show_screen_keyboard(window, props);
        Some(())
    }

    fn hide_screen_keyboard(&self, window: WindowID) -> Option<()> {
        keyboard::hide_screen_keyboard(window);
        Some(())
    }

    // * * * Clipboard

    fn implements_clipboard_text(&self) -> bool {
        true
    }

    fn set_clipboard_text(&self, text: &str) -> Option<Result<()>> {
        Some(clipboard::set_clipboard_text(text))
    }

    fn clipboard_text(&self) -> Option<Option<String>> {
        Some(Some(clipboard::get_clipboard_text()))
    }

    fn has_clipboard_text(&self) -> Option<bool> {
        Some(clipboard::has_clipboard_text())
    }
}

/// Translation of `Android_CreateDevice()` (whose data is
/// [`AndroidVideo`]'s). Note: upstream always runs in an app; a program
/// that isn't one (spawned from the shell) has no Java VM, and so no
/// activity and no surface: there the next driver gets its chance, as on
/// UIKit outside an app.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    crate::core::android::env::java_vm()?;
    Some(Arc::new(AndroidVideo::default()))
}

/// Translation of `Android_bootstrap`.
pub(crate) static ANDROID_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: ANDROID_VID_DRIVER_NAME,
    desc: "SDL Android video driver",
    create: create_device,
    show_message_box: Some(messagebox::show_message_box),
    is_preferred: false,
};

/// Called by the JNI layer when the screen changes size or format.
/// Translation of `Android_SetScreenResolution()`.
pub(crate) fn set_screen_resolution(
    surface_width: i32,
    surface_height: i32,
    device_width: i32,
    device_height: i32,
    density: f32,
    rate: f32,
) {
    let mut s = screen_state();
    s.surface_width = surface_width;
    s.surface_height = surface_height;
    s.device_width = device_width;
    s.device_height = device_height;
    s.density = if density > 0.0 { density } else { 1.0 };
    s.rate = rate;
}

/// `AHARDWAREBUFFER_FORMAT_*` (and the HAL's) to SDL's. Translation of
/// `format_to_pixelFormat()`.
fn format_to_pixel_format(format: i32) -> PixelFormat {
    if format == 1 {
        // AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM
        PixelFormat::RGBA8888
    } else if format == 2 {
        // AHARDWAREBUFFER_FORMAT_R8G8B8X8_UNORM
        PixelFormat::RGBX8888
    } else if format == 3 {
        // AHARDWAREBUFFER_FORMAT_R8G8B8_UNORM
        PixelFormat::RGB24
    } else if format == 4 {
        // AHARDWAREBUFFER_FORMAT_R5G6B5_UNORM
        PixelFormat::RGB565
    } else if format == 5 {
        PixelFormat::BGRA8888
    } else if format == 6 {
        PixelFormat::RGBA5551
    } else if format == 7 {
        PixelFormat::RGBA4444
    } else if format == 0x115 {
        // HAL_PIXEL_FORMAT_BGR_565
        PixelFormat::RGB565
    } else {
        PixelFormat::UNKNOWN
    }
}

/// The format an EGL window surface was asked for (its config's
/// `EGL_NATIVE_VISUAL_ID`) and got: the display's from then on, logged.
/// Translation of `Android_SetFormat()`.
pub(crate) fn set_format(format_wanted: i32, format_got: i32) {
    let pf_wanted = format_to_pixel_format(format_wanted);
    let pf_got = format_to_pixel_format(format_got);

    screen_state().format = pf_got;

    crate::log::log_app!(
        "pixel format wanted {} ({}), got {} ({})",
        pf_wanted.name(),
        format_wanted,
        pf_got.name(),
        format_got
    );
}

/// The first display (`device->displays[0]`), when video is up.
fn first_display() -> Option<(DisplayID, DisplayMode)> {
    crate::video::core::with_device(|v| v.displays.first().map(|d| (d.id, d.desktop_mode)))
        .ok()
        .flatten()
}

/// If we've received a compatible resize event, update the orientation
/// immediately, otherwise wait for the display resize event. Translation
/// of `Android_SendOrientationUpdate()`.
fn send_orientation_update() {
    if let Some((display, desktop_mode)) = first_display() {
        let orientation = screen_state().orientation;
        let mode_landscape = desktop_mode.w > desktop_mode.h;
        let sensor_landscape = orientation == DisplayOrientation::Landscape
            || orientation == DisplayOrientation::LandscapeFlipped;
        if sensor_landscape == mode_landscape {
            crate::events::window::send_display_event(
                display,
                EventType::DISPLAY_ORIENTATION,
                orientation as i32,
                0,
            );
        }
    }
}

/// Translation of `Android_SetOrientation()`.
pub(crate) fn set_orientation(orientation: DisplayOrientation) {
    screen_state().orientation = orientation;
    send_orientation_update();
}

/// Update the resolution of the desktop mode, so that the window can be
/// properly resized (the screen resolution change can for example happen
/// when the activity enters or exits immersive mode, which can happen
/// after `VideoInit()`), and resize `window`. Translation of
/// `Android_SendResize()`.
pub(crate) fn send_resize(window: Option<WindowID>) {
    if let Some((display, _)) = first_display() {
        let s = screen();
        let desktop_mode = DisplayMode {
            format: s.format,
            w: s.device_width,
            h: s.device_height,
            refresh_rate: s.rate,
            ..Default::default()
        };
        set_desktop_display_mode(display, &desktop_mode);
        send_orientation_update();
    }

    if let Some(window) = window {
        let s = screen();
        crate::events::window::send_window_event(
            window,
            EventType::WINDOW_RESIZED,
            s.surface_width,
            s.surface_height,
        );
    }
}

/// Translation of `Android_SetWindowSafeAreaInsets()`.
pub(crate) fn set_window_safe_area_insets(left: i32, right: i32, top: i32, bottom: i32) {
    {
        let mut s = screen_state();
        s.safe_inset_left = left;
        s.safe_inset_right = right;
        s.safe_inset_top = top;
        s.safe_inset_bottom = bottom;
    }

    if let Some(window) = window() {
        crate::video::core::set_window_safe_area_insets(window, left, right, top, bottom);
    }
}

/// The system's theme, from the activity's configuration (its night
/// mode), passed on to video (`SDL_SetSystemTheme()`, which sends
/// `SDL_EVENT_SYSTEM_THEME_CHANGED` when it changes) when it's
/// initialized, whatever its driver. Translation of `Android_SetDarkMode()`.
pub(crate) fn set_dark_mode(enabled: bool) {
    let initialized = crate::video::core::initialized();

    if enabled {
        SYSTEM_THEME.store(2, Ordering::Release); // SDL_SYSTEM_THEME_DARK
    } else {
        SYSTEM_THEME.store(1, Ordering::Release); // SDL_SYSTEM_THEME_LIGHT
    }

    if initialized {
        crate::video::core::set_system_theme(system_theme());
    }
}
