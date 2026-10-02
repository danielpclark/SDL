// Rust translation of src/events/SDL_keymap.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Keymaps: the scancode ⇄ keycode mapping for a keyboard layout, plus the
//! default US QWERTY tables used when no layout is bound.

use std::collections::HashMap;

use super::{Keycode, Keymod, Scancode};

/// A keyboard layout's mapping between scancodes and keycodes.
/// Translation of `SDL_Keymap`.
///
/// Upstream keymaps are heap objects with an `auto_release` flag describing
/// who frees them; here ownership does that job: [`set_keymap`](super::set_keymap)
/// takes the keymap by value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    /// `(modstate << 16 | scancode)` → keycode
    scancode_to_keycode: HashMap<u32, Keycode>,
    /// keycode → `(modstate << 16 | scancode)`
    keycode_to_scancode: HashMap<Keycode, u32>,
    next_reserved_scancode: u16,
    pub(crate) layout_determined: bool,
    pub(crate) french_numbers: bool,
    pub(crate) latin_letters: bool,
    pub(crate) thai_keyboard: bool,
}

/// Translation of `NormalizeModifierStateForKeymap()`.
fn normalize_modifier_state_for_keymap(mut modstate: Keymod) -> Keymod {
    // The modifiers that affect the keymap are: SHIFT, CAPS, ALT, MODE, and LEVEL5
    modstate &= Keymod::SHIFT | Keymod::CAPS | Keymod::ALT | Keymod::MODE | Keymod::LEVEL5;

    // If either right or left Shift are set, set both in the output
    if modstate.intersects(Keymod::SHIFT) {
        modstate |= Keymod::SHIFT;
    }

    // If either right or left Alt are set, set both in the output
    if modstate.intersects(Keymod::ALT) {
        modstate |= Keymod::ALT;
    }

    modstate
}

fn map_key(modstate: Keymod, scancode: Scancode) -> u32 {
    ((modstate.0 as u32) << 16) | scancode.0 as u32
}

impl Keymap {
    /// An empty keymap (everything falls through to the defaults).
    /// Translation of `SDL_CreateKeymap()`.
    pub fn new() -> Self {
        Keymap::default()
    }

    /// Translation of `SDL_SetKeymapEntry()`.
    pub fn set_entry(&mut self, scancode: Scancode, modstate: Keymod, keycode: Keycode) {
        let modstate = normalize_modifier_state_for_keymap(modstate);
        let key = map_key(modstate, scancode);

        if let Some(&existing_keycode) = self.scancode_to_keycode.get(&key) {
            if existing_keycode == keycode {
                // We already have this mapping
                return;
            }
            // insert will replace the existing entry in the keymap atomically.
        }
        self.scancode_to_keycode.insert(key, keycode);

        let mut update_keycode = true;
        if let Some(&existing_value) = self.keycode_to_scancode.get(&keycode) {
            let existing_modstate = Keymod((existing_value >> 16) as u16);

            // Keep the simplest combination of scancode and modifiers to generate this keycode
            if existing_modstate.0 <= modstate.0 {
                update_keycode = false;
            }
        }
        if update_keycode {
            self.keycode_to_scancode.insert(keycode, key);
        }
    }

