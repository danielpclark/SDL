// Rust translation of src/events/SDL_keyboard.c and parts of SDL_keymap.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! General keyboard handling code for SDL: keyboard devices, key state,
//! modifier state, keymaps, key/scancode names and text input events.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard, RwLock};
use std::time::Duration;

mod keycode;
pub mod keymap;
mod scancode;

pub use keycode::{Keycode, Keymod};
pub use keymap::Keymap;
pub use scancode::Scancode;

use super::queue;
use super::window::{self, WindowFlags};
use super::{
    Event, EventType, KeyboardDeviceEvent, KeyboardEvent, KeyboardID, TextEditingCandidatesEvent,
    TextEditingEvent, TextInputEvent, WindowID,
};
use crate::error::{Error, Result};
use crate::hints;
use crate::{err, timer};

/// The keyboard id for keys without a specific source keyboard.
/// Translation of `SDL_GLOBAL_KEYBOARD_ID`.
pub const GLOBAL_KEYBOARD_ID: KeyboardID = 0;
/// The id backends use for the one keyboard they can't tell apart.
/// Translation of `SDL_DEFAULT_KEYBOARD_ID`.
pub const DEFAULT_KEYBOARD_ID: KeyboardID = 1;

// Global keyboard information

const KEYBOARD_HARDWARE: u8 = 0x01;
const KEYBOARD_VIRTUAL: u8 = 0x02;
const KEYBOARD_AUTORELEASE: u8 = 0x04;
const KEYBOARD_IGNOREMODIFIERS: u8 = 0x08;
const KEYBOARD_SOURCE_MASK: u8 = KEYBOARD_HARDWARE | KEYBOARD_AUTORELEASE;

const KEYCODE_OPTION_HIDE_NUMPAD: u32 = 0x01;
const KEYCODE_OPTION_FRENCH_NUMBERS: u32 = 0x02;
const KEYCODE_OPTION_LATIN_LETTERS: u32 = 0x04;
const DEFAULT_KEYCODE_OPTIONS: u32 = KEYCODE_OPTION_FRENCH_NUMBERS | KEYCODE_OPTION_LATIN_LETTERS;

/// Translation of `struct SDL_Keyboard`.
struct Keyboard {
    // Data common to all keyboards
    focus: Option<WindowID>,
    modstate: Keymod,
    keysource: [u8; Scancode::COUNT],
    keystate: [bool; Scancode::COUNT],
    keymap: Option<Keymap>,
    keycode_options: u32,
    autorelease_pending: bool,
    hardware_timestamp: u64,
    // SDL_keyboards / SDL_keyboard_names
    keyboards: Vec<KeyboardID>,
    keyboard_names: Vec<(KeyboardID, String)>,
    quitting: bool,
    /// Kept alive so the `SDL_HINT_KEYCODE_OPTIONS` watcher stays registered.
    #[allow(dead_code)]
    hint_callback: Option<hints::Callback>,
}

/// Translation of `SDL_keyboard` and the keyboard device list statics.
static KEYBOARD: Mutex<Keyboard> = Mutex::new(Keyboard {
    focus: None,
    modstate: Keymod::NONE,
    keysource: [0; Scancode::COUNT],
    keystate: [false; Scancode::COUNT],
    keymap: None,
    keycode_options: DEFAULT_KEYCODE_OPTIONS,
    autorelease_pending: false,
    hardware_timestamp: 0,
    keyboards: Vec::new(),
    keyboard_names: Vec::new(),
    quitting: false,
    hint_callback: None,
});

/// Lock the keyboard state. Never hold this across a call into the event
/// queue or the video hooks.
fn keyboard() -> MutexGuard<'static, Keyboard> {
    KEYBOARD.lock().unwrap_or_else(|e| e.into_inner())
}

/// Translation of `SDL_KeycodeOptionsChanged()`.
fn keycode_options_changed(hint: Option<&str>) {
    let mut keyboard = keyboard();
    match hint {
        Some(hint) if !hint.is_empty() => {
            keyboard.keycode_options = 0;
            if !hint.contains("none") {
                if hint.contains("hide_numpad") {
                    keyboard.keycode_options |= KEYCODE_OPTION_HIDE_NUMPAD;
                }
                if hint.contains("french_numbers") {
                    keyboard.keycode_options |= KEYCODE_OPTION_FRENCH_NUMBERS;
                }
                if hint.contains("latin_letters") {
                    keyboard.keycode_options |= KEYCODE_OPTION_LATIN_LETTERS;
                }
            }
        }
        _ => keyboard.keycode_options = DEFAULT_KEYCODE_OPTIONS,
    }
}

// Public functions

/// Initialize the keyboard subsystem (called by the video subsystem's init).
/// Translation of `SDL_InitKeyboard()`.
pub fn init_keyboard() -> Result<()> {
    let cb = hints::watch(hints::KEYCODE_OPTIONS, |c| {
        keycode_options_changed(c.new_value)
    })?;
    keyboard().hint_callback = Some(cb);
    Ok(())
}

/// Whether a HID device with these properties is a real keyboard.
/// Translation of `SDL_IsKeyboard()`.
pub fn is_keyboard(_vendor: u16, _product: u16, num_keys: i32) -> bool {
    const REAL_KEYBOARD_KEY_COUNT: i32 = 50;

    if num_keys > 0 && num_keys < REAL_KEYBOARD_KEY_COUNT {
        return false;
    }

    // Eventually we'll have a blacklist of devices that enumerate as keyboards but aren't really
    true
}

/// Register a keyboard device and post `KEYBOARD_ADDED`.
/// Translation of `SDL_AddKeyboard()`.
pub fn add_keyboard(keyboard_id: KeyboardID, name: Option<&str>) {
    {
        let mut keyboard = keyboard();
        if keyboard.keyboards.contains(&keyboard_id) {
            // We already know about this keyboard
            return;
        }

        debug_assert!(keyboard_id != 0);
        keyboard.keyboards.push(keyboard_id);
        keyboard
            .keyboard_names
            .push((keyboard_id, name.unwrap_or("Keyboard").to_owned()));
    }

    let _ = queue::push(Event::KeyboardDevice(KeyboardDeviceEvent {
        event_type: EventType::KEYBOARD_ADDED,
        timestamp: Duration::ZERO,
        which: keyboard_id,
    }));
}

