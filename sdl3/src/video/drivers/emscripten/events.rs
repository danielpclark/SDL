// Rust translation of src/video/emscripten/SDL_emscriptenevents.c and
// SDL_emscriptenevents.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The DOM events: html5.h's callbacks for the keyboard (on the window's
//! keyboard element: `#window` by default), the canvas's `wheel`, the
//! window's `focus`, `blur` and `resize`, the document's
//! `fullscreenchange`, `pointerlockchange` and `visibilitychange`,
//! `beforeunload` and the screen's orientation `change`, with the canvas
//! resize callback of a fullscreen request ([`handle_canvas_resize`]); and
//! listeners of upstream's own JavaScript (`sdl3_emscripten::video::events`)
//! for what html5.h has no callback for: the canvas's Pointer Events
//! (mouse, touch and pen), the document's `pointerup` (the "global
//! mouseup", for a button let go outside the canvas), the canvas's drag
//! and drop, and the document's `keydown` lock-key check. Those call the
//! Rust functions upstream marks `EMSCRIPTEN_KEEPALIVE`, through the
//! callback registry ([`register_callbacks`]).
//!
//! Upstream passes its handlers the `SDL_VideoDevice *` (the global ones,
//! which walk its windows) or the `SDL_WindowData *` as their user data
//! (and its JavaScript hands them back to C); here the global ones walk the
//! video core's windows, and the others get the window's ID.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;
use std::sync::Arc;
use std::time::Duration;

use sdl3_emscripten::sys::{
    self, EmscriptenFocusEvent, EmscriptenFullscreenChangeEvent, EmscriptenKeyboardEvent,
    EmscriptenOrientationChangeEvent, EmscriptenPointerlockChangeEvent, EmscriptenUiEvent,
    EmscriptenVisibilityChangeEvent, EmscriptenWheelEvent, DOM_PK_CODE_TYPE,
    EMSCRIPTEN_EVENT_TARGET_DOCUMENT, EMSCRIPTEN_EVENT_TARGET_SCREEN,
    EMSCRIPTEN_EVENT_TARGET_WINDOW, EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
    EM_CALLBACK_THREAD_CONTEXT_MAIN_RUNTIME_THREAD,
};
use sdl3_emscripten::video::events as js;

use super::{fill_document_window, with_data, EmscriptenWindow};
use crate::events::keyboard::{self, Keycode, Keymod, Scancode, DEFAULT_KEYBOARD_ID};
use crate::events::mouse::{
    self, MouseButtonFlags, MouseWheelDirection, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT,
    BUTTON_X1, BUTTON_X2, DEFAULT_MOUSE_ID, GLOBAL_MOUSE_ID,
};
use crate::events::pen::{self, PenAxis, PenCapabilityFlags, PenID, PenInfo, PenSubtype};
use crate::events::queue::{event_enabled, send_app_event};
use crate::events::touch::{self, FingerID, TouchDeviceType, TouchID};
use crate::events::window::{
    send_display_event, send_drop_complete, send_drop_file, send_drop_position, send_drop_text,
    send_window_event, WindowFlags,
};
use crate::events::{EventType, WindowID};
use crate::stdlib::math::lroundf;
use crate::stdlib::string::{step_utf8, utf8strlen, INVALID_UNICODE_CODEPOINT};
use crate::video::core::{update_fullscreen_mode, window_ids, with_window};
use crate::video::sysvideo::{DisplayOrientation, FullscreenOp};

/*
Emscripten PK code to scancode
https://developer.mozilla.org/en-US/docs/Web/API/KeyboardEvent
https://developer.mozilla.org/en-US/docs/Web/API/KeyboardEvent/code
*/
/// Translation of `emscripten_scancode_table`.
const EMSCRIPTEN_SCANCODE_TABLE: [Scancode; 0x7F] = [
    /* 0x00 "Unidentified"   */ Scancode::UNKNOWN,
    /* 0x01 "Escape"         */ Scancode::ESCAPE,
    /* 0x02 "Digit0"         */ Scancode::N0,
    /* 0x03 "Digit1"         */ Scancode::N1,
    /* 0x04 "Digit2"         */ Scancode::N2,
    /* 0x05 "Digit3"         */ Scancode::N3,
    /* 0x06 "Digit4"         */ Scancode::N4,
    /* 0x07 "Digit5"         */ Scancode::N5,
    /* 0x08 "Digit6"         */ Scancode::N6,
    /* 0x09 "Digit7"         */ Scancode::N7,
    /* 0x0A "Digit8"         */ Scancode::N8,
    /* 0x0B "Digit9"         */ Scancode::N9,
    /* 0x0C "Minus"          */ Scancode::MINUS,
    /* 0x0D "Equal"          */ Scancode::EQUALS,
    /* 0x0E "Backspace"      */ Scancode::BACKSPACE,
    /* 0x0F "Tab"            */ Scancode::TAB,
    /* 0x10 "KeyQ"           */ Scancode::Q,
    /* 0x11 "KeyW"           */ Scancode::W,
    /* 0x12 "KeyE"           */ Scancode::E,
    /* 0x13 "KeyR"           */ Scancode::R,
    /* 0x14 "KeyT"           */ Scancode::T,
    /* 0x15 "KeyY"           */ Scancode::Y,
    /* 0x16 "KeyU"           */ Scancode::U,
    /* 0x17 "KeyI"           */ Scancode::I,
    /* 0x18 "KeyO"           */ Scancode::O,
    /* 0x19 "KeyP"           */ Scancode::P,
    /* 0x1A "BracketLeft"    */ Scancode::LEFTBRACKET,
    /* 0x1B "BracketRight"   */ Scancode::RIGHTBRACKET,
    /* 0x1C "Enter"          */ Scancode::RETURN,
    /* 0x1D "ControlLeft"    */ Scancode::LCTRL,
    /* 0x1E "KeyA"           */ Scancode::A,
    /* 0x1F "KeyS"           */ Scancode::S,
    /* 0x20 "KeyD"           */ Scancode::D,
    /* 0x21 "KeyF"           */ Scancode::F,
    /* 0x22 "KeyG"           */ Scancode::G,
    /* 0x23 "KeyH"           */ Scancode::H,
    /* 0x24 "KeyJ"           */ Scancode::J,
    /* 0x25 "KeyK"           */ Scancode::K,
    /* 0x26 "KeyL"           */ Scancode::L,
    /* 0x27 "Semicolon"      */ Scancode::SEMICOLON,
    /* 0x28 "Quote"          */ Scancode::APOSTROPHE,
    /* 0x29 "Backquote"      */ Scancode::GRAVE,
    /* 0x2A "ShiftLeft"      */ Scancode::LSHIFT,
    /* 0x2B "Backslash"      */ Scancode::BACKSLASH,
    /* 0x2C "KeyZ"           */ Scancode::Z,
    /* 0x2D "KeyX"           */ Scancode::X,
    /* 0x2E "KeyC"           */ Scancode::C,
    /* 0x2F "KeyV"           */ Scancode::V,
    /* 0x30 "KeyB"           */ Scancode::B,
    /* 0x31 "KeyN"           */ Scancode::N,
    /* 0x32 "KeyM"           */ Scancode::M,
    /* 0x33 "Comma"          */ Scancode::COMMA,
    /* 0x34 "Period"         */ Scancode::PERIOD,
    /* 0x35 "Slash"          */ Scancode::SLASH,
    /* 0x36 "ShiftRight"     */ Scancode::RSHIFT,
    /* 0x37 "NumpadMultiply" */ Scancode::KP_MULTIPLY,
    /* 0x38 "AltLeft"        */ Scancode::LALT,
    /* 0x39 "Space"          */ Scancode::SPACE,
    /* 0x3A "CapsLock"       */ Scancode::CAPSLOCK,
    /* 0x3B "F1"             */ Scancode::F1,
    /* 0x3C "F2"             */ Scancode::F2,
    /* 0x3D "F3"             */ Scancode::F3,
    /* 0x3E "F4"             */ Scancode::F4,
    /* 0x3F "F5"             */ Scancode::F5,
    /* 0x40 "F6"             */ Scancode::F6,
    /* 0x41 "F7"             */ Scancode::F7,
    /* 0x42 "F8"             */ Scancode::F8,
    /* 0x43 "F9"             */ Scancode::F9,
    /* 0x44 "F10"            */ Scancode::F10,
    /* 0x45 "Pause"          */ Scancode::PAUSE,
    /* 0x46 "ScrollLock"     */ Scancode::SCROLLLOCK,
    /* 0x47 "Numpad7"        */ Scancode::KP_7,
    /* 0x48 "Numpad8"        */ Scancode::KP_8,
    /* 0x49 "Numpad9"        */ Scancode::KP_9,
    /* 0x4A "NumpadSubtract" */ Scancode::KP_MINUS,
    /* 0x4B "Numpad4"        */ Scancode::KP_4,
    /* 0x4C "Numpad5"        */ Scancode::KP_5,
    /* 0x4D "Numpad6"        */ Scancode::KP_6,
    /* 0x4E "NumpadAdd"      */ Scancode::KP_PLUS,
    /* 0x4F "Numpad1"        */ Scancode::KP_1,
    /* 0x50 "Numpad2"        */ Scancode::KP_2,
    /* 0x51 "Numpad3"        */ Scancode::KP_3,
    /* 0x52 "Numpad0"        */ Scancode::KP_0,
    /* 0x53 "NumpadDecimal"  */ Scancode::KP_PERIOD,
    /* 0x54 "PrintScreen"    */ Scancode::PRINTSCREEN,
    /* 0x55                  */ Scancode::UNKNOWN,
    /* 0x56 "IntlBackslash"  */ Scancode::NONUSBACKSLASH,
    /* 0x57 "F11"            */ Scancode::F11,
    /* 0x58 "F12"            */ Scancode::F12,
    /* 0x59 "NumpadEqual"    */ Scancode::KP_EQUALS,
    /* 0x5A                  */ Scancode::UNKNOWN,
    /* 0x5B                  */ Scancode::UNKNOWN,
    /* 0x5C                  */ Scancode::UNKNOWN,
    /* 0x5D                  */ Scancode::UNKNOWN,
    /* 0x5E                  */ Scancode::UNKNOWN,
    /* 0x5F                  */ Scancode::UNKNOWN,
    /* 0x60                  */ Scancode::UNKNOWN,
    /* 0x61                  */ Scancode::UNKNOWN,
    /* 0x62                  */ Scancode::UNKNOWN,
    /* 0x63                  */ Scancode::UNKNOWN,
    /* 0x64 "F13"            */ Scancode::F13,
    /* 0x65 "F14"            */ Scancode::F14,
    /* 0x66 "F15"            */ Scancode::F15,
    /* 0x67 "F16"            */ Scancode::F16,
    /* 0x68 "F17"            */ Scancode::F17,
    /* 0x69 "F18"            */ Scancode::F18,
    /* 0x6A "F19"            */ Scancode::F19,
    /* 0x6B "F20"            */ Scancode::F20,
    /* 0x6C "F21"            */ Scancode::F21,
    /* 0x6D "F22"            */ Scancode::F22,
    /* 0x6E "F23"            */ Scancode::F23,
    /* 0x6F                  */ Scancode::UNKNOWN,
    /* 0x70 "KanaMode"       */ Scancode::INTERNATIONAL2,
    /* 0x71 "Lang2"          */ Scancode::LANG2,
    /* 0x72 "Lang1"          */ Scancode::LANG1,
    /* 0x73 "IntlRo"         */ Scancode::INTERNATIONAL1,
    /* 0x74                  */ Scancode::UNKNOWN,
    /* 0x75                  */ Scancode::UNKNOWN,
    /* 0x76 "F24"            */ Scancode::F24,
    /* 0x77                  */ Scancode::UNKNOWN,
    /* 0x78                  */ Scancode::UNKNOWN,
    /* 0x79 "Convert"        */ Scancode::INTERNATIONAL4,
    /* 0x7A                  */ Scancode::UNKNOWN,
    /* 0x7B "NonConvert"     */ Scancode::INTERNATIONAL5,
    /* 0x7C                  */ Scancode::UNKNOWN,
    /* 0x7D "IntlYen"        */ Scancode::INTERNATIONAL3,
    /* 0x7E "NumpadComma"    */ Scancode::KP_COMMA,
];