    /// The keycode a scancode produces under `modstate`, searching from the
    /// highest to lowest shift level and finally the default tables.
    /// Translation of `SDL_GetKeymapKeycode()` (`keymap != NULL` case).
    pub fn keycode(&self, scancode: Scancode, modstate: Keymod) -> Keycode {
        let normalized_modstate = normalize_modifier_state_for_keymap(modstate);
        let lookup = |m: Keymod| self.scancode_to_keycode.get(&map_key(m, scancode)).copied();

        // First, try the requested set of modifiers.
        if let Some(k) = lookup(normalized_modstate) {
            return k;
        }

        // If the requested set of modifiers was not found, search for the key from the highest to lowest modifier levels.
        if !normalized_modstate.is_empty() {
            let mut caps_mask = normalized_modstate & Keymod::CAPS;
            let passes = if caps_mask.is_empty() { 1 } else { 2 };
            for _ in 0..passes {
                // Shift level 5
                if normalized_modstate.intersects(Keymod::LEVEL5) {
                    let shifted_modstate = Keymod::LEVEL5 | caps_mask;
                    if shifted_modstate != normalized_modstate {
                        if let Some(k) = lookup(shifted_modstate) {
                            return k;
                        }
                    }
                }

                // Shift level 4 (Level 3 + Shift)
                if normalized_modstate.contains(Keymod::MODE | Keymod::SHIFT) {
                    let shifted_modstate = Keymod::MODE | Keymod::SHIFT | caps_mask;
                    if shifted_modstate != normalized_modstate {
                        if let Some(k) = lookup(shifted_modstate) {
                            return k;
                        }
                    }
                }

                // Shift level 3
                if normalized_modstate.intersects(Keymod::MODE) {
                    let shifted_modstate = Keymod::MODE | caps_mask;
                    if shifted_modstate != normalized_modstate {
                        if let Some(k) = lookup(shifted_modstate) {
                            return k;
                        }
                    }
                }

                // Shift level 2
                if normalized_modstate.intersects(Keymod::SHIFT) {
                    let shifted_modstate = Keymod::SHIFT | caps_mask;
                    if shifted_modstate != normalized_modstate {
                        if let Some(k) = lookup(shifted_modstate) {
                            return k;
                        }
                    }
                }

                // Shift Level 1 (unmodified)
                if let Some(k) = lookup(caps_mask) {
                    return k;
                }

                // Clear the capslock mask, if set.
                caps_mask = Keymod::NONE;
            }
        }

        default_key_from_scancode(scancode, modstate)
    }

    /// The scancode and modifiers that produce `keycode`.
    /// Translation of `SDL_GetKeymapScancode()` (`keymap != NULL` case).
    pub fn scancode(&self, keycode: Keycode) -> (Scancode, Keymod) {
        match self.keycode_to_scancode.get(&keycode) {
            Some(&value) => (
                Scancode((value & 0xFFFF) as u16),
                Keymod((value >> 16) as u16),
            ),
            None => default_scancode_from_key(keycode),
        }
    }

    /// Allocate a scancode in the reserved range for a key that has no
    /// physical scancode. Translation of `SDL_GetKeymapNextReservedScancode()`.
    pub fn next_reserved_scancode(&mut self) -> Scancode {
        let scancode = if self.next_reserved_scancode != 0
            && self.next_reserved_scancode < Scancode::RESERVED.0 + 100
        {
            self.next_reserved_scancode
        } else {
            Scancode::RESERVED.0
        };
        self.next_reserved_scancode = scancode + 1;
        Scancode(scancode)
    }

    /// Translation of `SDL_GetKeymapKeycode()` for an optional keymap.
    pub(crate) fn keycode_in(
        keymap: Option<&Keymap>,
        scancode: Scancode,
        modstate: Keymod,
    ) -> Keycode {
        match keymap {
            Some(k) => k.keycode(scancode, modstate),
            None => default_key_from_scancode(scancode, modstate),
        }
    }

    /// Translation of `SDL_GetKeymapScancode()` for an optional keymap.
    pub(crate) fn scancode_in(keymap: Option<&Keymap>, keycode: Keycode) -> (Scancode, Keymod) {
        match keymap {
            Some(k) => k.scancode(keycode),
            None => default_scancode_from_key(keycode),
        }
    }
}

/// Translation of `normal_default_symbols[]` (scancodes `1`..=`SLASH`).
static NORMAL_DEFAULT_SYMBOLS: [Keycode; 27] = [
    Keycode::N1,
    Keycode::N2,
    Keycode::N3,
    Keycode::N4,
    Keycode::N5,
    Keycode::N6,
    Keycode::N7,
    Keycode::N8,
    Keycode::N9,
    Keycode::N0,
    Keycode::RETURN,
    Keycode::ESCAPE,
    Keycode::BACKSPACE,
    Keycode::TAB,
    Keycode::SPACE,
    Keycode::MINUS,
    Keycode::EQUALS,
    Keycode::LEFTBRACKET,
    Keycode::RIGHTBRACKET,
    Keycode::BACKSLASH,
    Keycode::HASH,
    Keycode::SEMICOLON,
    Keycode::APOSTROPHE,
    Keycode::GRAVE,
    Keycode::COMMA,
    Keycode::PERIOD,
    Keycode::SLASH,
];

