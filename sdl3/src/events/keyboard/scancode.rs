// Rust translation of include/SDL3/SDL_scancode.h and the scancode name table of
// src/events/SDL_keymap.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Physical key codes (USB HID usage page 0x07 positions).

use std::fmt;

/// A physical key position on the keyboard, independent of layout.
/// Translation of `SDL_Scancode`.
///
/// Values follow the USB usage page 0x07 (Keyboard/Keypad) codes, so this is
/// a newtype with associated constants rather than a closed enum: SDL also
/// allocates dynamic scancodes in `RESERVED..RESERVED + 100` for keycodes
/// that have no physical key.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Scancode(pub u16);

#[allow(non_upper_case_globals)]
impl Scancode {
    pub const UNKNOWN: Scancode = Scancode(0);
    pub const A: Scancode = Scancode(4);
    pub const B: Scancode = Scancode(5);
    pub const C: Scancode = Scancode(6);
    pub const D: Scancode = Scancode(7);
    pub const E: Scancode = Scancode(8);
    pub const F: Scancode = Scancode(9);
    pub const G: Scancode = Scancode(10);
    pub const H: Scancode = Scancode(11);
    pub const I: Scancode = Scancode(12);
    pub const J: Scancode = Scancode(13);
    pub const K: Scancode = Scancode(14);
    pub const L: Scancode = Scancode(15);
    pub const M: Scancode = Scancode(16);
    pub const N: Scancode = Scancode(17);
    pub const O: Scancode = Scancode(18);
    pub const P: Scancode = Scancode(19);
    pub const Q: Scancode = Scancode(20);
    pub const R: Scancode = Scancode(21);
    pub const S: Scancode = Scancode(22);
    pub const T: Scancode = Scancode(23);
    pub const U: Scancode = Scancode(24);
    pub const V: Scancode = Scancode(25);
    pub const W: Scancode = Scancode(26);
    pub const X: Scancode = Scancode(27);
    pub const Y: Scancode = Scancode(28);
    pub const Z: Scancode = Scancode(29);
    pub const N1: Scancode = Scancode(30);
    pub const N2: Scancode = Scancode(31);
    pub const N3: Scancode = Scancode(32);
    pub const N4: Scancode = Scancode(33);
    pub const N5: Scancode = Scancode(34);
    pub const N6: Scancode = Scancode(35);
    pub const N7: Scancode = Scancode(36);
    pub const N8: Scancode = Scancode(37);
    pub const N9: Scancode = Scancode(38);
    pub const N0: Scancode = Scancode(39);
    pub const RETURN: Scancode = Scancode(40);
    pub const ESCAPE: Scancode = Scancode(41);
    pub const BACKSPACE: Scancode = Scancode(42);
    pub const TAB: Scancode = Scancode(43);
    pub const SPACE: Scancode = Scancode(44);
    pub const MINUS: Scancode = Scancode(45);
    pub const EQUALS: Scancode = Scancode(46);
    pub const LEFTBRACKET: Scancode = Scancode(47);
    pub const RIGHTBRACKET: Scancode = Scancode(48);
    pub const BACKSLASH: Scancode = Scancode(49);
    pub const NONUSHASH: Scancode = Scancode(50);
    pub const SEMICOLON: Scancode = Scancode(51);
    pub const APOSTROPHE: Scancode = Scancode(52);
    pub const GRAVE: Scancode = Scancode(53);
    pub const COMMA: Scancode = Scancode(54);
    pub const PERIOD: Scancode = Scancode(55);
    pub const SLASH: Scancode = Scancode(56);
    pub const CAPSLOCK: Scancode = Scancode(57);
    pub const F1: Scancode = Scancode(58);
    pub const F2: Scancode = Scancode(59);
    pub const F3: Scancode = Scancode(60);
    pub const F4: Scancode = Scancode(61);
    pub const F5: Scancode = Scancode(62);
    pub const F6: Scancode = Scancode(63);
    pub const F7: Scancode = Scancode(64);
    pub const F8: Scancode = Scancode(65);
    pub const F9: Scancode = Scancode(66);
    pub const F10: Scancode = Scancode(67);
    pub const F11: Scancode = Scancode(68);
    pub const F12: Scancode = Scancode(69);
    pub const PRINTSCREEN: Scancode = Scancode(70);
    pub const SCROLLLOCK: Scancode = Scancode(71);
    pub const PAUSE: Scancode = Scancode(72);
    pub const INSERT: Scancode = Scancode(73);
    pub const HOME: Scancode = Scancode(74);
    pub const PAGEUP: Scancode = Scancode(75);
    pub const DELETE: Scancode = Scancode(76);
    pub const END: Scancode = Scancode(77);
    pub const PAGEDOWN: Scancode = Scancode(78);
    pub const RIGHT: Scancode = Scancode(79);
    pub const LEFT: Scancode = Scancode(80);
    pub const DOWN: Scancode = Scancode(81);
    pub const UP: Scancode = Scancode(82);
    pub const NUMLOCKCLEAR: Scancode = Scancode(83);
    pub const KP_DIVIDE: Scancode = Scancode(84);
    pub const KP_MULTIPLY: Scancode = Scancode(85);
    pub const KP_MINUS: Scancode = Scancode(86);
    pub const KP_PLUS: Scancode = Scancode(87);
    pub const KP_ENTER: Scancode = Scancode(88);
    pub const KP_1: Scancode = Scancode(89);
    pub const KP_2: Scancode = Scancode(90);
    pub const KP_3: Scancode = Scancode(91);
    pub const KP_4: Scancode = Scancode(92);
    pub const KP_5: Scancode = Scancode(93);
    pub const KP_6: Scancode = Scancode(94);
    pub const KP_7: Scancode = Scancode(95);
    pub const KP_8: Scancode = Scancode(96);
    pub const KP_9: Scancode = Scancode(97);
    pub const KP_0: Scancode = Scancode(98);
    pub const KP_PERIOD: Scancode = Scancode(99);
    pub const NONUSBACKSLASH: Scancode = Scancode(100);
    pub const APPLICATION: Scancode = Scancode(101);
    pub const POWER: Scancode = Scancode(102);
    pub const KP_EQUALS: Scancode = Scancode(103);
    pub const F13: Scancode = Scancode(104);
    pub const F14: Scancode = Scancode(105);
    pub const F15: Scancode = Scancode(106);
    pub const F16: Scancode = Scancode(107);
    pub const F17: Scancode = Scancode(108);
    pub const F18: Scancode = Scancode(109);
    pub const F19: Scancode = Scancode(110);
    pub const F20: Scancode = Scancode(111);
    pub const F21: Scancode = Scancode(112);
    pub const F22: Scancode = Scancode(113);
    pub const F23: Scancode = Scancode(114);
    pub const F24: Scancode = Scancode(115);
    pub const EXECUTE: Scancode = Scancode(116);
    pub const HELP: Scancode = Scancode(117);
    pub const MENU: Scancode = Scancode(118);
    pub const SELECT: Scancode = Scancode(119);
    pub const STOP: Scancode = Scancode(120);
    pub const AGAIN: Scancode = Scancode(121);
    pub const UNDO: Scancode = Scancode(122);
    pub const CUT: Scancode = Scancode(123);
    pub const COPY: Scancode = Scancode(124);
    pub const PASTE: Scancode = Scancode(125);
    pub const FIND: Scancode = Scancode(126);
    pub const MUTE: Scancode = Scancode(127);
    pub const VOLUMEUP: Scancode = Scancode(128);
    pub const VOLUMEDOWN: Scancode = Scancode(129);
    pub const KP_COMMA: Scancode = Scancode(133);
    pub const KP_EQUALSAS400: Scancode = Scancode(134);
    pub const INTERNATIONAL1: Scancode = Scancode(135);
    pub const INTERNATIONAL2: Scancode = Scancode(136);
    pub const INTERNATIONAL3: Scancode = Scancode(137);
    pub const INTERNATIONAL4: Scancode = Scancode(138);
    pub const INTERNATIONAL5: Scancode = Scancode(139);
    pub const INTERNATIONAL6: Scancode = Scancode(140);
    pub const INTERNATIONAL7: Scancode = Scancode(141);
    pub const INTERNATIONAL8: Scancode = Scancode(142);
    pub const INTERNATIONAL9: Scancode = Scancode(143);
    pub const LANG1: Scancode = Scancode(144);
    pub const LANG2: Scancode = Scancode(145);
    pub const LANG3: Scancode = Scancode(146);
    pub const LANG4: Scancode = Scancode(147);
    pub const LANG5: Scancode = Scancode(148);
    pub const LANG6: Scancode = Scancode(149);
    pub const LANG7: Scancode = Scancode(150);
    pub const LANG8: Scancode = Scancode(151);
    pub const LANG9: Scancode = Scancode(152);
    pub const ALTERASE: Scancode = Scancode(153);
    pub const SYSREQ: Scancode = Scancode(154);
    pub const CANCEL: Scancode = Scancode(155);
    pub const CLEAR: Scancode = Scancode(156);
    pub const PRIOR: Scancode = Scancode(157);
    pub const RETURN2: Scancode = Scancode(158);
    pub const SEPARATOR: Scancode = Scancode(159);
    pub const OUT: Scancode = Scancode(160);
    pub const OPER: Scancode = Scancode(161);
    pub const CLEARAGAIN: Scancode = Scancode(162);
    pub const CRSEL: Scancode = Scancode(163);
    pub const EXSEL: Scancode = Scancode(164);
    pub const FRONT: Scancode = Scancode(165);
    pub const KP_00: Scancode = Scancode(176);
    pub const KP_000: Scancode = Scancode(177);
    pub const THOUSANDSSEPARATOR: Scancode = Scancode(178);
    pub const DECIMALSEPARATOR: Scancode = Scancode(179);
    pub const CURRENCYUNIT: Scancode = Scancode(180);
    pub const CURRENCYSUBUNIT: Scancode = Scancode(181);
    pub const KP_LEFTPAREN: Scancode = Scancode(182);
    pub const KP_RIGHTPAREN: Scancode = Scancode(183);
    pub const KP_LEFTBRACE: Scancode = Scancode(184);
    pub const KP_RIGHTBRACE: Scancode = Scancode(185);
    pub const KP_TAB: Scancode = Scancode(186);
    pub const KP_BACKSPACE: Scancode = Scancode(187);
    pub const KP_A: Scancode = Scancode(188);
    pub const KP_B: Scancode = Scancode(189);
    pub const KP_C: Scancode = Scancode(190);
    pub const KP_D: Scancode = Scancode(191);
    pub const KP_E: Scancode = Scancode(192);
    pub const KP_F: Scancode = Scancode(193);
    pub const KP_XOR: Scancode = Scancode(194);
    pub const KP_POWER: Scancode = Scancode(195);
    pub const KP_PERCENT: Scancode = Scancode(196);
    pub const KP_LESS: Scancode = Scancode(197);
    pub const KP_GREATER: Scancode = Scancode(198);
    pub const KP_AMPERSAND: Scancode = Scancode(199);
    pub const KP_DBLAMPERSAND: Scancode = Scancode(200);
    pub const KP_VERTICALBAR: Scancode = Scancode(201);
    pub const KP_DBLVERTICALBAR: Scancode = Scancode(202);
    pub const KP_COLON: Scancode = Scancode(203);
    pub const KP_HASH: Scancode = Scancode(204);
    pub const KP_SPACE: Scancode = Scancode(205);
    pub const KP_AT: Scancode = Scancode(206);
    pub const KP_EXCLAM: Scancode = Scancode(207);
    pub const KP_MEMSTORE: Scancode = Scancode(208);
    pub const KP_MEMRECALL: Scancode = Scancode(209);
    pub const KP_MEMCLEAR: Scancode = Scancode(210);
    pub const KP_MEMADD: Scancode = Scancode(211);
    pub const KP_MEMSUBTRACT: Scancode = Scancode(212);
    pub const KP_MEMMULTIPLY: Scancode = Scancode(213);
    pub const KP_MEMDIVIDE: Scancode = Scancode(214);
    pub const KP_PLUSMINUS: Scancode = Scancode(215);
    pub const KP_CLEAR: Scancode = Scancode(216);
    pub const KP_CLEARENTRY: Scancode = Scancode(217);
    pub const KP_BINARY: Scancode = Scancode(218);
    pub const KP_OCTAL: Scancode = Scancode(219);
    pub const KP_DECIMAL: Scancode = Scancode(220);
    pub const KP_HEXADECIMAL: Scancode = Scancode(221);
    pub const LCTRL: Scancode = Scancode(224);
    pub const LSHIFT: Scancode = Scancode(225);
    pub const LALT: Scancode = Scancode(226);
    pub const LGUI: Scancode = Scancode(227);
    pub const RCTRL: Scancode = Scancode(228);
    pub const RSHIFT: Scancode = Scancode(229);
    pub const RALT: Scancode = Scancode(230);
    pub const RGUI: Scancode = Scancode(231);
    pub const MODE: Scancode = Scancode(257);
    pub const SLEEP: Scancode = Scancode(258);
    pub const WAKE: Scancode = Scancode(259);
    pub const CHANNEL_INCREMENT: Scancode = Scancode(260);
    pub const CHANNEL_DECREMENT: Scancode = Scancode(261);
    pub const MEDIA_PLAY: Scancode = Scancode(262);
    pub const MEDIA_PAUSE: Scancode = Scancode(263);
    pub const MEDIA_RECORD: Scancode = Scancode(264);
    pub const MEDIA_FAST_FORWARD: Scancode = Scancode(265);
    pub const MEDIA_REWIND: Scancode = Scancode(266);
    pub const MEDIA_NEXT_TRACK: Scancode = Scancode(267);
    pub const MEDIA_PREVIOUS_TRACK: Scancode = Scancode(268);
    pub const MEDIA_STOP: Scancode = Scancode(269);
    pub const MEDIA_EJECT: Scancode = Scancode(270);
    pub const MEDIA_PLAY_PAUSE: Scancode = Scancode(271);
    pub const MEDIA_SELECT: Scancode = Scancode(272);
    pub const AC_NEW: Scancode = Scancode(273);
    pub const AC_OPEN: Scancode = Scancode(274);
    pub const AC_CLOSE: Scancode = Scancode(275);
    pub const AC_EXIT: Scancode = Scancode(276);
    pub const AC_SAVE: Scancode = Scancode(277);
    pub const AC_PRINT: Scancode = Scancode(278);
    pub const AC_PROPERTIES: Scancode = Scancode(279);
    pub const AC_SEARCH: Scancode = Scancode(280);
    pub const AC_HOME: Scancode = Scancode(281);
    pub const AC_BACK: Scancode = Scancode(282);
    pub const AC_FORWARD: Scancode = Scancode(283);
    pub const AC_STOP: Scancode = Scancode(284);
    pub const AC_REFRESH: Scancode = Scancode(285);
    pub const AC_BOOKMARKS: Scancode = Scancode(286);
    pub const SOFTLEFT: Scancode = Scancode(287);
    pub const SOFTRIGHT: Scancode = Scancode(288);
    pub const CALL: Scancode = Scancode(289);
    pub const ENDCALL: Scancode = Scancode(290);
    /// 400-500 reserved for dynamic keycodes
    pub const RESERVED: Scancode = Scancode(400);

