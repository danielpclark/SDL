// Rust translation of src/video/cocoa/SDL_cocoavideo.m and
// SDL_cocoavideo.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Cocoa video driver (macOS): `NSApplication` and its menus, the event
//! pump, `NSWindow`s with their delegate and content view, displays and
//! modes through CoreGraphics, the keyboard (keymaps from the keyboard
//! layout, text input and the input method through `NSTextInputClient`),
//! the mouse (cursors, warping, relative mode, `GCMouse` raw input), pens,
//! the pasteboard, `NSAlert` message boxes and window shapes.
//!
//! Upstream writes the driver in Objective-C; its classes (`SDL3Application`,
//! `SDL3AppDelegate`, `SDL3Window`, `SDL3Cocoa_WindowListener`, `SDL3View`,
//! `SDL3TranslatorResponder`, ...) are declared at run time here, their
//! methods implemented in Rust (see [`crate::core::apple::objc`]). The
//! Objective-C objects upstream keeps the driver state in
//! (`SDL_CocoaVideoData`, `SDL_CocoaWindowData`) are Rust structures.
//!
//! As upstream, the driver only starts on the main thread: AppKit must be
//! used from there.
//!
//! Metal views are `CAMetalLayer`-backed subviews of the content view
//! ([`metalview`]), on which Vulkan surfaces are made with MoltenVK
//! ([`vulkan`]). OpenGL contexts are `NSOpenGLContext`s ([`opengl`]); with an OpenGL ES
//! profile, or [`hints::VIDEO_FORCE_EGL`], the driver switches to EGL
//! ([`opengles`], with ANGLE). Windows are OpenGL windows by default, as
//! upstream's `SDL_DefaultGraphicsBackends()` makes them on macOS.

#![allow(non_upper_case_globals, non_snake_case)] // (the SDK's names are kept)

pub(crate) mod clipboard;
pub(crate) mod events;
pub(crate) mod keyboard;
pub(crate) mod messagebox;
pub(crate) mod metalview;
pub(crate) mod modes;
pub(crate) mod mouse;
pub(crate) mod opengl;
pub(crate) mod opengles;
pub(crate) mod pen;
pub(crate) mod shape;
pub(crate) mod sys;
pub(crate) mod vulkan;
pub(crate) mod window;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::core::apple::foundation::{ns_string, string_from_ns, CGFloat, CfRef, NSSize};
use crate::core::apple::objc::{
    autoreleasepool, class, msg_send, Bool, Id, NSInteger, StrongPtr, NO, YES,
};
use crate::error::{Error, Result};
use crate::events::mouse::{Cursor, CursorFrame, MouseButtonFlags, MouseFeature, SystemCursor};
use crate::events::{DisplayID, WindowID};
use crate::hints;
use crate::properties::Properties;
use crate::video::gl::RawGlContext;
use crate::video::sysvideo::{
    DeviceCaps, DisplayMode, FlashOperation, FullscreenOp, FullscreenResult, SystemTheme,
    VideoBootStrap, VideoDriver,
};
use crate::video::window::WindowOp;
use crate::video::{PixelFormat, Rect, Surface};

/// Translation of `OptionAsAlt`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OptionAsAlt {
    None,
    OnlyLeft,
    OnlyRight,
    Both,
}

/// The mutable part of `SDL_CocoaVideoData`.
pub(crate) struct VideoState {
    pub(crate) allow_spaces: bool,
    pub(crate) trackpad_is_touch_only: bool,
    pub(crate) modifier_flags: u32,
    /// The keyboard layout the keymap was made from (`key_layout`; owned
    /// here, where upstream leaks the copies it compares).
    pub(crate) key_layout: Option<CfRef>,
    /// The `SDL3TranslatorResponder` (`fieldEdit`).
    pub(crate) field_edit: StrongPtr,
    pub(crate) clipboard_count: NSInteger,
    pub(crate) screensaver_assertion: sys::IOPMAssertionID,
    pub(crate) option_as_alt: OptionAsAlt,
    pub(crate) main_display_height: CGFloat,
    /// The hint callbacks `Cocoa_VideoInit()` and `Cocoa_InitKeyboard()`
    /// add (removed on quit).
    pub(crate) hint_callbacks: Vec<hints::Callback>,
}

