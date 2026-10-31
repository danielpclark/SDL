// Rust translation of src/video/android/SDL_androidkeyboard.c and
// SDL_androidkeyboard.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The android video driver's keyboard: Android's key codes
//! (`KeyEvent.KEYCODE_*`, which `SDLActivity.handleKeyEvent()` passes on
//! through `onNativeKeyDown()` and `onNativeKeyUp()`) as SDL's scancodes,
//! and the screen keyboard: shown by the activity's text edit
//! (`SDLActivity.showTextInput()`, with the input type the text input
//! properties map to `InputType`'s flags), hidden by its
//! `COMMAND_TEXTEDIT_HIDE`, and shown again when the surface changes
//! ([`restore_screen_keyboard`]).

use crate::events::keyboard::{send_keyboard_key, Scancode, DEFAULT_KEYBOARD_ID};
use crate::events::WindowID;
use crate::properties::Properties;
use crate::video::textinput::{
    text_input_autocorrect, text_input_capitalization, text_input_multiline, text_input_type,
    Capitalization, TextInputType, PROP_TEXTINPUT_ANDROID_INPUTTYPE_NUMBER,
};
use std::time::Duration;

// `android.text.InputType`'s constants (those upstream uses)
const TYPE_CLASS_TEXT: i32 = 0x00000001;
const TYPE_CLASS_NUMBER: i32 = 0x00000002;

const TYPE_NUMBER_VARIATION_NORMAL: i32 = 0x00000000;
const TYPE_NUMBER_VARIATION_PASSWORD: i32 = 0x00000010;

const TYPE_TEXT_FLAG_CAP_CHARACTERS: i32 = 0x00001000;
const TYPE_TEXT_FLAG_CAP_WORDS: i32 = 0x00002000;
const TYPE_TEXT_FLAG_CAP_SENTENCES: i32 = 0x00004000;
const TYPE_TEXT_FLAG_AUTO_CORRECT: i32 = 0x00008000;
const TYPE_TEXT_FLAG_AUTO_COMPLETE: i32 = 0x00010000;
const TYPE_TEXT_FLAG_MULTI_LINE: i32 = 0x00020000;

const TYPE_TEXT_VARIATION_NORMAL: i32 = 0x00000000;
const TYPE_TEXT_VARIATION_EMAIL_ADDRESS: i32 = 0x00000020;
const TYPE_TEXT_VARIATION_PERSON_NAME: i32 = 0x00000060;
const TYPE_TEXT_VARIATION_PASSWORD: i32 = 0x00000080;
const TYPE_TEXT_VARIATION_VISIBLE_PASSWORD: i32 = 0x00000090;