/// Unregister a keyboard device and post `KEYBOARD_REMOVED`.
/// Translation of `SDL_RemoveKeyboard()`.
pub fn remove_keyboard(keyboard_id: KeyboardID) {
    let quitting = {
        let mut keyboard = keyboard();
        let Some(index) = keyboard.keyboards.iter().position(|&id| id == keyboard_id) else {
            // We don't know about this keyboard
            return;
        };
        keyboard.keyboards.remove(index);
        keyboard.quitting
    };

    if !quitting {
        let _ = queue::push(Event::KeyboardDevice(KeyboardDeviceEvent {
            event_type: EventType::KEYBOARD_REMOVED,
            timestamp: Duration::ZERO,
            which: keyboard_id,
        }));
    }
}

/// Whether a keyboard is currently connected. Translation of `SDL_HasKeyboard()`.
pub fn has_keyboard() -> bool {
    !keyboard().keyboards.is_empty()
}

/// The connected keyboard ids. Translation of `SDL_GetKeyboards()`.
pub fn keyboards() -> Vec<KeyboardID> {
    keyboard().keyboards.clone()
}

/// The name of a keyboard. Translation of `SDL_GetKeyboardNameForID()`.
pub fn keyboard_name(instance_id: KeyboardID) -> Result<String> {
    match instance_id {
        GLOBAL_KEYBOARD_ID => Ok("Keyboard".to_owned()),
        _ => keyboard()
            .keyboard_names
            .iter()
            .find(|(id, _)| *id == instance_id)
            .map(|(_, name)| name.clone())
            .ok_or_else(|| err!("Keyboard {instance_id} not found")),
    }
}

/// Release every pressed key. Translation of `SDL_ResetKeyboard()`.
pub fn reset_keyboard() {
    let pressed: Vec<Scancode> = {
        let keyboard = keyboard();
        (0..Scancode::COUNT as u16)
            .map(Scancode)
            .filter(|s| keyboard.keystate[s.0 as usize])
            .collect()
    };
    for scancode in pressed {
        send_keyboard_key(Duration::ZERO, GLOBAL_KEYBOARD_ID, 0, scancode, false);
    }
}

/// Run `f` with the keymap that applies given the current keycode options.
/// Translation of `SDL_GetCurrentKeymap()`.
///
/// This may pass `None` even when a keymap is bound, depending on the
/// current keyboard mapping options. Set `ignore_options` to always get the
/// keymap that is actually bound.
pub(crate) fn with_current_keymap<R>(
    ignore_options: bool,
    f: impl FnOnce(Option<&Keymap>) -> R,
) -> R {
    let keyboard = keyboard();
    f(current_keymap(&keyboard, ignore_options))
}

fn current_keymap(keyboard: &Keyboard, ignore_options: bool) -> Option<&Keymap> {
    let keymap = keyboard.keymap.as_ref();

    if !ignore_options {
        if keymap.is_some_and(|k| k.thai_keyboard) {
            // Thai keyboards are QWERTY plus Thai characters, use the default QWERTY keymap
            return None;
        }

        if (keyboard.keycode_options & KEYCODE_OPTION_LATIN_LETTERS) != 0
            && keymap.is_some_and(|k| !k.latin_letters)
        {
            // We'll use the default QWERTY keymap
            return None;
        }
    }
    keymap
}

/// Bind a keymap (or `None` for the US QWERTY default), optionally posting
/// `KEYMAP_CHANGED`. Translation of `SDL_SetKeymap()`.
pub fn set_keymap(keymap: Option<Keymap>, send_event: bool) {
    {
        let mut keyboard = keyboard();
        keyboard.keymap = keymap;

        if let Some(keymap) = keyboard.keymap.as_mut() {
            if !keymap.layout_determined {
                keymap.layout_determined = true;

                // Detect French number row (all symbols)
                keymap.french_numbers = true;
                for i in Scancode::N1.0..=Scancode::N0.0 {
                    let is_digit = |k: Keycode| k.to_char().is_some_and(|c| c.is_ascii_digit());
                    if is_digit(keymap.keycode(Scancode(i), Keymod::NONE))
                        || !is_digit(keymap.keycode(Scancode(i), Keymod::SHIFT))
                    {
                        keymap.french_numbers = false;
                        break;
                    }
                }

                // Detect non-Latin keymap
                keymap.thai_keyboard = false;
                keymap.latin_letters = false;
                for i in Scancode::A.0..=Scancode::D.0 {
                    let key = keymap.keycode(Scancode(i), Keymod::NONE);
                    if key.0 <= 0xFF {
                        keymap.latin_letters = true;
                        break;
                    }
                    if (0x0E00..=0x0E7F).contains(&key.0) {
                        keymap.thai_keyboard = true;
                        break;
                    }
                }
            }
        }
    }

    if send_event {
        queue::send_keymap_changed_event();
    }
}

/// Translation of `GetNextReservedScancode()`.
fn get_next_reserved_scancode(keyboard: &mut Keyboard) -> Scancode {
    keyboard
        .keymap
        .get_or_insert_with(Keymap::new)
        .next_reserved_scancode()
}

/// Translation of `SetKeymapEntry()`.
fn set_keymap_entry(
    keyboard: &mut Keyboard,
    scancode: Scancode,
    modstate: Keymod,
    keycode: Keycode,
) {
    keyboard
        .keymap
        .get_or_insert_with(Keymap::new)
        .set_entry(scancode, modstate, keycode);
}

/// The window with keyboard focus, if any. Translation of `SDL_GetKeyboardFocus()`.
pub fn keyboard_focus() -> Option<WindowID> {
    keyboard().focus
}

/// Whether text input is active for a window. Translation of `SDL_TextInputActive()`.
pub fn text_input_active(window_id: WindowID) -> bool {
    window::window(window_id).is_some_and(|w| w.text_input_active)
}

/// Move keyboard focus to `window` (or nowhere), sending the focus lost/gained
/// window events. Translation of `SDL_SetKeyboardFocus()`.
pub fn set_keyboard_focus(window_id: Option<WindowID>) -> Result<()> {
    let video = window::video();

    if let Some(id) = window_id {
        match window::window(id) {
            Some(w) if !w.is_destroying => {}
            _ => return Err(err!("Invalid window")),
        }
    }

    let old_focus = keyboard().focus;

    if old_focus.is_some() && window_id.is_none() {
        // We won't get anymore keyboard messages, so reset keyboard state
        reset_keyboard();
    }

    // See if the current window has lost focus
    if let Some(old) = old_focus {
        if window_id != Some(old) {
            window::send_window_event(old, EventType::WINDOW_FOCUS_LOST, 0, 0);

            // Ensures IME compositions are committed
            if text_input_active(old) {
                if let Some(video) = &video {
                    video.stop_text_input(old);
                }
            }
        }
    }

    if let Some(old) = old_focus {
        if window_id.is_none() {
            // Also leave mouse relative mode
            if super::mouse::relative_mode_enabled() {
                let _ = super::mouse::set_relative_mouse_mode(false);
                if let Some(focus) = window::window(old) {
                    if focus.flags.contains(WindowFlags::MINIMIZED) {
                        // We can't warp the mouse within minimized windows, so manually restore the position
                        let (mx, my) = super::mouse::position();
                        let x = focus.x as f32 + mx;
                        let y = focus.y as f32 + my;
                        let _ = super::mouse::warp_mouse_global(x, y);
                    }
                }
            }
        }
    }

    keyboard().focus = window_id;

    if let Some(new) = window_id {
        window::send_window_event(new, EventType::WINDOW_FOCUS_GAINED, 0, 0);

        if text_input_active(new) {
            if let Some(video) = &video {
                video.start_text_input(new);
            }
        }
    }

    let _ = super::mouse::update_relative_mouse_mode();

    Ok(())
}