/// Translation of `shifted_default_symbols[]`.
static SHIFTED_DEFAULT_SYMBOLS: [Keycode; 27] = [
    Keycode::EXCLAIM,
    Keycode::AT,
    Keycode::HASH,
    Keycode::DOLLAR,
    Keycode::PERCENT,
    Keycode::CARET,
    Keycode::AMPERSAND,
    Keycode::ASTERISK,
    Keycode::LEFTPAREN,
    Keycode::RIGHTPAREN,
    Keycode::RETURN,
    Keycode::ESCAPE,
    Keycode::BACKSPACE,
    Keycode::TAB,
    Keycode::SPACE,
    Keycode::UNDERSCORE,
    Keycode::PLUS,
    Keycode::LEFTBRACE,
    Keycode::RIGHTBRACE,
    Keycode::PIPE,
    Keycode::HASH,
    Keycode::COLON,
    Keycode::DBLAPOSTROPHE,
    Keycode::TILDE,
    Keycode::LESS,
    Keycode::GREATER,
    Keycode::QUESTION,
];

/// Translation of `extended_default_symbols[]`.
static EXTENDED_DEFAULT_SYMBOLS: [(Keycode, Scancode); 5] = [
    (Keycode::LEFT_TAB, Scancode::TAB),
    (Keycode::MULTI_KEY_COMPOSE, Scancode::APPLICATION), // Sun keyboards
    (Keycode::LMETA, Scancode::LGUI),
    (Keycode::RMETA, Scancode::RGUI),
    (Keycode::RHYPER, Scancode::APPLICATION),
];