/// Translation of `SDL_CocoaVideoData`.
pub(crate) struct VideoData {
    state: Mutex<VideoState>,
    /// `swaplock`: serializes the OpenGL contexts' buffer swaps (for
    /// `SDL_cocoaopengl.m`).
    pub(crate) swaplock: Mutex<()>,
}

impl VideoData {
    /// The driver state. Never call into AppKit or SDL's event functions
    /// while holding it (copy what's needed out first).
    pub(crate) fn state(&self) -> MutexGuard<'_, VideoState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `mainDisplayHeight`: the height of the main display, which flips
    /// Cocoa's bottom-up coordinates to SDL's top-down ones.
    pub(crate) fn main_display_height(&self) -> CGFloat {
        self.state().main_display_height
    }
}

/// The driver data of the running Cocoa video driver
/// (`SDL_GetVideoDevice()->internal`, for the Objective-C callbacks).
static VIDEO_DATA: Mutex<Option<Arc<VideoData>>> = Mutex::new(None);

/// The driver data of the running Cocoa video driver, if it is the current
/// one.
pub(crate) fn video_data() -> Option<Arc<VideoData>> {
    VIDEO_DATA.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Whether the Cocoa driver is the current video driver
/// (`SDL_strcmp(_this->name, "cocoa") == 0`).
pub(crate) fn is_current_driver() -> bool {
    crate::video::core::with_device(|v| v.name == "cocoa").unwrap_or(false)
}

/// The Cocoa video driver (the function pointers of the `SDL_VideoDevice`
/// that `Cocoa_CreateDevice()` fills in).
pub(crate) struct CocoaVideo {
    data: Arc<VideoData>,
}

impl Drop for CocoaVideo {
    /// Translation of `Cocoa_DeleteDevice()`.
    fn drop(&mut self) {
        let mut current = VIDEO_DATA.lock().unwrap_or_else(|e| e.into_inner());
        if current.as_ref().is_some_and(|d| Arc::ptr_eq(d, &self.data)) {
            *current = None;
        }
    }
}

/// Translation of `Cocoa_CreateDevice()`.
fn create_device() -> Option<Arc<dyn VideoDriver>> {
    autoreleasepool(|| {
        if !crate::core::apple::foundation::is_main_thread() {
            return None; // this doesn't SDL_SetError() because SDL_InitVideo is just going to overwrite it.
        }

        events::register_app();

        // Initialize all variables that we clean on shutdown
        let data = Arc::new(VideoData {
            state: Mutex::new(VideoState {
                allow_spaces: false,
                trackpad_is_touch_only: false,
                modifier_flags: 0,
                key_layout: None,
                field_edit: StrongPtr::nil(),
                clipboard_count: 0,
                screensaver_assertion: sys::kIOPMNullAssertionID,
                option_as_alt: OptionAsAlt::None,
                main_display_height: 0.0,
                hint_callbacks: Vec::new(),
            }),
            swaplock: Mutex::new(()),
        });
        *VIDEO_DATA.lock().unwrap_or_else(|e| e.into_inner()) = Some(data.clone());

        // (the device's GL_* functions: NSOpenGL's, or EGL's when forced)
        opengl::set_gl_backend(if hints::get_bool(hints::VIDEO_FORCE_EGL, false) {
            opengl::GlBackend::Egl
        } else {
            opengl::GlBackend::Cgl
        });

        let video: Arc<dyn VideoDriver> = Arc::new(CocoaVideo { data });
        Some(video)
    })
}

/// Translation of `COCOA_bootstrap`.
pub(crate) static COCOA_BOOTSTRAP: VideoBootStrap = VideoBootStrap {
    name: "cocoa",
    desc: "SDL Cocoa video driver",
    create: create_device,
    show_message_box: Some(messagebox::show_message_box),
    is_preferred: false,
};

/// Translation of `Cocoa_VideoInit()`.
fn video_init(data: &Arc<VideoData>) -> Result<()> {
    autoreleasepool(|| {
        modes::init_modes(data);
        keyboard::init_keyboard(data);
        mouse::init_mouse(data)?;
        pen::init_pen()?;

        // Initialize GCMouse for raw input on macOS 11.0+
        mouse::init_gc_mouse();

        // Add default keyboard and mouse if GCMouse didn't provide any
        crate::events::keyboard::add_keyboard(crate::events::keyboard::DEFAULT_KEYBOARD_ID, None);
        if !mouse::has_gc_mouse() {
            crate::events::mouse::add_mouse(crate::events::mouse::DEFAULT_MOUSE_ID, None);
        }

        let allow_spaces = hints::get_bool(hints::VIDEO_MAC_FULLSCREEN_SPACES, true);
        let trackpad_is_touch_only = hints::get_bool(hints::TRACKPAD_IS_TOUCH_ONLY, false);
        let watch = hints::watch(hints::VIDEO_MAC_FULLSCREEN_MENU_VISIBILITY, |change| {
            window::menu_visibility_callback(change.new_value)
        })
        .ok();
        {
            let mut st = data.state();
            st.allow_spaces = allow_spaces;
            st.trackpad_is_touch_only = trackpad_is_touch_only;
            st.hint_callbacks.extend(watch);
        }

        // (the swaplock is made with the driver data)
        Ok(())
    })
}

/// Translation of `Cocoa_VideoQuit()`.
fn video_quit(data: &Arc<VideoData>) {
    autoreleasepool(|| {
        // Run the CFRunLoop a little, so last-minute window animations can run, etc, but we hopefully don't change any significant state.
        // This needs to run multiple times over a time period, it's not enough to give it a longer timeout and use "false" for returnAfterSourceHandled.
        let start = crate::timer::ticks_ms();
        loop {
            // SAFETY: runs the current thread's run loop once, without blocking.
            unsafe {
                crate::core::apple::foundation::CFRunLoopRunInMode(
                    crate::core::apple::foundation::kCFRunLoopDefaultMode,
                    0.0,
                    0,
                );
            }
            if crate::timer::ticks_ms() - start > 20 {
                break;
            }
        }

        modes::quit_modes(data);
        keyboard::quit_keyboard(data);
        mouse::quit_gc_mouse();
        mouse::quit_mouse();
        pen::quit_pen();

        let callbacks = std::mem::take(&mut data.state().hint_callbacks);
        drop(callbacks);
    })
}

/// The system theme, from the app's effective appearance. Translation of
/// `Cocoa_GetSystemTheme()`.
///
/// This function assumes that it's called from within an autorelease pool
pub(crate) fn get_system_theme() -> SystemTheme {
    if sys::macos_available(10, 14) {
        // SAFETY: NSApplication's shared instance and its appearance.
        let dark = unsafe {
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let appearance: Id = msg_send![app, effectiveAppearance];
            let name: Id = msg_send![appearance, name];
            let dark = ns_string("Dark");
            let r: Bool = msg_send![name, containsString: dark.get()];
            r.is_true()
        };
        if dark {
            return SystemTheme::Dark;
        }
    }
    SystemTheme::Light
}

/// An `NSImage` of a surface (with its alternate images as
/// representations), owned. Translation of `Cocoa_CreateImage()`.
///
/// This function assumes that it's called from within an autorelease pool
pub(crate) fn create_image(surface: &Surface<'_>) -> Option<StrongPtr> {
    // SAFETY: NSImage and NSBitmapImageRep are created and filled as the
    // AppKit documentation describes; the bitmap's buffer is as large as
    // its height times its row bytes.
    unsafe {
        let img: Id = msg_send![class!(NSImage), alloc];
        let img: Id = msg_send![img, initWithSize: NSSize::new(surface.width() as f64, surface.height() as f64)];
        let img = StrongPtr::from_owned(img);
        if img.is_nil() {
            return None;
        }

        let images = std::iter::once(surface).chain(surface.alternate_images().iter());
        for image in images {
            let mut converted = image.convert(PixelFormat::RGBA32).ok()?;

            // Premultiply the alpha channel
            converted.premultiply_alpha(false).ok()?;

            let (w, h, pitch) = (converted.width(), converted.height(), converted.pitch());
            let imgrep: Id = msg_send![class!(NSBitmapImageRep), alloc];
            let imgrep: Id = msg_send![imgrep,
                initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>()
                pixelsWide: w as NSInteger
                pixelsHigh: h as NSInteger
                bitsPerSample: 8 as NSInteger
                samplesPerPixel: 4 as NSInteger
                hasAlpha: YES
                isPlanar: NO
                colorSpaceName: sys::NSDeviceRGBColorSpace
                bytesPerRow: pitch as NSInteger
                bitsPerPixel: converted.format().bits_per_pixel() as NSInteger];
            let imgrep = StrongPtr::from_owned(imgrep);
            if imgrep.is_nil() {
                return None;
            }

            // Copy the pixels
            let pixels: *mut u8 = msg_send![imgrep, bitmapData];
            let src = converted.pixels()?;
            let len = h as usize * pitch as usize;
            std::ptr::copy_nonoverlapping(src.as_ptr(), pixels, len.min(src.len()));

            // Add the image representation
            let _: () = msg_send![img, addRepresentation: imgrep.get()];
        }

        Some(img)
    }
}

/// `SDL_NSLog()`: write a log line to the unified log through `NSLog`.
///
/// This doesn't really have anything to do with the interfaces of the SDL video
///  subsystem, but upstream needs to stuff this into an Objective-C source code file.
#[allow(dead_code)]
pub(crate) fn ns_log(prefix: &str, text: &str) {
    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        fn NSLog(format: Id, ...);
    }
    autoreleasepool(|| {
        let format = ns_string("%@%@");
        let ns_prefix = ns_string(prefix);
        let ns_text = ns_string(text);
        // SAFETY: a format string with two object arguments.
        unsafe { NSLog(format.get(), ns_prefix.get(), ns_text.get()) };
    })
}

