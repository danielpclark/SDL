// Rust translation of src/video/android/SDL_androidwindow.c and
// SDL_androidwindow.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The driver's window: currently only one, the activity's `SDLSurface`,
//! whose `ANativeWindow` it holds (`SDL_PROP_WINDOW_ANDROID_WINDOW_POINTER`)
//! and, for an OpenGL window, an EGL window surface on it
//! (`SDL_PROP_WINDOW_ANDROID_SURFACE_POINTER`; a Vulkan window has none,
//! which would make it incompatible with `vkCreateAndroidSurfaceKHR`).
//! The surface's natives replace them as the activity's surface comes and
//! goes (`core::android`'s `onNativeSurfaceCreated()`, ...), under the
//! activity mutex, as these functions use them; the EGL context is kept
//! while the app is paused ([`super::events`]).
//!
//! The window's data (upstream's `window->internal`) is kept with the
//! driver's one window ([`with_window_data`]), for those natives, which
//! run on the activity's thread.

use std::ffi::c_void;
use std::ptr;
use std::sync::{Mutex, MutexGuard};

use crate::core::android::{self as core_android, ndk};
use crate::error::{Error, Result};
use crate::events::window::WindowFlags;
use crate::events::{DisplayID, EventType, WindowID};
use crate::properties::Properties;
use crate::video::core::with_window;
use crate::video::egl::{self, EGLSurface, EGL_NO_SURFACE};
use crate::video::gl::RawGlContext;
use crate::video::sysvideo::{FullscreenOp, FullscreenResult};
use crate::video::window::{
    Window, PROP_WINDOW_ANDROID_SURFACE_POINTER, PROP_WINDOW_ANDROID_WINDOW_POINTER,
};

/// The window's data. Translation of `SDL_WindowData`.
pub(crate) struct WindowData {
    pub(crate) egl_surface: EGLSurface,
    /// We use this to preserve the context when losing focus
    pub(crate) egl_context: Option<RawGlContext>,
    /// Save/Restore the swap interval / vsync
    pub(crate) has_swap_interval: bool,
    pub(crate) swap_interval: i32,
    pub(crate) backup_done: bool,
    pub(crate) native_window: *mut ndk::ANativeWindow,
}

/// `Android_Window` (currently only one window) and its data.
struct AndroidWindow {
    id: WindowID,
    data: WindowData,
}

// SAFETY: the native window and the EGL surface are used under the
// activity mutex, from any thread, as upstream uses them.
unsafe impl Send for AndroidWindow {}

static ANDROID_WINDOW: Mutex<Option<AndroidWindow>> = Mutex::new(None);

fn android_window_state() -> MutexGuard<'static, Option<AndroidWindow>> {
    ANDROID_WINDOW.lock().unwrap_or_else(|e| e.into_inner())
}

/// `Android_Window`.
pub(crate) fn android_window() -> Option<WindowID> {
    android_window_state().as_ref().map(|w| w.id)
}

/// Run `f` on `Android_Window` and its data (`Android_Window->internal`),
/// when there's such a window.
pub(crate) fn with_window_data<R>(f: impl FnOnce(WindowID, &mut WindowData) -> R) -> Option<R> {
    android_window_state()
        .as_mut()
        .map(|w| f(w.id, &mut w.data))
}

/// The window's EGL surface (`window->internal->egl_surface`;
/// `EGL_NO_SURFACE` for another window).
pub(crate) fn egl_surface(window: WindowID) -> EGLSurface {
    with_window_data(|id, data| {
        if id == window {
            data.egl_surface
        } else {
            EGL_NO_SURFACE
        }
    })
    .unwrap_or(EGL_NO_SURFACE)
}

/// The window's `ANativeWindow` (null for another window, or while the
/// activity has no surface).
pub(crate) fn native_window(window: WindowID) -> *mut ndk::ANativeWindow {
    with_window_data(|id, data| {
        if id == window {
            data.native_window
        } else {
            ptr::null_mut()
        }
    })
    .unwrap_or(ptr::null_mut())
}

/// The window's properties.
fn properties(window: WindowID) -> Option<Properties> {
    Window::from_raw(window).properties().ok()
}

/// `SDL_SetPointerProperty(props, name, pointer)` (a NULL pointer clears
/// it).
pub(crate) fn set_pointer_property(window: WindowID, name: &str, pointer: *mut c_void) {
    if let Some(props) = properties(window) {
        if pointer.is_null() {
            props.remove(name);
        } else {
            let _ = props.set(name, pointer as usize as i64);
        }
    }
}

/// `Android_JNI_SetOrientation()` with the window's size, its resizable
/// flag and `SDL_HINT_ORIENTATIONS`.
fn set_orientation_of(window: WindowID) {
    let Ok((w, h, resizable)) = with_window(window, |w| {
        (
            w.core.w,
            w.core.h,
            w.flags().contains(WindowFlags::RESIZABLE),
        )
    }) else {
        return;
    };
    core_android::set_orientation(
        w,
        h,
        resizable,
        crate::hints::get(crate::hints::ORIENTATIONS).as_deref(),
    );
}

/// Translation of `Android_CreateWindow()`.
pub(crate) fn create_window(window: WindowID, _create_props: &Properties) -> Result<()> {
    super::events::wait_active_and_lock_activity()?;

    let result = create_window_locked(window);

    core_android::unlock_activity_mutex();

    result
}

