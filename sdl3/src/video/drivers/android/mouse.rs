// Rust translation of src/video/android/SDL_androidmouse.c and
// SDL_androidmouse.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The mouse: its cursors are the surface's `PointerIcon`s (a system
//! cursor's `PointerIcon.getSystemIcon()`, a custom cursor's bitmap, a
//! hidden cursor an empty custom one: `SDLActivity`'s
//! `setSystemCursor()`, `createCustomCursor()`, `setCustomCursor()`), its
//! relative mode is the motion listener's (the pointer captured), and its
//! events are the surface's (`onNativeMouse()`, with the motion event's
//! action and its buttons' state).

use std::sync::Mutex;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::events::mouse::{self, Cursor, SystemCursor, DEFAULT_MOUSE_ID};
use crate::events::WindowID;
use crate::video::{PixelFormat, Surface};

// See Android's MotionEvent class for constants
const ACTION_DOWN: i32 = 0;
const ACTION_UP: i32 = 1;
const ACTION_MOVE: i32 = 2;
const ACTION_HOVER_MOVE: i32 = 7;
const ACTION_SCROLL: i32 = 8;
const BUTTON_PRIMARY: i32 = 1;
const BUTTON_SECONDARY: i32 = 2;
const BUTTON_TERTIARY: i32 = 4;
const BUTTON_BACK: i32 = 8;
const BUTTON_FORWARD: i32 = 16;

/// A cursor's data: its custom cursor's ID (the activity's), or its
/// system cursor. Translation of `struct SDL_CursorData`;
/// `Android_FreeCursor()` is its drop.
#[derive(Debug)]
pub(crate) struct CursorData {
    custom_cursor: i32,
    system_cursor: i32,
}

impl Drop for CursorData {
    /// Translation of `Android_FreeCursor()`.
    fn drop(&mut self) {
        if self.custom_cursor != 0 {
            crate::core::android::destroy_custom_cursor(self.custom_cursor);
        }
    }
}

/// Last known Android mouse button state (includes all buttons).
/// Translation of `last_state`.
static LAST_STATE: Mutex<i32> = Mutex::new(0);

/// Blank cursor. Translation of `empty_cursor`.
static EMPTY_CURSOR: Mutex<Option<Cursor>> = Mutex::new(None);

fn last_state() -> std::sync::MutexGuard<'static, i32> {
    LAST_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn empty_cursor() -> std::sync::MutexGuard<'static, Option<Cursor>> {
    EMPTY_CURSOR.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `Android_WrapCursor()`.
fn wrap_cursor(custom_cursor: i32, system_cursor: i32) -> Cursor {
    Cursor::with_internal(CursorData {
        custom_cursor,
        system_cursor,
    })
}

/// Translation of `Android_CreateDefaultCursor()`.
fn create_default_cursor() -> Cursor {
    let id = mouse::default_system_cursor();
    wrap_cursor(0, id as i32)
}

/// A custom cursor of the surface (converted to ARGB8888). Translation of
/// `Android_CreateCursor()`.
pub(crate) fn create_cursor(surface: &Surface<'_>, hot_x: i32, hot_y: i32) -> Result<Cursor> {
    let converted = surface.convert(PixelFormat::ARGB8888)?;
    let (w, h) = (converted.width(), converted.height());
    // (the pixels, `w * h` of them: the surface's rows, packed)
    let pixels: Vec<i32> = match converted.pixels() {
        Some(bytes) => bytes
            .chunks(converted.pitch() as usize)
            .take(h as usize)
            .flat_map(|row| {
                row[..w as usize * 4]
                    .chunks_exact(4)
                    .map(|p| i32::from_ne_bytes([p[0], p[1], p[2], p[3]]))
            })
            .collect(),
        None => Vec::new(),
    };
    let custom_cursor = crate::core::android::create_custom_cursor(&pixels, w, h, hot_x, hot_y);
    drop(converted);
    if custom_cursor == 0 {
        return Err(Error::unsupported());
    }
    Ok(wrap_cursor(custom_cursor, 0))
}

/// Translation of `Android_CreateSystemCursor()`.
pub(crate) fn create_system_cursor(id: SystemCursor) -> Cursor {
    wrap_cursor(0, id as i32)
}

/// The empty (1x1, transparent) custom cursor, made once. Translation of
/// `Android_CreateEmptyCursor()`.
fn create_empty_cursor() -> Result<Cursor> {
    let mut empty = empty_cursor();
    if empty.is_none() {
        let empty_surface = Surface::new(1, 1, PixelFormat::ARGB8888)?;
        *empty = Some(create_cursor(&empty_surface, 0, 0)?);
    }
    Ok(empty.clone().expect("the empty cursor"))
}