impl VideoDriver for CocoaVideo {
    fn caps(&self) -> DeviceCaps {
        DeviceCaps::HAS_POPUP_WINDOW_SUPPORT | DeviceCaps::SENDS_FULLSCREEN_DIMENSIONS
    }

    fn video_init(&self) -> Result<()> {
        // (Cocoa_CreateDevice() sets device->system_theme, which exists
        // once the driver is chosen; no event is sent)
        let theme = autoreleasepool(get_system_theme);
        let _ = crate::video::core::with_device(|v| v.system_theme = theme);
        video_init(&self.data)
    }

    fn video_quit(&self) {
        video_quit(&self.data)
    }

    // Display functions

    fn display_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        Some(modes::get_display_bounds(display))
    }

    fn display_usable_bounds(&self, display: DisplayID) -> Option<Result<Rect>> {
        Some(modes::get_display_usable_bounds(display))
    }

    fn display_modes(&self, display: DisplayID) -> Option<()> {
        modes::get_display_modes(display);
        Some(())
    }

    fn set_display_mode(&self, display: DisplayID, mode: &DisplayMode) -> Option<Result<()>> {
        Some(modes::set_display_mode(display, mode))
    }

    // Window functions

    fn create_window(&self, window: WindowID, create_props: &Properties) -> Option<Result<()>> {
        Some(window::create_window(&self.data, window, create_props))
    }
    fn set_window_title(&self, window: WindowID) -> Option<()> {
        window::set_window_title(window);
        Some(())
    }
    fn set_window_icon(&self, window: WindowID, icon: &Surface<'static>) -> Option<Result<()>> {
        Some(window::set_window_icon(window, icon))
    }
    fn set_window_position(&self, window: WindowID) -> Option<Result<()>> {
        Some(window::set_window_position(window))
    }
    fn set_window_size(&self, window: WindowID) -> Option<()> {
        window::set_window_size(window);
        Some(())
    }
    fn set_window_minimum_size(&self, window: WindowID) -> Option<()> {
        window::set_window_minimum_size(window);
        Some(())
    }
    fn set_window_maximum_size(&self, window: WindowID) -> Option<()> {
        window::set_window_maximum_size(window);
        Some(())
    }
    fn set_window_aspect_ratio(&self, window: WindowID) -> Option<()> {
        window::set_window_aspect_ratio(window);
        Some(())
    }
    fn window_size_in_pixels(&self, window: WindowID) -> Option<(i32, i32)> {
        window::get_window_size_in_pixels(window)
    }
    fn set_window_opacity(&self, window: WindowID, opacity: f32) -> Option<Result<()>> {
        Some(window::set_window_opacity(window, opacity))
    }
    fn set_window_parent(&self, window: WindowID, parent: Option<WindowID>) -> Option<Result<()>> {
        Some(window::set_window_parent(window, parent))
    }
    fn set_window_modal(&self, window: WindowID, modal: bool) -> Option<Result<()>> {
        Some(window::set_window_modal(window, modal))
    }
    fn show_window(&self, window: WindowID) -> Option<()> {
        window::show_window(window);
        Some(())
    }
    fn hide_window(&self, window: WindowID) -> Option<()> {
        window::hide_window(window);
        Some(())
    }
    fn raise_window(&self, window: WindowID) -> Option<()> {
        window::raise_window(window);
        Some(())
    }
    fn maximize_window(&self, window: WindowID) -> Option<()> {
        window::maximize_window(window);
        Some(())
    }
    fn minimize_window(&self, window: WindowID) -> Option<()> {
        window::minimize_window(window);
        Some(())
    }
    fn restore_window(&self, window: WindowID) -> Option<()> {
        window::restore_window(window);
        Some(())
    }
    fn set_window_bordered(&self, window: WindowID, bordered: bool) -> Option<()> {
        window::set_window_bordered(window, bordered);
        Some(())
    }
    fn set_window_resizable(&self, window: WindowID, resizable: bool) -> Option<()> {
        window::set_window_resizable(window, resizable);
        Some(())
    }
    fn set_window_always_on_top(&self, window: WindowID, on_top: bool) -> Option<()> {
        window::set_window_always_on_top(window, on_top);
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
    fn window_icc_profile(&self, window: WindowID) -> Option<Result<Vec<u8>>> {
        Some(window::get_window_icc_profile(window))
    }
    fn display_for_window(&self, window: WindowID) -> Option<DisplayID> {
        Some(window::get_display_for_window(window))
    }
    fn set_window_mouse_rect(&self, window: WindowID) -> Option<Result<()>> {
        Some(window::set_window_mouse_rect(window))
    }
    fn set_window_mouse_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        Some(window::set_window_mouse_grab(window, grabbed))
    }
    fn set_window_keyboard_grab(&self, window: WindowID, grabbed: bool) -> Option<Result<()>> {
        Some(keyboard::set_window_keyboard_grab(window, grabbed))
    }
    fn destroy_window(&self, window: WindowID) -> Option<()> {
        window::destroy_window(window);
        Some(())
    }
    fn update_window_shape(
        &self,
        window: WindowID,
        shape: Option<&Surface<'static>>,
    ) -> Option<Result<()>> {
        Some(shape::update_window_shape(window, shape))
    }
    fn flash_window(&self, window: WindowID, operation: FlashOperation) -> Option<Result<()>> {
        Some(window::flash_window(window, operation))
    }
    fn set_window_focusable(&self, window: WindowID, focusable: bool) -> Option<Result<()>> {
        Some(window::set_window_focusable(window, focusable))
    }
    fn sync_window(&self, window: WindowID) -> Option<Result<()>> {
        Some(if window::sync_window(window) {
            Ok(())
        } else {
            Err(Error::new(
                "Timed out waiting for window operations to complete",
            ))
        })
    }

    // OpenGL support (`Cocoa_GL_*` or `Cocoa_GLES_*`)

    fn implements_gl_contexts(&self) -> bool {
        true
    }
    fn gl_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(match opengl::gl_backend() {
            opengl::GlBackend::Cgl => opengl::cgl_load_library(path),
            opengl::GlBackend::Egl => opengles::gles_load_library(path),
        })
    }
    fn gl_unload_library(&self) -> Option<()> {
        match opengl::gl_backend() {
            opengl::GlBackend::Cgl => opengl::cgl_unload_library(),
            opengl::GlBackend::Egl => crate::video::egl::unload_library(),
        }
        Some(())
    }
    fn gl_get_proc_address(
        &self,
        proc_name: &str,
    ) -> Option<Option<std::ptr::NonNull<std::ffi::c_void>>> {
        Some(match opengl::gl_backend() {
            opengl::GlBackend::Cgl => opengl::cgl_get_proc_address(proc_name),
            opengl::GlBackend::Egl => crate::video::egl::get_proc_address_internal(proc_name),
        })
    }
    fn gl_create_context(&self, window: WindowID) -> Option<Result<RawGlContext>> {
        Some(match opengl::gl_backend() {
            opengl::GlBackend::Cgl => opengl::cgl_create_context(window),
            opengl::GlBackend::Egl => opengles::gles_create_context(window),
        })
    }
    fn gl_make_current(
        &self,
        window: Option<WindowID>,
        context: Option<RawGlContext>,
    ) -> Option<Result<()>> {
        Some(match opengl::gl_backend() {
            opengl::GlBackend::Cgl => opengl::cgl_make_current(window, context),
            opengl::GlBackend::Egl => opengles::gles_make_current(window, context),
        })
    }
    fn gl_set_swap_interval(&self, interval: i32) -> Option<Result<()>> {
        Some(match opengl::gl_backend() {
            opengl::GlBackend::Cgl => opengl::cgl_set_swap_interval(interval),
            opengl::GlBackend::Egl => crate::video::egl::set_swap_interval(interval),
        })
    }
    fn gl_get_swap_interval(&self) -> Option<Result<i32>> {
        Some(match opengl::gl_backend() {
            opengl::GlBackend::Cgl => opengl::cgl_get_swap_interval(),
            opengl::GlBackend::Egl => crate::video::egl::get_swap_interval(),
        })
    }
    fn gl_swap_window(&self, window: WindowID) -> Option<Result<()>> {
        Some(match opengl::gl_backend() {
            opengl::GlBackend::Cgl => opengl::cgl_swap_window(window),
            opengl::GlBackend::Egl => opengles::gles_swap_window(window),
        })
    }
    fn gl_destroy_context(&self, context: RawGlContext) -> Option<Result<()>> {
        Some(match opengl::gl_backend() {
            opengl::GlBackend::Cgl => opengl::cgl_destroy_context(context),
            opengl::GlBackend::Egl => {
                opengles::gles_destroy_context(context);
                Ok(())
            }
        })
    }
    fn gl_get_egl_surface(&self, window: WindowID) -> Option<*mut std::ffi::c_void> {
        match opengl::gl_backend() {
            opengl::GlBackend::Cgl => None,
            opengl::GlBackend::Egl => Some(opengles::gles_get_egl_surface(window)),
        }
    }

    // Vulkan support

    fn implements_vulkan_surfaces(&self) -> bool {
        true
    }
    fn vulkan_load_library(&self, path: Option<&str>) -> Option<Result<()>> {
        Some(vulkan::vulkan_load_library(path))
    }
    fn vulkan_unload_library(&self) -> Option<()> {
        vulkan::vulkan_unload_library();
        Some(())
    }
    fn vulkan_get_instance_proc_addr(&self) -> Option<usize> {
        vulkan::vk_get_instance_proc_addr()
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
        Some(vulkan::vulkan_create_surface(window, instance, allocator))
    }
    fn vulkan_destroy_surface(
        &self,
        instance: usize,
        surface: u64,
        allocator: usize,
    ) -> Option<()> {
        vulkan::vulkan_destroy_surface(instance, surface, allocator);
        Some(())
    }

    // Metal support

    fn implements_metal_views(&self) -> bool {
        true
    }
    fn metal_create_view(&self, window: WindowID) -> Option<Option<usize>> {
        Some(metalview::metal_create_view(window))
    }
    fn metal_destroy_view(&self, view: usize) -> Option<()> {
        metalview::metal_destroy_view(view);
        Some(())
    }
    fn metal_get_layer(&self, view: usize) -> Option<Option<usize>> {
        Some(metalview::metal_get_layer(view))
    }

    // Event manager functions

    fn wait_event_timeout(&self, timeout: Option<Duration>) -> Option<i32> {
        Some(events::wait_event_timeout(timeout))
    }
    fn send_wakeup_event(&self, window: WindowID) -> Option<()> {
        events::send_wakeup_event(window);
        Some(())
    }
    fn pump_events(&self) -> Option<()> {
        events::pump_events();
        Some(())
    }

    fn suspend_screen_saver(&self, suspend: bool) -> Option<Result<()>> {
        Some(events::suspend_screen_saver(&self.data, suspend))
    }

    // Hit-testing, drag and drop

    fn set_window_hit_test(&self, window: WindowID, enabled: bool) -> Option<Result<()>> {
        Some(window::set_window_hit_test(window, enabled))
    }
    fn accept_drag_and_drop(&self, window: WindowID, accept: bool) -> Option<()> {
        window::accept_drag_and_drop(window, accept);
        Some(())
    }

    // The SDL_Mouse entry points

    fn warp_mouse(&self, window: WindowID, x: f32, y: f32) -> Option<Result<()>> {
        Some(mouse::warp_mouse(window, x, y))
    }
    fn warp_mouse_global(&self, x: f32, y: f32) -> Option<Result<()>> {
        Some(mouse::warp_mouse_global(x, y))
    }
    fn set_relative_mouse_mode(&self, enabled: bool) -> Option<Result<()>> {
        Some(mouse::set_relative_mouse_mode(enabled))
    }
    fn capture_mouse(&self, window: Option<WindowID>) -> Option<Result<()>> {
        Some(mouse::capture_mouse(window))
    }
    fn global_mouse_state(&self) -> Option<(f32, f32, MouseButtonFlags)> {
        Some(mouse::get_global_mouse_state())
    }
    fn create_cursor(
        &self,
        surface: &Surface<'_>,
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(mouse::create_cursor(surface, hot_x, hot_y))
    }
    fn create_animated_cursor(
        &self,
        frames: &[CursorFrame<'_>],
        hot_x: i32,
        hot_y: i32,
    ) -> Option<Result<Cursor>> {
        Some(mouse::create_animated_cursor(frames, hot_x, hot_y))
    }
    fn create_system_cursor(&self, id: SystemCursor) -> Option<Result<Cursor>> {
        Some(mouse::create_system_cursor(id))
    }
    fn show_cursor(&self, cursor: Option<&Cursor>) -> Option<Result<()>> {
        Some(mouse::show_cursor(cursor))
    }
    fn implements_mouse_feature(&self, feature: MouseFeature) -> bool {
        matches!(
            feature,
            MouseFeature::WarpMouse
                | MouseFeature::WarpMouseGlobal
                | MouseFeature::SetRelativeMouseMode
                | MouseFeature::CaptureMouse
                | MouseFeature::GetGlobalMouseState
        )
    }

    fn implements_window_op(&self, op: WindowOp) -> bool {
        matches!(
            op,
            WindowOp::SetBordered
                | WindowOp::SetResizable
                | WindowOp::SetAlwaysOnTop
                | WindowOp::Maximize
                | WindowOp::Minimize
                | WindowOp::Restore
                | WindowOp::SetParent
                | WindowOp::SetModal
                | WindowOp::SetFocusable
                | WindowOp::SetMouseRect
                | WindowOp::SetHitTest
        )
    }
    fn implements_sync_window(&self) -> bool {
        true
    }
    fn can_wait_events(&self) -> bool {
        true
    }

    // Text input

    fn start_text_input(&self, window: WindowID, props: Option<&Properties>) -> Option<Result<()>> {
        Some(keyboard::start_text_input(&self.data, window, props))
    }
    fn stop_text_input(&self, window: WindowID) -> Option<Result<()>> {
        Some(keyboard::stop_text_input(&self.data, window))
    }
    fn update_text_input_area(&self, window: WindowID) -> Option<Result<()>> {
        Some(keyboard::update_text_input_area(&self.data, window))
    }

    // Clipboard

    fn set_clipboard_data(&self) -> Option<Result<()>> {
        Some(clipboard::set_clipboard_data(&self.data))
    }
    fn clipboard_data(&self, mime_type: &str) -> Option<Option<Vec<u8>>> {
        Some(clipboard::get_clipboard_data(mime_type))
    }
    fn has_clipboard_data(&self, mime_type: &str) -> Option<bool> {
        Some(clipboard::has_clipboard_data(mime_type))
    }
}

/// `[cls alloc]`.
///
/// # Safety
///
/// `cls` must be a class whose instances are then initialized.
pub(crate) unsafe fn alloc(cls: crate::core::apple::objc::Class) -> Id {
    // SAFETY: the caller's contract.
    unsafe { msg_send![cls, alloc] }
}

/// The UTF-8 contents of an `NSString`, or "" for nil.
///
/// # Safety
///
/// `s` must be nil or a valid `NSString`.
pub(crate) unsafe fn ns_to_string(s: Id) -> String {
    // SAFETY: the caller's contract.
    unsafe { string_from_ns(s) }.unwrap_or_default()
}