/// The scancode of a DOM `KeyboardEvent.code` (by emscripten's hash of it,
/// `emscripten_compute_dom_pk_code()`). Translation of
/// `Emscripten_MapScanCode()`.
pub(super) fn map_scan_code(code: &CStr) -> Scancode {
    // SAFETY: a NUL-terminated string (emscripten's libhtml5 C hashes it).
    let pk_code: DOM_PK_CODE_TYPE = unsafe { sys::emscripten_compute_dom_pk_code(code.as_ptr()) };
    // (C compares the int with a size_t: unsigned)
    if let Some(&scancode) = EMSCRIPTEN_SCANCODE_TABLE.get(pk_code as u32 as usize) {
        return scancode;
    }

    match pk_code {
        sys::DOM_PK_PASTE => Scancode::PASTE,
        sys::DOM_PK_MEDIA_TRACK_PREVIOUS => Scancode::MEDIA_PREVIOUS_TRACK,
        sys::DOM_PK_CUT => Scancode::CUT,
        sys::DOM_PK_COPY => Scancode::COPY,
        sys::DOM_PK_MEDIA_TRACK_NEXT => Scancode::MEDIA_NEXT_TRACK,
        sys::DOM_PK_NUMPAD_ENTER => Scancode::KP_ENTER,
        sys::DOM_PK_CONTROL_RIGHT => Scancode::RCTRL,
        sys::DOM_PK_AUDIO_VOLUME_MUTE => Scancode::MUTE,
        sys::DOM_PK_MEDIA_PLAY_PAUSE => Scancode::MEDIA_PLAY_PAUSE,
        sys::DOM_PK_MEDIA_STOP => Scancode::MEDIA_STOP,
        sys::DOM_PK_EJECT => Scancode::MEDIA_EJECT,
        sys::DOM_PK_AUDIO_VOLUME_DOWN => Scancode::VOLUMEDOWN,
        sys::DOM_PK_AUDIO_VOLUME_UP => Scancode::VOLUMEUP,
        sys::DOM_PK_BROWSER_HOME => Scancode::AC_HOME,
        sys::DOM_PK_NUMPAD_DIVIDE => Scancode::KP_DIVIDE,
        sys::DOM_PK_ALT_RIGHT => Scancode::RALT,
        sys::DOM_PK_HELP => Scancode::HELP,
        sys::DOM_PK_NUM_LOCK => Scancode::NUMLOCKCLEAR,
        sys::DOM_PK_HOME => Scancode::HOME,
        sys::DOM_PK_ARROW_UP => Scancode::UP,
        sys::DOM_PK_PAGE_UP => Scancode::PAGEUP,
        sys::DOM_PK_ARROW_LEFT => Scancode::LEFT,
        sys::DOM_PK_ARROW_RIGHT => Scancode::RIGHT,
        sys::DOM_PK_END => Scancode::END,
        sys::DOM_PK_ARROW_DOWN => Scancode::DOWN,
        sys::DOM_PK_PAGE_DOWN => Scancode::PAGEDOWN,
        sys::DOM_PK_INSERT => Scancode::INSERT,
        sys::DOM_PK_DELETE => Scancode::DELETE,
        sys::DOM_PK_META_LEFT => Scancode::LGUI,
        sys::DOM_PK_META_RIGHT => Scancode::RGUI,
        sys::DOM_PK_CONTEXT_MENU => Scancode::APPLICATION,
        sys::DOM_PK_POWER => Scancode::POWER,
        sys::DOM_PK_BROWSER_SEARCH => Scancode::AC_SEARCH,
        sys::DOM_PK_BROWSER_FAVORITES => Scancode::AC_BOOKMARKS,
        sys::DOM_PK_BROWSER_REFRESH => Scancode::AC_REFRESH,
        sys::DOM_PK_BROWSER_STOP => Scancode::AC_STOP,
        sys::DOM_PK_BROWSER_FORWARD => Scancode::AC_FORWARD,
        sys::DOM_PK_BROWSER_BACK => Scancode::AC_BACK,
        sys::DOM_PK_MEDIA_SELECT => Scancode::MEDIA_SELECT,
        _ => Scancode::UNKNOWN,
    }
}

/// An html5.h event structure's string (`EM_UTF8[]`, NUL-terminated by
/// emscripten), as bytes, without the NUL.
fn em_string(s: &[c_char]) -> &[u8] {
    // SAFETY: c_char and u8 have the same size and alignment.
    let bytes = unsafe { std::slice::from_raw_parts(s.as_ptr().cast::<u8>(), s.len()) };
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..len]
}

/// The window whose canvas has the focus, or the first window if the page
/// has it. Translation of `Emscripten_GetFocusedWindow()`.
fn get_focused_window() -> Option<WindowID> {
    let windows = window_ids();
    let mut found = None;
    for &window in &windows {
        let Some(canvas_id) = with_data(window, |d| d.canvas_id.clone()) else {
            continue;
        };

        // SAFETY: a NUL-terminated selector.
        let focused = unsafe { js::sdl3_canvas_is_focused(canvas_id.as_ptr()) };

        if focused != 0 {
            found = Some(window);
            break;
        }
    }
    // If the DOM is focused, then at least one canvas in the DOM should be considered focused.
    // So in this case, just assume that the first canvas is focused.
    if found.is_none() {
        // SAFETY: plain JavaScript.
        let focused = unsafe { js::sdl3_document_has_focus() };
        if focused != 0 {
            found = windows.first().copied();
        }
    }
    found
}