/// The US QWERTY default keycode for a scancode.
/// Translation of `SDL_GetDefaultKeyFromScancode()`.
pub fn default_key_from_scancode(scancode: Scancode, modstate: Keymod) -> Keycode {
    if !scancode.is_valid() {
        // SDL_InvalidParamError("scancode")
        return Keycode::UNKNOWN;
    }

    if scancode < Scancode::A {
        return Keycode::UNKNOWN;
    }

    if scancode < Scancode::N1 {
        let mut shifted = modstate.intersects(Keymod::SHIFT);
        if cfg!(any(target_os = "macos", target_os = "ios")) {
            // Apple maps to upper case for either shift or capslock inclusive
            if modstate.intersects(Keymod::CAPS) {
                shifted = true;
            }
        } else if modstate.intersects(Keymod::CAPS) {
            shifted = !shifted;
        }
        if modstate.intersects(Keymod::MODE) {
            return Keycode::UNKNOWN;
        }
        let offset = (scancode.0 - Scancode::A.0) as u32;
        return if !shifted {
            Keycode('a' as u32 + offset)
        } else {
            Keycode('A' as u32 + offset)
        };
    }

    if scancode < Scancode::CAPSLOCK {
        let shifted = modstate.intersects(Keymod::SHIFT);
        if modstate.intersects(Keymod::MODE) {
            return Keycode::UNKNOWN;
        }
        let index = (scancode.0 - Scancode::N1.0) as usize;
        return if !shifted {
            NORMAL_DEFAULT_SYMBOLS[index]
        } else {
            SHIFTED_DEFAULT_SYMBOLS[index]
        };
    }

    // These scancodes are not mapped to printable keycodes
    match scancode {
        Scancode::DELETE => Keycode::DELETE,
        Scancode::CAPSLOCK => Keycode::CAPSLOCK,
        Scancode::F1 => Keycode::F1,
        Scancode::F2 => Keycode::F2,
        Scancode::F3 => Keycode::F3,
        Scancode::F4 => Keycode::F4,
        Scancode::F5 => Keycode::F5,
        Scancode::F6 => Keycode::F6,
        Scancode::F7 => Keycode::F7,
        Scancode::F8 => Keycode::F8,
        Scancode::F9 => Keycode::F9,
        Scancode::F10 => Keycode::F10,
        Scancode::F11 => Keycode::F11,
        Scancode::F12 => Keycode::F12,
        Scancode::PRINTSCREEN => Keycode::PRINTSCREEN,
        Scancode::SCROLLLOCK => Keycode::SCROLLLOCK,
        Scancode::PAUSE => Keycode::PAUSE,
        Scancode::INSERT => Keycode::INSERT,
        Scancode::HOME => Keycode::HOME,
        Scancode::PAGEUP => Keycode::PAGEUP,
        Scancode::END => Keycode::END,
        Scancode::PAGEDOWN => Keycode::PAGEDOWN,
        Scancode::RIGHT => Keycode::RIGHT,
        Scancode::LEFT => Keycode::LEFT,
        Scancode::DOWN => Keycode::DOWN,
        Scancode::UP => Keycode::UP,
        Scancode::NUMLOCKCLEAR => Keycode::NUMLOCKCLEAR,
        Scancode::KP_DIVIDE => Keycode::KP_DIVIDE,
        Scancode::KP_MULTIPLY => Keycode::KP_MULTIPLY,
        Scancode::KP_MINUS => Keycode::KP_MINUS,
        Scancode::KP_PLUS => Keycode::KP_PLUS,
        Scancode::KP_ENTER => Keycode::KP_ENTER,
        Scancode::KP_1 => Keycode::KP_1,
        Scancode::KP_2 => Keycode::KP_2,
        Scancode::KP_3 => Keycode::KP_3,
        Scancode::KP_4 => Keycode::KP_4,
        Scancode::KP_5 => Keycode::KP_5,
        Scancode::KP_6 => Keycode::KP_6,
        Scancode::KP_7 => Keycode::KP_7,
        Scancode::KP_8 => Keycode::KP_8,
        Scancode::KP_9 => Keycode::KP_9,
        Scancode::KP_0 => Keycode::KP_0,
        Scancode::KP_PERIOD => Keycode::KP_PERIOD,
        Scancode::APPLICATION => Keycode::APPLICATION,
        Scancode::POWER => Keycode::POWER,
        Scancode::KP_EQUALS => Keycode::KP_EQUALS,
        Scancode::F13 => Keycode::F13,
        Scancode::F14 => Keycode::F14,
        Scancode::F15 => Keycode::F15,
        Scancode::F16 => Keycode::F16,
        Scancode::F17 => Keycode::F17,
        Scancode::F18 => Keycode::F18,
        Scancode::F19 => Keycode::F19,
        Scancode::F20 => Keycode::F20,
        Scancode::F21 => Keycode::F21,
        Scancode::F22 => Keycode::F22,
        Scancode::F23 => Keycode::F23,
        Scancode::F24 => Keycode::F24,
        Scancode::EXECUTE => Keycode::EXECUTE,
        Scancode::HELP => Keycode::HELP,
        Scancode::MENU => Keycode::MENU,
        Scancode::SELECT => Keycode::SELECT,
        Scancode::STOP => Keycode::STOP,
        Scancode::AGAIN => Keycode::AGAIN,
        Scancode::UNDO => Keycode::UNDO,
        Scancode::CUT => Keycode::CUT,
        Scancode::COPY => Keycode::COPY,
        Scancode::PASTE => Keycode::PASTE,
        Scancode::FIND => Keycode::FIND,
        Scancode::MUTE => Keycode::MUTE,
        Scancode::VOLUMEUP => Keycode::VOLUMEUP,
        Scancode::VOLUMEDOWN => Keycode::VOLUMEDOWN,
        Scancode::KP_COMMA => Keycode::KP_COMMA,
        Scancode::KP_EQUALSAS400 => Keycode::KP_EQUALSAS400,
        Scancode::ALTERASE => Keycode::ALTERASE,
        Scancode::SYSREQ => Keycode::SYSREQ,
        Scancode::CANCEL => Keycode::CANCEL,
        Scancode::CLEAR => Keycode::CLEAR,
        Scancode::PRIOR => Keycode::PRIOR,
        Scancode::RETURN2 => Keycode::RETURN2,
        Scancode::SEPARATOR => Keycode::SEPARATOR,
        Scancode::OUT => Keycode::OUT,
        Scancode::OPER => Keycode::OPER,
        Scancode::CLEARAGAIN => Keycode::CLEARAGAIN,
        Scancode::CRSEL => Keycode::CRSEL,
        Scancode::EXSEL => Keycode::EXSEL,
        Scancode::FRONT => Keycode::FRONT,
        Scancode::KP_00 => Keycode::KP_00,
        Scancode::KP_000 => Keycode::KP_000,
        Scancode::THOUSANDSSEPARATOR => Keycode::THOUSANDSSEPARATOR,
        Scancode::DECIMALSEPARATOR => Keycode::DECIMALSEPARATOR,
        Scancode::CURRENCYUNIT => Keycode::CURRENCYUNIT,
        Scancode::CURRENCYSUBUNIT => Keycode::CURRENCYSUBUNIT,
        Scancode::KP_LEFTPAREN => Keycode::KP_LEFTPAREN,
        Scancode::KP_RIGHTPAREN => Keycode::KP_RIGHTPAREN,
        Scancode::KP_LEFTBRACE => Keycode::KP_LEFTBRACE,
        Scancode::KP_RIGHTBRACE => Keycode::KP_RIGHTBRACE,
        Scancode::KP_TAB => Keycode::KP_TAB,
        Scancode::KP_BACKSPACE => Keycode::KP_BACKSPACE,
        Scancode::KP_A => Keycode::KP_A,
        Scancode::KP_B => Keycode::KP_B,
        Scancode::KP_C => Keycode::KP_C,
        Scancode::KP_D => Keycode::KP_D,
        Scancode::KP_E => Keycode::KP_E,
        Scancode::KP_F => Keycode::KP_F,
        Scancode::KP_XOR => Keycode::KP_XOR,
        Scancode::KP_POWER => Keycode::KP_POWER,
        Scancode::KP_PERCENT => Keycode::KP_PERCENT,
        Scancode::KP_LESS => Keycode::KP_LESS,
        Scancode::KP_GREATER => Keycode::KP_GREATER,
        Scancode::KP_AMPERSAND => Keycode::KP_AMPERSAND,
        Scancode::KP_DBLAMPERSAND => Keycode::KP_DBLAMPERSAND,
        Scancode::KP_VERTICALBAR => Keycode::KP_VERTICALBAR,
        Scancode::KP_DBLVERTICALBAR => Keycode::KP_DBLVERTICALBAR,
        Scancode::KP_COLON => Keycode::KP_COLON,
        Scancode::KP_HASH => Keycode::KP_HASH,
        Scancode::KP_SPACE => Keycode::KP_SPACE,
        Scancode::KP_AT => Keycode::KP_AT,
        Scancode::KP_EXCLAM => Keycode::KP_EXCLAM,
        Scancode::KP_MEMSTORE => Keycode::KP_MEMSTORE,
        Scancode::KP_MEMRECALL => Keycode::KP_MEMRECALL,
        Scancode::KP_MEMCLEAR => Keycode::KP_MEMCLEAR,
        Scancode::KP_MEMADD => Keycode::KP_MEMADD,
        Scancode::KP_MEMSUBTRACT => Keycode::KP_MEMSUBTRACT,
        Scancode::KP_MEMMULTIPLY => Keycode::KP_MEMMULTIPLY,
        Scancode::KP_MEMDIVIDE => Keycode::KP_MEMDIVIDE,
        Scancode::KP_PLUSMINUS => Keycode::KP_PLUSMINUS,
        Scancode::KP_CLEAR => Keycode::KP_CLEAR,
        Scancode::KP_CLEARENTRY => Keycode::KP_CLEARENTRY,
        Scancode::KP_BINARY => Keycode::KP_BINARY,
        Scancode::KP_OCTAL => Keycode::KP_OCTAL,
        Scancode::KP_DECIMAL => Keycode::KP_DECIMAL,
        Scancode::KP_HEXADECIMAL => Keycode::KP_HEXADECIMAL,
        Scancode::LCTRL => Keycode::LCTRL,
        Scancode::LSHIFT => Keycode::LSHIFT,
        Scancode::LALT => Keycode::LALT,
        Scancode::LGUI => Keycode::LGUI,
        Scancode::RCTRL => Keycode::RCTRL,
        Scancode::RSHIFT => Keycode::RSHIFT,
        Scancode::RALT => Keycode::RALT,
        Scancode::RGUI => Keycode::RGUI,
        Scancode::MODE => Keycode::MODE,
        Scancode::SLEEP => Keycode::SLEEP,
        Scancode::WAKE => Keycode::WAKE,
        Scancode::CHANNEL_INCREMENT => Keycode::CHANNEL_INCREMENT,
        Scancode::CHANNEL_DECREMENT => Keycode::CHANNEL_DECREMENT,
        Scancode::MEDIA_PLAY => Keycode::MEDIA_PLAY,
        Scancode::MEDIA_PAUSE => Keycode::MEDIA_PAUSE,
        Scancode::MEDIA_RECORD => Keycode::MEDIA_RECORD,
        Scancode::MEDIA_FAST_FORWARD => Keycode::MEDIA_FAST_FORWARD,
        Scancode::MEDIA_REWIND => Keycode::MEDIA_REWIND,
        Scancode::MEDIA_NEXT_TRACK => Keycode::MEDIA_NEXT_TRACK,
        Scancode::MEDIA_PREVIOUS_TRACK => Keycode::MEDIA_PREVIOUS_TRACK,
        Scancode::MEDIA_STOP => Keycode::MEDIA_STOP,
        Scancode::MEDIA_EJECT => Keycode::MEDIA_EJECT,
        Scancode::MEDIA_PLAY_PAUSE => Keycode::MEDIA_PLAY_PAUSE,
        Scancode::MEDIA_SELECT => Keycode::MEDIA_SELECT,
        Scancode::AC_NEW => Keycode::AC_NEW,
        Scancode::AC_OPEN => Keycode::AC_OPEN,
        Scancode::AC_CLOSE => Keycode::AC_CLOSE,
        Scancode::AC_EXIT => Keycode::AC_EXIT,
        Scancode::AC_SAVE => Keycode::AC_SAVE,
        Scancode::AC_PRINT => Keycode::AC_PRINT,
        Scancode::AC_PROPERTIES => Keycode::AC_PROPERTIES,
        Scancode::AC_SEARCH => Keycode::AC_SEARCH,
        Scancode::AC_HOME => Keycode::AC_HOME,
        Scancode::AC_BACK => Keycode::AC_BACK,
        Scancode::AC_FORWARD => Keycode::AC_FORWARD,
        Scancode::AC_STOP => Keycode::AC_STOP,
        Scancode::AC_REFRESH => Keycode::AC_REFRESH,
        Scancode::AC_BOOKMARKS => Keycode::AC_BOOKMARKS,
        Scancode::SOFTLEFT => Keycode::SOFTLEFT,
        Scancode::SOFTRIGHT => Keycode::SOFTRIGHT,
        Scancode::CALL => Keycode::CALL,
        Scancode::ENDCALL => Keycode::ENDCALL,
        _ => Keycode::UNKNOWN,
    }
}