/// `Android_Keycodes`: SDL's scancode of each of Android's key codes
/// (`AKEYCODE_*`, the index).
static ANDROID_KEYCODES: [Scancode; 280] = [
    Scancode::UNKNOWN,              // AKEYCODE_UNKNOWN
    Scancode::SOFTLEFT,             // AKEYCODE_SOFT_LEFT
    Scancode::SOFTRIGHT,            // AKEYCODE_SOFT_RIGHT
    Scancode::AC_HOME,              // AKEYCODE_HOME
    Scancode::AC_BACK,              // AKEYCODE_BACK
    Scancode::CALL,                 // AKEYCODE_CALL
    Scancode::ENDCALL,              // AKEYCODE_ENDCALL
    Scancode::N0,                   // AKEYCODE_0
    Scancode::N1,                   // AKEYCODE_1
    Scancode::N2,                   // AKEYCODE_2
    Scancode::N3,                   // AKEYCODE_3
    Scancode::N4,                   // AKEYCODE_4
    Scancode::N5,                   // AKEYCODE_5
    Scancode::N6,                   // AKEYCODE_6
    Scancode::N7,                   // AKEYCODE_7
    Scancode::N8,                   // AKEYCODE_8
    Scancode::N9,                   // AKEYCODE_9
    Scancode::UNKNOWN,              // AKEYCODE_STAR
    Scancode::UNKNOWN,              // AKEYCODE_POUND
    Scancode::UP,                   // AKEYCODE_DPAD_UP
    Scancode::DOWN,                 // AKEYCODE_DPAD_DOWN
    Scancode::LEFT,                 // AKEYCODE_DPAD_LEFT
    Scancode::RIGHT,                // AKEYCODE_DPAD_RIGHT
    Scancode::RETURN,               // AKEYCODE_DPAD_CENTER
    Scancode::VOLUMEUP,             // AKEYCODE_VOLUME_UP
    Scancode::VOLUMEDOWN,           // AKEYCODE_VOLUME_DOWN
    Scancode::POWER,                // AKEYCODE_POWER
    Scancode::UNKNOWN,              // AKEYCODE_CAMERA
    Scancode::CLEAR,                // AKEYCODE_CLEAR
    Scancode::A,                    // AKEYCODE_A
    Scancode::B,                    // AKEYCODE_B
    Scancode::C,                    // AKEYCODE_C
    Scancode::D,                    // AKEYCODE_D
    Scancode::E,                    // AKEYCODE_E
    Scancode::F,                    // AKEYCODE_F
    Scancode::G,                    // AKEYCODE_G
    Scancode::H,                    // AKEYCODE_H
    Scancode::I,                    // AKEYCODE_I
    Scancode::J,                    // AKEYCODE_J
    Scancode::K,                    // AKEYCODE_K
    Scancode::L,                    // AKEYCODE_L
    Scancode::M,                    // AKEYCODE_M
    Scancode::N,                    // AKEYCODE_N
    Scancode::O,                    // AKEYCODE_O
    Scancode::P,                    // AKEYCODE_P
    Scancode::Q,                    // AKEYCODE_Q
    Scancode::R,                    // AKEYCODE_R
    Scancode::S,                    // AKEYCODE_S
    Scancode::T,                    // AKEYCODE_T
    Scancode::U,                    // AKEYCODE_U
    Scancode::V,                    // AKEYCODE_V
    Scancode::W,                    // AKEYCODE_W
    Scancode::X,                    // AKEYCODE_X
    Scancode::Y,                    // AKEYCODE_Y
    Scancode::Z,                    // AKEYCODE_Z
    Scancode::COMMA,                // AKEYCODE_COMMA
    Scancode::PERIOD,               // AKEYCODE_PERIOD
    Scancode::LALT,                 // AKEYCODE_ALT_LEFT
    Scancode::RALT,                 // AKEYCODE_ALT_RIGHT
    Scancode::LSHIFT,               // AKEYCODE_SHIFT_LEFT
    Scancode::RSHIFT,               // AKEYCODE_SHIFT_RIGHT
    Scancode::TAB,                  // AKEYCODE_TAB
    Scancode::SPACE,                // AKEYCODE_SPACE
    Scancode::UNKNOWN,              // AKEYCODE_SYM
    Scancode::UNKNOWN,              // AKEYCODE_EXPLORER
    Scancode::UNKNOWN,              // AKEYCODE_ENVELOPE
    Scancode::RETURN,               // AKEYCODE_ENTER
    Scancode::BACKSPACE,            // AKEYCODE_DEL
    Scancode::GRAVE,                // AKEYCODE_GRAVE
    Scancode::MINUS,                // AKEYCODE_MINUS
    Scancode::EQUALS,               // AKEYCODE_EQUALS
    Scancode::LEFTBRACKET,          // AKEYCODE_LEFT_BRACKET
    Scancode::RIGHTBRACKET,         // AKEYCODE_RIGHT_BRACKET
    Scancode::BACKSLASH,            // AKEYCODE_BACKSLASH
    Scancode::SEMICOLON,            // AKEYCODE_SEMICOLON
    Scancode::APOSTROPHE,           // AKEYCODE_APOSTROPHE
    Scancode::SLASH,                // AKEYCODE_SLASH
    Scancode::UNKNOWN,              // AKEYCODE_AT
    Scancode::UNKNOWN,              // AKEYCODE_NUM
    Scancode::UNKNOWN,              // AKEYCODE_HEADSETHOOK
    Scancode::UNKNOWN,              // AKEYCODE_FOCUS
    Scancode::UNKNOWN,              // AKEYCODE_PLUS
    Scancode::MENU,                 // AKEYCODE_MENU
    Scancode::UNKNOWN,              // AKEYCODE_NOTIFICATION
    Scancode::AC_SEARCH,            // AKEYCODE_SEARCH
    Scancode::MEDIA_PLAY_PAUSE,     // AKEYCODE_MEDIA_PLAY_PAUSE
    Scancode::MEDIA_STOP,           // AKEYCODE_MEDIA_STOP
    Scancode::MEDIA_NEXT_TRACK,     // AKEYCODE_MEDIA_NEXT
    Scancode::MEDIA_PREVIOUS_TRACK, // AKEYCODE_MEDIA_PREVIOUS
    Scancode::MEDIA_REWIND,         // AKEYCODE_MEDIA_REWIND
    Scancode::MEDIA_FAST_FORWARD,   // AKEYCODE_MEDIA_FAST_FORWARD
    Scancode::MUTE,                 // AKEYCODE_MUTE
    Scancode::PAGEUP,               // AKEYCODE_PAGE_UP
    Scancode::PAGEDOWN,             // AKEYCODE_PAGE_DOWN
    Scancode::UNKNOWN,              // AKEYCODE_PICTSYMBOLS
    Scancode::UNKNOWN,              // AKEYCODE_SWITCH_CHARSET
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_A
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_B
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_C
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_X
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_Y
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_Z
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_L1
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_R1
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_L2
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_R2
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_THUMBL
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_THUMBR
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_START
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_SELECT
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_MODE
    Scancode::ESCAPE,               // AKEYCODE_ESCAPE
    Scancode::DELETE,               // AKEYCODE_FORWARD_DEL
    Scancode::LCTRL,                // AKEYCODE_CTRL_LEFT
    Scancode::RCTRL,                // AKEYCODE_CTRL_RIGHT
    Scancode::CAPSLOCK,             // AKEYCODE_CAPS_LOCK
    Scancode::SCROLLLOCK,           // AKEYCODE_SCROLL_LOCK
    Scancode::LGUI,                 // AKEYCODE_META_LEFT
    Scancode::RGUI,                 // AKEYCODE_META_RIGHT
    Scancode::UNKNOWN,              // AKEYCODE_FUNCTION
    Scancode::PRINTSCREEN,          // AKEYCODE_SYSRQ
    Scancode::PAUSE,                // AKEYCODE_BREAK
    Scancode::HOME,                 // AKEYCODE_MOVE_HOME
    Scancode::END,                  // AKEYCODE_MOVE_END
    Scancode::INSERT,               // AKEYCODE_INSERT
    Scancode::AC_FORWARD,           // AKEYCODE_FORWARD
    Scancode::MEDIA_PLAY,           // AKEYCODE_MEDIA_PLAY
    Scancode::MEDIA_PAUSE,          // AKEYCODE_MEDIA_PAUSE
    Scancode::UNKNOWN,              // AKEYCODE_MEDIA_CLOSE
    Scancode::MEDIA_EJECT,          // AKEYCODE_MEDIA_EJECT
    Scancode::MEDIA_RECORD,         // AKEYCODE_MEDIA_RECORD
    Scancode::F1,                   // AKEYCODE_F1
    Scancode::F2,                   // AKEYCODE_F2
    Scancode::F3,                   // AKEYCODE_F3
    Scancode::F4,                   // AKEYCODE_F4
    Scancode::F5,                   // AKEYCODE_F5
    Scancode::F6,                   // AKEYCODE_F6
    Scancode::F7,                   // AKEYCODE_F7
    Scancode::F8,                   // AKEYCODE_F8
    Scancode::F9,                   // AKEYCODE_F9
    Scancode::F10,                  // AKEYCODE_F10
    Scancode::F11,                  // AKEYCODE_F11
    Scancode::F12,                  // AKEYCODE_F12
    Scancode::NUMLOCKCLEAR,         // AKEYCODE_NUM_LOCK
    Scancode::KP_0,                 // AKEYCODE_NUMPAD_0
    Scancode::KP_1,                 // AKEYCODE_NUMPAD_1
    Scancode::KP_2,                 // AKEYCODE_NUMPAD_2
    Scancode::KP_3,                 // AKEYCODE_NUMPAD_3
    Scancode::KP_4,                 // AKEYCODE_NUMPAD_4
    Scancode::KP_5,                 // AKEYCODE_NUMPAD_5
    Scancode::KP_6,                 // AKEYCODE_NUMPAD_6
    Scancode::KP_7,                 // AKEYCODE_NUMPAD_7
    Scancode::KP_8,                 // AKEYCODE_NUMPAD_8
    Scancode::KP_9,                 // AKEYCODE_NUMPAD_9
    Scancode::KP_DIVIDE,            // AKEYCODE_NUMPAD_DIVIDE
    Scancode::KP_MULTIPLY,          // AKEYCODE_NUMPAD_MULTIPLY
    Scancode::KP_MINUS,             // AKEYCODE_NUMPAD_SUBTRACT
    Scancode::KP_PLUS,              // AKEYCODE_NUMPAD_ADD
    Scancode::KP_PERIOD,            // AKEYCODE_NUMPAD_DOT
    Scancode::KP_COMMA,             // AKEYCODE_NUMPAD_COMMA
    Scancode::KP_ENTER,             // AKEYCODE_NUMPAD_ENTER
    Scancode::KP_EQUALS,            // AKEYCODE_NUMPAD_EQUALS
    Scancode::KP_LEFTPAREN,         // AKEYCODE_NUMPAD_LEFT_PAREN
    Scancode::KP_RIGHTPAREN,        // AKEYCODE_NUMPAD_RIGHT_PAREN
    Scancode::UNKNOWN,              // AKEYCODE_VOLUME_MUTE
    Scancode::UNKNOWN,              // AKEYCODE_INFO
    Scancode::CHANNEL_INCREMENT,    // AKEYCODE_CHANNEL_UP
    Scancode::CHANNEL_DECREMENT,    // AKEYCODE_CHANNEL_DOWN
    Scancode::UNKNOWN,              // AKEYCODE_ZOOM_IN
    Scancode::UNKNOWN,              // AKEYCODE_ZOOM_OUT
    Scancode::UNKNOWN,              // AKEYCODE_TV
    Scancode::UNKNOWN,              // AKEYCODE_WINDOW
    Scancode::UNKNOWN,              // AKEYCODE_GUIDE
    Scancode::UNKNOWN,              // AKEYCODE_DVR
    Scancode::AC_BOOKMARKS,         // AKEYCODE_BOOKMARK
    Scancode::UNKNOWN,              // AKEYCODE_CAPTIONS
    Scancode::UNKNOWN,              // AKEYCODE_SETTINGS
    Scancode::UNKNOWN,              // AKEYCODE_TV_POWER
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT
    Scancode::UNKNOWN,              // AKEYCODE_STB_POWER
    Scancode::UNKNOWN,              // AKEYCODE_STB_INPUT
    Scancode::UNKNOWN,              // AKEYCODE_AVR_POWER
    Scancode::UNKNOWN,              // AKEYCODE_AVR_INPUT
    Scancode::UNKNOWN,              // AKEYCODE_PROG_RED
    Scancode::UNKNOWN,              // AKEYCODE_PROG_GREEN
    Scancode::UNKNOWN,              // AKEYCODE_PROG_YELLOW
    Scancode::UNKNOWN,              // AKEYCODE_PROG_BLUE
    Scancode::UNKNOWN,              // AKEYCODE_APP_SWITCH
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_1
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_2
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_3
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_4
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_5
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_6
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_7
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_8
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_9
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_10
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_11
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_12
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_13
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_14
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_15
    Scancode::UNKNOWN,              // AKEYCODE_BUTTON_16
    Scancode::UNKNOWN,              // AKEYCODE_LANGUAGE_SWITCH
    Scancode::UNKNOWN,              // AKEYCODE_MANNER_MODE
    Scancode::UNKNOWN,              // AKEYCODE_3D_MODE
    Scancode::UNKNOWN,              // AKEYCODE_CONTACTS
    Scancode::UNKNOWN,              // AKEYCODE_CALENDAR
    Scancode::UNKNOWN,              // AKEYCODE_MUSIC
    Scancode::UNKNOWN,              // AKEYCODE_CALCULATOR
    Scancode::LANG5,                // AKEYCODE_ZENKAKU_HANKAKU
    Scancode::UNKNOWN,              // AKEYCODE_EISU
    Scancode::INTERNATIONAL5,       // AKEYCODE_MUHENKAN
    Scancode::INTERNATIONAL4,       // AKEYCODE_HENKAN
    Scancode::LANG3,                // AKEYCODE_KATAKANA_HIRAGANA
    Scancode::INTERNATIONAL3,       // AKEYCODE_YEN
    Scancode::UNKNOWN,              // AKEYCODE_RO
    Scancode::UNKNOWN,              // AKEYCODE_KANA
    Scancode::UNKNOWN,              // AKEYCODE_ASSIST
    Scancode::UNKNOWN,              // AKEYCODE_BRIGHTNESS_DOWN
    Scancode::UNKNOWN,              // AKEYCODE_BRIGHTNESS_UP
    Scancode::UNKNOWN,              // AKEYCODE_MEDIA_AUDIO_TRACK
    Scancode::SLEEP,                // AKEYCODE_SLEEP
    Scancode::UNKNOWN,              // AKEYCODE_WAKEUP
    Scancode::UNKNOWN,              // AKEYCODE_PAIRING
    Scancode::UNKNOWN,              // AKEYCODE_MEDIA_TOP_MENU
    Scancode::UNKNOWN,              // AKEYCODE_11
    Scancode::UNKNOWN,              // AKEYCODE_12
    Scancode::UNKNOWN,              // AKEYCODE_LAST_CHANNEL
    Scancode::UNKNOWN,              // AKEYCODE_TV_DATA_SERVICE
    Scancode::UNKNOWN,              // AKEYCODE_VOICE_ASSIST
    Scancode::UNKNOWN,              // AKEYCODE_TV_RADIO_SERVICE
    Scancode::UNKNOWN,              // AKEYCODE_TV_TELETEXT
    Scancode::UNKNOWN,              // AKEYCODE_TV_NUMBER_ENTRY
    Scancode::UNKNOWN,              // AKEYCODE_TV_TERRESTRIAL_ANALOG
    Scancode::UNKNOWN,              // AKEYCODE_TV_TERRESTRIAL_DIGITAL
    Scancode::UNKNOWN,              // AKEYCODE_TV_SATELLITE
    Scancode::UNKNOWN,              // AKEYCODE_TV_SATELLITE_BS
    Scancode::UNKNOWN,              // AKEYCODE_TV_SATELLITE_CS
    Scancode::UNKNOWN,              // AKEYCODE_TV_SATELLITE_SERVICE
    Scancode::UNKNOWN,              // AKEYCODE_TV_NETWORK
    Scancode::UNKNOWN,              // AKEYCODE_TV_ANTENNA_CABLE
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT_HDMI_1
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT_HDMI_2
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT_HDMI_3
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT_HDMI_4
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT_COMPOSITE_1
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT_COMPOSITE_2
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT_COMPONENT_1
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT_COMPONENT_2
    Scancode::UNKNOWN,              // AKEYCODE_TV_INPUT_VGA_1
    Scancode::UNKNOWN,              // AKEYCODE_TV_AUDIO_DESCRIPTION
    Scancode::UNKNOWN,              // AKEYCODE_TV_AUDIO_DESCRIPTION_MIX_UP
    Scancode::UNKNOWN,              // AKEYCODE_TV_AUDIO_DESCRIPTION_MIX_DOWN
    Scancode::UNKNOWN,              // AKEYCODE_TV_ZOOM_MODE
    Scancode::UNKNOWN,              // AKEYCODE_TV_CONTENTS_MENU
    Scancode::UNKNOWN,              // AKEYCODE_TV_MEDIA_CONTEXT_MENU
    Scancode::UNKNOWN,              // AKEYCODE_TV_TIMER_PROGRAMMING
    Scancode::HELP,                 // AKEYCODE_HELP
    Scancode::UNKNOWN,              // AKEYCODE_NAVIGATE_PREVIOUS
    Scancode::UNKNOWN,              // AKEYCODE_NAVIGATE_NEXT
    Scancode::UNKNOWN,              // AKEYCODE_NAVIGATE_IN
    Scancode::UNKNOWN,              // AKEYCODE_NAVIGATE_OUT
    Scancode::UNKNOWN,              // AKEYCODE_STEM_PRIMARY
    Scancode::UNKNOWN,              // AKEYCODE_STEM_1
    Scancode::UNKNOWN,              // AKEYCODE_STEM_2
    Scancode::UNKNOWN,              // AKEYCODE_STEM_3
    Scancode::UNKNOWN,              // AKEYCODE_DPAD_UP_LEFT
    Scancode::UNKNOWN,              // AKEYCODE_DPAD_DOWN_LEFT
    Scancode::UNKNOWN,              // AKEYCODE_DPAD_UP_RIGHT
    Scancode::UNKNOWN,              // AKEYCODE_DPAD_DOWN_RIGHT
    Scancode::UNKNOWN,              // AKEYCODE_MEDIA_SKIP_FORWARD
    Scancode::UNKNOWN,              // AKEYCODE_MEDIA_SKIP_BACKWARD
    Scancode::UNKNOWN,              // AKEYCODE_MEDIA_STEP_FORWARD
    Scancode::UNKNOWN,              // AKEYCODE_MEDIA_STEP_BACKWARD
    Scancode::UNKNOWN,              // AKEYCODE_SOFT_SLEEP
    Scancode::CUT,                  // AKEYCODE_CUT
    Scancode::COPY,                 // AKEYCODE_COPY
    Scancode::PASTE,                // AKEYCODE_PASTE
];

