// Rust translation of src/events/SDL_keysym_to_keycode.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Mapping X keysyms (as used by X11 and Wayland) to key codes.

use super::im_ks_to_ucs::key_sym_to_ucs4;
use super::keyboard::{Keycode, Keymap, Keymod, Scancode};
use super::keysym_to_scancode::get_scancode_from_key_sym;

// Extended key code mappings
const KEYSYM_TO_KEYCODE_TABLE: &[(u32, Keycode)] = &[
    (0xfe03, Keycode::MODE),              // XK_ISO_Level3_Shift
    (0xfe11, Keycode::LEVEL5_SHIFT),      // XK_ISO_Level5_Shift
    (0xfe20, Keycode::LEFT_TAB),          // XK_ISO_Left_Tab
    (0xff20, Keycode::MULTI_KEY_COMPOSE), // XK_Multi_key
    (0xffe7, Keycode::LMETA),             // XK_Meta_L
    (0xffe8, Keycode::RMETA),             // XK_Meta_R
    (0xffed, Keycode::LHYPER),            // XK_Hyper_L
    (0xffee, Keycode::RHYPER),            // XK_Hyper_R
];

/// The key code of an X keysym typed with an X key code and modifiers.
/// Translation of `SDL_GetKeyCodeFromKeySym()`.
pub(crate) fn get_key_code_from_key_sym(keysym: u32, keycode: u32, modifiers: Keymod) -> Keycode {
    let mut sdl_keycode = Keycode(key_sym_to_ucs4(keysym));

    if sdl_keycode.0 == 0 {
        for &(sym, code) in KEYSYM_TO_KEYCODE_TABLE {
            if keysym == sym {
                return code;
            }
        }
    }

    if sdl_keycode.0 == 0 {
        let scancode = get_scancode_from_key_sym(keysym, keycode);
        if scancode != Scancode::UNKNOWN {
            sdl_keycode = Keymap::keycode_in(None, scancode, modifiers);
        }
    }

    sdl_keycode
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycodes() {
        assert_eq!(
            get_key_code_from_key_sym(0x61, 38, Keymod::NONE),
            Keycode::A
        );
        assert_eq!(
            get_key_code_from_key_sym(0xfe03, 92, Keymod::NONE),
            Keycode::MODE
        );
        // XK_Escape has no character: through the scancode
        assert_eq!(
            get_key_code_from_key_sym(0xff1b, 9, Keymod::NONE),
            Keycode::ESCAPE
        );
    }
}