/// Translation of `Emscripten_HandlePointerLockChange()`.
fn handle_pointer_lock_change(
    window: WindowID,
    change_event: &EmscriptenPointerlockChangeEvent,
) -> bool {
    // keep track of lock losses, so we can regrab if/when appropriate.
    with_data(window, |d| d.has_pointer_lock = change_event.isActive);
    false
}

/// Translation of `Emscripten_HandlePointerLockChangeGlobal()`.
unsafe extern "C-unwind" fn handle_pointer_lock_change_global(
    _event_type: c_int,
    change_event: *const EmscriptenPointerlockChangeEvent,
    _user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten's event, for the call.
    let change_event = unsafe { &*change_event };
    let mut prevent_default = false;

    for window in window_ids() {
        prevent_default |= handle_pointer_lock_change(window, change_event);
    }

    prevent_default
}

/// The canvas's `wheel`. Translation of `Emscripten_HandleWheel()`.
unsafe extern "C-unwind" fn handle_wheel(
    _event_type: c_int,
    wheel_event: *const EmscriptenWheelEvent,
    user_data: *mut c_void,
) -> bool {
    let window = user_data as usize as WindowID;
    // SAFETY: emscripten's event, for the call.
    let wheel_event = unsafe { &*wheel_event };

    let (delta_x, delta_y) = wheel_steps(wheel_event);

    mouse::send_mouse_wheel(
        Duration::ZERO,
        Some(window),
        DEFAULT_MOUSE_ID,
        delta_x,
        -delta_y,
        MouseWheelDirection::Normal,
    );
    event_enabled(EventType::MOUSE_WHEEL)
}

/// A wheel event's deltas, in steps. (The first part of
/// `Emscripten_HandleWheel()`.)
pub(super) fn wheel_steps(wheel_event: &EmscriptenWheelEvent) -> (f32, f32) {
    let mut delta_y = wheel_event.deltaY as f32;
    let mut delta_x = wheel_event.deltaX as f32;

    match wheel_event.deltaMode {
        sys::DOM_DELTA_PIXEL => {
            delta_x /= 100.0; // 100 pixels make up a step
            delta_y /= 100.0; // 100 pixels make up a step
        }
        sys::DOM_DELTA_LINE => {
            delta_x /= 3.0; // 3 lines make up a step
            delta_y /= 3.0; // 3 lines make up a step
        }
        sys::DOM_DELTA_PAGE => {
            delta_x *= 80.0; // A page makes up 80 steps
            delta_y *= 80.0; // A page makes up 80 steps
        }
        _ => {}
    }

    (delta_x, delta_y)
}

/// The window's `focus` and `blur`. Translation of `Emscripten_HandleFocus()`.
unsafe extern "C-unwind" fn handle_focus(
    event_type: c_int,
    _focus_event: *const EmscriptenFocusEvent,
    _user_data: *mut c_void,
) -> bool {
    let window = get_focused_window();

    /* If the user switches away while keys are pressed (such as
     * via Alt+Tab), key release events won't be received. */
    if event_type == sys::EMSCRIPTEN_EVENT_BLUR {
        keyboard::reset_keyboard();
    }

    let sdl_event_type = if event_type == sys::EMSCRIPTEN_EVENT_FOCUS {
        EventType::WINDOW_FOCUS_GAINED
    } else {
        EventType::WINDOW_FOCUS_LOST
    };
    let _ = keyboard::set_keyboard_focus(if sdl_event_type == EventType::WINDOW_FOCUS_GAINED {
        window
    } else {
        None
    });
    event_enabled(sdl_event_type)
}

/// Translation of `IsFunctionKey()`.
fn is_function_key(scancode: Scancode) -> bool {
    if scancode.0 >= Scancode::F1.0 && scancode.0 <= Scancode::F12.0 {
        return true;
    }
    if scancode.0 >= Scancode::F13.0 && scancode.0 <= Scancode::F24.0 {
        return true;
    }
    false
}

/// The scancode of a key event: its `code`'s ([`map_scan_code`]), or for
/// the keys without one, its `key`'s. (The first part of
/// `Emscripten_HandleKey()`.)
pub(super) fn key_event_scancode(code: &CStr, key: &[u8]) -> Scancode {
    let mut scancode = map_scan_code(code);

    if scancode == Scancode::UNKNOWN {
        scancode = match key {
            b"Sleep" => Scancode::SLEEP,
            b"ChannelUp" => Scancode::CHANNEL_INCREMENT,
            b"ChannelDown" => Scancode::CHANNEL_DECREMENT,
            b"MediaPlay" => Scancode::MEDIA_PLAY,
            b"MediaPause" => Scancode::MEDIA_PAUSE,
            b"MediaRecord" => Scancode::MEDIA_RECORD,
            b"MediaFastForward" => Scancode::MEDIA_FAST_FORWARD,
            b"MediaRewind" => Scancode::MEDIA_REWIND,
            b"Close" => Scancode::AC_CLOSE,
            b"New" => Scancode::AC_NEW,
            b"Open" => Scancode::AC_OPEN,
            b"Print" => Scancode::AC_PRINT,
            b"Save" => Scancode::AC_SAVE,
            b"Props" => Scancode::AC_PROPERTIES,
            _ => Scancode::UNKNOWN,
        };
    }

    if scancode == Scancode::UNKNOWN {
        // KaiOS Left Soft Key and Right Soft Key, they act as OK/Next/Menu and Cancel/Back/Clear
        if key == b"SoftLeft" {
            scancode = Scancode::AC_FORWARD;
        } else if key == b"SoftRight" {
            scancode = Scancode::AC_BACK;
        }
    }

    scancode
}

/// The keycode of a key event: its `key`'s character, for a key of the
/// standard location (0) whose `key` is one; [`Keycode::UNKNOWN`] for the
/// rest. (The second part of `Emscripten_HandleKey()`.)
pub(super) fn key_event_keycode(location: u32, key: &[u8]) -> Keycode {
    let mut keycode = Keycode::UNKNOWN;
    if location == 0 && utf8strlen(key) == 1 {
        let mut key = key;
        keycode = Keycode(step_utf8(&mut key));
        if keycode.0 == INVALID_UNICODE_CODEPOINT {
            keycode = Keycode::UNKNOWN;
        }
    }
    keycode
}

/* This is a great tool to see web keyboard events live:
 * https://w3c.github.io/uievents/tools/key-event-viewer.html
 */
/// The keyboard element's `keydown` and `keyup`. Translation of
/// `Emscripten_HandleKey()`.
unsafe extern "C-unwind" fn handle_key(
    event_type: c_int,
    key_event: *const EmscriptenKeyboardEvent,
    user_data: *mut c_void,
) -> bool {
    let window = user_data as usize as WindowID;
    // SAFETY: emscripten's event, for the call.
    let key_event = unsafe { &*key_event };
    let key = em_string(&key_event.key);
    let code = CString::new(em_string(&key_event.code)).unwrap_or_default();
    let scancode = key_event_scancode(&code, key);
    let keycode = key_event_keycode(key_event.location, key);
    let down = event_type == sys::EMSCRIPTEN_EVENT_KEYDOWN;
    let mut is_nav_key = false;

    let mut prevent_default = if keycode != Keycode::UNKNOWN {
        keyboard::send_keyboard_key_and_keycode(
            Duration::ZERO,
            DEFAULT_KEYBOARD_ID,
            0,
            scancode,
            keycode,
            down,
        )
    } else {
        keyboard::send_keyboard_key(Duration::ZERO, DEFAULT_KEYBOARD_ID, 0, scancode, down)
    };

    /* if TEXTINPUT events are enabled we can't prevent keydown or we won't get keypress
     * we need to ALWAYS prevent backspace and tab otherwise chrome takes action and does bad navigation UX
     */
    if scancode == Scancode::BACKSPACE
        || scancode == Scancode::TAB
        || scancode == Scancode::LEFT
        || scancode == Scancode::UP
        || scancode == Scancode::RIGHT
        || scancode == Scancode::DOWN
        || is_function_key(scancode)
        || key_event.ctrlKey
    {
        is_nav_key = true;
    }

    if down && keyboard::text_input_active(window) && !is_nav_key {
        prevent_default = false;
    }

    prevent_default
}

/// The keyboard element's `keypress`: text, while text input is active.
/// Translation of `Emscripten_HandleKeyPress()`.
unsafe extern "C-unwind" fn handle_key_press(
    _event_type: c_int,
    key_event: *const EmscriptenKeyboardEvent,
    user_data: *mut c_void,
) -> bool {
    let window = user_data as usize as WindowID;
    // SAFETY: emscripten's event, for the call.
    let key_event = unsafe { &*key_event };
    let key = em_string(&key_event.key);

    if keyboard::text_input_active(window) && utf8strlen(key) == 1 {
        // (emscripten writes the key as UTF-8, whole characters only)
        keyboard::send_keyboard_text(&String::from_utf8_lossy(key));
        return true;
    }
    false
}