/// `AKEYCODE_BUTTON_A`.
const AKEYCODE_BUTTON_A: i32 = 96;
/// `AKEYCODE_ESCAPE`.
const AKEYCODE_ESCAPE: i32 = 111;

/// Translation of `TranslateKeycode()`.
fn translate_keycode(keycode: i32) -> Scancode {
    let mut scancode = Scancode::UNKNOWN;

    // (upstream compares the int with the array's size: a negative key
    // code, converted, is out of the array too)
    if let Some(&s) = usize::try_from(keycode)
        .ok()
        .and_then(|i| ANDROID_KEYCODES.get(i))
    {
        scancode = s;
    }
    if scancode == Scancode::UNKNOWN {
        if (AKEYCODE_BUTTON_A..AKEYCODE_ESCAPE).contains(&keycode) {
            // Ignore game controller buttons
        } else {
            crate::core::android::ndk::log_write(
                crate::core::android::ndk::LogPriority::Info,
                "SDL",
                &format!("Unknown keycode {keycode}"),
            );
        }
    }
    scancode
}

/// Translation of `Android_OnKeyDown()`.
pub(crate) fn on_key_down(keycode: i32) {
    send_keyboard_key(
        Duration::ZERO,
        DEFAULT_KEYBOARD_ID,
        keycode,
        translate_keycode(keycode),
        true,
    );
}