/// Translation of `SDL_ConvertNumpadKeycode()`.
fn convert_numpad_keycode(keycode: Keycode, numlock: bool) -> Keycode {
    match keycode {
        Keycode::KP_DIVIDE => Keycode::SLASH,
        Keycode::KP_MULTIPLY => Keycode::ASTERISK,
        Keycode::KP_MINUS => Keycode::MINUS,
        Keycode::KP_PLUS => Keycode::PLUS,
        Keycode::KP_ENTER => Keycode::RETURN,
        Keycode::KP_1 => {
            if numlock {
                Keycode::N1
            } else {
                Keycode::END
            }
        }
        Keycode::KP_2 => {
            if numlock {
                Keycode::N2
            } else {
                Keycode::DOWN
            }
        }
        Keycode::KP_3 => {
            if numlock {
                Keycode::N3
            } else {
                Keycode::PAGEDOWN
            }
        }
        Keycode::KP_4 => {
            if numlock {
                Keycode::N4
            } else {
                Keycode::LEFT
            }
        }
        Keycode::KP_5 => {
            if numlock {
                Keycode::N5
            } else {
                Keycode::CLEAR
            }
        }
        Keycode::KP_6 => {
            if numlock {
                Keycode::N6
            } else {
                Keycode::RIGHT
            }
        }
        Keycode::KP_7 => {
            if numlock {
                Keycode::N7
            } else {
                Keycode::HOME
            }
        }
        Keycode::KP_8 => {
            if numlock {
                Keycode::N8
            } else {
                Keycode::UP
            }
        }
        Keycode::KP_9 => {
            if numlock {
                Keycode::N9
            } else {
                Keycode::PAGEUP
            }
        }
        Keycode::KP_0 => {
            if numlock {
                Keycode::N0
            } else {
                Keycode::INSERT
            }
        }
        Keycode::KP_PERIOD => {
            if numlock {
                Keycode::PERIOD
            } else {
                Keycode::DELETE
            }
        }
        Keycode::KP_EQUALS => Keycode::EQUALS,
        Keycode::KP_COMMA => Keycode::COMMA,
        Keycode::KP_EQUALSAS400 => Keycode::EQUALS,
        Keycode::KP_LEFTPAREN => Keycode::LEFTPAREN,
        Keycode::KP_RIGHTPAREN => Keycode::RIGHTPAREN,
        Keycode::KP_LEFTBRACE => Keycode::LEFTBRACE,
        Keycode::KP_RIGHTBRACE => Keycode::RIGHTBRACE,
        Keycode::KP_TAB => Keycode::TAB,
        Keycode::KP_BACKSPACE => Keycode::BACKSPACE,
        Keycode::KP_A => Keycode::A,
        Keycode::KP_B => Keycode::B,
        Keycode::KP_C => Keycode::C,
        Keycode::KP_D => Keycode::D,
        Keycode::KP_E => Keycode::E,
        Keycode::KP_F => Keycode::F,
        Keycode::KP_PERCENT => Keycode::PERCENT,
        Keycode::KP_LESS => Keycode::LESS,
        Keycode::KP_GREATER => Keycode::GREATER,
        Keycode::KP_AMPERSAND => Keycode::AMPERSAND,
        Keycode::KP_COLON => Keycode::COLON,
        Keycode::KP_HASH => Keycode::HASH,
        Keycode::KP_SPACE => Keycode::SPACE,
        Keycode::KP_AT => Keycode::AT,
        Keycode::KP_EXCLAM => Keycode::EXCLAIM,
        Keycode::KP_PLUSMINUS => Keycode::PLUSMINUS,
        _ => keycode,
    }
}

fn key_from_scancode_locked(
    keyboard: &Keyboard,
    scancode: Scancode,
    modstate: Keymod,
    key_event: bool,
) -> Keycode {
    if key_event {
        let keymap = current_keymap(keyboard, false);
        let numlock = modstate.intersects(Keymod::NUM);

        // We won't be applying any modifiers by default
        let mut modstate = Keymod::NONE;

        if (keyboard.keycode_options & KEYCODE_OPTION_FRENCH_NUMBERS) != 0
            && keymap.is_some_and(|k| k.french_numbers)
            && (scancode >= Scancode::N1 && scancode <= Scancode::N0)
        {
            // Add the shift state to generate a numeric keycode
            modstate |= Keymod::SHIFT;
        }

        let mut keycode = Keymap::keycode_in(keymap, scancode, modstate);

        if (keyboard.keycode_options & KEYCODE_OPTION_HIDE_NUMPAD) != 0 {
            keycode = convert_numpad_keycode(keycode, numlock);
        }
        return keycode;
    }

    Keymap::keycode_in(keyboard.keymap.as_ref(), scancode, modstate)
}

/// The key code for a scancode under the current keymap.
///
/// With `key_event` the keycode options (`SDL_HINT_KEYCODE_OPTIONS`) are
/// applied, as for the `key` field of a keyboard event.
/// Translation of `SDL_GetKeyFromScancode()`.
pub fn key_from_scancode(scancode: Scancode, modstate: Keymod, key_event: bool) -> Keycode {
    let keyboard = keyboard();
    key_from_scancode_locked(&keyboard, scancode, modstate, key_event)
}

/// The scancode and modifiers that produce a key code under the current keymap.
/// Translation of `SDL_GetScancodeFromKey()`.
pub fn scancode_from_key(key: Keycode) -> (Scancode, Keymod) {
    let keyboard = keyboard();
    Keymap::scancode_in(keyboard.keymap.as_ref(), key)
}