/// Translation of `Emscripten_HandleFullscreenChange()`.
fn handle_fullscreen_change(
    window: WindowID,
    fullscreen_change_event: &EmscriptenFullscreenChangeEvent,
) -> bool {
    with_data(window, |d| d.fullscreen_change_in_progress = false);

    if fullscreen_change_event.isFullscreen {
        send_window_event(window, EventType::WINDOW_ENTER_FULLSCREEN, 0, 0);
        with_data(window, |d| d.fullscreen_mode_flags = WindowFlags::NONE);
    } else {
        send_window_event(window, EventType::WINDOW_LEAVE_FULLSCREEN, 0, 0);
    }

    let _ = update_fullscreen_mode(
        window,
        if fullscreen_change_event.isFullscreen {
            FullscreenOp::Enter
        } else {
            FullscreenOp::Leave
        },
        false,
    );

    false
}

/// Translation of `Emscripten_HandleFullscreenChangeGlobal()`.
unsafe extern "C-unwind" fn handle_fullscreen_change_global(
    _event_type: c_int,
    fullscreen_change_event: *const EmscriptenFullscreenChangeEvent,
    _user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten's event, for the call.
    let fullscreen_change_event = unsafe { &*fullscreen_change_event };
    // SAFETY: emscripten writes a NUL-terminated id (cut to the array).
    let id = unsafe { CStr::from_ptr(fullscreen_change_event.id.as_ptr()) };

    let window = window_ids().into_iter().find(|&window| {
        with_data(window, |d| {
            let canvas_id = d.canvas_id.to_bytes();
            let canvas_id = canvas_id.strip_prefix(b"#").unwrap_or(canvas_id);
            canvas_id == id.to_bytes() // this is the window.
        })
        .unwrap_or(false)
    });

    if let Some(window) = window {
        return handle_fullscreen_change(window, fullscreen_change_event);
    }

    false
}

/// Translation of `Emscripten_HandleResize()`.
fn handle_resize(window: WindowID, ui_event: &EmscriptenUiEvent) -> bool {
    let Some((pixel_ratio, external_size, canvas_id)) = with_data(window, |d| {
        (d.pixel_ratio, d.external_size, d.canvas_id.clone())
    }) else {
        return false;
    };
    let Ok((flags, window_w, window_h)) = with_window(window, |w| (w.flags(), w.core.w, w.core.h))
    else {
        return false;
    };

    let mut force = false;
    let mut pixel_ratio = pixel_ratio;

    // update pixel ratio
    if flags.contains(WindowFlags::HIGH_PIXEL_DENSITY) {
        // SAFETY: a plain call.
        let ratio = unsafe { sys::emscripten_get_device_pixel_ratio() };
        if f64::from(pixel_ratio) != ratio {
            pixel_ratio = ratio as f32;
            with_data(window, |d| d.pixel_ratio = pixel_ratio);
            force = true;
        }
    }

    let fill_document = fill_document_window() == Some(window);
    let fullscreen = flags.contains(WindowFlags::FULLSCREEN); // fullscreen windows can resize on Emscripten, and the canvas should fill it.
    let resizable = flags.contains(WindowFlags::RESIZABLE);
    if fill_document || fullscreen || resizable {
        let (w, h) = if fill_document || fullscreen {
            (
                f64::from(ui_event.windowInnerWidth),
                f64::from(ui_event.windowInnerHeight),
            )
        } else {
            debug_assert!(flags.contains(WindowFlags::RESIZABLE));
            // this will only work if the canvas size is set through css
            if external_size {
                super::element_css_size(&canvas_id)
            } else {
                (f64::from(window_w), f64::from(window_h))
            }
        };

        // (double times float: double arithmetic, as C's)
        super::set_canvas_element_size(
            &canvas_id,
            lroundf((w * f64::from(pixel_ratio)) as f32),
            lroundf((h * f64::from(pixel_ratio)) as f32),
        );

        // set_canvas_size unsets this
        if !external_size && pixel_ratio != 1.0 {
            super::set_element_css_size(&canvas_id, w, h);
        }

        if force {
            // force the event to trigger, so pixel ratio changes can be handled
            let _ = with_window(window, |w| {
                w.core.w = 0;
                w.core.h = 0;
            });
        }

        send_window_event(
            window,
            EventType::WINDOW_RESIZED,
            lroundf(w as f32) as i32,
            lroundf(h as f32) as i32,
        );
    }

    false
}

/// Translation of `Emscripten_HandleResizeGlobal()`.
unsafe extern "C-unwind" fn handle_resize_global(
    _event_type: c_int,
    ui_event: *const EmscriptenUiEvent,
    _user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten's event, for the call.
    let ui_event = unsafe { &*ui_event };
    let mut prevent_default = false;

    for window in window_ids() {
        prevent_default |= handle_resize(window, ui_event);
    }

    prevent_default
}

/// The canvas resize callback of a fullscreen request (the window's ID is
/// the user data). Translation of `Emscripten_HandleCanvasResize()`.
pub(super) unsafe extern "C-unwind" fn handle_canvas_resize(
    _event_type: c_int,
    _reserved: *const c_void,
    user_data: *mut c_void,
) -> bool {
    // this is used during fullscreen changes
    let window = user_data as usize as WindowID;

    if let Some((true, canvas_id)) =
        with_data(window, |d| (d.fullscreen_resize, d.canvas_id.clone()))
    {
        let (css_w, css_h) = super::element_css_size(&canvas_id);
        send_window_event(
            window,
            EventType::WINDOW_RESIZED,
            lroundf(css_w as f32) as i32,
            lroundf(css_h as f32) as i32,
        );
    }

    false
}

/// The document's `visibilitychange`. Translation of
/// `Emscripten_HandleVisibilityChange()`.
unsafe extern "C-unwind" fn handle_visibility_change(
    _event_type: c_int,
    vis_event: *const EmscriptenVisibilityChangeEvent,
    user_data: *mut c_void,
) -> bool {
    let window = user_data as usize as WindowID;
    // SAFETY: emscripten's event, for the call.
    let vis_event = unsafe { &*vis_event };
    send_window_event(
        window,
        if vis_event.hidden {
            EventType::WINDOW_HIDDEN
        } else {
            EventType::WINDOW_SHOWN
        },
        0,
        0,
    );
    false
}

/// The window's `beforeunload`. Translation of
/// `Emscripten_HandleBeforeUnload()`.
unsafe extern "C-unwind" fn handle_before_unload(
    _event_type: c_int,
    _reserved: *const c_void,
    _user_data: *mut c_void,
) -> *const c_char {
    /* This event will need to be handled synchronously, e.g. using
    SDL_AddEventWatch, as the page is being closed *now*. */
    // No need to send a SDL_EVENT_QUIT, the app won't get control again.
    send_app_event(EventType::TERMINATING);
    c"".as_ptr() // don't trigger confirmation dialog
}

/// The SDL orientation of an html5.h one (`orientationIndex`). The
/// `CHECK_ORIENTATION` switch of `Emscripten_HandleOrientationChange()`.
pub(super) fn display_orientation(orientation_index: c_int) -> DisplayOrientation {
    match orientation_index {
        sys::EMSCRIPTEN_ORIENTATION_LANDSCAPE_PRIMARY => DisplayOrientation::Landscape,
        sys::EMSCRIPTEN_ORIENTATION_LANDSCAPE_SECONDARY => DisplayOrientation::LandscapeFlipped,
        sys::EMSCRIPTEN_ORIENTATION_PORTRAIT_PRIMARY => DisplayOrientation::Portrait,
        sys::EMSCRIPTEN_ORIENTATION_PORTRAIT_SECONDARY => DisplayOrientation::PortraitFlipped,
        _ => DisplayOrientation::Unknown,
    }
}