/// Translation of `Android_OnKeyUp()`.
pub(crate) fn on_key_up(keycode: i32) {
    send_keyboard_key(
        Duration::ZERO,
        DEFAULT_KEYBOARD_ID,
        keycode,
        translate_keycode(keycode),
        false,
    );
}

/// Translation of `Android_HasScreenKeyboardSupport()`.
pub(crate) fn has_screen_keyboard_support() -> bool {
    true
}

/// The `InputType` of the text input properties `props`, as
/// `Android_ShowScreenKeyboard()` makes it: `PROP_TEXTINPUT_ANDROID_INPUTTYPE_NUMBER`
/// when they have it, else their type's class and variation, with the
/// flags of their capitalization, autocorrection and lines.
fn input_type(props: Option<&Properties>) -> i32 {
    if let Some(p) = props.filter(|p| p.contains(PROP_TEXTINPUT_ANDROID_INPUTTYPE_NUMBER)) {
        return p
            .get_number(PROP_TEXTINPUT_ANDROID_INPUTTYPE_NUMBER)
            .unwrap_or(0) as i32;
    }

    let mut input_type = match text_input_type(props) {
        TextInputType::Text => TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_NORMAL,
        TextInputType::TextName => TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_PERSON_NAME,
        TextInputType::TextEmail => TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_EMAIL_ADDRESS,
        TextInputType::TextUsername => TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_NORMAL,
        TextInputType::TextPasswordHidden => TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_PASSWORD,
        TextInputType::TextPasswordVisible => {
            TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
        }
        TextInputType::Number => TYPE_CLASS_NUMBER | TYPE_NUMBER_VARIATION_NORMAL,
        TextInputType::NumberPasswordHidden => TYPE_CLASS_NUMBER | TYPE_NUMBER_VARIATION_PASSWORD,
        TextInputType::NumberPasswordVisible => TYPE_CLASS_NUMBER | TYPE_NUMBER_VARIATION_NORMAL,
    };

    match text_input_capitalization(props) {
        Capitalization::None => {}
        Capitalization::Letters => input_type |= TYPE_TEXT_FLAG_CAP_CHARACTERS,
        Capitalization::Words => input_type |= TYPE_TEXT_FLAG_CAP_WORDS,
        Capitalization::Sentences => input_type |= TYPE_TEXT_FLAG_CAP_SENTENCES,
    }

    if text_input_autocorrect(props) {
        input_type |= TYPE_TEXT_FLAG_AUTO_CORRECT | TYPE_TEXT_FLAG_AUTO_COMPLETE;
    }

    if text_input_multiline(props) {
        input_type |= TYPE_TEXT_FLAG_MULTI_LINE;
    }
    input_type
}