/// Translation of `SDL_SendKeyboardKeyInternal()`.
fn send_keyboard_key_internal(
    timestamp: Duration,
    flags: u8,
    keyboard_id: KeyboardID,
    rawcode: i32,
    scancode: Scancode,
    down: bool,
) -> bool {
    let mut posted = false;
    let mut keycode = Keycode::UNKNOWN;
    let mut repeat = false;
    let source = flags & KEYBOARD_SOURCE_MASK;

    // Figure out what type of event this is
    let ty = if down {
        EventType::KEY_DOWN
    } else {
        EventType::KEY_UP
    };

    let (event, focus, modstate) = {
        let mut keyboard = keyboard();

        if scancode > Scancode::UNKNOWN && scancode.is_valid() {
            let i = scancode.0 as usize;
            // Drop events that don't change state
            if down {
                if keyboard.keystate[i] {
                    if (keyboard.keysource[i] & source) == 0 {
                        keyboard.keysource[i] |= source;
                        return false;
                    }
                    repeat = true;
                }
                keyboard.keysource[i] |= source;
            } else {
                if !keyboard.keystate[i] {
                    return false;
                }
                keyboard.keysource[i] = 0;
            }

            // Update internal keyboard state
            keyboard.keystate[i] = down;

            keycode = key_from_scancode_locked(&keyboard, scancode, keyboard.modstate, true);
        } else if rawcode == 0 {
            // Nothing to do!
            return false;
        }

        if source == KEYBOARD_HARDWARE {
            keyboard.hardware_timestamp = timer::ticks_ms();
        } else if source == KEYBOARD_AUTORELEASE {
            keyboard.autorelease_pending = true;
        }

        // Update modifiers state if applicable
        if (flags & KEYBOARD_IGNOREMODIFIERS) == 0 && !repeat {
            let modifier = match keycode {
                Keycode::LCTRL => Keymod::LCTRL,
                Keycode::RCTRL => Keymod::RCTRL,
                Keycode::LSHIFT => Keymod::LSHIFT,
                Keycode::RSHIFT => Keymod::RSHIFT,
                Keycode::LALT => Keymod::LALT,
                Keycode::RALT => Keymod::RALT,
                Keycode::LGUI => Keymod::LGUI,
                Keycode::RGUI => Keymod::RGUI,
                Keycode::MODE => Keymod::MODE,
                _ => Keymod::NONE,
            };
            if ty == EventType::KEY_DOWN {
                match keycode {
                    Keycode::NUMLOCKCLEAR => keyboard.modstate ^= Keymod::NUM,
                    Keycode::CAPSLOCK => keyboard.modstate ^= Keymod::CAPS,
                    Keycode::SCROLLLOCK => keyboard.modstate ^= Keymod::SCROLL,
                    _ => keyboard.modstate |= modifier,
                }
            } else {
                keyboard.modstate &= !modifier;
            }
        }

        let event = Event::Key(KeyboardEvent {
            timestamp,
            window_id: keyboard.focus.unwrap_or(0),
            which: keyboard_id,
            scancode,
            key: keycode,
            modifiers: keyboard.modstate,
            raw: rawcode as u16,
            down,
            repeat,
        });
        (event, keyboard.focus, keyboard.modstate)
    };

    // Post the event, if desired
    if queue::event_enabled(ty) {
        posted = queue::push(event).unwrap_or(false);
    }

    /* If the keyboard is grabbed and the grabbed window is in full-screen,
    minimize the window when we receive Alt+Tab, unless the application
    has explicitly opted out of this behavior. */
    if keycode == Keycode::TAB && down && modstate.intersects(Keymod::ALT) {
        if let Some(focus) = focus {
            let flags = window::window(focus).map(|w| w.flags).unwrap_or_default();
            if flags.contains(WindowFlags::KEYBOARD_GRABBED)
                && flags.contains(WindowFlags::FULLSCREEN)
                && hints::get_bool(hints::ALLOW_ALT_TAB_WHILE_GRABBED, true)
            {
                /* We will temporarily forfeit our grab by minimizing our window,
                allowing the user to escape the application */
                if let Some(video) = window::video() {
                    video.minimize_window(focus);
                }
            }
        }
    }

    posted
}

/// Send a key press and release for a Unicode character (used by on-screen
/// keyboards and the like). Translation of `SDL_SendKeyboardUnicodeKey()`.
pub fn send_keyboard_unicode_key(timestamp: Duration, ch: char) {
    let mut ch = ch as u32;
    if ch == '\n' as u32 {
        ch = Keycode::RETURN.0;
    }

    let (scancode, modstate) = {
        let mut keyboard = keyboard();
        let (mut scancode, modstate) = Keymap::scancode_in(keyboard.keymap.as_ref(), Keycode(ch));

        // Make sure we have this keycode in our keymap
        if scancode == Scancode::UNKNOWN && ch < Keycode::SCANCODE_MASK {
            scancode = get_next_reserved_scancode(&mut keyboard);
            set_keymap_entry(&mut keyboard, scancode, modstate, Keycode(ch));
        }
        (scancode, modstate)
    };

    if modstate.intersects(Keymod::SHIFT) {
        // If the character uses shift, press shift down
        send_keyboard_key_internal(
            timestamp,
            KEYBOARD_VIRTUAL,
            GLOBAL_KEYBOARD_ID,
            0,
            Scancode::LSHIFT,
            true,
        );
    }

    // Send a keydown and keyup for the character
    send_keyboard_key_internal(
        timestamp,
        KEYBOARD_VIRTUAL,
        GLOBAL_KEYBOARD_ID,
        0,
        scancode,
        true,
    );
    send_keyboard_key_internal(
        timestamp,
        KEYBOARD_VIRTUAL,
        GLOBAL_KEYBOARD_ID,
        0,
        scancode,
        false,
    );

    if modstate.intersects(Keymod::SHIFT) {
        // If the character uses shift, release shift
        send_keyboard_key_internal(
            timestamp,
            KEYBOARD_VIRTUAL,
            GLOBAL_KEYBOARD_ID,
            0,
            Scancode::LSHIFT,
            false,
        );
    }
}

/// Report a hardware key press or release. `timestamp` zero means "now".
/// Returns `true` if an event was posted. Translation of `SDL_SendKeyboardKey()`.
pub fn send_keyboard_key(
    timestamp: Duration,
    keyboard_id: KeyboardID,
    rawcode: i32,
    scancode: Scancode,
    down: bool,
) -> bool {
    send_keyboard_key_internal(
        timestamp,
        KEYBOARD_HARDWARE,
        keyboard_id,
        rawcode,
        scancode,
        down,
    )
}

