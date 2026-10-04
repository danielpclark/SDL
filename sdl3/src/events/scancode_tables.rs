// Rust translation of src/events/SDL_scancode_tables.c, scancodes_darwin.h,
// scancodes_linux.h and scancodes_xfree86.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Tables mapping the key codes of several keyboard drivers (Mac virtual
//! key codes, Linux input key codes, XFree86 key codes and Xvnc) to
//! scancodes, for the backends that have to guess their keyboard's codes.

use super::keyboard::Scancode;

/// The tables. Translation of `SDL_ScancodeTable`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ScancodeTable {
    Darwin,
    Linux,
    Xfree86_1,
    Xfree86_2,
    Xvnc,
}

/// The table of `table`. Translation of `SDL_GetScancodeTable()` (the
/// slice carries `num_entries`).
pub(crate) fn get_scancode_table(table: ScancodeTable) -> &'static [Scancode] {
    match table {
        ScancodeTable::Darwin => DARWIN_SCANCODE_TABLE,
        ScancodeTable::Linux => LINUX_SCANCODE_TABLE,
        ScancodeTable::Xfree86_1 => XFREE86_SCANCODE_TABLE,
        ScancodeTable::Xfree86_2 => XFREE86_SCANCODE_TABLE2,
        ScancodeTable::Xvnc => XVNC_SCANCODE_TABLE,
    }
}

/// The scancode of `keycode` in `table` (unknown if out of range).
/// Translation of `SDL_GetScancodeFromTable()`.
pub(crate) fn get_scancode_from_table(table: ScancodeTable, keycode: i32) -> Scancode {
    let mut scancode = Scancode::UNKNOWN;
    let scancodes = get_scancode_table(table);

    if keycode >= 0 && (keycode as usize) < scancodes.len() {
        scancode = scancodes[keycode as usize];
    }
    scancode
}