/// Translation of `Android_ShowScreenKeyboard()`: the activity's text edit
/// over the window's text input area.
pub(crate) fn show_screen_keyboard(window: WindowID, props: Option<&Properties>) {
    let input_type = input_type(props);
    let rect = crate::video::core::with_window(window, |w| w.text_input_rect).unwrap_or_default();
    crate::core::android::show_screen_keyboard(input_type, &rect);
}

/// Translation of `Android_HideScreenKeyboard()`.
pub(crate) fn hide_screen_keyboard(_window: WindowID) {
    crate::core::android::hide_screen_keyboard();
}

/// The screen keyboard shown again (with the window's text input
/// properties) if it was shown: the surface changed. Translation of
/// `Android_RestoreScreenKeyboard()`.
pub(crate) fn restore_screen_keyboard(window: WindowID) {
    let shown = crate::video::core::with_device(|v| v.screen_keyboard_shown).unwrap_or(false);
    if shown {
        let props = crate::video::core::with_window(window, |w| w.text_input_props.clone())
            .ok()
            .flatten();
        show_screen_keyboard(window, props.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::textinput::{
        PROP_TEXTINPUT_AUTOCORRECT_BOOLEAN, PROP_TEXTINPUT_CAPITALIZATION_NUMBER,
        PROP_TEXTINPUT_MULTILINE_BOOLEAN, PROP_TEXTINPUT_TYPE_NUMBER,
    };

    #[test]
    fn keycodes() {
        assert_eq!(translate_keycode(4), Scancode::AC_BACK); // AKEYCODE_BACK
        assert_eq!(translate_keycode(7), Scancode::N0); // AKEYCODE_0
        assert_eq!(translate_keycode(29), Scancode::A); // AKEYCODE_A
        assert_eq!(translate_keycode(66), Scancode::RETURN); // AKEYCODE_ENTER
        assert_eq!(translate_keycode(96), Scancode::UNKNOWN); // AKEYCODE_BUTTON_A
        assert_eq!(translate_keycode(111), Scancode::ESCAPE); // AKEYCODE_ESCAPE
        assert_eq!(translate_keycode(279), Scancode::PASTE); // AKEYCODE_PASTE
        assert_eq!(translate_keycode(280), Scancode::UNKNOWN);
        assert_eq!(translate_keycode(-1), Scancode::UNKNOWN);
    }

    #[test]
    fn input_types() {
        let props = |f: &dyn Fn(&Properties)| {
            let p = Properties::new();
            f(&p);
            p
        };
        // (TEXT: sentences, autocorrected, multiline)
        assert_eq!(input_type(None), 0x3c001);
        let p = props(&|p| p.set(PROP_TEXTINPUT_TYPE_NUMBER, 1i64).unwrap());
        assert_eq!(input_type(Some(&p)), 0x3a061);
        let p = props(&|p| p.set(PROP_TEXTINPUT_TYPE_NUMBER, 7i64).unwrap());
        assert_eq!(input_type(Some(&p)), 0x38012);
        let p = props(&|p| {
            p.set(PROP_TEXTINPUT_CAPITALIZATION_NUMBER, 3i64).unwrap();
            p.set(PROP_TEXTINPUT_AUTOCORRECT_BOOLEAN, false).unwrap();
            p.set(PROP_TEXTINPUT_MULTILINE_BOOLEAN, false).unwrap();
        });
        assert_eq!(input_type(Some(&p)), 0x1001);
        let p = props(&|p| {
            p.set(PROP_TEXTINPUT_TYPE_NUMBER, 6i64).unwrap();
            p.set(PROP_TEXTINPUT_ANDROID_INPUTTYPE_NUMBER, 0x21i64)
                .unwrap();
        });
        assert_eq!(input_type(Some(&p)), 0x21);
    }
}