fn create_window_locked(window: WindowID) -> Result<()> {
    if android_window().is_some() {
        return Err(Error::new("Android only supports one window"));
    }

    // Set orientation
    set_orientation_of(window);

    // Adjust the window data to match the screen
    let screen = super::screen();
    let opengl = with_window(window, |w| {
        w.core.x = 0;
        w.core.y = 0;
        w.core.w = screen.surface_width;
        w.core.h = screen.surface_height;
        w.flags().contains(WindowFlags::OPENGL)
    })?;

    // One window, it always has focus
    crate::events::mouse::set_mouse_focus(Some(window));
    let _ = crate::events::keyboard::set_keyboard_focus(Some(window));

    let native_window = core_android::get_native_window();
    if native_window.is_null() {
        return Err(Error::new("Could not fetch native window"));
    }
    set_pointer_property(
        window,
        PROP_WINDOW_ANDROID_WINDOW_POINTER,
        native_window.cast(),
    );

    /* Do not create EGLSurface for Vulkan window since it will then make the window
    incompatible with vkCreateAndroidSurfaceKHR */
    let mut egl_surface = EGL_NO_SURFACE;
    if opengl {
        match egl::create_surface(Some(window), native_window.cast()) {
            Ok(surface) => egl_surface = surface,
            Err(e) => {
                // (Note (upstream): the window's
                // SDL_PROP_WINDOW_ANDROID_WINDOW_POINTER is left set to
                // the window released here)
                // SAFETY: the reference ANativeWindow_fromSurface() gave.
                unsafe { ndk::ANativeWindow_release(native_window) };
                return Err(e);
            }
        }
    }
    set_pointer_property(window, PROP_WINDOW_ANDROID_SURFACE_POINTER, egl_surface);

    crate::video::core::set_window_safe_area_insets(
        window,
        screen.safe_inset_left,
        screen.safe_inset_right,
        screen.safe_inset_top,
        screen.safe_inset_bottom,
    );

    *android_window_state() = Some(AndroidWindow {
        id: window,
        data: WindowData {
            egl_surface,
            egl_context: None,
            has_swap_interval: false,
            swap_interval: 0,
            backup_done: false,
            native_window,
        },
    });

    Ok(())
}

/// Translation of `Android_SetWindowTitle()`.
pub(crate) fn set_window_title(window: WindowID) {
    let title = with_window(window, |w| w.title.clone())
        .ok()
        .flatten()
        .unwrap_or_default();
    core_android::set_activity_title(&title);
}

/// Translation of `Android_SetWindowFullscreen()`.
pub(crate) fn set_window_fullscreen(
    window: WindowID,
    _display: DisplayID,
    fullscreen: FullscreenOp,
) -> FullscreenResult {
    core_android::lock_activity_mutex();

    if Some(window) == android_window() {
        // If the window is being destroyed don't change visible state
        let destroying = with_window(window, |w| w.core.is_destroying).unwrap_or(false);
        if !destroying {
            core_android::set_window_style(fullscreen != FullscreenOp::Leave);
        }

        /* Ensure our size matches reality after we've executed the window style change.
         *
         * It is possible that we've set width and height to the full-size display, but on
         * Samsung DeX or Chromebooks or other windowed Android environments, our window may
         * still not be the full display size.
         */
        if core_android::is_dex_mode() || core_android::is_chromebook() {
            update_size_from_native_window(window);
        }
    }

    core_android::unlock_activity_mutex();

    FullscreenResult::Succeeded
}

/// The window resized to its `ANativeWindow`'s size, when that differs (on
/// windowed Android environments). Part of `Android_SetWindowFullscreen()`.
fn update_size_from_native_window(window: WindowID) {
    let native_window = with_window_data(|_, data| data.native_window);
    let Some(native_window) = native_window else {
        return;
    };
    if native_window.is_null() {
        // (Note (upstream): the error SDL_SetError("Missing native
        // window") sets isn't returned: the fullscreen change succeeds)
        return;
    }

    let Ok((old_w, old_h)) = with_window(window, |w| (w.core.w, w.core.h)) else {
        return;
    };

    // SAFETY: the window's native window, under the activity mutex.
    let (new_w, new_h) = unsafe {
        (
            ndk::ANativeWindow_getWidth(native_window),
            ndk::ANativeWindow_getHeight(native_window),
        )
    };

    // (Note (upstream): its error, "ANativeWindow_getWidth/Height() fails",
    // isn't returned either, and the size is sent all the same)

    if old_w != new_w || old_h != new_h {
        crate::events::window::send_window_event(window, EventType::WINDOW_RESIZED, new_w, new_h);
    }
}

/// Translation of `Android_MinimizeWindow()`.
pub(crate) fn minimize_window(_window: WindowID) {
    core_android::minimize_window();
}

/// Translation of `Android_SetWindowResizable()`.
pub(crate) fn set_window_resizable(window: WindowID, _resizable: bool) {
    // Set orientation
    set_orientation_of(window);
}

/// Translation of `Android_DestroyWindow()`.
pub(crate) fn destroy_window(window: WindowID) {
    core_android::lock_activity_mutex();

    let mut state = android_window_state();
    if state.as_ref().is_some_and(|w| w.id == window) {
        let data = state.take().expect("the window").data;
        drop(state);

        if data.egl_surface != EGL_NO_SURFACE {
            egl::destroy_surface(data.egl_surface);
        }

        if !data.native_window.is_null() {
            // SAFETY: the window's reference.
            unsafe { ndk::ANativeWindow_release(data.native_window) };
        }
    } else {
        drop(state);
    }

    core_android::unlock_activity_mutex();
}