/// The screen orientation's `change`. Translation of
/// `Emscripten_HandleOrientationChange()`.
unsafe extern "C-unwind" fn handle_orientation_change(
    _event_type: c_int,
    orientation_change_event: *const EmscriptenOrientationChangeEvent,
    user_data: *mut c_void,
) -> bool {
    // SAFETY: emscripten's event, for the call.
    let orientation_change_event = unsafe { &*orientation_change_event };
    let orientation = display_orientation(orientation_change_event.orientationIndex);

    let window = user_data as usize as WindowID;
    let display = crate::video::window::display_for_window(window).unwrap_or(0);
    send_display_event(
        display,
        EventType::DISPLAY_ORIENTATION,
        orientation as i32,
        0,
    );

    // fake a UI event so we can tell the app the canvas might have resized.
    // SAFETY: plain JavaScript.
    let ui_event = unsafe {
        EmscriptenUiEvent {
            detail: 0,
            documentBodyClientWidth: js::sdl3_document_body_client_width(),
            documentBodyClientHeight: js::sdl3_document_body_client_height(),
            windowInnerWidth: sdl3_emscripten::video::sdl3_window_inner_width(),
            windowInnerHeight: sdl3_emscripten::video::sdl3_window_inner_height(),
            windowOuterWidth: js::sdl3_window_outer_width(),
            windowOuterHeight: js::sdl3_window_outer_height(),
            // (upstream's, crossed: nothing reads them)
            scrollTop: js::sdl3_window_page_x_offset(),
            scrollLeft: js::sdl3_window_page_y_offset(),
        }
    };
    handle_resize(window, &ui_event);

    false
}

// IF YOU CHANGE THIS STRUCTURE, YOU NEED TO UPDATE THE JAVASCRIPT THAT FILLS IT IN: SDL3.makePointerEventCStruct, below.
/// Translation of `PTRTYPE_MOUSE`.
const PTRTYPE_MOUSE: c_int = 1;
/// Translation of `PTRTYPE_TOUCH`.
const PTRTYPE_TOUCH: c_int = 2;
/// Translation of `PTRTYPE_PEN`.
const PTRTYPE_PEN: c_int = 3;

/// A pointer event, as `Module['SDL3'].makePointerEventCStruct()` writes
/// it (`sdl3_emscripten::video::events::sdl3_prep_pointer_event_callbacks`).
/// Translation of `Emscripten_PointerEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct PointerEvent {
    pub(super) pointer_type: c_int,
    pub(super) pointerid: c_int,
    pub(super) button: c_int,
    pub(super) buttons: c_int,
    pub(super) down: c_int,
    pub(super) movement_x: f32,
    pub(super) movement_y: f32,
    pub(super) target_x: f32,
    pub(super) target_y: f32,
    pub(super) pressure: f32,
    pub(super) tangential_pressure: f32,
    pub(super) tiltx: f32,
    pub(super) tilty: f32,
    pub(super) rotation: f32,
}

impl PointerEvent {
    /// The event the JavaScript wrote at `event`, which writes the last five
    /// fields (pressure to rotation) only for a pen: zero here for the rest
    /// (C leaves them as `SDL_malloc()` gave them, and doesn't read them).
    ///
    /// # Safety
    ///
    /// `event` points at such an event.
    unsafe fn read(event: *const PointerEvent) -> PointerEvent {
        // SAFETY: the caller's; the fields read are the ones written.
        unsafe {
            let mut e = PointerEvent {
                pointer_type: (*event).pointer_type,
                pointerid: (*event).pointerid,
                button: (*event).button,
                buttons: (*event).buttons,
                down: (*event).down,
                movement_x: (*event).movement_x,
                movement_y: (*event).movement_y,
                target_x: (*event).target_x,
                target_y: (*event).target_y,
                ..PointerEvent::default()
            };
            if e.pointer_type == PTRTYPE_PEN {
                e.pressure = (*event).pressure;
                e.tangential_pressure = (*event).tangential_pressure;
                e.tiltx = (*event).tiltx;
                e.tilty = (*event).tilty;
                e.rotation = (*event).rotation;
            }
            e
        }
    }
}

/// What the pointer handlers read of a window: its flags, its size, its
/// canvas's CSS size and whether it has the pointer lock.
struct PointerTarget {
    flags: WindowFlags,
    w: i32,
    h: i32,
    canvas_id: CString,
    has_pointer_lock: bool,
}

fn pointer_target(window: WindowID) -> Option<PointerTarget> {
    let (canvas_id, has_pointer_lock) =
        with_data(window, |d| (d.canvas_id.clone(), d.has_pointer_lock))?;
    let (flags, w, h) = with_window(window, |w| (w.flags(), w.core.w, w.core.h)).ok()?;
    Some(PointerTarget {
        flags,
        w,
        h,
        canvas_id,
        has_pointer_lock,
    })
}

impl PointerTarget {
    /// `emscripten_get_element_css_size()` of the canvas.
    fn client_size(&self) -> (f64, f64) {
        super::element_css_size(&self.canvas_id)
    }

    /// The event's position in the window (its movement, with the pointer
    /// lock), rescaled from the canvas's CSS size (in case canvas is being
    /// scaled).
    fn scaled(&self, event: &PointerEvent) -> (f32, f32) {
        // rescale (in case canvas is being scaled)
        let (client_w, client_h) = self.client_size();
        let xscale = f64::from(self.w) / client_w;
        let yscale = f64::from(self.h) / client_h;

        let is_pointer_locked = self.has_pointer_lock;
        if is_pointer_locked {
            (
                (f64::from(event.movement_x) * xscale) as f32,
                (f64::from(event.movement_y) * yscale) as f32,
            )
        } else {
            (
                (f64::from(event.target_x) * xscale) as f32,
                (f64::from(event.target_y) * yscale) as f32,
            )
        }
    }

    /// The event's position in the canvas, normalized (0.5 for a canvas of
    /// 1 pixel or less), as a touch's.
    fn normalized(&self, event: &PointerEvent) -> (f32, f32) {
        let (client_w, client_h) = self.client_size();
        let x = if client_w <= 1.0 {
            0.5
        } else {
            (f64::from(event.target_x) / (client_w - 1.0)) as f32
        };
        let y = if client_h <= 1.0 {
            0.5
        } else {
            (f64::from(event.target_y) / (client_h - 1.0)) as f32
        };
        (x, y)
    }
}

/// A mouse pointer event's SDL button (0 for none) and whether it's down
/// (a `pointerdown`, or the button in `buttons`). The `CHECK_MOUSE_BUTTON`
/// switch of `Emscripten_HandleMouseButton()`.
pub(super) fn mouse_button(event: &PointerEvent) -> (u8, bool) {
    // (jsbutton, downflag, sdlbutton)
    let check = |downflag: c_int, sdl_button: u8| {
        (
            sdl_button,
            (event.down != 0) || ((event.buttons & downflag) != 0),
        )
    };
    match event.button {
        0 => check(1, BUTTON_LEFT),
        1 => check(4, BUTTON_MIDDLE),
        2 => check(2, BUTTON_RIGHT),
        3 => check(8, BUTTON_X1),
        4 => check(16, BUTTON_X2),
        _ => (0, false),
    }
}

/// Translation of `Emscripten_HandleMouseButton()`.
fn handle_mouse_button(window: WindowID, event: &PointerEvent) {
    let (sdl_button, down) = mouse_button(event);

    if sdl_button != 0 {
        let Some((canvas_id, has_pointer_lock)) =
            with_data(window, |d| (d.canvas_id.clone(), d.has_pointer_lock))
        else {
            return;
        };

        if down && mouse::with_mouse(|m| m.relative_mode) && !has_pointer_lock {
            // try to regrab lost pointer lock.
            // SAFETY: a NUL-terminated selector.
            unsafe { sys::emscripten_request_pointerlock(canvas_id.as_ptr(), false) };
        }

        mouse::send_mouse_button(
            Duration::ZERO,
            Some(window),
            DEFAULT_MOUSE_ID,
            sdl_button,
            down,
        );

        // We have an imaginary mouse capture, because we need SDL to not drop our imaginary mouse focus when we leave the canvas.
        if mouse::with_mouse(|m| m.auto_capture) {
            let captured = mouse::mouse_state().2 != MouseButtonFlags::NONE;
            let _ = with_window(window, |w| {
                w.core.flags.set(WindowFlags::MOUSE_CAPTURE, captured);
            });
        }

        let pending = with_data(window, |d| d.mouse_focus_loss_pending).unwrap_or(false);
        if !down && pending {
            let captured = with_window(window, |w| w.flags().contains(WindowFlags::MOUSE_CAPTURE))
                .unwrap_or(false);
            with_data(window, |d| d.mouse_focus_loss_pending = captured);
            if !captured {
                mouse::set_mouse_focus(None);
            }
        }
    }
}

/// Translation of `Emscripten_UpdateMouseFromEvent()`.
fn update_mouse_from_event(window: WindowID, event: &PointerEvent) {
    crate::sdl_assert!(event.pointer_type == PTRTYPE_MOUSE);

    let Some(target) = pointer_target(window) else {
        return;
    };

    // Hidden windows (e.g. a shared GL-context window) may share the DOM canvas with the
    // visible window. Their pointer-event listeners would otherwise fire alongside the
    // visible window's, fighting over `mouse->focus` and producing events tagged with the
    // hidden window's ID -- causing downstream consumers that key by window ID to silently
    // drop them. Hidden windows shouldn't take part in user-input dispatch.
    if target.flags.contains(WindowFlags::HIDDEN) {
        return;
    }

    let (mx, my) = target.scaled(event);

    mouse::send_mouse_motion(
        Duration::ZERO,
        Some(window),
        DEFAULT_MOUSE_ID,
        target.has_pointer_lock,
        mx,
        my,
    );

    handle_mouse_button(window, event);
}