    /// Not a key, just marks the number of scancodes for array bounds. Translation of `SDL_SCANCODE_COUNT`.
    pub const COUNT: usize = 512;

    /// The raw USB usage code.
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// True for `UNKNOWN < self < COUNT`, i.e. a code that fits SDL's key state tables.
    pub const fn is_valid(self) -> bool {
        self.0 > 0 && (self.0 as usize) < Scancode::COUNT
    }

    /// The default human-readable name, or `""`. Translation of the static
    /// part of `SDL_GetScancodeName()`; see [`Scancode::name`] for the
    /// version honoring [`Scancode::set_name`] overrides.
    pub fn default_name(self) -> &'static str {
        match SCANCODE_NAMES.get(self.0 as usize) {
            Some(Some(n)) => n,
            _ => "",
        }
    }
}

impl fmt::Debug for Scancode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.default_name();
        if n.is_empty() {
            write!(f, "Scancode({})", self.0)
        } else {
            write!(f, "Scancode({} {:?})", self.0, n)
        }
    }
}

/// Translation of `SDL_scancode_names[]`.
pub(crate) static SCANCODE_NAMES: [Option<&str>; 291] = [
    None,                       // 0
    None,                       // 1
    None,                       // 2
    None,                       // 3
    Some("A"),                  // 4
    Some("B"),                  // 5
    Some("C"),                  // 6
    Some("D"),                  // 7
    Some("E"),                  // 8
    Some("F"),                  // 9
    Some("G"),                  // 10
    Some("H"),                  // 11
    Some("I"),                  // 12
    Some("J"),                  // 13
    Some("K"),                  // 14
    Some("L"),                  // 15
    Some("M"),                  // 16
    Some("N"),                  // 17
    Some("O"),                  // 18
    Some("P"),                  // 19
    Some("Q"),                  // 20
    Some("R"),                  // 21
    Some("S"),                  // 22
    Some("T"),                  // 23
    Some("U"),                  // 24
    Some("V"),                  // 25
    Some("W"),                  // 26
    Some("X"),                  // 27
    Some("Y"),                  // 28
    Some("Z"),                  // 29
    Some("1"),                  // 30
    Some("2"),                  // 31
    Some("3"),                  // 32
    Some("4"),                  // 33
    Some("5"),                  // 34
    Some("6"),                  // 35
    Some("7"),                  // 36
    Some("8"),                  // 37
    Some("9"),                  // 38
    Some("0"),                  // 39
    Some("Return"),             // 40
    Some("Escape"),             // 41
    Some("Backspace"),          // 42
    Some("Tab"),                // 43
    Some("Space"),              // 44
    Some("-"),                  // 45
    Some("="),                  // 46
    Some("["),                  // 47
    Some("]"),                  // 48
    Some("\\"),                 // 49
    Some("#"),                  // 50
    Some(";"),                  // 51
    Some("'"),                  // 52
    Some("`"),                  // 53
    Some(","),                  // 54
    Some("."),                  // 55
    Some("/"),                  // 56
    Some("CapsLock"),           // 57
    Some("F1"),                 // 58
    Some("F2"),                 // 59
    Some("F3"),                 // 60
    Some("F4"),                 // 61
    Some("F5"),                 // 62
    Some("F6"),                 // 63
    Some("F7"),                 // 64
    Some("F8"),                 // 65
    Some("F9"),                 // 66
    Some("F10"),                // 67
    Some("F11"),                // 68
    Some("F12"),                // 69
    Some("PrintScreen"),        // 70
    Some("ScrollLock"),         // 71
    Some("Pause"),              // 72
    Some("Insert"),             // 73
    Some("Home"),               // 74
    Some("PageUp"),             // 75
    Some("Delete"),             // 76
    Some("End"),                // 77
    Some("PageDown"),           // 78
    Some("Right"),              // 79
    Some("Left"),               // 80
    Some("Down"),               // 81
    Some("Up"),                 // 82
    Some("Numlock"),            // 83
    Some("Keypad /"),           // 84
    Some("Keypad *"),           // 85
    Some("Keypad -"),           // 86
    Some("Keypad +"),           // 87
    Some("Keypad Enter"),       // 88
    Some("Keypad 1"),           // 89
    Some("Keypad 2"),           // 90
    Some("Keypad 3"),           // 91
    Some("Keypad 4"),           // 92
    Some("Keypad 5"),           // 93
    Some("Keypad 6"),           // 94
    Some("Keypad 7"),           // 95
    Some("Keypad 8"),           // 96
    Some("Keypad 9"),           // 97
    Some("Keypad 0"),           // 98
    Some("Keypad ."),           // 99
    Some("NonUSBackslash"),     // 100
    Some("Application"),        // 101
    Some("Power"),              // 102
    Some("Keypad ="),           // 103
    Some("F13"),                // 104
    Some("F14"),                // 105
    Some("F15"),                // 106
    Some("F16"),                // 107
    Some("F17"),                // 108
    Some("F18"),                // 109
    Some("F19"),                // 110
    Some("F20"),                // 111
    Some("F21"),                // 112
    Some("F22"),                // 113
    Some("F23"),                // 114
    Some("F24"),                // 115
    Some("Execute"),            // 116
    Some("Help"),               // 117
    Some("Menu"),               // 118
    Some("Select"),             // 119
    Some("Stop"),               // 120
    Some("Again"),              // 121
    Some("Undo"),               // 122
    Some("Cut"),                // 123
    Some("Copy"),               // 124
    Some("Paste"),              // 125
    Some("Find"),               // 126
    Some("Mute"),               // 127
    Some("VolumeUp"),           // 128
    Some("VolumeDown"),         // 129
    None,                       // 130
    None,                       // 131
    None,                       // 132
    Some("Keypad ,"),           // 133
    Some("Keypad = (AS400)"),   // 134
    Some("International 1"),    // 135
    Some("International 2"),    // 136
    Some("International 3"),    // 137
    Some("International 4"),    // 138
    Some("International 5"),    // 139
    Some("International 6"),    // 140
    Some("International 7"),    // 141
    Some("International 8"),    // 142
    Some("International 9"),    // 143
    Some("Language 1"),         // 144
    Some("Language 2"),         // 145
    Some("Language 3"),         // 146
    Some("Language 4"),         // 147
    Some("Language 5"),         // 148
    Some("Language 6"),         // 149
    Some("Language 7"),         // 150
    Some("Language 8"),         // 151
    Some("Language 9"),         // 152
    Some("AltErase"),           // 153
    Some("SysReq"),             // 154
    Some("Cancel"),             // 155
    Some("Clear"),              // 156
    Some("Prior"),              // 157
    Some("Return"),             // 158
    Some("Separator"),          // 159
    Some("Out"),                // 160
    Some("Oper"),               // 161
    Some("Clear / Again"),      // 162
    Some("CrSel"),              // 163
    Some("ExSel"),              // 164
    Some("Front"),              // 165
    None,                       // 166
    None,                       // 167
    None,                       // 168
    None,                       // 169
    None,                       // 170
    None,                       // 171
    None,                       // 172
    None,                       // 173
    None,                       // 174
    None,                       // 175
    Some("Keypad 00"),          // 176
    Some("Keypad 000"),         // 177
    Some("ThousandsSeparator"), // 178
    Some("DecimalSeparator"),   // 179
    Some("CurrencyUnit"),       // 180
    Some("CurrencySubUnit"),    // 181
    Some("Keypad ("),           // 182
    Some("Keypad )"),           // 183
    Some("Keypad {"),           // 184
    Some("Keypad }"),           // 185
    Some("Keypad Tab"),         // 186
    Some("Keypad Backspace"),   // 187
    Some("Keypad A"),           // 188
    Some("Keypad B"),           // 189
    Some("Keypad C"),           // 190
    Some("Keypad D"),           // 191
    Some("Keypad E"),           // 192
    Some("Keypad F"),           // 193
    Some("Keypad XOR"),         // 194
    Some("Keypad ^"),           // 195
    Some("Keypad %"),           // 196
    Some("Keypad <"),           // 197
    Some("Keypad >"),           // 198
    Some("Keypad &"),           // 199
    Some("Keypad &&"),          // 200
    Some("Keypad |"),           // 201
    Some("Keypad ||"),          // 202
    Some("Keypad :"),           // 203
    Some("Keypad #"),           // 204
    Some("Keypad Space"),       // 205
    Some("Keypad @"),           // 206
    Some("Keypad !"),           // 207
    Some("Keypad MemStore"),    // 208
    Some("Keypad MemRecall"),   // 209
    Some("Keypad MemClear"),    // 210
    Some("Keypad MemAdd"),      // 211
    Some("Keypad MemSubtract"), // 212
    Some("Keypad MemMultiply"), // 213
    Some("Keypad MemDivide"),   // 214
    Some("Keypad +/-"),         // 215
    Some("Keypad Clear"),       // 216
    Some("Keypad ClearEntry"),  // 217
    Some("Keypad Binary"),      // 218
    Some("Keypad Octal"),       // 219
    Some("Keypad Decimal"),     // 220
    Some("Keypad Hexadecimal"), // 221
    None,                       // 222
    None,                       // 223
    Some("Left Ctrl"),          // 224
    Some("Left Shift"),         // 225
    Some("Left Alt"),           // 226
    Some("Left GUI"),           // 227
    Some("Right Ctrl"),         // 228
    Some("Right Shift"),        // 229
    Some("Right Alt"),          // 230
    Some("Right GUI"),          // 231
    None,                       // 232
    None,                       // 233
    None,                       // 234
    None,                       // 235
    None,                       // 236
    None,                       // 237
    None,                       // 238
    None,                       // 239
    None,                       // 240
    None,                       // 241
    None,                       // 242
    None,                       // 243
    None,                       // 244
    None,                       // 245
    None,                       // 246
    None,                       // 247
    None,                       // 248
    None,                       // 249
    None,                       // 250
    None,                       // 251
    None,                       // 252
    None,                       // 253
    None,                       // 254
    None,                       // 255
    None,                       // 256
    Some("ModeSwitch"),         // 257
    Some("Sleep"),              // 258
    Some("Wake"),               // 259
    Some("ChannelUp"),          // 260
    Some("ChannelDown"),        // 261
    Some("MediaPlay"),          // 262
    Some("MediaPause"),         // 263
    Some("MediaRecord"),        // 264
    Some("MediaFastForward"),   // 265
    Some("MediaRewind"),        // 266
    Some("MediaTrackNext"),     // 267
    Some("MediaTrackPrevious"), // 268
    Some("MediaStop"),          // 269
    Some("Eject"),              // 270
    Some("MediaPlayPause"),     // 271
    Some("MediaSelect"),        // 272
    Some("AC New"),             // 273
    Some("AC Open"),            // 274
    Some("AC Close"),           // 275
    Some("AC Exit"),            // 276
    Some("AC Save"),            // 277
    Some("AC Print"),           // 278
    Some("AC Properties"),      // 279
    Some("AC Search"),          // 280
    Some("AC Home"),            // 281
    Some("AC Back"),            // 282
    Some("AC Forward"),         // 283
    Some("AC Stop"),            // 284
    Some("AC Refresh"),         // 285
    Some("AC Bookmarks"),       // 286
    Some("SoftLeft"),           // 287
    Some("SoftRight"),          // 288
    Some("Call"),               // 289
    Some("EndCall"),            // 290
];

/// Translation of `SDL_extended_key_names[]` (names for `SDLK_LEFT_TAB` .. `SDLK_RHYPER`).
pub(crate) static EXTENDED_KEY_NAMES: [&str; 7] = [
    "LeftTab",
    "Level5Shift",
    "MultiKeyCompose",
    "Left Meta",
    "Right Meta",
    "Left Hyper",
    "Right Hyper",
];