/// Report a key press whose keycode the backend already knows, teaching the
/// keymap along the way. Translation of `SDL_SendKeyboardKeyAndKeycode()`.
pub fn send_keyboard_key_and_keycode(
    timestamp: Duration,
    keyboard_id: KeyboardID,
    rawcode: i32,
    scancode: Scancode,
    keycode: Keycode,
    down: bool,
) -> bool {
    if down {
        // Make sure we have this keycode in our keymap
        let mut keyboard = keyboard();
        let modstate = keyboard.modstate;
        set_keymap_entry(&mut keyboard, scancode, modstate, keycode);
    }
    send_keyboard_key_internal(
        timestamp,
        KEYBOARD_HARDWARE,
        keyboard_id,
        rawcode,
        scancode,
        down,
    )
}

/// Report a key press that must not affect the modifier state.
/// Translation of `SDL_SendKeyboardKeyIgnoreModifiers()`.
pub fn send_keyboard_key_ignore_modifiers(
    timestamp: Duration,
    keyboard_id: KeyboardID,
    rawcode: i32,
    scancode: Scancode,
    down: bool,
) -> bool {
    send_keyboard_key_internal(
        timestamp,
        KEYBOARD_HARDWARE | KEYBOARD_IGNOREMODIFIERS,
        keyboard_id,
        rawcode,
        scancode,
        down,
    )
}

/// Press a key that is automatically released on the next event pump.
/// Translation of `SDL_SendKeyboardKeyAutoRelease()`.
pub fn send_keyboard_key_auto_release(timestamp: Duration, scancode: Scancode) -> bool {
    send_keyboard_key_internal(
        timestamp,
        KEYBOARD_AUTORELEASE,
        GLOBAL_KEYBOARD_ID,
        0,
        scancode,
        true,
    )
}

/// Translation of `SDL_ReleaseAutoReleaseKeys()`; called from the event pump.
pub(crate) fn release_auto_release_keys() {
    let pending: Option<Vec<Scancode>> = {
        let keyboard = keyboard();
        keyboard.autorelease_pending.then(|| {
            (0..Scancode::COUNT as u16)
                .map(Scancode)
                .filter(|s| keyboard.keysource[s.0 as usize] == KEYBOARD_AUTORELEASE)
                .collect()
        })
    };
    if let Some(pending) = pending {
        for scancode in pending {
            send_keyboard_key_internal(
                Duration::ZERO,
                KEYBOARD_AUTORELEASE,
                GLOBAL_KEYBOARD_ID,
                0,
                scancode,
                false,
            );
        }
        keyboard().autorelease_pending = false;
    }

    let mut keyboard = keyboard();
    if keyboard.hardware_timestamp != 0 {
        // Keep hardware keyboard "active" for 250 ms
        if timer::ticks_ms() >= keyboard.hardware_timestamp + 250 {
            keyboard.hardware_timestamp = 0;
        }
    }
}

/// Whether a hardware keyboard key is pressed, or was within the last 250ms.
/// Translation of `SDL_HardwareKeyboardKeyPressed()`.
pub fn hardware_keyboard_key_pressed() -> bool {
    let keyboard = keyboard();
    if keyboard
        .keysource
        .iter()
        .any(|&s| (s & KEYBOARD_HARDWARE) != 0)
    {
        return true;
    }
    keyboard.hardware_timestamp != 0
}

/// Deliver text input to the focused window (if text input is active there).
/// Translation of `SDL_SendKeyboardText()`.
pub fn send_keyboard_text(text: &str) {
    let Some(focus) = keyboard().focus else {
        return;
    };
    if !text_input_active(focus) {
        return;
    }

    if text.is_empty() {
        return;
    }

    // Don't post text events for unprintable characters
    if text.as_bytes()[0].is_ascii_control() {
        return;
    }

    // Post the event, if desired
    if queue::event_enabled(EventType::TEXT_INPUT) {
        if hints::get_bool("SDL2_COMPAT", false) {
            // Limit SDL_EVENT_TEXT_INPUT events to 32 bytes for SDL2 compatibility
            let mut pos = 0;
            while pos < text.len() {
                // SDL_utf8strlcpy: as many whole characters as fit in 31 bytes
                let mut end = pos;
                for (i, c) in text[pos..].char_indices() {
                    if i + c.len_utf8() > 31 {
                        break;
                    }
                    end = pos + i + c.len_utf8();
                }
                if end == pos {
                    break;
                }
                let trimmed_text = text[pos..end].to_owned();
                pos = end;
                let _ = queue::push(Event::TextInput(TextInputEvent {
                    timestamp: Duration::ZERO,
                    window_id: focus,
                    text: trimmed_text,
                }));
            }
        } else {
            let _ = queue::push(Event::TextInput(TextInputEvent {
                timestamp: Duration::ZERO,
                window_id: focus,
                text: text.to_owned(),
            }));
        }
    }
}

/// Deliver IME composition text to the focused window.
/// Translation of `SDL_SendEditingText()`.
pub fn send_editing_text(text: &str, start: i32, length: i32) {
    let Some(focus) = keyboard().focus else {
        return;
    };
    if !text_input_active(focus) {
        return;
    }

    // Post the event, if desired
    if queue::event_enabled(EventType::TEXT_EDITING) {
        let _ = queue::push(Event::TextEditing(TextEditingEvent {
            timestamp: Duration::ZERO,
            window_id: focus,
            text: text.to_owned(),
            start,
            length,
        }));
    }
}

/// Deliver IME candidate list to the focused window.
/// Translation of `SDL_SendEditingTextCandidates()`.
pub fn send_editing_text_candidates(
    candidates: &[String],
    selected_candidate: i32,
    horizontal: bool,
) {
    let Some(focus) = keyboard().focus else {
        return;
    };
    if !text_input_active(focus) {
        return;
    }

    // Post the event, if desired
    if queue::event_enabled(EventType::TEXT_EDITING_CANDIDATES) {
        let event = if !candidates.is_empty() {
            TextEditingCandidatesEvent {
                timestamp: Duration::ZERO,
                window_id: focus,
                candidates: candidates.to_vec(),
                selected_candidate,
                horizontal,
            }
        } else {
            TextEditingCandidatesEvent {
                timestamp: Duration::ZERO,
                window_id: focus,
                candidates: Vec::new(),
                selected_candidate: -1,
                horizontal: false,
            }
        };
        let _ = queue::push(Event::TextEditingCandidates(event));
    }
}

