// Rust translation of src/events/scancodes_windows.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Windows scancode to SDL scancode mapping table
//! <https://learn.microsoft.com/windows/win32/inputdev/about-keyboard-input#scan-codes>
//!
//! The table is indexed by the scan code packed into one byte: the low
//! seven bits of the make code, with the high bit set for the `0xE0`
//! prefix (the `0xe0XX` entries are at `0x80 | XX`).

use super::keyboard::Scancode;

/// Translation of `windows_scancode_table`.
#[rustfmt::skip]
pub(crate) static WINDOWS_SCANCODE_TABLE: [Scancode; 256] = [
    /*0x00*/ Scancode::UNKNOWN,
    /*0x01*/ Scancode::ESCAPE,
    /*0x02*/ Scancode::N1,
    /*0x03*/ Scancode::N2,
    /*0x04*/ Scancode::N3,
    /*0x05*/ Scancode::N4,
    /*0x06*/ Scancode::N5,
    /*0x07*/ Scancode::N6,
    /*0x08*/ Scancode::N7,
    /*0x09*/ Scancode::N8,
    /*0x0a*/ Scancode::N9,
    /*0x0b*/ Scancode::N0,
    /*0x0c*/ Scancode::MINUS,
    /*0x0d*/ Scancode::EQUALS,
    /*0x0e*/ Scancode::BACKSPACE,
    /*0x0f*/ Scancode::TAB,
    /*0x10*/ Scancode::Q,
    /*0x11*/ Scancode::W,
    /*0x12*/ Scancode::E,
    /*0x13*/ Scancode::R,
    /*0x14*/ Scancode::T,
    /*0x15*/ Scancode::Y,
    /*0x16*/ Scancode::U,
    /*0x17*/ Scancode::I,
    /*0x18*/ Scancode::O,
    /*0x19*/ Scancode::P,
    /*0x1a*/ Scancode::LEFTBRACKET,
    /*0x1b*/ Scancode::RIGHTBRACKET,
    /*0x1c*/ Scancode::RETURN,
    /*0x1d*/ Scancode::LCTRL,
    /*0x1e*/ Scancode::A,
    /*0x1f*/ Scancode::S,
    /*0x20*/ Scancode::D,
    /*0x21*/ Scancode::F,
    /*0x22*/ Scancode::G,
    /*0x23*/ Scancode::H,
    /*0x24*/ Scancode::J,
    /*0x25*/ Scancode::K,
    /*0x26*/ Scancode::L,
    /*0x27*/ Scancode::SEMICOLON,
    /*0x28*/ Scancode::APOSTROPHE,
    /*0x29*/ Scancode::GRAVE,
    /*0x2a*/ Scancode::LSHIFT,
    /*0x2b*/ Scancode::BACKSLASH,
    /*0x2c*/ Scancode::Z,
    /*0x2d*/ Scancode::X,
    /*0x2e*/ Scancode::C,
    /*0x2f*/ Scancode::V,
    /*0x30*/ Scancode::B,
    /*0x31*/ Scancode::N,
    /*0x32*/ Scancode::M,
    /*0x33*/ Scancode::COMMA,
    /*0x34*/ Scancode::PERIOD,
    /*0x35*/ Scancode::SLASH,
    /*0x36*/ Scancode::RSHIFT,
    /*0x37*/ Scancode::KP_MULTIPLY,
    /*0x38*/ Scancode::LALT,
    /*0x39*/ Scancode::SPACE,
    /*0x3a*/ Scancode::CAPSLOCK,
    /*0x3b*/ Scancode::F1,
    /*0x3c*/ Scancode::F2,
    /*0x3d*/ Scancode::F3,
    /*0x3e*/ Scancode::F4,
    /*0x3f*/ Scancode::F5,
    /*0x40*/ Scancode::F6,
    /*0x41*/ Scancode::F7,
    /*0x42*/ Scancode::F8,
    /*0x43*/ Scancode::F9,
    /*0x44*/ Scancode::F10,
    /*0x45*/ Scancode::NUMLOCKCLEAR,
    /*0x46*/ Scancode::SCROLLLOCK,
    /*0x47*/ Scancode::KP_7,
    /*0x48*/ Scancode::KP_8,
    /*0x49*/ Scancode::KP_9,
    /*0x4a*/ Scancode::KP_MINUS,
    /*0x4b*/ Scancode::KP_4,
    /*0x4c*/ Scancode::KP_5,
    /*0x4d*/ Scancode::KP_6,
    /*0x4e*/ Scancode::KP_PLUS,
    /*0x4f*/ Scancode::KP_1,
    /*0x50*/ Scancode::KP_2,
    /*0x51*/ Scancode::KP_3,
    /*0x52*/ Scancode::KP_0,
    /*0x53*/ Scancode::KP_PERIOD,
    /*0x54*/ Scancode::UNKNOWN,
    /*0x55*/ Scancode::UNKNOWN,
    /*0x56*/ Scancode::NONUSBACKSLASH,
    /*0x57*/ Scancode::F11,
    /*0x58*/ Scancode::F12,
    /*0x59*/ Scancode::KP_EQUALS,
    /*0x5a*/ Scancode::UNKNOWN,
    /*0x5b*/ Scancode::UNKNOWN,
    /*0x5c*/ Scancode::INTERNATIONAL6,
    /*0x5d*/ Scancode::UNKNOWN,
    /*0x5e*/ Scancode::UNKNOWN,
    /*0x5f*/ Scancode::UNKNOWN,
    /*0x60*/ Scancode::UNKNOWN,
    /*0x61*/ Scancode::UNKNOWN,
    /*0x62*/ Scancode::UNKNOWN,
    /*0x63*/ Scancode::UNKNOWN,
    /*0x64*/ Scancode::F13,
    /*0x65*/ Scancode::F14,
    /*0x66*/ Scancode::F15,
    /*0x67*/ Scancode::F16,
    /*0x68*/ Scancode::F17,
    /*0x69*/ Scancode::F18,
    /*0x6a*/ Scancode::F19,
    /*0x6b*/ Scancode::F20,
    /*0x6c*/ Scancode::F21,
    /*0x6d*/ Scancode::F22,
    /*0x6e*/ Scancode::F23,
    /*0x6f*/ Scancode::UNKNOWN,
    /*0x70*/ Scancode::INTERNATIONAL2,
    /*0x71*/ Scancode::LANG2,
    /*0x72*/ Scancode::LANG1,
    /*0x73*/ Scancode::INTERNATIONAL1,
    /*0x74*/ Scancode::UNKNOWN,
    /*0x75*/ Scancode::UNKNOWN,
    /*0x76*/ Scancode::F24,
    /*0x77*/ Scancode::LANG4,
    /*0x78*/ Scancode::LANG3,
    /*0x79*/ Scancode::INTERNATIONAL4,
    /*0x7a*/ Scancode::UNKNOWN,
    /*0x7b*/ Scancode::INTERNATIONAL5,
    /*0x7c*/ Scancode::UNKNOWN,
    /*0x7d*/ Scancode::INTERNATIONAL3,
    /*0x7e*/ Scancode::KP_COMMA,
    /*0x7f*/ Scancode::UNKNOWN,
    /*0xe000*/ Scancode::UNKNOWN,
    /*0xe001*/ Scancode::UNKNOWN, // Fn+Esc on HP 250 G7 Notebook PC and HP Laptop 14-cm0xxx keyboards
    /*0xe002*/ Scancode::UNKNOWN,
    /*0xe003*/ Scancode::UNKNOWN,
    /*0xe004*/ Scancode::UNKNOWN,
    /*0xe005*/ Scancode::UNKNOWN, // "Messenger" on Natural Multimedia Keyboard
    /*0xe006*/ Scancode::UNKNOWN,
    /*0xe007*/ Scancode::AGAIN, // "Redo" on Natural Multimedia Keyboard
    /*0xe008*/ Scancode::UNDO, // "Undo" on Natural Multimedia Keyboard
    /*0xe009*/ Scancode::UNKNOWN,
    /*0xe00a*/ Scancode::PASTE,
    /*0xe00b*/ Scancode::UNKNOWN,
    /*0xe00c*/ Scancode::UNKNOWN,
    /*0xe00d*/ Scancode::UNKNOWN,
    /*0xe00e*/ Scancode::UNKNOWN,
    /*0xe00f*/ Scancode::UNKNOWN,
    /*0xe010*/ Scancode::MEDIA_PREVIOUS_TRACK,
    /*0xe011*/ Scancode::UNKNOWN,
    /*0xe012*/ Scancode::UNKNOWN,
    /*0xe013*/ Scancode::UNKNOWN,
    /*0xe014*/ Scancode::UNKNOWN,
    /*0xe015*/ Scancode::UNKNOWN,
    /*0xe016*/ Scancode::UNKNOWN, // "Log Off" on Natural Multimedia Keyboard
    /*0xe017*/ Scancode::CUT,
    /*0xe018*/ Scancode::COPY,
    /*0xe019*/ Scancode::MEDIA_NEXT_TRACK,
    /*0xe01a*/ Scancode::UNKNOWN,
    /*0xe01b*/ Scancode::UNKNOWN,
    /*0xe01c*/ Scancode::KP_ENTER,
    /*0xe01d*/ Scancode::RCTRL,
    /*0xe01e*/ Scancode::UNKNOWN,
    /*0xe01f*/ Scancode::UNKNOWN,
    /*0xe020*/ Scancode::MUTE,
    /*0xe021*/ Scancode::UNKNOWN, // LaunchApp2; "Calculator" on Natural Multimedia Keyboard and on Turbo multimedia keyboards
    /*0xe022*/ Scancode::MEDIA_PLAY_PAUSE,
    /*0xe023*/ Scancode::UNKNOWN, // "Spell" on Natural Multimedia Keyboard
    /*0xe024*/ Scancode::MEDIA_STOP,
    /*0xe025*/ Scancode::UNKNOWN,
    /*0xe026*/ Scancode::UNKNOWN,
    /*0xe027*/ Scancode::UNKNOWN,
    /*0xe028*/ Scancode::UNKNOWN,
    /*0xe029*/ Scancode::UNKNOWN,
    /*0xe02a*/ Scancode::UNKNOWN,
    /*0xe02b*/ Scancode::UNKNOWN, // Brightness up & brightness down on HP 250 G7 Notebook PC and HP Laptop 14-cm0xxx keyboards
    /*0xe02c*/ Scancode::MEDIA_EJECT,
    /*0xe02d*/ Scancode::UNKNOWN,
    /*0xe02e*/ Scancode::VOLUMEDOWN,
    /*0xe02f*/ Scancode::UNKNOWN,
    /*0xe030*/ Scancode::VOLUMEUP,
    /*0xe031*/ Scancode::UNKNOWN,
    /*0xe032*/ Scancode::AC_HOME,
    /*0xe033*/ Scancode::UNKNOWN,
    /*0xe034*/ Scancode::UNKNOWN,
    /*0xe035*/ Scancode::KP_DIVIDE,
    /*0xe036*/ Scancode::RSHIFT, // see https://mojira.dev/MC-311424
    /*0xe037*/ Scancode::PRINTSCREEN,
    /*0xe038*/ Scancode::RALT,
    /*0xe039*/ Scancode::UNKNOWN,
    /*0xe03a*/ Scancode::UNKNOWN,
    /*0xe03b*/ Scancode::HELP,
    /*0xe03c*/ Scancode::UNKNOWN, // "My Music" on Natural Multimedia Keyboard
    /*0xe03d*/ Scancode::UNKNOWN,
    /*0xe03e*/ Scancode::AC_NEW, // "New" on Natural Multimedia Keyboard
    /*0xe03f*/ Scancode::AC_OPEN, // "Open" on Natural Multimedia Keyboard
    /*0xe040*/ Scancode::AC_CLOSE, // "Close" on Natural Multimedia Keyboard
    /*0xe041*/ Scancode::UNKNOWN, // "Reply" on Natural Multimedia Keyboard
    /*0xe042*/ Scancode::UNKNOWN, // "Fwd" on Natural Multimedia Keyboard
    /*0xe043*/ Scancode::UNKNOWN, // "Send" on Natural Multimedia Keyboard
    /*0xe044*/ Scancode::UNKNOWN,
    /*0xe045*/ Scancode::NUMLOCKCLEAR,
    /*0xe046*/ Scancode::PAUSE,
    /*0xe047*/ Scancode::HOME,
    /*0xe048*/ Scancode::UP,
    /*0xe049*/ Scancode::PAGEUP,
    /*0xe04a*/ Scancode::UNKNOWN,
    /*0xe04b*/ Scancode::LEFT,
    /*0xe04c*/ Scancode::UNKNOWN, // "My Documents" on Natural Multimedia Keyboard
    /*0xe04d*/ Scancode::RIGHT,
    /*0xe04e*/ Scancode::UNKNOWN,
    /*0xe04f*/ Scancode::END,
    /*0xe050*/ Scancode::DOWN,
    /*0xe051*/ Scancode::PAGEDOWN,
    /*0xe052*/ Scancode::INSERT,
    /*0xe053*/ Scancode::DELETE,
    /*0xe054*/ Scancode::UNKNOWN,
    /*0xe055*/ Scancode::UNKNOWN,
    /*0xe056*/ Scancode::UNKNOWN,
    /*0xe057*/ Scancode::AC_SAVE, // "Save" on Natural Multimedia Keyboard
    /*0xe058*/ Scancode::AC_PRINT, // "Print" on Natural Multimedia Keyboard
    /*0xe059*/ Scancode::UNKNOWN,
    /*0xe05a*/ Scancode::UNKNOWN,
    /*0xe05b*/ Scancode::LGUI,
    /*0xe05c*/ Scancode::RGUI,
    /*0xe05d*/ Scancode::APPLICATION,
    /*0xe05e*/ Scancode::POWER,
    /*0xe05f*/ Scancode::SLEEP,
    /*0xe060*/ Scancode::UNKNOWN,
    /*0xe061*/ Scancode::UNKNOWN,
    /*0xe062*/ Scancode::UNKNOWN,
    /*0xe063*/ Scancode::WAKE, // "Wake Up" on Turbo multimedia keyboards
    /*0xe064*/ Scancode::UNKNOWN, // "My Pictures" on Natural Multimedia Keyboard
    /*0xe065*/ Scancode::AC_SEARCH,
    /*0xe066*/ Scancode::AC_BOOKMARKS,
    /*0xe067*/ Scancode::AC_REFRESH,
    /*0xe068*/ Scancode::AC_STOP,
    /*0xe069*/ Scancode::AC_FORWARD,
    /*0xe06a*/ Scancode::AC_BACK,
    /*0xe06b*/ Scancode::UNKNOWN, // LaunchApp1; "My computer" on Turbo multimedia keyboards
    /*0xe06c*/ Scancode::UNKNOWN, // LaunchMail; "Mail" on Natural Multimedia Keyboard and on Turbo multimedia keyboards
    /*0xe06d*/ Scancode::MEDIA_SELECT,
    /*0xe06e*/ Scancode::UNKNOWN,
    /*0xe06f*/ Scancode::UNKNOWN,
    /*0xe070*/ Scancode::UNKNOWN,
    /*0xe071*/ Scancode::UNKNOWN,
    /*0xe072*/ Scancode::UNKNOWN,
    /*0xe073*/ Scancode::UNKNOWN,
    /*0xe074*/ Scancode::UNKNOWN,
    /*0xe075*/ Scancode::UNKNOWN,
    /*0xe076*/ Scancode::UNKNOWN,
    /*0xe077*/ Scancode::UNKNOWN,
    /*0xe078*/ Scancode::UNKNOWN,
    /*0xe079*/ Scancode::UNKNOWN,
    /*0xe07a*/ Scancode::UNKNOWN,
    /*0xe07b*/ Scancode::UNKNOWN,
    /*0xe07c*/ Scancode::UNKNOWN,
    /*0xe07d*/ Scancode::UNKNOWN,
    /*0xe07e*/ Scancode::UNKNOWN,
    /*0xe07f*/ Scancode::UNKNOWN,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Entries checked against the literal values in scancodes_windows.h
    /// (and SDL_scancode.h for the numbers).
    #[test]
    fn table_matches_header() {
        assert_eq!(WINDOWS_SCANCODE_TABLE.len(), 256);
        let expect = [
            (0x00, 0),          // SDL_SCANCODE_UNKNOWN
            (0x01, 41),         // SDL_SCANCODE_ESCAPE
            (0x02, 30),         // SDL_SCANCODE_1
            (0x0b, 39),         // SDL_SCANCODE_0
            (0x1c, 40),         // SDL_SCANCODE_RETURN
            (0x1e, 4),          // SDL_SCANCODE_A
            (0x2a, 225),        // SDL_SCANCODE_LSHIFT
            (0x39, 44),         // SDL_SCANCODE_SPACE
            (0x45, 83),         // SDL_SCANCODE_NUMLOCKCLEAR
            (0x56, 100),        // SDL_SCANCODE_NONUSBACKSLASH
            (0x59, 103),        // SDL_SCANCODE_KP_EQUALS
            (0x76, 115),        // SDL_SCANCODE_F24
            (0x7e, 133),        // SDL_SCANCODE_KP_COMMA
            (0x80 | 0x1c, 88),  // 0xe01c SDL_SCANCODE_KP_ENTER
            (0x80 | 0x1d, 228), // 0xe01d SDL_SCANCODE_RCTRL
            (0x80 | 0x36, 229), // 0xe036 SDL_SCANCODE_RSHIFT
            (0x80 | 0x38, 230), // 0xe038 SDL_SCANCODE_RALT
            (0x80 | 0x46, 72),  // 0xe046 SDL_SCANCODE_PAUSE
            (0x80 | 0x48, 82),  // 0xe048 SDL_SCANCODE_UP
            (0x80 | 0x53, 76),  // 0xe053 SDL_SCANCODE_DELETE
            (0x80 | 0x5b, 227), // 0xe05b SDL_SCANCODE_LGUI
            (0x80 | 0x5d, 101), // 0xe05d SDL_SCANCODE_APPLICATION
            (0x80 | 0x7f, 0),   // 0xe07f SDL_SCANCODE_UNKNOWN
        ];
        for (index, scancode) in expect {
            assert_eq!(
                WINDOWS_SCANCODE_TABLE[index].0, scancode,
                "index {index:#x}"
            );
        }
        // (95 of the 256 entries are SDL_SCANCODE_UNKNOWN)
        let unknown = WINDOWS_SCANCODE_TABLE
            .iter()
            .filter(|&&s| s == Scancode::UNKNOWN)
            .count();
        assert_eq!(unknown, 95);
    }
}