/// The touch device's ID (`deviceId`).
const TOUCH_DEVICE_ID: TouchID = 1;

/// A pointer's finger ID: its `pointerId`, plus one.
fn finger_id(event: &PointerEvent) -> FingerID {
    event.pointerid.wrapping_add(1) as FingerID
}

/// Translation of `Emscripten_UpdateTouchFromEvent()`.
fn update_touch_from_event(window: WindowID, event: &PointerEvent) {
    crate::sdl_assert!(event.pointer_type == PTRTYPE_TOUCH);

    let device_id = TOUCH_DEVICE_ID;
    // (SDL_AddTouch() can't fail here)
    touch::add_touch(device_id, TouchDeviceType::Direct, "");

    let Some(target) = pointer_target(window) else {
        return;
    };

    let id = finger_id(event);
    let (x, y) = target.normalized(event);

    let down = (event.buttons & 1) != 0;
    if event.button == 0 {
        // touch is starting or ending if this is zero (-1 means "no change").
        if down {
            touch::send_touch(
                Duration::ZERO,
                device_id,
                id,
                Some(window),
                EventType::FINGER_DOWN,
                x,
                y,
                1.0,
            );
        }
    }

    touch::send_touch_motion(Duration::ZERO, device_id, id, Some(window), x, y, 1.0);

    if event.button == 0 {
        // touch is starting or ending if this is zero (-1 means "no change").
        if !down {
            touch::send_touch(
                Duration::ZERO,
                device_id,
                id,
                Some(window),
                EventType::FINGER_UP,
                x,
                y,
                1.0,
            );
        }
    }
}

/// The one pen's handle (upstream's `(void *) (size_t) 1`: "something > 0
/// for the single pen handle").
struct PenHandle;

/// The one pen, if it's there (`SDL_FindPenByHandle((void *) (size_t) 1)`).
fn find_pen() -> PenID {
    pen::find_pen_by_callback(|handle| handle.is::<PenHandle>())
}

/// Translation of `Emscripten_UpdatePenFromEvent()`.
fn update_pen_from_event(window: WindowID, event: &PointerEvent) {
    crate::sdl_assert!(event.pointer_type == PTRTYPE_PEN);
    let pen = find_pen(); // something > 0 for the single pen handle.
    if pen != 0 {
        let Some(target) = pointer_target(window) else {
            return;
        };
        // rescale (in case canvas is being scaled)
        let (mx, my) = target.scaled(event);
        let timestamp = Duration::ZERO;
        let window = Some(window);

        pen::send_pen_motion(timestamp, pen, window, mx, my);

        if event.button == 0 {
            // pen touch
            let down = (event.buttons & 1) != 0;
            pen::send_pen_touch(timestamp, pen, window, false, down);
        } else if event.button == 5 {
            // eraser touch...? Not sure if this is right...
            let down = (event.buttons & 32) != 0;
            pen::send_pen_touch(timestamp, pen, window, true, down);
        } else if event.button == 1 {
            let down = (event.buttons & 4) != 0;
            pen::send_pen_button(timestamp, pen, window, 2, down);
        } else if event.button == 2 {
            let down = (event.buttons & 2) != 0;
            pen::send_pen_button(timestamp, pen, window, 1, down);
        }

        pen::send_pen_axis(timestamp, pen, window, PenAxis::Pressure, event.pressure);
        pen::send_pen_axis(
            timestamp,
            pen,
            window,
            PenAxis::TangentialPressure,
            event.tangential_pressure,
        );
        pen::send_pen_axis(timestamp, pen, window, PenAxis::XTilt, event.tiltx);
        pen::send_pen_axis(timestamp, pen, window, PenAxis::YTilt, event.tilty);
        pen::send_pen_axis(timestamp, pen, window, PenAxis::Rotation, event.rotation);
    }
}

/// Translation of `Emscripten_UpdatePointerFromEvent()`.
fn update_pointer_from_event(window: WindowID, event: &PointerEvent) {
    if event.pointer_type == PTRTYPE_MOUSE {
        update_mouse_from_event(window, event);
    } else if event.pointer_type == PTRTYPE_TOUCH {
        update_touch_from_event(window, event);
    } else if event.pointer_type == PTRTYPE_PEN {
        update_pen_from_event(window, event);
    } else {
        crate::sdl_assert!(!"Unexpected pointer event type");
    }
}

/// Translation of `Emscripten_HandleMouseFocus()`.
fn handle_mouse_focus(window: WindowID, event: &PointerEvent, isenter: bool) {
    crate::sdl_assert!(event.pointer_type == PTRTYPE_MOUSE);

    let Some(target) = pointer_target(window) else {
        return;
    };

    // Hidden windows shouldn't ever become the mouse-focus target.
    if target.flags.contains(WindowFlags::HIDDEN) {
        return;
    }

    let is_pointer_locked = target.has_pointer_lock;

    if !is_pointer_locked {
        // rescale (in case canvas is being scaled)
        let (client_w, client_h) = target.client_size();

        let mx = (f64::from(event.target_x) * (f64::from(target.w) / client_w)) as f32;
        let my = (f64::from(event.target_y) * (f64::from(target.h) / client_h)) as f32;
        mouse::send_mouse_motion(
            Duration::ZERO,
            Some(window),
            GLOBAL_MOUSE_ID,
            is_pointer_locked,
            mx,
            my,
        );
    }

    let pending = with_data(window, |d| d.mouse_focus_loss_pending).unwrap_or(false);
    let captured =
        with_window(window, |w| w.flags().contains(WindowFlags::MOUSE_CAPTURE)).unwrap_or(false);
    if isenter && pending {
        with_data(window, |d| d.mouse_focus_loss_pending = false); // just drop the state, but don't send the enter event.
    } else if !isenter && captured {
        with_data(window, |d| d.mouse_focus_loss_pending = true); // waiting on a mouse button to let go before we send the mouse focus update.
    } else {
        mouse::set_mouse_focus(if isenter { Some(window) } else { None });
    }
}

/// Translation of `Emscripten_HandlePenEnter()`.
fn handle_pen_enter(window: WindowID, event: &PointerEvent) {
    crate::sdl_assert!(event.pointer_type == PTRTYPE_PEN);

    // event->pointerid is one continuous interaction; it doesn't necessarily track a specific tool over time, like the same finger's ID changed on each new touch event.
    // as such, we only expose a single pen, and when the touch ends, we say it lost proximity instead of the calling SDL_RemovePenDevice().

    let pen = find_pen(); // something > 0 for the single pen handle.
    if pen != 0 {
        pen::send_pen_proximity(Duration::ZERO, pen, Some(window), true, true);
    } else {
        // Web browsers offer almost none of this information as specifics, but can without warning offer any of these specific things.
        let peninfo = PenInfo {
            capabilities: PenCapabilityFlags(
                PenCapabilityFlags::PRESSURE.0
                    | PenCapabilityFlags::ROTATION.0
                    | PenCapabilityFlags::XTILT.0
                    | PenCapabilityFlags::YTILT.0
                    | PenCapabilityFlags::TANGENTIAL_PRESSURE.0
                    | PenCapabilityFlags::ERASER.0,
            ),
            max_tilt: 90,
            num_buttons: 2,
            subtype: PenSubtype::Pen,
            ..PenInfo::default()
        };
        pen::add_pen_device(
            Duration::ZERO,
            None,
            Some(window),
            Some(&peninfo),
            Arc::new(PenHandle),
            true,
        );
    }

    update_pen_from_event(window, event);
}

/// `_Emscripten_HandlePointerEnter(window, event)`: the canvas's
/// `pointerenter`. Translation of `Emscripten_HandlePointerEnter()`.
extern "C-unwind" fn handle_pointer_enter(window: WindowID, event: *const PointerEvent) {
    crate::sdl_assert!(!event.is_null());
    // SAFETY: the JavaScript's event, for the call.
    let event = unsafe { PointerEvent::read(event) };
    if event.pointer_type == PTRTYPE_MOUSE {
        handle_mouse_focus(window, &event, true);
    } else if event.pointer_type == PTRTYPE_PEN {
        handle_pen_enter(window, &event);
    } else if event.pointer_type == PTRTYPE_TOUCH {
        // do nothing.
    } else {
        crate::sdl_assert!(!"Unexpected pointer event type");
    }
}

/// Translation of `Emscripten_HandlePenLeave()`.
fn handle_pen_leave(window: WindowID, event: &PointerEvent) {
    let pen = find_pen(); // something > 0 for the single pen handle.
    if pen != 0 {
        update_pointer_from_event(window, event); // last data updates?
        pen::send_pen_proximity(Duration::ZERO, pen, Some(window), false, false);
    }
}