/// Translation of `Android_DestroyEmptyCursor()`.
fn destroy_empty_cursor() {
    // (Android_FreeCursor(): the cursor's drop, once nothing has it)
    empty_cursor().take();
}

/// The cursor as the surface's pointer icon (the empty cursor for `None`).
/// Translation of `Android_ShowCursor()`.
pub(crate) fn show_cursor(cursor: Option<&Cursor>) -> Result<()> {
    let cursor = match cursor {
        Some(cursor) => cursor.clone(),
        // (an SDL error from create_empty_cursor())
        None => create_empty_cursor()?,
    };
    let Some(data) = cursor.internal::<CursorData>() else {
        return Err(Error::unsupported());
    };
    if data.custom_cursor != 0 {
        if !crate::core::android::set_custom_cursor(data.custom_cursor) {
            return Err(Error::unsupported());
        }
    } else if !crate::core::android::set_system_cursor(data.system_cursor) {
        return Err(Error::unsupported());
    }
    Ok(())
}

/// Translation of `Android_SetRelativeMouseMode()`.
pub(crate) fn set_relative_mouse_mode(enabled: bool) -> Result<()> {
    if !crate::core::android::supports_relative_mouse() {
        return Err(Error::unsupported());
    }

    if !crate::core::android::set_relative_mouse_enabled(enabled) {
        return Err(Error::unsupported());
    }

    Ok(())
}

/// Translation of `Android_InitMouse()` (the entry points are the
/// driver's).
pub(crate) fn init_mouse() {
    mouse::set_default_cursor(Some(create_default_cursor()));

    *last_state() = 0;
}

/// Translation of `Android_QuitMouse()`.
pub(crate) fn quit_mouse() {
    destroy_empty_cursor();
}

/// Translate Android mouse button state to SDL mouse button. Translation
/// of `TranslateButton()`.
fn translate_button(state: i32) -> u8 {
    if state & BUTTON_PRIMARY != 0 {
        mouse::BUTTON_LEFT
    } else if state & BUTTON_SECONDARY != 0 {
        mouse::BUTTON_RIGHT
    } else if state & BUTTON_TERTIARY != 0 {
        mouse::BUTTON_MIDDLE
    } else if state & BUTTON_FORWARD != 0 {
        mouse::BUTTON_X1
    } else if state & BUTTON_BACK != 0 {
        mouse::BUTTON_X2
    } else {
        0
    }
}

/// A mouse event of the surface's: the motion event's buttons' state, its
/// action, its position (or motion, `relative`; the wheel's axes for a
/// scroll). Translation of `Android_OnMouse()`. Note (upstream): a down or
/// up whose buttons' state doesn't change (a mouse's event without
/// buttons, as adb's `input mouse tap` makes) sends button 0's down or
/// up.
pub(crate) fn on_mouse(
    window: Option<WindowID>,
    state: i32,
    action: i32,
    x: f32,
    y: f32,
    relative: bool,
) {
    let Some(window) = window else {
        return;
    };

    match action {
        ACTION_DOWN => {
            let button = {
                let mut last_state = last_state();
                let changes = state & !*last_state;
                *last_state = state;
                translate_button(changes)
            };
            mouse::send_mouse_motion(
                Duration::ZERO,
                Some(window),
                DEFAULT_MOUSE_ID,
                relative,
                x,
                y,
            );
            mouse::send_mouse_button(Duration::ZERO, Some(window), DEFAULT_MOUSE_ID, button, true);
        }

        ACTION_UP => {
            let button = {
                let mut last_state = last_state();
                let changes = *last_state & !state;
                *last_state = state;
                translate_button(changes)
            };
            mouse::send_mouse_motion(
                Duration::ZERO,
                Some(window),
                DEFAULT_MOUSE_ID,
                relative,
                x,
                y,
            );
            mouse::send_mouse_button(
                Duration::ZERO,
                Some(window),
                DEFAULT_MOUSE_ID,
                button,
                false,
            );
        }

        ACTION_MOVE | ACTION_HOVER_MOVE => {
            mouse::send_mouse_motion(
                Duration::ZERO,
                Some(window),
                DEFAULT_MOUSE_ID,
                relative,
                x,
                y,
            );
        }

        ACTION_SCROLL => {
            mouse::send_mouse_wheel(
                Duration::ZERO,
                Some(window),
                DEFAULT_MOUSE_ID,
                x,
                y,
                mouse::MouseWheelDirection::Normal,
            );
        }

        _ => {}
    }
}