#[rustfmt::skip]
static DARWIN_SCANCODE_TABLE: &[Scancode] = &[
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
    /*  10 */   Scancode::NONUSBACKSLASH, // Scancode::NONUSBACKSLASH on ANSI and JIS keyboards (if this key would exist there), Scancode::GRAVE on ISO. (The USB keyboard driver actually translates these usage codes to different virtual key codes depending on whether the keyboard is ISO/ANSI/JIS. That's why you have to help it identify the keyboard type when you plug in a PC USB keyboard. It's a historical thing - ADB keyboards are wired this way.)
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
    /*  50 */   Scancode::GRAVE, // Scancode::GRAVE on ANSI and JIS keyboards, Scancode::NONUSBACKSLASH on ISO (see comment about virtual key code 10 above)
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
    /* 105 */   Scancode::PRINTSCREEN, // On ADB keyboards, this key is labeled "F13/print screen". Problem: USB has different usage codes for these two functions. On Apple USB keyboards, the key is labeled "F13" and sends the F13 usage code (Scancode::F13). I decided to use Scancode::PRINTSCREEN here nevertheless since SDL applications are more likely to assume the presence of a print screen key than an F13 key.
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

#[rustfmt::skip]
static LINUX_SCANCODE_TABLE: &[Scancode] = &[
    /*   0, 0x000 */    Scancode::UNKNOWN,           // KEY_RESERVED
    /*   1, 0x001 */    Scancode::ESCAPE,            // KEY_ESC
    /*   2, 0x002 */    Scancode::N1,                 // KEY_1
    /*   3, 0x003 */    Scancode::N2,                 // KEY_2
    /*   4, 0x004 */    Scancode::N3,                 // KEY_3
    /*   5, 0x005 */    Scancode::N4,                 // KEY_4
    /*   6, 0x006 */    Scancode::N5,                 // KEY_5
    /*   7, 0x007 */    Scancode::N6,                 // KEY_6
    /*   8, 0x008 */    Scancode::N7,                 // KEY_7
    /*   9, 0x009 */    Scancode::N8,                 // KEY_8
    /*  10, 0x00a */    Scancode::N9,                 // KEY_9
    /*  11, 0x00b */    Scancode::N0,                 // KEY_0
    /*  12, 0x00c */    Scancode::MINUS,             // KEY_MINUS
    /*  13, 0x00d */    Scancode::EQUALS,            // KEY_EQUAL
    /*  14, 0x00e */    Scancode::BACKSPACE,         // KEY_BACKSPACE
    /*  15, 0x00f */    Scancode::TAB,               // KEY_TAB
    /*  16, 0x010 */    Scancode::Q,                 // KEY_Q
    /*  17, 0x011 */    Scancode::W,                 // KEY_W
    /*  18, 0x012 */    Scancode::E,                 // KEY_E
    /*  19, 0x013 */    Scancode::R,                 // KEY_R
    /*  20, 0x014 */    Scancode::T,                 // KEY_T
    /*  21, 0x015 */    Scancode::Y,                 // KEY_Y
    /*  22, 0x016 */    Scancode::U,                 // KEY_U
    /*  23, 0x017 */    Scancode::I,                 // KEY_I
    /*  24, 0x018 */    Scancode::O,                 // KEY_O
    /*  25, 0x019 */    Scancode::P,                 // KEY_P
    /*  26, 0x01a */    Scancode::LEFTBRACKET,       // KEY_LEFTBRACE
    /*  27, 0x01b */    Scancode::RIGHTBRACKET,      // KEY_RIGHTBRACE
    /*  28, 0x01c */    Scancode::RETURN,            // KEY_ENTER
    /*  29, 0x01d */    Scancode::LCTRL,             // KEY_LEFTCTRL
    /*  30, 0x01e */    Scancode::A,                 // KEY_A
    /*  31, 0x01f */    Scancode::S,                 // KEY_S
    /*  32, 0x020 */    Scancode::D,                 // KEY_D
    /*  33, 0x021 */    Scancode::F,                 // KEY_F
    /*  34, 0x022 */    Scancode::G,                 // KEY_G
    /*  35, 0x023 */    Scancode::H,                 // KEY_H
    /*  36, 0x024 */    Scancode::J,                 // KEY_J
    /*  37, 0x025 */    Scancode::K,                 // KEY_K
    /*  38, 0x026 */    Scancode::L,                 // KEY_L
    /*  39, 0x027 */    Scancode::SEMICOLON,         // KEY_SEMICOLON
    /*  40, 0x028 */    Scancode::APOSTROPHE,        // KEY_APOSTROPHE
    /*  41, 0x029 */    Scancode::GRAVE,             // KEY_GRAVE
    /*  42, 0x02a */    Scancode::LSHIFT,            // KEY_LEFTSHIFT
    /*  43, 0x02b */    Scancode::BACKSLASH,         // KEY_BACKSLASH
    /*  44, 0x02c */    Scancode::Z,                 // KEY_Z
    /*  45, 0x02d */    Scancode::X,                 // KEY_X
    /*  46, 0x02e */    Scancode::C,                 // KEY_C
    /*  47, 0x02f */    Scancode::V,                 // KEY_V
    /*  48, 0x030 */    Scancode::B,                 // KEY_B
    /*  49, 0x031 */    Scancode::N,                 // KEY_N
    /*  50, 0x032 */    Scancode::M,                 // KEY_M
    /*  51, 0x033 */    Scancode::COMMA,             // KEY_COMMA
    /*  52, 0x034 */    Scancode::PERIOD,            // KEY_DOT
    /*  53, 0x035 */    Scancode::SLASH,             // KEY_SLASH
    /*  54, 0x036 */    Scancode::RSHIFT,            // KEY_RIGHTSHIFT
    /*  55, 0x037 */    Scancode::KP_MULTIPLY,       // KEY_KPASTERISK
    /*  56, 0x038 */    Scancode::LALT,              // KEY_LEFTALT
    /*  57, 0x039 */    Scancode::SPACE,             // KEY_SPACE
    /*  58, 0x03a */    Scancode::CAPSLOCK,          // KEY_CAPSLOCK
    /*  59, 0x03b */    Scancode::F1,                // KEY_F1
    /*  60, 0x03c */    Scancode::F2,                // KEY_F2
    /*  61, 0x03d */    Scancode::F3,                // KEY_F3
    /*  62, 0x03e */    Scancode::F4,                // KEY_F4
    /*  63, 0x03f */    Scancode::F5,                // KEY_F5
    /*  64, 0x040 */    Scancode::F6,                // KEY_F6
    /*  65, 0x041 */    Scancode::F7,                // KEY_F7
    /*  66, 0x042 */    Scancode::F8,                // KEY_F8
    /*  67, 0x043 */    Scancode::F9,                // KEY_F9
    /*  68, 0x044 */    Scancode::F10,               // KEY_F10
    /*  69, 0x045 */    Scancode::NUMLOCKCLEAR,      // KEY_NUMLOCK
    /*  70, 0x046 */    Scancode::SCROLLLOCK,        // KEY_SCROLLLOCK
    /*  71, 0x047 */    Scancode::KP_7,              // KEY_KP7
    /*  72, 0x048 */    Scancode::KP_8,              // KEY_KP8
    /*  73, 0x049 */    Scancode::KP_9,              // KEY_KP9
    /*  74, 0x04a */    Scancode::KP_MINUS,          // KEY_KPMINUS
    /*  75, 0x04b */    Scancode::KP_4,              // KEY_KP4
    /*  76, 0x04c */    Scancode::KP_5,              // KEY_KP5
    /*  77, 0x04d */    Scancode::KP_6,              // KEY_KP6
    /*  78, 0x04e */    Scancode::KP_PLUS,           // KEY_KPPLUS
    /*  79, 0x04f */    Scancode::KP_1,              // KEY_KP1
    /*  80, 0x050 */    Scancode::KP_2,              // KEY_KP2
    /*  81, 0x051 */    Scancode::KP_3,              // KEY_KP3
    /*  82, 0x052 */    Scancode::KP_0,              // KEY_KP0
    /*  83, 0x053 */    Scancode::KP_PERIOD,         // KEY_KPDOT
    /*  84, 0x054 */    Scancode::UNKNOWN,
    /*  85, 0x055 */    Scancode::LANG5,             // KEY_ZENKAKUHANKAKU
    /*  86, 0x056 */    Scancode::NONUSBACKSLASH,    // KEY_102ND
    /*  87, 0x057 */    Scancode::F11,               // KEY_F11
    /*  88, 0x058 */    Scancode::F12,               // KEY_F12
    /*  89, 0x059 */    Scancode::INTERNATIONAL1,    // KEY_RO
    /*  90, 0x05a */    Scancode::LANG3,             // KEY_KATAKANA
    /*  91, 0x05b */    Scancode::LANG4,             // KEY_HIRAGANA
    /*  92, 0x05c */    Scancode::INTERNATIONAL4,    // KEY_HENKAN
    /*  93, 0x05d */    Scancode::INTERNATIONAL2,    // KEY_KATAKANAHIRAGANA
    /*  94, 0x05e */    Scancode::INTERNATIONAL5,    // KEY_MUHENKAN
    /*  95, 0x05f */    Scancode::INTERNATIONAL6,    // KEY_KPJPCOMMA
    /*  96, 0x060 */    Scancode::KP_ENTER,          // KEY_KPENTER
    /*  97, 0x061 */    Scancode::RCTRL,             // KEY_RIGHTCTRL
    /*  98, 0x062 */    Scancode::KP_DIVIDE,         // KEY_KPSLASH
    /*  99, 0x063 */    Scancode::SYSREQ,            // KEY_SYSRQ
    /* 100, 0x064 */    Scancode::RALT,              // KEY_RIGHTALT
    /* 101, 0x065 */    Scancode::UNKNOWN,           // KEY_LINEFEED
    /* 102, 0x066 */    Scancode::HOME,              // KEY_HOME
    /* 103, 0x067 */    Scancode::UP,                // KEY_UP
    /* 104, 0x068 */    Scancode::PAGEUP,            // KEY_PAGEUP
    /* 105, 0x069 */    Scancode::LEFT,              // KEY_LEFT
    /* 106, 0x06a */    Scancode::RIGHT,             // KEY_RIGHT
    /* 107, 0x06b */    Scancode::END,               // KEY_END
    /* 108, 0x06c */    Scancode::DOWN,              // KEY_DOWN
    /* 109, 0x06d */    Scancode::PAGEDOWN,          // KEY_PAGEDOWN
    /* 110, 0x06e */    Scancode::INSERT,            // KEY_INSERT
    /* 111, 0x06f */    Scancode::DELETE,            // KEY_DELETE
    /* 112, 0x070 */    Scancode::UNKNOWN,           // KEY_MACRO
    /* 113, 0x071 */    Scancode::MUTE,              // KEY_MUTE
    /* 114, 0x072 */    Scancode::VOLUMEDOWN,        // KEY_VOLUMEDOWN
    /* 115, 0x073 */    Scancode::VOLUMEUP,          // KEY_VOLUMEUP
    /* 116, 0x074 */    Scancode::POWER,             // KEY_POWER
    /* 117, 0x075 */    Scancode::KP_EQUALS,         // KEY_KPEQUAL
    /* 118, 0x076 */    Scancode::KP_PLUSMINUS,      // KEY_KPPLUSMINUS
    /* 119, 0x077 */    Scancode::PAUSE,             // KEY_PAUSE
    /* 120, 0x078 */    Scancode::UNKNOWN,           // KEY_SCALE
    /* 121, 0x079 */    Scancode::KP_COMMA,          // KEY_KPCOMMA
    /* 122, 0x07a */    Scancode::LANG1,             // KEY_HANGEUL
    /* 123, 0x07b */    Scancode::LANG2,             // KEY_HANJA
    /* 124, 0x07c */    Scancode::INTERNATIONAL3,    // KEY_YEN
    /* 125, 0x07d */    Scancode::LGUI,              // KEY_LEFTMETA
    /* 126, 0x07e */    Scancode::RGUI,              // KEY_RIGHTMETA
    /* 127, 0x07f */    Scancode::APPLICATION,       // KEY_COMPOSE
    /* 128, 0x080 */    Scancode::STOP,              // KEY_STOP
    /* 129, 0x081 */    Scancode::AGAIN,             // KEY_AGAIN
    /* 130, 0x082 */    Scancode::AC_PROPERTIES,     // KEY_PROPS
    /* 131, 0x083 */    Scancode::UNDO,              // KEY_UNDO
    /* 132, 0x084 */    Scancode::FRONT,             // KEY_FRONT
    /* 133, 0x085 */    Scancode::COPY,              // KEY_COPY
    /* 134, 0x086 */    Scancode::AC_OPEN,           // KEY_OPEN
    /* 135, 0x087 */    Scancode::PASTE,             // KEY_PASTE
    /* 136, 0x088 */    Scancode::FIND,              // KEY_FIND
    /* 137, 0x089 */    Scancode::CUT,               // KEY_CUT
    /* 138, 0x08a */    Scancode::HELP,              // KEY_HELP
    /* 139, 0x08b */    Scancode::MENU,              // KEY_MENU
    /* 140, 0x08c */    Scancode::UNKNOWN,           // KEY_CALC
    /* 141, 0x08d */    Scancode::UNKNOWN,           // KEY_SETUP
    /* 142, 0x08e */    Scancode::SLEEP,             // KEY_SLEEP
    /* 143, 0x08f */    Scancode::WAKE,              // KEY_WAKEUP
    /* 144, 0x090 */    Scancode::UNKNOWN,           // KEY_FILE
    /* 145, 0x091 */    Scancode::UNKNOWN,           // KEY_SENDFILE
    /* 146, 0x092 */    Scancode::UNKNOWN,           // KEY_DELETEFILE
    /* 147, 0x093 */    Scancode::UNKNOWN,           // KEY_XFER
    /* 148, 0x094 */    Scancode::UNKNOWN,           // KEY_PROG1
    /* 149, 0x095 */    Scancode::UNKNOWN,           // KEY_PROG2
    /* 150, 0x096 */    Scancode::UNKNOWN,           // KEY_WWW
    /* 151, 0x097 */    Scancode::UNKNOWN,           // KEY_MSDOS
    /* 152, 0x098 */    Scancode::UNKNOWN,           // KEY_COFFEE
    /* 153, 0x099 */    Scancode::UNKNOWN,           // KEY_ROTATE_DISPLAY
    /* 154, 0x09a */    Scancode::UNKNOWN,           // KEY_CYCLEWINDOWS
    /* 155, 0x09b */    Scancode::UNKNOWN,           // KEY_MAIL
    /* 156, 0x09c */    Scancode::AC_BOOKMARKS,      // KEY_BOOKMARKS
    /* 157, 0x09d */    Scancode::UNKNOWN,           // KEY_COMPUTER
    /* 158, 0x09e */    Scancode::AC_BACK,           // KEY_BACK
    /* 159, 0x09f */    Scancode::AC_FORWARD,        // KEY_FORWARD
    /* 160, 0x0a0 */    Scancode::UNKNOWN,           // KEY_CLOSECD
    /* 161, 0x0a1 */    Scancode::MEDIA_EJECT,       // KEY_EJECTCD
    /* 162, 0x0a2 */    Scancode::MEDIA_EJECT,       // KEY_EJECTCLOSECD
    /* 163, 0x0a3 */    Scancode::MEDIA_NEXT_TRACK,  // KEY_NEXTSONG
    /* 164, 0x0a4 */    Scancode::MEDIA_PLAY_PAUSE,   // KEY_PLAYPAUSE
    /* 165, 0x0a5 */    Scancode::MEDIA_PREVIOUS_TRACK, // KEY_PREVIOUSSONG
    /* 166, 0x0a6 */    Scancode::MEDIA_STOP,        // KEY_STOPCD
    /* 167, 0x0a7 */    Scancode::MEDIA_RECORD,      // KEY_RECORD
    /* 168, 0x0a8 */    Scancode::MEDIA_REWIND,      // KEY_REWIND
    /* 169, 0x0a9 */    Scancode::UNKNOWN,           // KEY_PHONE
    /* 170, 0x0aa */    Scancode::UNKNOWN,           // KEY_ISO
    /* 171, 0x0ab */    Scancode::UNKNOWN,           // KEY_CONFIG
    /* 172, 0x0ac */    Scancode::AC_HOME,           // KEY_HOMEPAGE
    /* 173, 0x0ad */    Scancode::AC_REFRESH,        // KEY_REFRESH
    /* 174, 0x0ae */    Scancode::AC_EXIT,           // KEY_EXIT
    /* 175, 0x0af */    Scancode::UNKNOWN,           // KEY_MOVE
    /* 176, 0x0b0 */    Scancode::UNKNOWN,           // KEY_EDIT
    /* 177, 0x0b1 */    Scancode::UNKNOWN,           // KEY_SCROLLUP
    /* 178, 0x0b2 */    Scancode::UNKNOWN,           // KEY_SCROLLDOWN
    /* 179, 0x0b3 */    Scancode::KP_LEFTPAREN,      // KEY_KPLEFTPAREN
    /* 180, 0x0b4 */    Scancode::KP_RIGHTPAREN,     // KEY_KPRIGHTPAREN
    /* 181, 0x0b5 */    Scancode::AC_NEW,            // KEY_NEW
    /* 182, 0x0b6 */    Scancode::AGAIN,             // KEY_REDO
    /* 183, 0x0b7 */    Scancode::F13,               // KEY_F13
    /* 184, 0x0b8 */    Scancode::F14,               // KEY_F14
    /* 185, 0x0b9 */    Scancode::F15,               // KEY_F15
    /* 186, 0x0ba */    Scancode::F16,               // KEY_F16
    /* 187, 0x0bb */    Scancode::F17,               // KEY_F17
    /* 188, 0x0bc */    Scancode::F18,               // KEY_F18
    /* 189, 0x0bd */    Scancode::F19,               // KEY_F19
    /* 190, 0x0be */    Scancode::F20,               // KEY_F20
    /* 191, 0x0bf */    Scancode::F21,               // KEY_F21
    /* 192, 0x0c0 */    Scancode::F22,               // KEY_F22
    /* 193, 0x0c1 */    Scancode::F23,               // KEY_F23
    /* 194, 0x0c2 */    Scancode::F24,               // KEY_F24
    /* 195, 0x0c3 */    Scancode::UNKNOWN,
    /* 196, 0x0c4 */    Scancode::UNKNOWN,
    /* 197, 0x0c5 */    Scancode::UNKNOWN,
    /* 198, 0x0c6 */    Scancode::UNKNOWN,
    /* 199, 0x0c7 */    Scancode::UNKNOWN,
    /* 200, 0x0c8 */    Scancode::MEDIA_PLAY,        // KEY_PLAYCD
    /* 201, 0x0c9 */    Scancode::MEDIA_PAUSE,       // KEY_PAUSECD
    /* 202, 0x0ca */    Scancode::UNKNOWN,           // KEY_PROG3
    /* 203, 0x0cb */    Scancode::UNKNOWN,           // KEY_PROG4
    /* 204, 0x0cc */    Scancode::UNKNOWN,           // KEY_ALL_APPLICATIONS
    /* 205, 0x0cd */    Scancode::UNKNOWN,           // KEY_SUSPEND
    /* 206, 0x0ce */    Scancode::AC_CLOSE,          // KEY_CLOSE
    /* 207, 0x0cf */    Scancode::MEDIA_PLAY,        // KEY_PLAY
    /* 208, 0x0d0 */    Scancode::MEDIA_FAST_FORWARD, // KEY_FASTFORWARD
    /* 209, 0x0d1 */    Scancode::UNKNOWN,           // KEY_BASSBOOST
    /* 210, 0x0d2 */    Scancode::PRINTSCREEN,       // KEY_PRINT
    /* 211, 0x0d3 */    Scancode::UNKNOWN,           // KEY_HP
    /* 212, 0x0d4 */    Scancode::UNKNOWN,           // KEY_CAMERA
    /* 213, 0x0d5 */    Scancode::UNKNOWN,           // KEY_SOUND
    /* 214, 0x0d6 */    Scancode::UNKNOWN,           // KEY_QUESTION
    /* 215, 0x0d7 */    Scancode::UNKNOWN,           // KEY_EMAIL
    /* 216, 0x0d8 */    Scancode::UNKNOWN,           // KEY_CHAT
    /* 217, 0x0d9 */    Scancode::AC_SEARCH,         // KEY_SEARCH
    /* 218, 0x0da */    Scancode::UNKNOWN,           // KEY_CONNECT
    /* 219, 0x0db */    Scancode::UNKNOWN,           // KEY_FINANCE
    /* 220, 0x0dc */    Scancode::UNKNOWN,           // KEY_SPORT
    /* 221, 0x0dd */    Scancode::UNKNOWN,           // KEY_SHOP
    /* 222, 0x0de */    Scancode::ALTERASE,          // KEY_ALTERASE
    /* 223, 0x0df */    Scancode::CANCEL,            // KEY_CANCEL
    /* 224, 0x0e0 */    Scancode::UNKNOWN,           // KEY_BRIGHTNESSDOWN
    /* 225, 0x0e1 */    Scancode::UNKNOWN,           // KEY_BRIGHTNESSUP
    /* 226, 0x0e2 */    Scancode::MEDIA_SELECT,      // KEY_MEDIA
    /* 227, 0x0e3 */    Scancode::UNKNOWN,           // KEY_SWITCHVIDEOMODE
    /* 228, 0x0e4 */    Scancode::UNKNOWN,           // KEY_KBDILLUMTOGGLE
    /* 229, 0x0e5 */    Scancode::UNKNOWN,           // KEY_KBDILLUMDOWN
    /* 230, 0x0e6 */    Scancode::UNKNOWN,           // KEY_KBDILLUMUP
    /* 231, 0x0e7 */    Scancode::UNKNOWN,           // KEY_SEND
    /* 232, 0x0e8 */    Scancode::UNKNOWN,           // KEY_REPLY
    /* 233, 0x0e9 */    Scancode::UNKNOWN,           // KEY_FORWARDMAIL
    /* 234, 0x0ea */    Scancode::AC_SAVE,           // KEY_SAVE
    /* 235, 0x0eb */    Scancode::UNKNOWN,           // KEY_DOCUMENTS
    /* 236, 0x0ec */    Scancode::UNKNOWN,           // KEY_BATTERY
    /* 237, 0x0ed */    Scancode::UNKNOWN,           // KEY_BLUETOOTH
    /* 238, 0x0ee */    Scancode::UNKNOWN,           // KEY_WLAN
    /* 239, 0x0ef */    Scancode::UNKNOWN,           // KEY_UWB
    /* 240, 0x0f0 */    Scancode::UNKNOWN,           // KEY_UNKNOWN
    /* 241, 0x0f1 */    Scancode::UNKNOWN,           // KEY_VIDEO_NEXT
    /* 242, 0x0f2 */    Scancode::UNKNOWN,           // KEY_VIDEO_PREV
    /* 243, 0x0f3 */    Scancode::UNKNOWN,           // KEY_BRIGHTNESS_CYCLE
    /* 244, 0x0f4 */    Scancode::UNKNOWN,           // KEY_BRIGHTNESS_AUTO
    /* 245, 0x0f5 */    Scancode::UNKNOWN,           // KEY_DISPLAY_OFF
    /* 246, 0x0f6 */    Scancode::UNKNOWN,           // KEY_WWAN
    /* 247, 0x0f7 */    Scancode::UNKNOWN,           // KEY_RFKILL
    /* 248, 0x0f8 */    Scancode::UNKNOWN,           // KEY_MICMUTE
    /* 249, 0x0f9 */    Scancode::UNKNOWN,
    /* 250, 0x0fa */    Scancode::UNKNOWN,
    /* 251, 0x0fb */    Scancode::UNKNOWN,
    /* 252, 0x0fc */    Scancode::UNKNOWN,
    /* 253, 0x0fd */    Scancode::UNKNOWN,
    /* 254, 0x0fe */    Scancode::UNKNOWN,
    /* 255, 0x0ff */    Scancode::UNKNOWN,
    /* 256, 0x100 */    Scancode::UNKNOWN,
    /* 257, 0x101 */    Scancode::UNKNOWN,
    /* 258, 0x102 */    Scancode::UNKNOWN,
    /* 259, 0x103 */    Scancode::UNKNOWN,
    /* 260, 0x104 */    Scancode::UNKNOWN,
    /* 261, 0x105 */    Scancode::UNKNOWN,
    /* 262, 0x106 */    Scancode::UNKNOWN,
    /* 263, 0x107 */    Scancode::UNKNOWN,
    /* 264, 0x108 */    Scancode::UNKNOWN,
    /* 265, 0x109 */    Scancode::UNKNOWN,
    /* 266, 0x10a */    Scancode::UNKNOWN,
    /* 267, 0x10b */    Scancode::UNKNOWN,
    /* 268, 0x10c */    Scancode::UNKNOWN,
    /* 269, 0x10d */    Scancode::UNKNOWN,
    /* 270, 0x10e */    Scancode::UNKNOWN,
    /* 271, 0x10f */    Scancode::UNKNOWN,
    /* 272, 0x110 */    Scancode::UNKNOWN,
    /* 273, 0x111 */    Scancode::UNKNOWN,
    /* 274, 0x112 */    Scancode::UNKNOWN,
    /* 275, 0x113 */    Scancode::UNKNOWN,
    /* 276, 0x114 */    Scancode::UNKNOWN,
    /* 277, 0x115 */    Scancode::UNKNOWN,
    /* 278, 0x116 */    Scancode::UNKNOWN,
    /* 279, 0x117 */    Scancode::UNKNOWN,
    /* 280, 0x118 */    Scancode::UNKNOWN,
    /* 281, 0x119 */    Scancode::UNKNOWN,
    /* 282, 0x11a */    Scancode::UNKNOWN,
    /* 283, 0x11b */    Scancode::UNKNOWN,
    /* 284, 0x11c */    Scancode::UNKNOWN,
    /* 285, 0x11d */    Scancode::UNKNOWN,
    /* 286, 0x11e */    Scancode::UNKNOWN,
    /* 287, 0x11f */    Scancode::UNKNOWN,
    /* 288, 0x120 */    Scancode::UNKNOWN,
    /* 289, 0x121 */    Scancode::UNKNOWN,
    /* 290, 0x122 */    Scancode::UNKNOWN,
    /* 291, 0x123 */    Scancode::UNKNOWN,
    /* 292, 0x124 */    Scancode::UNKNOWN,
    /* 293, 0x125 */    Scancode::UNKNOWN,
    /* 294, 0x126 */    Scancode::UNKNOWN,
    /* 295, 0x127 */    Scancode::UNKNOWN,
    /* 296, 0x128 */    Scancode::UNKNOWN,
    /* 297, 0x129 */    Scancode::UNKNOWN,
    /* 298, 0x12a */    Scancode::UNKNOWN,
    /* 299, 0x12b */    Scancode::UNKNOWN,
    /* 300, 0x12c */    Scancode::UNKNOWN,
    /* 301, 0x12d */    Scancode::UNKNOWN,
    /* 302, 0x12e */    Scancode::UNKNOWN,
    /* 303, 0x12f */    Scancode::UNKNOWN,
    /* 304, 0x130 */    Scancode::UNKNOWN,
    /* 305, 0x131 */    Scancode::UNKNOWN,
    /* 306, 0x132 */    Scancode::UNKNOWN,
    /* 307, 0x133 */    Scancode::UNKNOWN,
    /* 308, 0x134 */    Scancode::UNKNOWN,
    /* 309, 0x135 */    Scancode::UNKNOWN,
    /* 310, 0x136 */    Scancode::UNKNOWN,
    /* 311, 0x137 */    Scancode::UNKNOWN,
    /* 312, 0x138 */    Scancode::UNKNOWN,
    /* 313, 0x139 */    Scancode::UNKNOWN,
    /* 314, 0x13a */    Scancode::UNKNOWN,
    /* 315, 0x13b */    Scancode::UNKNOWN,
    /* 316, 0x13c */    Scancode::UNKNOWN,
    /* 317, 0x13d */    Scancode::UNKNOWN,
    /* 318, 0x13e */    Scancode::UNKNOWN,
    /* 319, 0x13f */    Scancode::UNKNOWN,
    /* 320, 0x140 */    Scancode::UNKNOWN,
    /* 321, 0x141 */    Scancode::UNKNOWN,
    /* 322, 0x142 */    Scancode::UNKNOWN,
    /* 323, 0x143 */    Scancode::UNKNOWN,
    /* 324, 0x144 */    Scancode::UNKNOWN,
    /* 325, 0x145 */    Scancode::UNKNOWN,
    /* 326, 0x146 */    Scancode::UNKNOWN,
    /* 327, 0x147 */    Scancode::UNKNOWN,
    /* 328, 0x148 */    Scancode::UNKNOWN,
    /* 329, 0x149 */    Scancode::UNKNOWN,
    /* 330, 0x14a */    Scancode::UNKNOWN,
    /* 331, 0x14b */    Scancode::UNKNOWN,
    /* 332, 0x14c */    Scancode::UNKNOWN,
    /* 333, 0x14d */    Scancode::UNKNOWN,
    /* 334, 0x14e */    Scancode::UNKNOWN,
    /* 335, 0x14f */    Scancode::UNKNOWN,
    /* 336, 0x150 */    Scancode::UNKNOWN,
    /* 337, 0x151 */    Scancode::UNKNOWN,
    /* 338, 0x152 */    Scancode::UNKNOWN,
    /* 339, 0x153 */    Scancode::UNKNOWN,
    /* 340, 0x154 */    Scancode::UNKNOWN,
    /* 341, 0x155 */    Scancode::UNKNOWN,
    /* 342, 0x156 */    Scancode::UNKNOWN,
    /* 343, 0x157 */    Scancode::UNKNOWN,
    /* 344, 0x158 */    Scancode::UNKNOWN,
    /* 345, 0x159 */    Scancode::UNKNOWN,
    /* 346, 0x15a */    Scancode::UNKNOWN,
    /* 347, 0x15b */    Scancode::UNKNOWN,
    /* 348, 0x15c */    Scancode::UNKNOWN,
    /* 349, 0x15d */    Scancode::UNKNOWN,
    /* 350, 0x15e */    Scancode::UNKNOWN,
    /* 351, 0x15f */    Scancode::UNKNOWN,
    /* 352, 0x160 */    Scancode::UNKNOWN,            // KEY_OK
    /* 353, 0x161 */    Scancode::SELECT,             // KEY_SELECT
    /* 354, 0x162 */    Scancode::UNKNOWN,            // KEY_GOTO
    /* 355, 0x163 */    Scancode::CLEAR,              // KEY_CLEAR
    /* 356, 0x164 */    Scancode::UNKNOWN,            // KEY_POWER2
    /* 357, 0x165 */    Scancode::UNKNOWN,            // KEY_OPTION
    /* 358, 0x166 */    Scancode::UNKNOWN,            // KEY_INFO
    /* 359, 0x167 */    Scancode::UNKNOWN,            // KEY_TIME
    /* 360, 0x168 */    Scancode::UNKNOWN,            // KEY_VENDOR
    /* 361, 0x169 */    Scancode::UNKNOWN,            // KEY_ARCHIVE
    /* 362, 0x16a */    Scancode::UNKNOWN,            // KEY_PROGRAM
    /* 363, 0x16b */    Scancode::UNKNOWN,            // KEY_CHANNEL
    /* 364, 0x16c */    Scancode::UNKNOWN,            // KEY_FAVORITES
    /* 365, 0x16d */    Scancode::UNKNOWN,            // KEY_EPG
    /* 366, 0x16e */    Scancode::UNKNOWN,            // KEY_PVR
    /* 367, 0x16f */    Scancode::UNKNOWN,            // KEY_MHP
    /* 368, 0x170 */    Scancode::UNKNOWN,            // KEY_LANGUAGE
    /* 369, 0x171 */    Scancode::UNKNOWN,            // KEY_TITLE
    /* 370, 0x172 */    Scancode::UNKNOWN,            // KEY_SUBTITLE
    /* 371, 0x173 */    Scancode::UNKNOWN,            // KEY_ANGLE
    /* 372, 0x174 */    Scancode::UNKNOWN,            // KEY_FULL_SCREEN
    /* 373, 0x175 */    Scancode::MODE,               // KEY_MODE
    /* 374, 0x176 */    Scancode::UNKNOWN,            // KEY_KEYBOARD
    /* 375, 0x177 */    Scancode::UNKNOWN,            // KEY_ASPECT_RATIO
    /* 376, 0x178 */    Scancode::UNKNOWN,            // KEY_PC
    /* 377, 0x179 */    Scancode::UNKNOWN,            // KEY_TV
    /* 378, 0x17a */    Scancode::UNKNOWN,            // KEY_TV2
    /* 379, 0x17b */    Scancode::UNKNOWN,            // KEY_VCR
    /* 380, 0x17c */    Scancode::UNKNOWN,            // KEY_VCR2
    /* 381, 0x17d */    Scancode::UNKNOWN,            // KEY_SAT
    /* 382, 0x17e */    Scancode::UNKNOWN,            // KEY_SAT2
    /* 383, 0x17f */    Scancode::UNKNOWN,            // KEY_CD
    /* 384, 0x180 */    Scancode::UNKNOWN,            // KEY_TAPE
    /* 385, 0x181 */    Scancode::UNKNOWN,            // KEY_RADIO
    /* 386, 0x182 */    Scancode::UNKNOWN,            // KEY_TUNER
    /* 387, 0x183 */    Scancode::UNKNOWN,            // KEY_PLAYER
    /* 388, 0x184 */    Scancode::UNKNOWN,            // KEY_TEXT
    /* 389, 0x185 */    Scancode::UNKNOWN,            // KEY_DVD
    /* 390, 0x186 */    Scancode::UNKNOWN,            // KEY_AUX
    /* 391, 0x187 */    Scancode::UNKNOWN,            // KEY_MP3
    /* 392, 0x188 */    Scancode::UNKNOWN,            // KEY_AUDIO
    /* 393, 0x189 */    Scancode::UNKNOWN,            // KEY_VIDEO
    /* 394, 0x18a */    Scancode::UNKNOWN,            // KEY_DIRECTORY
    /* 395, 0x18b */    Scancode::UNKNOWN,            // KEY_LIST
    /* 396, 0x18c */    Scancode::UNKNOWN,            // KEY_MEMO
    /* 397, 0x18d */    Scancode::UNKNOWN,            // KEY_CALENDAR
    /* 398, 0x18e */    Scancode::UNKNOWN,            // KEY_RED
    /* 399, 0x18f */    Scancode::UNKNOWN,            // KEY_GREEN
    /* 400, 0x190 */    Scancode::UNKNOWN,            // KEY_YELLOW
    /* 401, 0x191 */    Scancode::UNKNOWN,            // KEY_BLUE
    /* 402, 0x192 */    Scancode::CHANNEL_INCREMENT,  // KEY_CHANNELUP
    /* 403, 0x193 */    Scancode::CHANNEL_DECREMENT,  // KEY_CHANNELDOWN
    // (#if 0 upstream: We don't have any mapped scancodes after this point (yet))
];

#[rustfmt::skip]
static XFREE86_SCANCODE_TABLE: &[Scancode] = &[
    /*  0 */    Scancode::UNKNOWN,
    /*  1 */    Scancode::ESCAPE,
    /*  2 */    Scancode::N1,
    /*  3 */    Scancode::N2,
    /*  4 */    Scancode::N3,
    /*  5 */    Scancode::N4,
    /*  6 */    Scancode::N5,
    /*  7 */    Scancode::N6,
    /*  8 */    Scancode::N7,
    /*  9 */    Scancode::N8,
    /*  10 */   Scancode::N9,
    /*  11 */   Scancode::N0,
    /*  12 */   Scancode::MINUS,
    /*  13 */   Scancode::EQUALS,
    /*  14 */   Scancode::BACKSPACE,
    /*  15 */   Scancode::TAB,
    /*  16 */   Scancode::Q,
    /*  17 */   Scancode::W,
    /*  18 */   Scancode::E,
    /*  19 */   Scancode::R,
    /*  20 */   Scancode::T,
    /*  21 */   Scancode::Y,
    /*  22 */   Scancode::U,
    /*  23 */   Scancode::I,
    /*  24 */   Scancode::O,
    /*  25 */   Scancode::P,
    /*  26 */   Scancode::LEFTBRACKET,
    /*  27 */   Scancode::RIGHTBRACKET,
    /*  28 */   Scancode::RETURN,
    /*  29 */   Scancode::LCTRL,
    /*  30 */   Scancode::A,
    /*  31 */   Scancode::S,
    /*  32 */   Scancode::D,
    /*  33 */   Scancode::F,
    /*  34 */   Scancode::G,
    /*  35 */   Scancode::H,
    /*  36 */   Scancode::J,
    /*  37 */   Scancode::K,
    /*  38 */   Scancode::L,
    /*  39 */   Scancode::SEMICOLON,
    /*  40 */   Scancode::APOSTROPHE,
    /*  41 */   Scancode::GRAVE,
    /*  42 */   Scancode::LSHIFT,
    /*  43 */   Scancode::BACKSLASH,
    /*  44 */   Scancode::Z,
    /*  45 */   Scancode::X,
    /*  46 */   Scancode::C,
    /*  47 */   Scancode::V,
    /*  48 */   Scancode::B,
    /*  49 */   Scancode::N,
    /*  50 */   Scancode::M,
    /*  51 */   Scancode::COMMA,
    /*  52 */   Scancode::PERIOD,
    /*  53 */   Scancode::SLASH,
    /*  54 */   Scancode::RSHIFT,
    /*  55 */   Scancode::KP_MULTIPLY,
    /*  56 */   Scancode::LALT,
    /*  57 */   Scancode::SPACE,
    /*  58 */   Scancode::CAPSLOCK,
    /*  59 */   Scancode::F1,
    /*  60 */   Scancode::F2,
    /*  61 */   Scancode::F3,
    /*  62 */   Scancode::F4,
    /*  63 */   Scancode::F5,
    /*  64 */   Scancode::F6,
    /*  65 */   Scancode::F7,
    /*  66 */   Scancode::F8,
    /*  67 */   Scancode::F9,
    /*  68 */   Scancode::F10,
    /*  69 */   Scancode::NUMLOCKCLEAR,
    /*  70 */   Scancode::SCROLLLOCK,
    /*  71 */   Scancode::KP_7,
    /*  72 */   Scancode::KP_8,
    /*  73 */   Scancode::KP_9,
    /*  74 */   Scancode::KP_MINUS,
    /*  75 */   Scancode::KP_4,
    /*  76 */   Scancode::KP_5,
    /*  77 */   Scancode::KP_6,
    /*  78 */   Scancode::KP_PLUS,
    /*  79 */   Scancode::KP_1,
    /*  80 */   Scancode::KP_2,
    /*  81 */   Scancode::KP_3,
    /*  82 */   Scancode::KP_0,
    /*  83 */   Scancode::KP_PERIOD,
    /*  84 */   Scancode::SYSREQ,
    /*  85 */   Scancode::MODE,
    /*  86 */   Scancode::NONUSBACKSLASH,
    /*  87 */   Scancode::F11,
    /*  88 */   Scancode::F12,
    /*  89 */   Scancode::HOME,
    /*  90 */   Scancode::UP,
    /*  91 */   Scancode::PAGEUP,
    /*  92 */   Scancode::LEFT,
    /*  93 */   Scancode::UNKNOWN, // on PowerBook G4 / KEY_Begin
    /*  94 */   Scancode::RIGHT,
    /*  95 */   Scancode::END,
    /*  96 */   Scancode::DOWN,
    /*  97 */   Scancode::PAGEDOWN,
    /*  98 */   Scancode::INSERT,
    /*  99 */   Scancode::DELETE,
    /*  100 */  Scancode::KP_ENTER,
    /*  101 */  Scancode::RCTRL,
    /*  102 */  Scancode::PAUSE,
    /*  103 */  Scancode::PRINTSCREEN,
    /*  104 */  Scancode::KP_DIVIDE,
    /*  105 */  Scancode::RALT,
    /*  106 */  Scancode::UNKNOWN, // BREAK
    /*  107 */  Scancode::LGUI,
    /*  108 */  Scancode::RGUI,
    /*  109 */  Scancode::APPLICATION,
    /*  110 */  Scancode::F13,
    /*  111 */  Scancode::F14,
    /*  112 */  Scancode::F15,
    /*  113 */  Scancode::F16,
    /*  114 */  Scancode::F17,
    /*  115 */  Scancode::INTERNATIONAL1, // \_
    /*  116 */  Scancode::UNKNOWN, /* is translated to XK_ISO_Level3_Shift by my X server, but I have no keyboard that generates this code, so I don't know what the correct SDL_SCANCODE_* for it is */
    /*  117 */  Scancode::UNKNOWN,
    /*  118 */  Scancode::KP_EQUALS,
    /*  119 */  Scancode::UNKNOWN,
    /*  120 */  Scancode::UNKNOWN,
    /*  121 */  Scancode::INTERNATIONAL4, // Henkan_Mode
    /*  122 */  Scancode::UNKNOWN,
    /*  123 */  Scancode::INTERNATIONAL5, // Muhenkan
    /*  124 */  Scancode::UNKNOWN,
    /*  125 */  Scancode::INTERNATIONAL3, // Yen
    /*  126 */  Scancode::UNKNOWN,
    /*  127 */  Scancode::UNKNOWN,
    /*  128 */  Scancode::UNKNOWN,
    /*  129 */  Scancode::UNKNOWN,
    /*  130 */  Scancode::UNKNOWN,
    /*  131 */  Scancode::UNKNOWN,
    /*  132 */  Scancode::POWER,
    /*  133 */  Scancode::MUTE,
    /*  134 */  Scancode::VOLUMEDOWN,
    /*  135 */  Scancode::VOLUMEUP,
    /*  136 */  Scancode::HELP,
    /*  137 */  Scancode::STOP,
    /*  138 */  Scancode::AGAIN,
    /*  139 */  Scancode::UNKNOWN, // PROPS
    /*  140 */  Scancode::UNDO,
    /*  141 */  Scancode::UNKNOWN, // FRONT
    /*  142 */  Scancode::COPY,
    /*  143 */  Scancode::UNKNOWN, // OPEN
    /*  144 */  Scancode::PASTE,
    /*  145 */  Scancode::FIND,
    /*  146 */  Scancode::CUT,
];

// This is largely identical to the Linux keycode mapping
#[rustfmt::skip]
static XFREE86_SCANCODE_TABLE2: &[Scancode] = &[
    /*   0, 0x000 */   Scancode::UNKNOWN,            // NoSymbol
    /*   1, 0x001 */   Scancode::ESCAPE,             // Escape
    /*   2, 0x002 */   Scancode::N1,                  // 1
    /*   3, 0x003 */   Scancode::N2,                  // 2
    /*   4, 0x004 */   Scancode::N3,                  // 3
    /*   5, 0x005 */   Scancode::N4,                  // 4
    /*   6, 0x006 */   Scancode::N5,                  // 5
    /*   7, 0x007 */   Scancode::N6,                  // 6
    /*   8, 0x008 */   Scancode::N7,                  // 7
    /*   9, 0x009 */   Scancode::N8,                  // 8
    /*  10, 0x00a */   Scancode::N9,                  // 9
    /*  11, 0x00b */   Scancode::N0,                  // 0
    /*  12, 0x00c */   Scancode::MINUS,              // minus
    /*  13, 0x00d */   Scancode::EQUALS,             // equal
    /*  14, 0x00e */   Scancode::BACKSPACE,          // BackSpace
    /*  15, 0x00f */   Scancode::TAB,                // Tab
    /*  16, 0x010 */   Scancode::Q,                  // q
    /*  17, 0x011 */   Scancode::W,                  // w
    /*  18, 0x012 */   Scancode::E,                  // e
    /*  19, 0x013 */   Scancode::R,                  // r
    /*  20, 0x014 */   Scancode::T,                  // t
    /*  21, 0x015 */   Scancode::Y,                  // y
    /*  22, 0x016 */   Scancode::U,                  // u
    /*  23, 0x017 */   Scancode::I,                  // i
    /*  24, 0x018 */   Scancode::O,                  // o
    /*  25, 0x019 */   Scancode::P,                  // p
    /*  26, 0x01a */   Scancode::LEFTBRACKET,        // bracketleft
    /*  27, 0x01b */   Scancode::RIGHTBRACKET,       // bracketright
    /*  28, 0x01c */   Scancode::RETURN,             // Return
    /*  29, 0x01d */   Scancode::LCTRL,              // Control_L
    /*  30, 0x01e */   Scancode::A,                  // a
    /*  31, 0x01f */   Scancode::S,                  // s
    /*  32, 0x020 */   Scancode::D,                  // d
    /*  33, 0x021 */   Scancode::F,                  // f
    /*  34, 0x022 */   Scancode::G,                  // g
    /*  35, 0x023 */   Scancode::H,                  // h
    /*  36, 0x024 */   Scancode::J,                  // j
    /*  37, 0x025 */   Scancode::K,                  // k
    /*  38, 0x026 */   Scancode::L,                  // l
    /*  39, 0x027 */   Scancode::SEMICOLON,          // semicolon
    /*  40, 0x028 */   Scancode::APOSTROPHE,         // apostrophe
    /*  41, 0x029 */   Scancode::GRAVE,              // grave
    /*  42, 0x02a */   Scancode::LSHIFT,             // Shift_L
    /*  43, 0x02b */   Scancode::BACKSLASH,          // backslash
    /*  44, 0x02c */   Scancode::Z,                  // z
    /*  45, 0x02d */   Scancode::X,                  // x
    /*  46, 0x02e */   Scancode::C,                  // c
    /*  47, 0x02f */   Scancode::V,                  // v
    /*  48, 0x030 */   Scancode::B,                  // b
    /*  49, 0x031 */   Scancode::N,                  // n
    /*  50, 0x032 */   Scancode::M,                  // m
    /*  51, 0x033 */   Scancode::COMMA,              // comma
    /*  52, 0x034 */   Scancode::PERIOD,             // period
    /*  53, 0x035 */   Scancode::SLASH,              // slash
    /*  54, 0x036 */   Scancode::RSHIFT,             // Shift_R
    /*  55, 0x037 */   Scancode::KP_MULTIPLY,        // KP_Multiply
    /*  56, 0x038 */   Scancode::LALT,               // Alt_L
    /*  57, 0x039 */   Scancode::SPACE,              // space
    /*  58, 0x03a */   Scancode::CAPSLOCK,           // Caps_Lock
    /*  59, 0x03b */   Scancode::F1,                 // F1
    /*  60, 0x03c */   Scancode::F2,                 // F2
    /*  61, 0x03d */   Scancode::F3,                 // F3
    /*  62, 0x03e */   Scancode::F4,                 // F4
    /*  63, 0x03f */   Scancode::F5,                 // F5
    /*  64, 0x040 */   Scancode::F6,                 // F6
    /*  65, 0x041 */   Scancode::F7,                 // F7
    /*  66, 0x042 */   Scancode::F8,                 // F8
    /*  67, 0x043 */   Scancode::F9,                 // F9
    /*  68, 0x044 */   Scancode::F10,                // F10
    /*  69, 0x045 */   Scancode::NUMLOCKCLEAR,       // Num_Lock
    /*  70, 0x046 */   Scancode::SCROLLLOCK,         // Scroll_Lock
    /*  71, 0x047 */   Scancode::KP_7,               // KP_Home
    /*  72, 0x048 */   Scancode::KP_8,               // KP_Up
    /*  73, 0x049 */   Scancode::KP_9,               // KP_Prior
    /*  74, 0x04a */   Scancode::KP_MINUS,           // KP_Subtract
    /*  75, 0x04b */   Scancode::KP_4,               // KP_Left
    /*  76, 0x04c */   Scancode::KP_5,               // KP_Begin
    /*  77, 0x04d */   Scancode::KP_6,               // KP_Right
    /*  78, 0x04e */   Scancode::KP_PLUS,            // KP_Add
    /*  79, 0x04f */   Scancode::KP_1,               // KP_End
    /*  80, 0x050 */   Scancode::KP_2,               // KP_Down
    /*  81, 0x051 */   Scancode::KP_3,               // KP_Next
    /*  82, 0x052 */   Scancode::KP_0,               // KP_Insert
    /*  83, 0x053 */   Scancode::KP_PERIOD,          // KP_Delete
    /*  84, 0x054 */   Scancode::RALT,               // ISO_Level3_Shift
    /*  85, 0x055 */   Scancode::LANG5,              // ????
    /*  86, 0x056 */   Scancode::NONUSBACKSLASH,     // less
    /*  87, 0x057 */   Scancode::F11,                // F11
    /*  88, 0x058 */   Scancode::F12,                // F12
    /*  89, 0x059 */   Scancode::INTERNATIONAL1,     // \_
    /*  90, 0x05a */   Scancode::LANG3,              // Katakana
    /*  91, 0x05b */   Scancode::LANG4,              // Hiragana
    /*  92, 0x05c */   Scancode::INTERNATIONAL4,     // Henkan_Mode
    /*  93, 0x05d */   Scancode::INTERNATIONAL2,     // Hiragana_Katakana
    /*  94, 0x05e */   Scancode::INTERNATIONAL5,     // Muhenkan
    /*  95, 0x05f */   Scancode::INTERNATIONAL6,     // NoSymbol
    /*  96, 0x060 */   Scancode::KP_ENTER,           // KP_Enter
    /*  97, 0x061 */   Scancode::RCTRL,              // Control_R
    /*  98, 0x062 */   Scancode::KP_DIVIDE,          // KP_Divide
    /*  99, 0x063 */   Scancode::SYSREQ,             // Print
    /* 100, 0x064 */   Scancode::RALT,               // ISO_Level3_Shift, ALTGR, RALT
    /* 101, 0x065 */   Scancode::UNKNOWN,            // Linefeed
    /* 102, 0x066 */   Scancode::HOME,               // Home
    /* 103, 0x067 */   Scancode::UP,                 // Up
    /* 104, 0x068 */   Scancode::PAGEUP,             // Prior
    /* 105, 0x069 */   Scancode::LEFT,               // Left
    /* 106, 0x06a */   Scancode::RIGHT,              // Right
    /* 107, 0x06b */   Scancode::END,                // End
    /* 108, 0x06c */   Scancode::DOWN,               // Down
    /* 109, 0x06d */   Scancode::PAGEDOWN,           // Next
    /* 110, 0x06e */   Scancode::INSERT,             // Insert
    /* 111, 0x06f */   Scancode::DELETE,             // Delete
    /* 112, 0x070 */   Scancode::UNKNOWN,            // NoSymbol
    /* 113, 0x071 */   Scancode::MUTE,               // XF86AudioMute
    /* 114, 0x072 */   Scancode::VOLUMEDOWN,         // XF86AudioLowerVolume
    /* 115, 0x073 */   Scancode::VOLUMEUP,           // XF86AudioRaiseVolume
    /* 116, 0x074 */   Scancode::POWER,              // XF86PowerOff
    /* 117, 0x075 */   Scancode::KP_EQUALS,          // KP_Equal
    /* 118, 0x076 */   Scancode::KP_PLUSMINUS,       // plusminus
    /* 119, 0x077 */   Scancode::PAUSE,              // Pause
    /* 120, 0x078 */   Scancode::UNKNOWN,            // XF86LaunchA
    /* 121, 0x079 */   Scancode::KP_COMMA,           // KP_Decimal
    /* 122, 0x07a */   Scancode::LANG1,              // Hangul
    /* 123, 0x07b */   Scancode::LANG2,              // Hangul_Hanja
    /* 124, 0x07c */   Scancode::INTERNATIONAL3,     // Yen
    /* 125, 0x07d */   Scancode::LGUI,               // Super_L
    /* 126, 0x07e */   Scancode::RGUI,               // Super_R
    /* 127, 0x07f */   Scancode::APPLICATION,        // Menu
    /* 128, 0x080 */   Scancode::STOP,               // Cancel
    /* 129, 0x081 */   Scancode::AGAIN,              // Redo
    /* 130, 0x082 */   Scancode::AC_PROPERTIES,      // SunProps
    /* 131, 0x083 */   Scancode::UNDO,               // Undo
    /* 132, 0x084 */   Scancode::FRONT,              // SunFront
    /* 133, 0x085 */   Scancode::COPY,               // XF86Copy
    /* 134, 0x086 */   Scancode::AC_OPEN,            // SunOpen, XF86Open
    /* 135, 0x087 */   Scancode::PASTE,              // XF86Paste
    /* 136, 0x088 */   Scancode::FIND,               // Find
    /* 137, 0x089 */   Scancode::CUT,                // XF86Cut
    /* 138, 0x08a */   Scancode::HELP,               // Help
    /* 139, 0x08b */   Scancode::MENU,               // XF86MenuKB
    /* 140, 0x08c */   Scancode::UNKNOWN,            // XF86Calculator
    /* 141, 0x08d */   Scancode::UNKNOWN,            // NoSymbol
    /* 142, 0x08e */   Scancode::SLEEP,              // XF86Sleep
    /* 143, 0x08f */   Scancode::WAKE,               // XF86WakeUp
    /* 144, 0x090 */   Scancode::UNKNOWN,            // XF86Explorer
    /* 145, 0x091 */   Scancode::UNKNOWN,            // XF86Send
    /* 146, 0x092 */   Scancode::UNKNOWN,            // NoSymbol
    /* 147, 0x093 */   Scancode::UNKNOWN,            // XF86Xfer
    /* 148, 0x094 */   Scancode::UNKNOWN,            // XF86Launch1
    /* 149, 0x095 */   Scancode::UNKNOWN,            // XF86Launch2
    /* 150, 0x096 */   Scancode::UNKNOWN,            // XF86WWW
    /* 151, 0x097 */   Scancode::UNKNOWN,            // XF86DOS
    /* 152, 0x098 */   Scancode::UNKNOWN,            // XF86ScreenSaver
    /* 153, 0x099 */   Scancode::UNKNOWN,            // XF86RotateWindows
    /* 154, 0x09a */   Scancode::UNKNOWN,            // XF86TaskPane
    /* 155, 0x09b */   Scancode::UNKNOWN,            // XF86Mail
    /* 156, 0x09c */   Scancode::AC_BOOKMARKS,       // XF86Favorites
    /* 157, 0x09d */   Scancode::UNKNOWN,            // XF86MyComputer
    /* 158, 0x09e */   Scancode::AC_BACK,            // XF86Back
    /* 159, 0x09f */   Scancode::AC_FORWARD,         // XF86Forward
    /* 160, 0x0a0 */   Scancode::UNKNOWN,            // NoSymbol
    /* 161, 0x0a1 */   Scancode::MEDIA_EJECT,        // XF86Eject
    /* 162, 0x0a2 */   Scancode::MEDIA_EJECT,        // XF86Eject
    /* 163, 0x0a3 */   Scancode::MEDIA_NEXT_TRACK,   // XF86AudioNext
    /* 164, 0x0a4 */   Scancode::MEDIA_PLAY_PAUSE,   // XF86AudioPlay
    /* 165, 0x0a5 */   Scancode::MEDIA_PREVIOUS_TRACK, // XF86AudioPrev
    /* 166, 0x0a6 */   Scancode::MEDIA_STOP,         // XF86AudioStop
    /* 167, 0x0a7 */   Scancode::MEDIA_RECORD,       // XF86AudioRecord
    /* 168, 0x0a8 */   Scancode::MEDIA_REWIND,       // XF86AudioRewind
    /* 169, 0x0a9 */   Scancode::UNKNOWN,            // XF86Phone
    /* 170, 0x0aa */   Scancode::UNKNOWN,            // NoSymbol
    /* 171, 0x0ab */   Scancode::UNKNOWN,            // XF86Tools
    /* 172, 0x0ac */   Scancode::AC_HOME,            // XF86HomePage
    /* 173, 0x0ad */   Scancode::AC_REFRESH,         // XF86Reload
    /* 174, 0x0ae */   Scancode::AC_EXIT,            // XF86Close
    /* 175, 0x0af */   Scancode::UNKNOWN,            // NoSymbol
    /* 176, 0x0b0 */   Scancode::UNKNOWN,            // NoSymbol
    /* 177, 0x0b1 */   Scancode::UNKNOWN,            // XF86ScrollUp
    /* 178, 0x0b2 */   Scancode::UNKNOWN,            // XF86ScrollDown
    /* 179, 0x0b3 */   Scancode::KP_LEFTPAREN,       // parenleft
    /* 180, 0x0b4 */   Scancode::KP_RIGHTPAREN,      // parenright
    /* 181, 0x0b5 */   Scancode::AC_NEW,             // XF86New
    /* 182, 0x0b6 */   Scancode::AGAIN,              // Redo
    /* 183, 0x0b7 */   Scancode::F13,                // XF86Tools
    /* 184, 0x0b8 */   Scancode::F14,                // XF86Launch5
    /* 185, 0x0b9 */   Scancode::F15,                // XF86Launch6
    /* 186, 0x0ba */   Scancode::F16,                // XF86Launch7
    /* 187, 0x0bb */   Scancode::F17,                // XF86Launch8
    /* 188, 0x0bc */   Scancode::F18,                // XF86Launch9
    /* 189, 0x0bd */   Scancode::F19,                // NoSymbol
    /* 190, 0x0be */   Scancode::F20,                // XF86AudioMicMute
    /* 191, 0x0bf */   Scancode::F21,                // XF86TouchpadToggle
    /* 192, 0x0c0 */   Scancode::F22,                // XF86TouchpadOn
    /* 193, 0x0c1 */   Scancode::F23,                // XF86TouchpadOff
    /* 194, 0x0c2 */   Scancode::F24,                // NoSymbol
    /* 195, 0x0c3 */   Scancode::UNKNOWN,            // Mode_switch
    /* 196, 0x0c4 */   Scancode::UNKNOWN,            // NoSymbol
    /* 197, 0x0c5 */   Scancode::UNKNOWN,            // NoSymbol
    /* 198, 0x0c6 */   Scancode::UNKNOWN,            // NoSymbol
    /* 199, 0x0c7 */   Scancode::UNKNOWN,            // NoSymbol
    /* 200, 0x0c8 */   Scancode::MEDIA_PLAY,         // XF86AudioPlay
    /* 201, 0x0c9 */   Scancode::MEDIA_PAUSE,        // XF86AudioPause
    /* 202, 0x0ca */   Scancode::UNKNOWN,            // XF86Launch3
    /* 203, 0x0cb */   Scancode::UNKNOWN,            // XF86Launch4
    /* 204, 0x0cc */   Scancode::UNKNOWN,            // XF86LaunchB
    /* 205, 0x0cd */   Scancode::UNKNOWN,            // XF86Suspend
    /* 206, 0x0ce */   Scancode::AC_CLOSE,           // XF86Close
    /* 207, 0x0cf */   Scancode::MEDIA_PLAY,         // XF86AudioPlay
    /* 208, 0x0d0 */   Scancode::MEDIA_FAST_FORWARD, // XF86AudioForward
    /* 209, 0x0d1 */   Scancode::UNKNOWN,            // NoSymbol
    /* 210, 0x0d2 */   Scancode::PRINTSCREEN,        // Print
    /* 211, 0x0d3 */   Scancode::UNKNOWN,            // NoSymbol
    /* 212, 0x0d4 */   Scancode::UNKNOWN,            // XF86WebCam
    /* 213, 0x0d5 */   Scancode::UNKNOWN,            // XF86AudioPreset
    /* 214, 0x0d6 */   Scancode::UNKNOWN,            // NoSymbol
    /* 215, 0x0d7 */   Scancode::UNKNOWN,            // XF86Mail
    /* 216, 0x0d8 */   Scancode::UNKNOWN,            // XF86Messenger
    /* 217, 0x0d9 */   Scancode::AC_SEARCH,          // XF86Search
    /* 218, 0x0da */   Scancode::UNKNOWN,            // XF86Go
    /* 219, 0x0db */   Scancode::UNKNOWN,            // XF86Finance
    /* 220, 0x0dc */   Scancode::UNKNOWN,            // XF86Game
    /* 221, 0x0dd */   Scancode::UNKNOWN,            // XF86Shop
    /* 222, 0x0de */   Scancode::ALTERASE,           // NoSymbol
    /* 223, 0x0df */   Scancode::CANCEL,             // Cancel
    /* 224, 0x0e0 */   Scancode::UNKNOWN,            // XF86MonBrightnessDown
    /* 225, 0x0e1 */   Scancode::UNKNOWN,            // XF86MonBrightnessUp
    /* 226, 0x0e2 */   Scancode::MEDIA_SELECT,       // XF86AudioMedia
    /* 227, 0x0e3 */   Scancode::UNKNOWN,            // XF86Display
    /* 228, 0x0e4 */   Scancode::UNKNOWN,            // XF86KbdLightOnOff
    /* 229, 0x0e5 */   Scancode::UNKNOWN,            // XF86KbdBrightnessDown
    /* 230, 0x0e6 */   Scancode::UNKNOWN,            // XF86KbdBrightnessUp
    /* 231, 0x0e7 */   Scancode::UNKNOWN,            // XF86Send
    /* 232, 0x0e8 */   Scancode::UNKNOWN,            // XF86Reply
    /* 233, 0x0e9 */   Scancode::UNKNOWN,            // XF86MailForward
    /* 234, 0x0ea */   Scancode::AC_SAVE,            // XF86Save
    /* 235, 0x0eb */   Scancode::UNKNOWN,            // XF86Documents
    /* 236, 0x0ec */   Scancode::UNKNOWN,            // XF86Battery
    /* 237, 0x0ed */   Scancode::UNKNOWN,            // XF86Bluetooth
    /* 238, 0x0ee */   Scancode::UNKNOWN,            // XF86WLAN
    /* 239, 0x0ef */   Scancode::UNKNOWN,            // XF86UWB
    /* 240, 0x0f0 */   Scancode::UNKNOWN,            // NoSymbol
    /* 241, 0x0f1 */   Scancode::UNKNOWN,            // XF86Next_VMode
    /* 242, 0x0f2 */   Scancode::UNKNOWN,            // XF86Prev_VMode
    /* 243, 0x0f3 */   Scancode::UNKNOWN,            // XF86MonBrightnessCycle
    /* 244, 0x0f4 */   Scancode::UNKNOWN,            // XF86BrightnessAuto
    /* 245, 0x0f5 */   Scancode::UNKNOWN,            // XF86DisplayOff
    /* 246, 0x0f6 */   Scancode::UNKNOWN,            // XF86WWAN
    /* 247, 0x0f7 */   Scancode::UNKNOWN,            // XF86RFKill
];

// Xvnc / Xtightvnc scancodes from xmodmap -pk
#[rustfmt::skip]
static XVNC_SCANCODE_TABLE: &[Scancode] = &[
    /*  0 */    Scancode::LCTRL,
    /*  1 */    Scancode::RCTRL,
    /*  2 */    Scancode::LSHIFT,
    /*  3 */    Scancode::RSHIFT,
    /*  4 */    Scancode::UNKNOWN, // Meta_L
    /*  5 */    Scancode::UNKNOWN, // Meta_R
    /*  6 */    Scancode::LALT,
    /*  7 */    Scancode::RALT,
    /*  8 */    Scancode::SPACE,
    /*  9 */    Scancode::N0,
    /*  10 */   Scancode::N1,
    /*  11 */   Scancode::N2,
    /*  12 */   Scancode::N3,
    /*  13 */   Scancode::N4,
    /*  14 */   Scancode::N5,
    /*  15 */   Scancode::N6,
    /*  16 */   Scancode::N7,
    /*  17 */   Scancode::N8,
    /*  18 */   Scancode::N9,
    /*  19 */   Scancode::MINUS,
    /*  20 */   Scancode::EQUALS,
    /*  21 */   Scancode::LEFTBRACKET,
    /*  22 */   Scancode::RIGHTBRACKET,
    /*  23 */   Scancode::SEMICOLON,
    /*  24 */   Scancode::APOSTROPHE,
    /*  25 */   Scancode::GRAVE,
    /*  26 */   Scancode::COMMA,
    /*  27 */   Scancode::PERIOD,
    /*  28 */   Scancode::SLASH,
    /*  29 */   Scancode::BACKSLASH,
    /*  30 */   Scancode::A,
    /*  31 */   Scancode::B,
    /*  32 */   Scancode::C,
    /*  33 */   Scancode::D,
    /*  34 */   Scancode::E,
    /*  35 */   Scancode::F,
    /*  36 */   Scancode::G,
    /*  37 */   Scancode::H,
    /*  38 */   Scancode::I,
    /*  39 */   Scancode::J,
    /*  40 */   Scancode::K,
    /*  41 */   Scancode::L,
    /*  42 */   Scancode::M,
    /*  43 */   Scancode::N,
    /*  44 */   Scancode::O,
    /*  45 */   Scancode::P,
    /*  46 */   Scancode::Q,
    /*  47 */   Scancode::R,
    /*  48 */   Scancode::S,
    /*  49 */   Scancode::T,
    /*  50 */   Scancode::U,
    /*  51 */   Scancode::V,
    /*  52 */   Scancode::W,
    /*  53 */   Scancode::X,
    /*  54 */   Scancode::Y,
    /*  55 */   Scancode::Z,
    /*  56 */   Scancode::BACKSPACE,
    /*  57 */   Scancode::RETURN,
    /*  58 */   Scancode::TAB,
    /*  59 */   Scancode::ESCAPE,
    /*  60 */   Scancode::DELETE,
    /*  61 */   Scancode::HOME,
    /*  62 */   Scancode::END,
    /*  63 */   Scancode::PAGEUP,
    /*  64 */   Scancode::PAGEDOWN,
    /*  65 */   Scancode::UP,
    /*  66 */   Scancode::DOWN,
    /*  67 */   Scancode::LEFT,
    /*  68 */   Scancode::RIGHT,
    /*  69 */   Scancode::F1,
    /*  70 */   Scancode::F2,
    /*  71 */   Scancode::F3,
    /*  72 */   Scancode::F4,
    /*  73 */   Scancode::F5,
    /*  74 */   Scancode::F6,
    /*  75 */   Scancode::F7,
    /*  76 */   Scancode::F8,
    /*  77 */   Scancode::F9,
    /*  78 */   Scancode::F10,
    /*  79 */   Scancode::F11,
    /*  80 */   Scancode::F12,
];
