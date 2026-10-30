// Rust translation of src/video/android/ from Simple DirectMedia Layer
// (part: SDL_androidevents.c, and SDL_androidvideo.c's dark mode).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The android video driver, so far what SDL's event pump needs on
//! Android, which it uses whatever the video driver ([`events`], the
//! activity's lifecycle), and what the activity tells the driver about the
//! system's theme ([`set_dark_mode`]). The driver itself (its device, its
//! display, `Android_Window`, the keyboard, the EGL contexts' backup)
//! comes with `SDLSurface`: until then there's no window of the driver's
//! ([`window`]), and what upstream does with it isn't reached.

pub(crate) mod events;

use std::sync::atomic::{AtomicU8, Ordering};

use crate::events::WindowID;
use crate::video::SystemTheme;

/// `Android_Window`, the driver's window: none until the driver is
/// translated.
pub(crate) fn window() -> Option<WindowID> {
    None
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

/// A key pressed. Translation of `Android_OnKeyDown()`, which upstream's
/// `onNativeKeyDown()` calls when there's a window ([`window`]): the
/// keyboard comes with the driver's windows.
pub(crate) fn on_key_down(_keycode: i32) {}

/// A key released. Translation of `Android_OnKeyUp()` (see
/// [`on_key_down`]).
pub(crate) fn on_key_up(_keycode: i32) {}