/// Shut down the keyboard subsystem. Translation of `SDL_QuitKeyboard()`.
pub fn quit_keyboard() {
    let ids: Vec<KeyboardID> = {
        let mut keyboard = keyboard();
        keyboard.quitting = true;
        keyboard.keyboards.iter().rev().copied().collect()
    };
    for id in ids {
        remove_keyboard(id);
    }

    let mut keyboard = keyboard();
    keyboard.keyboards.clear();
    keyboard.keyboard_names.clear();
    keyboard.keymap = None;
    keyboard.hint_callback = None;
    keyboard.quitting = false;
}

/// A snapshot of the current key state, indexed by scancode.
/// Translation of `SDL_GetKeyboardState()`.
pub fn keyboard_state() -> [bool; Scancode::COUNT] {
    keyboard().keystate
}

/// Whether a key is currently pressed.
pub fn is_pressed(scancode: Scancode) -> bool {
    scancode.is_valid() && keyboard().keystate[scancode.0 as usize]
}

/// The current modifier state. Translation of `SDL_GetModState()`.
pub fn mod_state() -> Keymod {
    keyboard().modstate
}

/// Set the current modifier state. Translation of `SDL_SetModState()`.
pub fn set_mod_state(modstate: Keymod) {
    keyboard().modstate = modstate;
}

/// Set or clear modifiers. Translation of the internal `SDL_ToggleModState()`.
pub fn toggle_mod_state(modstate: Keymod, toggle: bool) {
    let mut keyboard = keyboard();
    if toggle {
        keyboard.modstate |= modstate;
    } else {
        keyboard.modstate &= !modstate;
    }
}

// ---------------------------------------------------------------------------
// Names (SDL_keymap.c)
// ---------------------------------------------------------------------------

/// Application overrides installed with [`Scancode::set_name`]
/// (upstream writes into `SDL_scancode_names[]` directly).
static SCANCODE_NAME_OVERRIDES: LazyLock<RwLock<HashMap<u16, Option<String>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

impl Scancode {
    /// Set a human-readable name for a scancode. Translation of `SDL_SetScancodeName()`.
    pub fn set_name(self, name: Option<&str>) -> Result<()> {
        if !self.is_valid() {
            return Err(Error::invalid_param("scancode"));
        }
        SCANCODE_NAME_OVERRIDES
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.0, name.map(str::to_owned));
        Ok(())
    }

    /// The human-readable name of a scancode (empty if it has none).
    /// Translation of `SDL_GetScancodeName()`.
    pub fn name(self) -> Cow<'static, str> {
        if !self.is_valid() {
            // SDL_InvalidParamError("scancode")
            return Cow::Borrowed("");
        }
        if let Some(over) = SCANCODE_NAME_OVERRIDES
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&self.0)
        {
            return Cow::Owned(over.clone().unwrap_or_default());
        }
        Cow::Borrowed(self.default_name())
    }

    /// The scancode with this human-readable name (case-insensitive).
    /// Translation of `SDL_GetScancodeFromName()`.
    pub fn from_name(name: &str) -> Result<Scancode> {
        if name.is_empty() {
            return Err(Error::invalid_param("name"));
        }

        let overrides = SCANCODE_NAME_OVERRIDES
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for i in 0..Scancode::COUNT as u16 {
            let candidate: Option<&str> = match overrides.get(&i) {
                Some(over) => over.as_deref(),
                None => scancode::SCANCODE_NAMES.get(i as usize).copied().flatten(),
            };
            if let Some(candidate) = candidate {
                if name.eq_ignore_ascii_case(candidate) {
                    return Ok(Scancode(i));
                }
            }
        }

        Err(Error::invalid_param("name"))
    }
}

impl Keycode {
    /// The human-readable name of a key (empty if it has none).
    /// Translation of `SDL_GetKeyName()`.
    pub fn name(self) -> String {
        const UPPERCASE: bool = true;

        if self.is_scancode_key() {
            return Scancode((self.0 & !Keycode::SCANCODE_MASK) as u16)
                .name()
                .into_owned();
        }

        if self.is_extended_key() {
            let idx = (self.0 & !Keycode::EXTENDED_MASK) as usize;
            if idx > 0 && (idx - 1) < scancode::EXTENDED_KEY_NAMES.len() {
                return scancode::EXTENDED_KEY_NAMES[idx - 1].to_owned();
            }
            // Key out of name index bounds.
            // SDL_InvalidParamError("key")
            return String::new();
        }

        match self {
            Keycode::RETURN => Scancode::RETURN.name().into_owned(),
            Keycode::ESCAPE => Scancode::ESCAPE.name().into_owned(),
            Keycode::BACKSPACE => Scancode::BACKSPACE.name().into_owned(),
            Keycode::TAB => Scancode::TAB.name().into_owned(),
            Keycode::SPACE => Scancode::SPACE.name().into_owned(),
            Keycode::DELETE => Scancode::DELETE.name().into_owned(),
            _ => {
                let mut key = self;
                if UPPERCASE {
                    // SDL_Keycode is defined as the unshifted key on the keyboard,
                    // but the key name is defined as the letter printed on that key,
                    // which is usually the shifted capital letter.
                    if key.0 > 0x7F || (key.0 >= 'a' as u32 && key.0 <= 'z' as u32) {
                        with_current_keymap(false, |keymap| {
                            let (scancode, modstate) = Keymap::scancode_in(keymap, key);
                            if scancode != Scancode::UNKNOWN && !modstate.intersects(Keymod::SHIFT)
                            {
                                let capital = Keymap::keycode_in(keymap, scancode, Keymod::SHIFT);
                                if capital.0 > 0x7F
                                    || (capital.0 >= 'A' as u32 && capital.0 <= 'Z' as u32)
                                {
                                    key = capital;
                                }
                            }
                        });
                    }
                }
                // SDL_UCS4ToUTF8
                char::from_u32(key.0).map(String::from).unwrap_or_default()
            }
        }
    }