/// Translation of `Emscripten_HandleTouchCancel()`.
fn handle_touch_cancel(window: WindowID, event: &PointerEvent) {
    crate::sdl_assert!(event.pointer_type == PTRTYPE_TOUCH);

    let device_id = TOUCH_DEVICE_ID;
    // (SDL_AddTouch() can't fail here)
    touch::add_touch(device_id, TouchDeviceType::Direct, "");

    let Some(target) = pointer_target(window) else {
        return;
    };

    let id = finger_id(event);
    let (x, y) = target.normalized(event);

    touch::send_touch(
        Duration::ZERO,
        device_id,
        id,
        Some(window),
        EventType::FINGER_CANCELED,
        x,
        y,
        1.0,
    );
}

/// `_Emscripten_HandlePointerLeave(window, event)`: the canvas's
/// `pointerleave` and `pointercancel`. Translation of
/// `Emscripten_HandlePointerLeave()`.
extern "C-unwind" fn handle_pointer_leave(window: WindowID, event: *const PointerEvent) {
    crate::sdl_assert!(!event.is_null());
    // SAFETY: the JavaScript's event, for the call.
    let event = unsafe { PointerEvent::read(event) };
    if event.pointer_type == PTRTYPE_MOUSE {
        handle_mouse_focus(window, &event, false);
    } else if event.pointer_type == PTRTYPE_PEN {
        handle_pen_leave(window, &event);
    } else if event.pointer_type == PTRTYPE_TOUCH {
        handle_touch_cancel(window, &event);
    } else {
        crate::sdl_assert!(!"Unexpected pointer event type");
    }
}

/// `_Emscripten_HandlePointerGeneric(window, event)`: the canvas's
/// `pointerdown`, `pointermove` and `pointerup`. Translation of
/// `Emscripten_HandlePointerGeneric()`.
extern "C-unwind" fn handle_pointer_generic(window: WindowID, event: *const PointerEvent) {
    crate::sdl_assert!(!event.is_null());
    // SAFETY: the JavaScript's event, for the call.
    let event = unsafe { PointerEvent::read(event) };
    update_pointer_from_event(window, &event);
}

/// Translation of `Emscripten_prep_pointer_event_callbacks()`.
fn prep_pointer_event_callbacks() {
    // SAFETY: plain JavaScript (it allocates with _SDL_malloc, registered).
    unsafe { js::sdl3_prep_pointer_event_callbacks(size_of::<PointerEvent>()) };
}

/// Translation of `Emscripten_set_pointer_event_callbacks()`.
fn set_pointer_event_callbacks(window: WindowID, canvas_id: &CStr) {
    prep_pointer_event_callbacks();

    // SAFETY: a NUL-terminated selector; the functions it calls are
    // registered.
    unsafe { js::sdl3_set_pointer_event_callbacks(window, canvas_id.as_ptr()) };
}

/// Translation of `Emscripten_unset_pointer_event_callbacks()`.
fn unset_pointer_event_callbacks(canvas_id: &CStr) {
    // SAFETY: a NUL-terminated selector.
    unsafe { js::sdl3_unset_pointer_event_callbacks(canvas_id.as_ptr()) };
}

/// `_Emscripten_HandleMouseButtonUpGlobal(device, event)`: the document's
/// `pointerup` (the device is 0: the windows are the video core's).
/// Translation of `Emscripten_HandleMouseButtonUpGlobal()`.
extern "C-unwind" fn handle_mouse_button_up_global(_device: u32, event: *const PointerEvent) {
    crate::sdl_assert!(!event.is_null());
    // SAFETY: the JavaScript's event, for the call.
    let event = unsafe { PointerEvent::read(event) };
    if event.pointer_type == PTRTYPE_MOUSE {
        for window in window_ids() {
            handle_mouse_button(window, &event);
        }
    }
}

/// Translation of `Emscripten_set_global_mouseup_callback()`.
fn set_global_mouseup_callback() {
    prep_pointer_event_callbacks();

    // SAFETY: plain JavaScript; the function it calls is registered.
    unsafe { js::sdl3_set_global_mouseup_callback(0) };
}

/// Translation of `Emscripten_unset_global_mouseup_callback()`.
fn unset_global_mouseup_callback() {
    // SAFETY: plain JavaScript.
    unsafe { js::sdl3_unset_global_mouseup_callback() };
}

// IF YOU CHANGE THIS STRUCTURE, YOU NEED TO UPDATE THE JAVASCRIPT THAT FILLS IT IN: makeDropEventCStruct, below.
/// A drag's position, as the `dragover` listener writes it
/// (`sdl3_emscripten::video::events::sdl3_set_drag_event_callbacks`).
/// Translation of `Emscripten_DropEvent`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct DropEvent {
    pub(super) x: c_int,
    pub(super) y: c_int,
}

// Note (upstream): a file whose FileReader finishes after its window was
// destroyed comes with the window's ID here, which names no window any
// more (the drop events carry it); C reads the SDL_WindowData it freed.

/// `_Emscripten_SendDragEvent(window, event)`: the canvas's `dragover`.
/// Translation of `Emscripten_SendDragEvent()`.
extern "C-unwind" fn send_drag_event(window: WindowID, event: *const DropEvent) {
    // SAFETY: the JavaScript's event, for the call.
    let event = unsafe { *event };
    send_drop_position(Some(window), event.x as f32, event.y as f32);
}

/// `_Emscripten_SendDragCompleteEvent(window)`. Translation of
/// `Emscripten_SendDragCompleteEvent()`.
extern "C-unwind" fn send_drag_complete_event(window: WindowID) {
    send_drop_complete(Some(window));
}

/// `_Emscripten_SendDragTextEvent(window, text)`. Translation of
/// `Emscripten_SendDragTextEvent()`.
extern "C-unwind" fn send_drag_text_event(window: WindowID, text: *mut c_char) {
    // SAFETY: the JavaScript's NUL-terminated UTF-8, for the call.
    let text = unsafe { CStr::from_ptr(text) };
    send_drop_text(Some(window), &text.to_string_lossy());
}

/// `_Emscripten_SendDragFileEvent(window, filename)`. Translation of
/// `Emscripten_SendDragFileEvent()`.
extern "C-unwind" fn send_drag_file_event(window: WindowID, filename: *mut c_char) {
    // SAFETY: the JavaScript's NUL-terminated UTF-8, for the call.
    let filename = unsafe { CStr::from_ptr(filename) };
    send_drop_file(Some(window), None, &filename.to_string_lossy());
}

/// Translation of `Emscripten_set_drag_event_callbacks()`.
fn set_drag_event_callbacks(window: WindowID, canvas_id: &CStr) {
    // SAFETY: a NUL-terminated selector; the functions it calls are
    // registered.
    unsafe {
        js::sdl3_set_drag_event_callbacks(window, canvas_id.as_ptr(), size_of::<DropEvent>())
    };
}

/// Translation of `Emscripten_unset_drag_event_callbacks()`.
fn unset_drag_event_callbacks(canvas_id: &CStr) {
    // SAFETY: a NUL-terminated selector.
    unsafe { js::sdl3_unset_drag_event_callbacks(canvas_id.as_ptr()) };
}

/// The html5.h target of a keyboard element (the
/// [`hints::EMSCRIPTEN_KEYBOARD_ELEMENT`](crate::hints::EMSCRIPTEN_KEYBOARD_ELEMENT)
/// hint's value): `None` for `#none` (no keyboard handlers), the special
/// targets for `#window`, `#document` and `#screen`, the selector itself
/// for the rest. Translation of `Emscripten_GetKeyboardTargetElement()`.
pub(super) fn keyboard_target_element(target: &CStr) -> Option<*const c_char> {
    match target.to_bytes() {
        b"#none" => None,
        b"#window" => Some(EMSCRIPTEN_EVENT_TARGET_WINDOW),
        b"#document" => Some(EMSCRIPTEN_EVENT_TARGET_DOCUMENT),
        b"#screen" => Some(EMSCRIPTEN_EVENT_TARGET_SCREEN),
        _ => Some(target.as_ptr()),
    }
}

