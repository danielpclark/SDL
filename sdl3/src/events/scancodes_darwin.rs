// Rust translation of src/events/scancodes_darwin.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Mac virtual key code to SDL scancode mapping table
//! Sources:
//! - Inside Macintosh: Text <http://developer.apple.com/documentation/mac/Text/Text-571.html>
//! - Apple USB keyboard driver source <http://darwinsource.opendarwin.org/10.4.6.ppc/IOHIDFamily-172.8/IOHIDFamily/Cosmo_USB2ADB.c>
//! - experimentation on various ADB and USB ISO keyboards and one ADB ANSI keyboard

use super::keyboard::Scancode;

/// Translation of `darwin_scancode_table`.
#[rustfmt::skip]
pub(crate) static DARWIN_SCANCODE_TABLE: [Scancode; 128] = [
    /*   0 */   Scancode::A,
    /*   1 */   Scancode::S,
    /*   2 */   Scancode::D,
    /*   3 */   Scancode::F,
    /*   4 */   Scancode::H,
    /*   5 */   Scancode::G,
    /*   6 */   Scancode::Z,
    /*   7 */   Scancode::X,
    /*   8 */   Scancode::C,
    /*   9 */   Scancode::V,
    /*  10 */   Scancode::NONUSBACKSLASH, // SDL_SCANCODE_NONUSBACKSLASH on ANSI and JIS keyboards (if this key would exist there), SDL_SCANCODE_GRAVE on ISO. (The USB keyboard driver actually translates these usage codes to different virtual key codes depending on whether the keyboard is ISO/ANSI/JIS. That's why you have to help it identify the keyboard type when you plug in a PC USB keyboard. It's a historical thing - ADB keyboards are wired this way.)
    /*  11 */   Scancode::B,
    /*  12 */   Scancode::Q,
    /*  13 */   Scancode::W,
    /*  14 */   Scancode::E,
    /*  15 */   Scancode::R,
    /*  16 */   Scancode::Y,
    /*  17 */   Scancode::T,
    /*  18 */   Scancode::N1,
    /*  19 */   Scancode::N2,
    /*  20 */   Scancode::N3,
    /*  21 */   Scancode::N4,
    /*  22 */   Scancode::N6,
    /*  23 */   Scancode::N5,
    /*  24 */   Scancode::EQUALS,
    /*  25 */   Scancode::N9,
    /*  26 */   Scancode::N7,
    /*  27 */   Scancode::MINUS,
    /*  28 */   Scancode::N8,
    /*  29 */   Scancode::N0,
    /*  30 */   Scancode::RIGHTBRACKET,
    /*  31 */   Scancode::O,
    /*  32 */   Scancode::U,
    /*  33 */   Scancode::LEFTBRACKET,
    /*  34 */   Scancode::I,
    /*  35 */   Scancode::P,
    /*  36 */   Scancode::RETURN,
    /*  37 */   Scancode::L,
    /*  38 */   Scancode::J,
    /*  39 */   Scancode::APOSTROPHE,
    /*  40 */   Scancode::K,
    /*  41 */   Scancode::SEMICOLON,
    /*  42 */   Scancode::BACKSLASH,
    /*  43 */   Scancode::COMMA,
    /*  44 */   Scancode::SLASH,
    /*  45 */   Scancode::N,
    /*  46 */   Scancode::M,
    /*  47 */   Scancode::PERIOD,
    /*  48 */   Scancode::TAB,
    /*  49 */   Scancode::SPACE,
    /*  50 */   Scancode::GRAVE, // SDL_SCANCODE_GRAVE on ANSI and JIS keyboards, SDL_SCANCODE_NONUSBACKSLASH on ISO (see comment about virtual key code 10 above)
    /*  51 */   Scancode::BACKSPACE,
    /*  52 */   Scancode::KP_ENTER, // keyboard enter on portables
    /*  53 */   Scancode::ESCAPE,
    /*  54 */   Scancode::RGUI,
    /*  55 */   Scancode::LGUI,
    /*  56 */   Scancode::LSHIFT,
    /*  57 */   Scancode::CAPSLOCK,
    /*  58 */   Scancode::LALT,
    /*  59 */   Scancode::LCTRL,
    /*  60 */   Scancode::RSHIFT,
    /*  61 */   Scancode::RALT,
    /*  62 */   Scancode::RCTRL,
    /*  63 */   Scancode::RGUI, // fn on portables, acts as a hardware-level modifier already, so we don't generate events for it, also XK_Meta_R
    /*  64 */   Scancode::F17,
    /*  65 */   Scancode::KP_PERIOD,
    /*  66 */   Scancode::UNKNOWN, // unknown (unused?)
    /*  67 */   Scancode::KP_MULTIPLY,
    /*  68 */   Scancode::UNKNOWN, // unknown (unused?)
    /*  69 */   Scancode::KP_PLUS,
    /*  70 */   Scancode::UNKNOWN, // unknown (unused?)
    /*  71 */   Scancode::NUMLOCKCLEAR,
    /*  72 */   Scancode::VOLUMEUP,
    /*  73 */   Scancode::VOLUMEDOWN,
    /*  74 */   Scancode::MUTE,
    /*  75 */   Scancode::KP_DIVIDE,
    /*  76 */   Scancode::KP_ENTER, // keypad enter on external keyboards, fn-return on portables
    /*  77 */   Scancode::UNKNOWN, // unknown (unused?)
    /*  78 */   Scancode::KP_MINUS,
    /*  79 */   Scancode::F18,
    /*  80 */   Scancode::F19,
    /*  81 */   Scancode::KP_EQUALS,
    /*  82 */   Scancode::KP_0,
    /*  83 */   Scancode::KP_1,
    /*  84 */   Scancode::KP_2,
    /*  85 */   Scancode::KP_3,
    /*  86 */   Scancode::KP_4,
    /*  87 */   Scancode::KP_5,
    /*  88 */   Scancode::KP_6,
    /*  89 */   Scancode::KP_7,
    /*  90 */   Scancode::UNKNOWN, // unknown (unused?)
    /*  91 */   Scancode::KP_8,
    /*  92 */   Scancode::KP_9,
    /*  93 */   Scancode::INTERNATIONAL3, // Cosmo_USB2ADB.c says "Yen (JIS)"
    /*  94 */   Scancode::INTERNATIONAL1, // Cosmo_USB2ADB.c says "Ro (JIS)"
    /*  95 */   Scancode::INTERNATIONAL6, // Cosmo_USB2ADB.c says ", JIS only"
    /*  96 */   Scancode::F5,
    /*  97 */   Scancode::F6,
    /*  98 */   Scancode::F7,
    /*  99 */   Scancode::F3,
    /* 100 */   Scancode::F8,
    /* 101 */   Scancode::F9,
    /* 102 */   Scancode::LANG2, // Cosmo_USB2ADB.c says "Eisu"
    /* 103 */   Scancode::F11,
    /* 104 */   Scancode::LANG1, // Cosmo_USB2ADB.c says "Kana"
    /* 105 */   Scancode::PRINTSCREEN, // On ADB keyboards, this key is labeled "F13/print screen". Problem: USB has different usage codes for these two functions. On Apple USB keyboards, the key is labeled "F13" and sends the F13 usage code (SDL_SCANCODE_F13). I decided to use SDL_SCANCODE_PRINTSCREEN here nevertheless since SDL applications are more likely to assume the presence of a print screen key than an F13 key.
    /* 106 */   Scancode::F16,
    /* 107 */   Scancode::SCROLLLOCK, // F14/scroll lock, see comment about F13/print screen above
    /* 108 */   Scancode::UNKNOWN, // unknown (unused?)
    /* 109 */   Scancode::F10,
    /* 110 */   Scancode::APPLICATION, // windows contextual menu key, fn-enter on portables
    /* 111 */   Scancode::F12,
    /* 112 */   Scancode::UNKNOWN, // unknown (unused?)
    /* 113 */   Scancode::PAUSE, // F15/pause, see comment about F13/print screen above
    /* 114 */   Scancode::INSERT, // the key is actually labeled "help" on Apple keyboards, and works as such in Mac OS, but it sends the "insert" usage code even on Apple USB keyboards
    /* 115 */   Scancode::HOME,
    /* 116 */   Scancode::PAGEUP,
    /* 117 */   Scancode::DELETE,
    /* 118 */   Scancode::F4,
    /* 119 */   Scancode::END,
    /* 120 */   Scancode::F2,
    /* 121 */   Scancode::PAGEDOWN,
    /* 122 */   Scancode::F1,
    /* 123 */   Scancode::LEFT,
    /* 124 */   Scancode::RIGHT,
    /* 125 */   Scancode::DOWN,
    /* 126 */   Scancode::UP,
    /* 127 */   Scancode::POWER,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Spot checks against the literal values of scancodes_darwin.h (the
    /// SDL_SCANCODE_* numbers of SDL_scancode.h).
    #[test]
    fn table_matches_upstream() {
        let raw: Vec<u32> = DARWIN_SCANCODE_TABLE.iter().map(|s| s.0 as u32).collect();
        assert_eq!(raw.len(), 128);
        // 0..=9: A S D F H G Z X C V
        assert_eq!(&raw[0..10], &[4, 22, 7, 9, 11, 10, 29, 27, 6, 25]);
        // 10: NONUSBACKSLASH, 50: GRAVE (swapped on ISO keyboards)
        assert_eq!((raw[10], raw[50]), (100, 53));
        // 18..=29: 1 2 3 4 6 5 = 9 7 - 8 0
        assert_eq!(
            &raw[18..30],
            &[30, 31, 32, 33, 35, 34, 46, 38, 36, 45, 37, 39]
        );
        // 36 RETURN, 48 TAB, 49 SPACE, 51 BACKSPACE, 53 ESCAPE
        assert_eq!(
            (raw[36], raw[48], raw[49], raw[51], raw[53]),
            (40, 43, 44, 42, 41)
        );
        // 54 RGUI, 55 LGUI, 56 LSHIFT, 57 CAPSLOCK, 58 LALT, 59 LCTRL,
        // 60 RSHIFT, 61 RALT, 62 RCTRL, 63 RGUI
        assert_eq!(
            &raw[54..64],
            &[231, 227, 225, 57, 226, 224, 229, 230, 228, 231]
        );
        // the unknown slots
        for i in [66, 68, 70, 77, 90, 108, 112] {
            assert_eq!(raw[i], 0, "slot {i}");
        }
        // 52 and 76: KP_ENTER; 93..=95: INTERNATIONAL3/1/6; 102/104: LANG2/LANG1
        assert_eq!((raw[52], raw[76]), (88, 88));
        assert_eq!(&raw[93..96], &[137, 135, 140]);
        assert_eq!((raw[102], raw[104]), (145, 144));
        // 105 PRINTSCREEN, 107 SCROLLLOCK, 113 PAUSE, 114 INSERT, 117 DELETE
        assert_eq!(
            (raw[105], raw[107], raw[113], raw[114], raw[117]),
            (70, 71, 72, 73, 76)
        );
        // 122 F1, 120 F2, 99 F3, 118 F4, 96 F5, 97 F6, 98 F7, 100 F8, 101 F9,
        // 109 F10, 103 F11, 111 F12
        let f: Vec<u32> = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111]
            .iter()
            .map(|&i| raw[i])
            .collect();
        assert_eq!(f, (58..=69).collect::<Vec<u32>>());
        // 123..=126 LEFT RIGHT DOWN UP, 127 POWER
        assert_eq!(&raw[123..128], &[80, 79, 81, 82, 102]);
    }
}