    /// The key code for a human-readable name. Translation of `SDL_GetKeyFromName()`.
    pub fn from_name(name: &str) -> Keycode {
        const UPPERCASE: bool = true;

        // If it's a single UTF-8 character, then that's the keycode itself
        let mut chars = name.chars();
        let key = match (chars.next(), chars.next()) {
            (Some(c), None) => Keycode(c as u32),
            _ => Keycode::UNKNOWN,
        };

        if key != Keycode::UNKNOWN {
            if UPPERCASE {
                // SDL_Keycode is defined as the unshifted key on the keyboard,
                // but the key name is defined as the letter printed on that key,
                // which is usually the shifted capital letter.
                return with_current_keymap(false, |keymap| {
                    let (scancode, modstate) = Keymap::scancode_in(keymap, key);
                    if scancode != Scancode::UNKNOWN
                        && modstate.intersects(Keymod::SHIFT | Keymod::CAPS)
                    {
                        Keymap::keycode_in(keymap, scancode, Keymod::NONE)
                    } else {
                        key
                    }
                });
            }
            return key;
        }

        // Check the extended key names
        for (i, ext) in scancode::EXTENDED_KEY_NAMES.iter().enumerate() {
            if name.eq_ignore_ascii_case(ext) {
                return Keycode((i as u32 + 1) | Keycode::EXTENDED_MASK);
            }
        }

        let scancode = Scancode::from_name(name).unwrap_or(Scancode::UNKNOWN);
        key_from_scancode(scancode, Keymod::NONE, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::window::tests::with_video;
    use crate::events::window::VideoHooks;

    #[test]
    fn names() {
        assert_eq!(Scancode::A.name(), "A");
        assert_eq!(Scancode::N1.name(), "1");
        assert_eq!(Scancode::LGUI.name(), "Left GUI");
        assert_eq!(Scancode::UNKNOWN.name(), "");
        assert_eq!(Scancode::from_name("left gui").unwrap(), Scancode::LGUI);
        assert!(Scancode::from_name("").is_err());
        assert!(Scancode::from_name("no such key").is_err());
        assert_eq!(Scancode(600).name(), "");

        assert_eq!(Keycode::F1.name(), "F1");
        assert_eq!(Keycode::RETURN.name(), "Return");
        assert_eq!(Keycode::DELETE.name(), "Delete");
        assert_eq!(Keycode::LEFT_TAB.name(), "LeftTab");
        assert_eq!(Keycode::LHYPER.name(), "Left Hyper");
        assert_eq!(Keycode(0x20000050).name(), "");
        assert_eq!(Keycode::from_name("F1"), Keycode::F1);
        assert_eq!(Keycode::from_name("Return"), Keycode::RETURN);
        assert_eq!(Keycode::from_name("lefttab"), Keycode::LEFT_TAB);
        assert_eq!(Keycode::from_name("Bogus"), Keycode::UNKNOWN);
    }

    #[test]
    fn letter_names_are_uppercase_via_keymap() {
        with_video(&[], |_| {
            assert_eq!(Keycode::A.name(), "A");
            assert_eq!(Keycode::from_name("A"), Keycode::A);
            assert_eq!(Keycode::from_name("a"), Keycode::A);
            assert_eq!(Keycode::from_name("?"), Keycode::SLASH);
            assert_eq!(Keycode(0xE9).name(), "é");
            // A scancode name override is honored by both lookups.
            Scancode::N1.set_name(Some("Une")).unwrap();
            assert_eq!(Scancode::N1.name(), "Une");
            assert_eq!(Scancode::from_name("une").unwrap(), Scancode::N1);
            Scancode::N1.set_name(None).unwrap();
            assert_eq!(Scancode::N1.name(), "");
            SCANCODE_NAME_OVERRIDES.write().unwrap().clear();
            assert!(Scancode(600).set_name(Some("x")).is_err());
        });
    }

    #[test]
    fn key_events_and_modifiers() {
        with_video(&[1], |_| {
            init_keyboard().unwrap();
            set_keyboard_focus(Some(1)).unwrap();
            assert_eq!(keyboard_focus(), Some(1));
            queue::flush_events(EventType::FIRST, EventType::LAST);

            assert!(send_keyboard_key(
                Duration::ZERO,
                DEFAULT_KEYBOARD_ID,
                0x1e,
                Scancode::LSHIFT,
                true
            ));
            assert!(mod_state().contains(Keymod::LSHIFT));
            assert!(send_keyboard_key(
                Duration::ZERO,
                DEFAULT_KEYBOARD_ID,
                0x1e,
                Scancode::A,
                true
            ));
            assert!(is_pressed(Scancode::A));
            // Repeats are flagged, duplicate presses from a different source are dropped
            assert!(send_keyboard_key(
                Duration::ZERO,
                DEFAULT_KEYBOARD_ID,
                0x1e,
                Scancode::A,
                true
            ));
            assert!(!send_keyboard_key_auto_release(Duration::ZERO, Scancode::A));
            assert!(send_keyboard_key(
                Duration::ZERO,
                DEFAULT_KEYBOARD_ID,
                0x1e,
                Scancode::A,
                false
            ));
            assert!(!send_keyboard_key(
                Duration::ZERO,
                DEFAULT_KEYBOARD_ID,
                0x1e,
                Scancode::A,
                false
            ));
            assert!(hardware_keyboard_key_pressed());

            let keys: Vec<KeyboardEvent> =
                queue::get_events(EventType::KEY_DOWN, EventType::KEY_UP, 10)
                    .unwrap()
                    .into_iter()
                    .filter_map(|e| if let Event::Key(k) = e { Some(k) } else { None })
                    .collect();
            assert_eq!(keys.len(), 4);
            assert_eq!(keys[0].key, Keycode::LSHIFT);
            // The key field is the unshifted keycode even with shift held
            assert_eq!(keys[1].key, Keycode::A);
            assert!(keys[1].modifiers.contains(Keymod::LSHIFT));
            assert_eq!(keys[1].window_id, 1);
            assert_eq!(keys[1].which, DEFAULT_KEYBOARD_ID);
            assert_eq!(keys[1].raw, 0x1e);
            assert!(!keys[1].repeat && keys[2].repeat);
            assert!(!keys[3].down);

            // Lock keys toggle on press
            send_keyboard_key(
                Duration::ZERO,
                DEFAULT_KEYBOARD_ID,
                0,
                Scancode::CAPSLOCK,
                true,
            );
            send_keyboard_key(
                Duration::ZERO,
                DEFAULT_KEYBOARD_ID,
                0,
                Scancode::CAPSLOCK,
                false,
            );
            assert!(mod_state().contains(Keymod::CAPS));
            set_mod_state(Keymod::NONE);

            // Auto-release keys are released by the pump
            send_keyboard_key_auto_release(Duration::ZERO, Scancode::B);
            assert!(is_pressed(Scancode::B));
            release_auto_release_keys();
            assert!(!is_pressed(Scancode::B));

            // Losing focus resets the keyboard
            send_keyboard_key(Duration::ZERO, DEFAULT_KEYBOARD_ID, 0, Scancode::C, true);
            set_keyboard_focus(None).unwrap();
            assert!(!is_pressed(Scancode::C));
            assert!(queue::has_event(EventType::WINDOW_FOCUS_LOST));
            assert!(set_keyboard_focus(Some(42)).is_err());
            quit_keyboard();
        });
    }

    #[test]
    fn keycode_options_and_keymaps() {
        with_video(&[], |_| {
            init_keyboard().unwrap();
            assert_eq!(
                key_from_scancode(Scancode::KP_1, Keymod::NONE, true),
                Keycode::KP_1
            );
            hints::set(hints::KEYCODE_OPTIONS, "hide_numpad").unwrap();
            assert_eq!(
                key_from_scancode(Scancode::KP_1, Keymod::NONE, true),
                Keycode::END
            );
            assert_eq!(
                key_from_scancode(Scancode::KP_1, Keymod::NUM, true),
                Keycode::N1
            );
            assert_eq!(
                key_from_scancode(Scancode::KP_1, Keymod::NUM, false),
                Keycode::KP_1
            );
            hints::reset(hints::KEYCODE_OPTIONS);

            // A French-style layout: digits need shift; french_numbers makes key events report digits.
            let mut km = Keymap::new();
            for (i, (lower, upper)) in ["&1", "é2", "\"3", "'4", "(5", "-6", "è7", "_8", "ç9", "à0"]
                .iter()
                .map(|s| {
                    let mut c = s.chars();
                    (c.next().unwrap(), c.next().unwrap())
                })
                .enumerate()
            {
                let sc = Scancode(Scancode::N1.0 + i as u16);
                km.set_entry(sc, Keymod::NONE, Keycode(lower as u32));
                km.set_entry(sc, Keymod::SHIFT, Keycode(upper as u32));
            }
            km.set_entry(Scancode::A, Keymod::NONE, Keycode('q' as u32));
            set_keymap(Some(km), false);
            assert_eq!(
                key_from_scancode(Scancode::N1, Keymod::NONE, true),
                Keycode::N1
            );
            assert_eq!(
                key_from_scancode(Scancode::N1, Keymod::NONE, false),
                Keycode('&' as u32)
            );
            assert_eq!(
                key_from_scancode(Scancode::A, Keymod::NONE, true),
                Keycode('q' as u32)
            );
            assert_eq!(
                scancode_from_key(Keycode('q' as u32)),
                (Scancode::A, Keymod::NONE)
            );

            // A non-Latin layout falls back to QWERTY for key events unless latin_letters is off.
            let mut cyr = Keymap::new();
            for sc in [Scancode::A, Scancode::B, Scancode::C, Scancode::D] {
                cyr.set_entry(sc, Keymod::NONE, Keycode(0x444));
            }
            set_keymap(Some(cyr), true);
            assert!(queue::has_event(EventType::KEYMAP_CHANGED));
            assert_eq!(
                key_from_scancode(Scancode::A, Keymod::NONE, true),
                Keycode::A
            );
            hints::set(hints::KEYCODE_OPTIONS, "none").unwrap();
            assert_eq!(
                key_from_scancode(Scancode::A, Keymod::NONE, true),
                Keycode(0x444)
            );
            hints::reset(hints::KEYCODE_OPTIONS);
            set_keymap(None, false);

            // Unicode keys get reserved scancodes
            send_keyboard_unicode_key(Duration::ZERO, 'ß');
            let (sc, _) = scancode_from_key(Keycode('ß' as u32));
            assert_eq!(sc, Scancode::RESERVED);
            quit_keyboard();
        });
    }

    #[test]
    fn devices_and_text() {
        with_video(&[1], |video| {
            init_keyboard().unwrap();
            assert!(!has_keyboard());
            add_keyboard(5, None);
            add_keyboard(5, Some("dup"));
            add_keyboard(6, Some("Fancy"));
            assert_eq!(keyboards(), vec![5, 6]);
            assert_eq!(keyboard_name(5).unwrap(), "Keyboard");
            assert_eq!(keyboard_name(6).unwrap(), "Fancy");
            assert_eq!(keyboard_name(GLOBAL_KEYBOARD_ID).unwrap(), "Keyboard");
            assert_eq!(
                keyboard_name(7).unwrap_err().message(),
                "Keyboard 7 not found"
            );
            remove_keyboard(5);
            remove_keyboard(5);
            assert_eq!(keyboards(), vec![6]);
            let devs =
                queue::get_events(EventType::KEYBOARD_ADDED, EventType::KEYBOARD_REMOVED, 10)
                    .unwrap();
            assert_eq!(devs.len(), 3);
            assert!(is_keyboard(0, 0, 104) && !is_keyboard(0, 0, 3));

            set_keyboard_focus(Some(1)).unwrap();
            queue::flush_events(EventType::FIRST, EventType::LAST);
            // Text input requires an active text input state on the focus window.
            send_keyboard_text("hi");
            assert!(!queue::has_event(EventType::TEXT_INPUT));
            video.with_window(1, &mut |w| w.text_input_active = true);
            send_keyboard_text("\x08");
            assert!(!queue::has_event(EventType::TEXT_INPUT));
            send_keyboard_text("hi");
            send_editing_text("h", 0, 1);
            send_editing_text_candidates(&["a".into(), "b".into()], 1, true);
            send_editing_text_candidates(&[], 0, true);
            match queue::poll() {
                Some(Event::TextInput(t)) => assert_eq!((t.window_id, t.text.as_str()), (1, "hi")),
                other => panic!("{other:?}"),
            }
            assert!(matches!(
                queue::poll(),
                Some(Event::TextEditing(TextEditingEvent {
                    start: 0,
                    length: 1,
                    ..
                }))
            ));
            match queue::poll() {
                Some(Event::TextEditingCandidates(c)) => assert_eq!(
                    (c.candidates.len(), c.selected_candidate, c.horizontal),
                    (2, 1, true)
                ),
                other => panic!("{other:?}"),
            }
            match queue::poll() {
                Some(Event::TextEditingCandidates(c)) => assert_eq!(
                    (c.candidates.len(), c.selected_candidate, c.horizontal),
                    (0, -1, false)
                ),
                other => panic!("{other:?}"),
            }

            // SDL2 compatibility splits long text at 31 bytes on character boundaries.
            hints::set("SDL2_COMPAT", "1").unwrap();
            let long: String = "é".repeat(20); // 40 bytes
            send_keyboard_text(&long);
            hints::reset("SDL2_COMPAT");
            let parts: Vec<String> =
                queue::get_events(EventType::TEXT_INPUT, EventType::TEXT_INPUT, 10)
                    .unwrap()
                    .into_iter()
                    .filter_map(|e| {
                        if let Event::TextInput(t) = e {
                            Some(t.text)
                        } else {
                            None
                        }
                    })
                    .collect();
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[0].len(), 30);
            assert_eq!(parts.concat(), long);

            set_keyboard_focus(None).unwrap();
            quit_keyboard();
            assert!(!has_keyboard());
        });
    }
}