/// The US QWERTY default scancode (and modifiers) for a keycode.
/// Translation of `SDL_GetDefaultScancodeFromKey()`.
pub fn default_scancode_from_key(key: Keycode) -> (Scancode, Keymod) {
    if key == Keycode::UNKNOWN {
        return (Scancode::UNKNOWN, Keymod::NONE);
    }

    if key.is_extended_key() {
        for &(keycode, scancode) in &EXTENDED_DEFAULT_SYMBOLS {
            if keycode == key {
                return (scancode, Keymod::NONE);
            }
        }
        return (Scancode::UNKNOWN, Keymod::NONE);
    }

    if key.is_scancode_key() {
        return (
            Scancode((key.0 & !Keycode::SCANCODE_MASK) as u16),
            Keymod::NONE,
        );
    }

    if key >= Keycode::A && key <= Keycode::Z {
        return (
            Scancode(Scancode::A.0 + (key.0 - Keycode::A.0) as u16),
            Keymod::NONE,
        );
    }

    if key.0 >= 'A' as u32 && key.0 <= 'Z' as u32 {
        return (
            Scancode(Scancode::A.0 + (key.0 - 'A' as u32) as u16),
            Keymod::SHIFT,
        );
    }

    for (i, &k) in NORMAL_DEFAULT_SYMBOLS.iter().enumerate() {
        if key == k {
            return (Scancode(Scancode::N1.0 + i as u16), Keymod::NONE);
        }
    }

    for (i, &k) in SHIFTED_DEFAULT_SYMBOLS.iter().enumerate() {
        if key == k {
            return (Scancode(Scancode::N1.0 + i as u16), Keymod::SHIFT);
        }
    }

    if key == Keycode::DELETE {
        return (Scancode::DELETE, Keymod::NONE);
    }

    (Scancode::UNKNOWN, Keymod::NONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        assert_eq!(
            default_key_from_scancode(Scancode::A, Keymod::NONE),
            Keycode::A
        );
        assert_eq!(
            default_key_from_scancode(Scancode::A, Keymod::LSHIFT),
            Keycode('A' as u32)
        );
        assert_eq!(
            default_key_from_scancode(Scancode::A, Keymod::CAPS),
            Keycode('A' as u32)
        );
        assert_eq!(
            default_key_from_scancode(Scancode::N1, Keymod::NONE),
            Keycode::N1
        );
        assert_eq!(
            default_key_from_scancode(Scancode::N1, Keymod::SHIFT),
            Keycode::EXCLAIM
        );
        assert_eq!(
            default_key_from_scancode(Scancode::SLASH, Keymod::SHIFT),
            Keycode::QUESTION
        );
        assert_eq!(
            default_key_from_scancode(Scancode::CAPSLOCK, Keymod::NONE),
            Keycode::CAPSLOCK
        );
        assert_eq!(
            default_key_from_scancode(Scancode::DELETE, Keymod::NONE),
            Keycode::DELETE
        );
        assert_eq!(
            default_key_from_scancode(Scancode::NONUSHASH, Keymod::NONE),
            Keycode::HASH
        );
        assert_eq!(
            default_key_from_scancode(Scancode::LANG1, Keymod::NONE),
            Keycode::UNKNOWN
        );
        assert_eq!(
            default_key_from_scancode(Scancode(600), Keymod::NONE),
            Keycode::UNKNOWN
        );

        assert_eq!(
            default_scancode_from_key(Keycode::QUESTION),
            (Scancode::SLASH, Keymod::SHIFT)
        );
        assert_eq!(
            default_scancode_from_key(Keycode('Q' as u32)),
            (Scancode::Q, Keymod::SHIFT)
        );
        assert_eq!(
            default_scancode_from_key(Keycode::F5),
            (Scancode::F5, Keymod::NONE)
        );
        assert_eq!(
            default_scancode_from_key(Keycode::LEFT_TAB),
            (Scancode::TAB, Keymod::NONE)
        );
        assert_eq!(
            default_scancode_from_key(Keycode::DELETE),
            (Scancode::DELETE, Keymod::NONE)
        );
        assert_eq!(
            default_scancode_from_key(Keycode(0xE9)),
            (Scancode::UNKNOWN, Keymod::NONE)
        );
    }

    #[test]
    fn keymap_levels_and_reverse() {
        let mut km = Keymap::new();
        // AZERTY-ish: unshifted 'a' key produces 'q', shift produces 'Q', AltGr produces '@'
        km.set_entry(Scancode::A, Keymod::NONE, Keycode('q' as u32));
        km.set_entry(Scancode::A, Keymod::LSHIFT, Keycode('Q' as u32));
        km.set_entry(Scancode::A, Keymod::MODE, Keycode('@' as u32));
        assert_eq!(km.keycode(Scancode::A, Keymod::NONE), Keycode('q' as u32));
        assert_eq!(km.keycode(Scancode::A, Keymod::RSHIFT), Keycode('Q' as u32));
        // Level 4 (Mode+Shift) falls back to level 3
        assert_eq!(
            km.keycode(Scancode::A, Keymod::MODE | Keymod::SHIFT),
            Keycode('@' as u32)
        );
        // Caps + Shift falls back through caps-less levels
        assert_eq!(
            km.keycode(Scancode::A, Keymod::CAPS | Keymod::SHIFT),
            Keycode('Q' as u32)
        );
        // Ctrl doesn't affect the keymap
        assert_eq!(km.keycode(Scancode::A, Keymod::LCTRL), Keycode('q' as u32));
        // Unknown scancodes fall back to the defaults
        assert_eq!(km.keycode(Scancode::B, Keymod::NONE), Keycode::B);

        // Reverse map keeps the simplest modifier combination
        km.set_entry(Scancode::B, Keymod::SHIFT, Keycode('q' as u32));
        assert_eq!(
            km.scancode(Keycode('q' as u32)),
            (Scancode::A, Keymod::NONE)
        );
        assert_eq!(
            km.scancode(Keycode('@' as u32)),
            (Scancode::A, Keymod::MODE)
        );
        assert_eq!(km.scancode(Keycode::F1), (Scancode::F1, Keymod::NONE));

        assert_eq!(km.next_reserved_scancode(), Scancode::RESERVED);
        assert_eq!(km.next_reserved_scancode(), Scancode(401));
        km.next_reserved_scancode = 499;
        assert_eq!(km.next_reserved_scancode(), Scancode(499));
        assert_eq!(km.next_reserved_scancode(), Scancode::RESERVED);
    }
}