/// Makes the functions upstream marks `EMSCRIPTEN_KEEPALIVE` in this file
/// callable from JavaScript (see `sdl3_emscripten::Callback`), once.
pub(super) fn register_callbacks() {
    // SAFETY: extern "C-unwind" functions with the parameters the
    // JavaScript passes; they live as long as the program.
    unsafe {
        js::EMSCRIPTEN_HANDLE_POINTER_ENTER.register(handle_pointer_enter as *const ());
        js::EMSCRIPTEN_HANDLE_POINTER_LEAVE.register(handle_pointer_leave as *const ());
        js::EMSCRIPTEN_HANDLE_POINTER_GENERIC.register(handle_pointer_generic as *const ());
        js::EMSCRIPTEN_HANDLE_MOUSE_BUTTON_UP_GLOBAL
            .register(handle_mouse_button_up_global as *const ());
        js::EMSCRIPTEN_SEND_DRAG_EVENT.register(send_drag_event as *const ());
        js::EMSCRIPTEN_SEND_DRAG_COMPLETE_EVENT.register(send_drag_complete_event as *const ());
        js::EMSCRIPTEN_SEND_DRAG_TEXT_EVENT.register(send_drag_text_event as *const ());
        js::EMSCRIPTEN_SEND_DRAG_FILE_EVENT.register(send_drag_file_event as *const ());
        js::EMSCRIPTEN_HANDLE_LOCK_KEYS_CHECK.register(handle_lock_keys_check as *const ());
    }
}

/// Translation of `Emscripten_RegisterGlobalEventHandlers()`.
pub(super) fn register_global_event_handlers() {
    set_global_mouseup_callback();

    // SAFETY: the special targets, and handlers with emscripten's
    // signatures, which use no user data.
    unsafe {
        sys::emscripten_set_focus_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_WINDOW,
            ptr::null_mut(),
            false,
            Some(handle_focus),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );
        sys::emscripten_set_blur_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_WINDOW,
            ptr::null_mut(),
            false,
            Some(handle_focus),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_pointerlockchange_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_DOCUMENT,
            ptr::null_mut(),
            false,
            Some(handle_pointer_lock_change_global),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_fullscreenchange_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_DOCUMENT,
            ptr::null_mut(),
            false,
            Some(handle_fullscreen_change_global),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_resize_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_WINDOW,
            ptr::null_mut(),
            false,
            Some(handle_resize_global),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );
    }
}

/// Translation of `Emscripten_UnregisterGlobalEventHandlers()`.
pub(super) fn unregister_global_event_handlers() {
    unset_global_mouseup_callback();

    // SAFETY: the special targets; no handlers.
    unsafe {
        sys::emscripten_set_focus_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_WINDOW,
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );
        sys::emscripten_set_blur_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_WINDOW,
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_pointerlockchange_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_DOCUMENT,
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_fullscreenchange_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_DOCUMENT,
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_resize_callback_on_thread(
            EMSCRIPTEN_EVENT_TARGET_WINDOW,
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );
    }
}

/// The lock keys' modifier state the browser reports, as SDL's, in
/// `modstate` (the other modifiers as they are). (The computation of
/// `Emscripten_HandleLockKeysCheck()`.)
pub(super) fn lock_keys_mod_state(
    modstate: Keymod,
    capslock: bool,
    numlock: bool,
    scrolllock: bool,
) -> Option<Keymod> {
    let locks = Keymod::CAPS | Keymod::NUM | Keymod::SCROLL;
    let mut new_mods = Keymod::NONE;
    if capslock {
        new_mods |= Keymod::CAPS;
    }
    if numlock {
        new_mods |= Keymod::NUM;
    }
    if scrolllock {
        new_mods |= Keymod::SCROLL;
    }
    let mut modstate = modstate;
    if (modstate & locks) != new_mods {
        modstate &= !locks;
        modstate |= new_mods;
        return Some(modstate);
    }
    None
}

/// `_Emscripten_HandleLockKeysCheck(window, capslock, numlock,
/// scrolllock)`: the document's `keydown`, but for the lock keys.
/// Translation of `Emscripten_HandleLockKeysCheck()`.
extern "C-unwind" fn handle_lock_keys_check(
    _window: WindowID,
    capslock: bool,
    numlock: bool,
    scrolllock: bool,
) {
    if let Some(modstate) =
        lock_keys_mod_state(keyboard::mod_state(), capslock, numlock, scrolllock)
    {
        keyboard::set_mod_state(modstate);
    }
}

/// Translation of `Emscripten_RegisterEventHandlers()`.
pub(super) fn register_event_handlers(window: WindowID) {
    let Some((canvas_id, keyboard_element)) = with_data(window, |d| {
        (d.canvas_id.clone(), d.keyboard_element.clone())
    }) else {
        return;
    };
    let data = window as usize as *mut c_void;

    // SAFETY: NUL-terminated selectors (or the special targets) that live
    // through the calls, and handlers with emscripten's signatures, which
    // get the window's ID.
    unsafe {
        // There is only one window and that window is the canvas
        sys::emscripten_set_wheel_callback_on_thread(
            canvas_id.as_ptr(),
            data,
            false,
            Some(handle_wheel),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_orientationchange_callback_on_thread(
            data,
            false,
            Some(handle_orientation_change),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        if let Some(key_element) = keyboard_target_element(&keyboard_element) {
            // Emscripten's HTML5 helpers do not deduplicate `addEventListener` calls:
            // see `registerOrRemoveHandler` in `emscripten/src/lib/libhtml5.js`.
            //
            // If a previous SDL window already registered keyboard handlers on the same
            // target, the new registration would *stack* a second listener, causing every
            // browser keydown to fire `Emscripten_HandleKey` twice.
            //
            // The duplicate calls then produce two `SDL_EVENT_KEY_DOWN` events per physical
            // keypress (the second one with `repeat=true`, due to the keystate-based repeat
            // detection in `SDL_SendKeyboardKeyInternal`).
            //
            // We must clear any prior handler on this target before installing ours:
            for set in [
                sys::emscripten_set_keydown_callback_on_thread,
                sys::emscripten_set_keyup_callback_on_thread,
                sys::emscripten_set_keypress_callback_on_thread,
            ] {
                set(
                    key_element,
                    ptr::null_mut(),
                    false,
                    None,
                    EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
                );
            }

            js::sdl3_set_lock_keys_check(window);

            sys::emscripten_set_keydown_callback_on_thread(
                key_element,
                data,
                false,
                Some(handle_key),
                EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
            );
            sys::emscripten_set_keyup_callback_on_thread(
                key_element,
                data,
                false,
                Some(handle_key),
                EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
            );
            sys::emscripten_set_keypress_callback_on_thread(
                key_element,
                data,
                false,
                Some(handle_key_press),
                EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
            );
        }

        sys::emscripten_set_visibilitychange_callback_on_thread(
            data,
            false,
            Some(handle_visibility_change),
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        // (html5.h's emscripten_set_beforeunload_callback() is the main
        // thread's)
        sys::emscripten_set_beforeunload_callback_on_thread(
            data,
            Some(handle_before_unload),
            EM_CALLBACK_THREAD_CONTEXT_MAIN_RUNTIME_THREAD,
        );
    }

    // !!! FIXME: currently Emscripten doesn't have a Pointer Events functions like emscripten_set_*_callback, but we should use those when they do:
    // !!! FIXME:  https://github.com/emscripten-core/emscripten/issues/7278#issuecomment-2280024621
    set_pointer_event_callbacks(window, &canvas_id);

    // !!! FIXME: currently Emscripten doesn't have a Drop Events functions like emscripten_set_*_callback, but we should use those when they do:
    set_drag_event_callbacks(window, &canvas_id);
}

/// Translation of `Emscripten_UnregisterEventHandlers()` (of the window
/// data `destroy_window()` took from the window).
pub(super) fn unregister_event_handlers(data: &EmscriptenWindow) {
    // !!! FIXME: currently Emscripten doesn't have a Drop Events functions like emscripten_set_*_callback, but we should use those when they do:
    unset_drag_event_callbacks(&data.canvas_id);

    // !!! FIXME: currently Emscripten doesn't have a Pointer Events functions like emscripten_set_*_callback, but we should use those when they do:
    // !!! FIXME:  https://github.com/emscripten-core/emscripten/issues/7278#issuecomment-2280024621
    unset_pointer_event_callbacks(&data.canvas_id);

    // SAFETY: NUL-terminated selectors (or the special targets); no
    // handlers.
    unsafe {
        // only works due to having one window
        sys::emscripten_set_wheel_callback_on_thread(
            data.canvas_id.as_ptr(),
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_orientationchange_callback_on_thread(
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        if let Some(key_element) = keyboard_target_element(&data.keyboard_element) {
            for set in [
                sys::emscripten_set_keydown_callback_on_thread,
                sys::emscripten_set_keyup_callback_on_thread,
                sys::emscripten_set_keypress_callback_on_thread,
            ] {
                set(
                    key_element,
                    ptr::null_mut(),
                    false,
                    None,
                    EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
                );
            }
            js::sdl3_unset_lock_keys_check();
        }

        sys::emscripten_set_visibilitychange_callback_on_thread(
            ptr::null_mut(),
            false,
            None,
            EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD,
        );

        sys::emscripten_set_beforeunload_callback_on_thread(
            ptr::null_mut(),
            None,
            EM_CALLBACK_THREAD_CONTEXT_MAIN_RUNTIME_THREAD,
        );
    }
}
