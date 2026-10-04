// Rust translation of src/video/x11/SDL_x11keyboard.c and SDL_x11keyboard.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The keyboard: guessing the X server's key codes (to scancodes), the
//! keymaps (with XKB, one per keyboard group, or from the core keyboard
//! mapping), the X input method and its pre-edit (composition) callbacks,
//! and the Steam on-screen keyboard.

use std::ffi::{c_char, c_int, c_uint, c_ulong, CStr};

use super::sys::*;
use super::video::X11Video;
use super::window::{with_x11_window, IcClientData, X11WindowData};
use crate::error::Result;
use crate::events::keyboard::{self, Keycode, Keymap, Keymod, Scancode};
use crate::events::keysym_to_keycode::get_key_code_from_key_sym;
use crate::events::keysym_to_scancode::get_scancode_from_key_sym;
use crate::events::scancode_tables::{get_scancode_table, ScancodeTable};
use crate::events::WindowID;
use crate::hints;
use crate::properties::Properties;
use crate::video::core::with_window;
use crate::video::textinput::{
    send_screen_keyboard_hidden, send_screen_keyboard_shown, text_input_multiline, text_input_type,
    TextInputType,
};

const XK_Home: KeySym = 0xff50;
const XK_Left: KeySym = 0xff51;
const XK_Up: KeySym = 0xff52;
const XK_Prior: KeySym = 0xff55;
const XK_KP_Enter: KeySym = 0xff8d;
const XK_Delete: KeySym = 0xffff;

/// The XKB part of the keyboard data (`keyboard.xkb`): modern XKB keyboard
/// handling.
pub(crate) struct XkbKeyboardData {
    pub(crate) desc_ptr: XkbDescPtr,
    pub(crate) keymaps: [Option<Keymap>; XkbNumKbdGroups],
    pub(crate) last_map_serial: c_ulong,
    pub(crate) event: c_int,
    pub(crate) current_group: u32,
}

/// The core part of the keyboard data (`keyboard.core`): legacy core
/// keyboard handling.
pub(crate) struct CoreKeyboardData {
    pub(crate) keysym_map: *mut KeySym,
    pub(crate) keysyms_per_key: c_int,
    pub(crate) min_keycode: c_int,
    pub(crate) max_keycode: c_int,
}

/// The `keyboard` member of `SDL_VideoData` (upstream's union of the XKB
/// and core data is two members here).
pub(crate) struct KeyboardData {
    pub(crate) xkb_enabled: bool,
    pub(crate) key_layout: [Scancode; 256],

    pub(crate) xkb: XkbKeyboardData,
    pub(crate) core: CoreKeyboardData,

    pub(crate) pressed_modifiers: u32,
    pub(crate) locked_modifiers: u32,
    pub(crate) sdl_pressed_modifiers: Keymod,
    pub(crate) sdl_physically_pressed_modifiers: Keymod,
    pub(crate) sdl_locked_modifiers: Keymod,

    // Virtual modifiers looked up by name.
    pub(crate) alt_mask: u32,
    pub(crate) gui_mask: u32,
    pub(crate) level3_mask: u32,
    pub(crate) level5_mask: u32,
    pub(crate) numlock_mask: u32,
    pub(crate) scrolllock_mask: u32,
}

impl Default for KeyboardData {
    fn default() -> Self {
        KeyboardData {
            xkb_enabled: false,
            key_layout: [Scancode::UNKNOWN; 256],
            xkb: XkbKeyboardData {
                desc_ptr: std::ptr::null_mut(),
                keymaps: Default::default(),
                last_map_serial: 0,
                event: 0,
                current_group: 0,
            },
            core: CoreKeyboardData {
                keysym_map: std::ptr::null_mut(),
                keysyms_per_key: 0,
                min_keycode: 0,
                max_keycode: 0,
            },
            pressed_modifiers: 0,
            locked_modifiers: 0,
            sdl_pressed_modifiers: Keymod::NONE,
            sdl_physically_pressed_modifiers: Keymod::NONE,
            sdl_locked_modifiers: Keymod::NONE,
            alt_mask: 0,
            gui_mask: 0,
            level3_mask: 0,
            level5_mask: 0,
            numlock_mask: 0,
            scrolllock_mask: 0,
        }
    }
}

/// The scancode tables tried against the server's key codes
/// (`scancode_set`).
const SCANCODE_SET: [ScancodeTable; 4] = [
    ScancodeTable::Darwin,
    ScancodeTable::Xfree86_1,
    ScancodeTable::Xfree86_2,
    ScancodeTable::Xvnc,
];

/// Translation of `X11_ScancodeIsRemappable()`.
fn x11_scancode_is_remappable(scancode: Scancode) -> bool {
    /*
     * XKB remappings can assign different keysyms for these scancodes, but
     * as these keys are in fixed positions, the scancodes themselves shouldn't
     * be switched. Mark them as not being remappable.
     */
    !matches!(
        scancode,
        Scancode::ESCAPE
            | Scancode::CAPSLOCK
            | Scancode::NUMLOCKCLEAR
            | Scancode::LSHIFT
            | Scancode::RSHIFT
            | Scancode::LCTRL
            | Scancode::RCTRL
            | Scancode::LALT
            | Scancode::RALT
            | Scancode::LGUI
            | Scancode::RGUI
    )
}

impl X11Video {
    /// Translation of `X11_KeyCodeToSym()`.
    fn x11_key_code_to_sym(
        &self,
        kb: &KeyboardData,
        keycode: KeyCode,
        group: c_uint,
        level: c_uint,
    ) -> KeySym {
        if kb.xkb_enabled {
            if let Some(xkb) = &self.x.xkb {
                // SAFETY: the display is open.
                return unsafe { (xkb.XkbKeycodeToKeysym)(self.display, keycode, group, level) };
            }
        }
        // TODO: Handle groups on the legacy path.
        let keycode = keycode as c_int;
        if keycode >= kb.core.min_keycode
            && keycode <= kb.core.max_keycode
            && !kb.core.keysym_map.is_null()
        {
            let index = ((keycode - kb.core.min_keycode) * kb.core.keysyms_per_key) as usize;
            // SAFETY: XGetKeyboardMapping returned keysyms_per_key keysyms
            // for each key code it was asked about.
            unsafe { *kb.core.keysym_map.add(index) }
        } else {
            NoSymbol
        }
    }

    /// This function only correctly maps letters and numbers for keyboards in US QWERTY layout.
    /// Translation of `X11_KeyCodeToSDLScancode()`.
    fn x11_key_code_to_sdl_scancode(&self, kb: &KeyboardData, keycode: KeyCode) -> Scancode {
        let keysym = self.x11_key_code_to_sym(kb, keycode, 0, 0);

        if keysym == NoSymbol {
            return Scancode::UNKNOWN;
        }

        get_scancode_from_key_sym(keysym as u32, keycode as u32)
    }

    /// Translation of `X11_InitIM()`.
    fn x11_init_im(&self) -> bool {
        let Some(utf8) = &self.x.utf8 else {
            return false;
        };
        /* Set the locale, and call XSetLocaleModifiers before XOpenIM so that
         * Compose keys will work correctly.
         */
        // SAFETY: setlocale with NULL only queries; the strings are copied
        // before the locale changes again.
        let (prev_locale, prev_xmods) = unsafe {
            let prev_locale = libc::setlocale(libc::LC_ALL, std::ptr::null());
            let prev_xmods = (utf8.XSetLocaleModifiers)(std::ptr::null());
            (
                (!prev_locale.is_null()).then(|| CStr::from_ptr(prev_locale).to_owned()),
                (!prev_xmods.is_null()).then(|| CStr::from_ptr(prev_xmods).to_owned()),
            )
        };

        // SAFETY: the display is open; the strings are NUL-terminated.
        let im = unsafe {
            libc::setlocale(libc::LC_ALL, c"".as_ptr());
            (utf8.XSetLocaleModifiers)(c"".as_ptr());

            let im = (utf8.XOpenIM)(
                self.display,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );

            /* Reset the locale + X locale modifiers back to how they were,
             * locale first because the X locale modifiers depend on it.
             */
            libc::setlocale(
                libc::LC_ALL,
                prev_locale
                    .as_deref()
                    .map_or(std::ptr::null(), CStr::as_ptr),
            );
            (utf8.XSetLocaleModifiers)(
                prev_xmods.as_deref().map_or(std::ptr::null(), CStr::as_ptr),
            );
            im
        };

        if !im.is_null() {
            self.with_data(|d| d.im = im);

            let destroyed_callback = XIMCallback {
                client_data: self as *const X11Video as XPointer,
                callback: Some(im_destroyed_handler),
            };

            // SAFETY: the input method is open; the callback is copied by
            // Xlib; the list ends with NULL.
            unsafe {
                (utf8.XSetIMValues)(
                    im,
                    XNDestroyCallback.as_ptr(),
                    &destroyed_callback as *const XIMCallback,
                    std::ptr::null::<c_char>(),
                );
            }

            /* Attach the new IC to any existing windows.
             *
             * If we wound up here before any old input contexts were destroyed,
             * they will be recreated in the IC destroyed callback.
             */
            let windows = self.with_data(|d| d.windowlist.clone());
            for w in windows {
                let _ = with_x11_window(w.window, |_, data| {
                    if data.ic.is_null() {
                        self.x11_create_input_context(data);
                    }
                });
            }

            return true;
        }

        false
    }

    /// Translation of `X11_InitKeyboard()`.
    pub(crate) fn x11_init_keyboard(&self) -> Result<()> {
        let display = self.display;
        let x = &self.x;
        let mut min_keycode: c_int = 0;
        let mut max_keycode: c_int = 0;
        struct Fingerprint {
            scancode: Scancode,
            keysym: KeySym,
            value: c_int,
        }
        #[rustfmt::skip]
        let mut fingerprint = [
            Fingerprint { scancode: Scancode::HOME, keysym: XK_Home, value: 0 },
            Fingerprint { scancode: Scancode::PAGEUP, keysym: XK_Prior, value: 0 },
            Fingerprint { scancode: Scancode::UP, keysym: XK_Up, value: 0 },
            Fingerprint { scancode: Scancode::LEFT, keysym: XK_Left, value: 0 },
            Fingerprint { scancode: Scancode::DELETE, keysym: XK_Delete, value: 0 },
            Fingerprint { scancode: Scancode::KP_ENTER, keysym: XK_KP_Enter, value: 0 },
        ];

        let mut xkb_major: c_int = XkbMajorVersion;
        let mut xkb_minor: c_int = XkbMinorVersion;

        let mut xkb_event: c_int = 0;
        // SAFETY: the display is open; the out-parameters are valid or NULL.
        let have_xkb = x.xkb.as_ref().is_some_and(|xkb| unsafe {
            (xkb.XkbQueryExtension)(
                display,
                std::ptr::null_mut(),
                &mut xkb_event,
                std::ptr::null_mut(),
                &mut xkb_major,
                &mut xkb_minor,
            ) != 0
        });
        if let (true, Some(xkb)) = (have_xkb, &x.xkb) {
            let mut xkb_repeat: Bool = 0;
            // SAFETY: the display is open.
            let desc_ptr = unsafe { (xkb.XkbGetMap)(display, XkbAllClientInfoMask, XkbUseCoreKbd) };
            self.with_data(|d| {
                d.keyboard.xkb.event = xkb_event;
                d.keyboard.xkb_enabled = true;
                d.keyboard.xkb.desc_ptr = desc_ptr;
            });

            // SAFETY: the display is open; xkb_repeat is an out-parameter.
            unsafe {
                // This will remove KeyRelease events for held keys.
                (xkb.XkbSetDetectableAutoRepeat)(display, True, &mut xkb_repeat);

                // Enable the key mapping and state events.
                let mask = XkbNewKeyboardNotifyMask | XkbMapNotifyMask | XkbStateNotifyMask;
                (xkb.XkbSelectEvents)(display, XkbUseCoreKbd, mask as c_uint, mask as c_uint);
                let details = XkbGroupStateMask | XkbModifierStateMask;
                (xkb.XkbSelectEventDetails)(
                    display,
                    XkbUseCoreKbd,
                    XkbStateNotify as c_uint,
                    details,
                    details,
                );
            }
        } else {
            // If XKB isn't available, initialize the legacy path.
            let mut min: c_int = 0;
            let mut max: c_int = 0;
            let mut per_key: c_int = 0;
            // SAFETY: the display is open; the out-parameters are valid.
            let map = unsafe {
                (x.XDisplayKeycodes)(display, &mut min, &mut max);
                (x.XGetKeyboardMapping)(display, min as KeyCode, max - min, &mut per_key)
            };
            self.with_data(|d| {
                d.keyboard.core.min_keycode = min;
                d.keyboard.core.max_keycode = max;
                d.keyboard.core.keysym_map = map;
                d.keyboard.core.keysyms_per_key = per_key;
            });
        }

        // Open a connection to the X input manager
        if let Some(utf8) = &x.utf8 {
            if !self.x11_init_im() {
                // SAFETY: the display is open; the handler is a valid extern
                // fn and the client data is this device, which unregisters
                // it in X11_QuitKeyboard().
                unsafe {
                    (utf8.XRegisterIMInstantiateCallback)(
                        display,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        Some(im_instantiate_handler),
                        self as *const X11Video as XPointer,
                    );
                }
            }
        }
        // Try to determine which scancodes are being used based on fingerprint
        let mut best_distance = fingerprint.len() as c_int + 1;
        let mut best_index: isize = -1;
        // SAFETY: the display is open; the out-parameters are valid.
        unsafe {
            (x.XDisplayKeycodes)(display, &mut min_keycode, &mut max_keycode);
        }
        for f in fingerprint.iter_mut() {
            // SAFETY: the display is open.
            f.value = unsafe { (x.XKeysymToKeycode)(display, f.keysym) } as c_int - min_keycode;
        }
        for (i, &set) in SCANCODE_SET.iter().enumerate() {
            let table = get_scancode_table(set);
            let table_size = table.len() as c_int;

            let mut distance = 0;
            for f in &fingerprint {
                if f.value < 0 || f.value >= table_size || table[f.value as usize] != f.scancode {
                    distance += 1;
                }
            }
            if distance < best_distance {
                best_distance = distance;
                best_index = i as isize;
            }
        }
        if best_index < 0 || best_distance > 2 {
            // This is likely to be SDL_SCANCODE_TABLE_XFREE86_2 with remapped keys, double check a rarely remapped value
            // SAFETY: the display is open.
            let fingerprint_value = unsafe {
                (x.XKeysymToKeycode)(display, 0x1008FF5B /* XF86Documents */)
            } as c_int
                - min_keycode;
            if fingerprint_value == 235 {
                for (i, &set) in SCANCODE_SET.iter().enumerate() {
                    if set == ScancodeTable::Xfree86_2 {
                        best_index = i as isize;
                        best_distance = 0;
                        break;
                    }
                }
            }
        }

        let mut kb = self.with_data(|d| std::mem::take(&mut d.keyboard));
        if best_index >= 0 && best_distance <= 2 {
            let table = get_scancode_table(SCANCODE_SET[best_index as usize]);
            let mut table_size = table.len();

            // (DEBUG_KEYBOARD: "Using scancode set %d, min_keycode = %d, max_keycode = %d, table_size = %d")
            // This should never happen, but just in case...
            if table_size > kb.key_layout.len() - min_keycode as usize {
                table_size = kb.key_layout.len() - min_keycode as usize;
            }
            kb.key_layout[min_keycode as usize..min_keycode as usize + table_size]
                .copy_from_slice(&table[..table_size]);

            /* Scancodes represent physical locations on the keyboard, unaffected by keyboard mapping.
            However, there are a number of extended scancodes that have no standard location, so use
            the X11 mapping for all non-character keys.
            */
            for i in min_keycode..=max_keycode {
                let scancode = self.x11_key_code_to_sdl_scancode(&kb, i as KeyCode);
                // (DEBUG_KEYBOARD logs the key code and keysym)
                if scancode == kb.key_layout[i as usize] {
                    continue;
                }
                if (Keymap::keycode_in(Option::None, scancode, Keymod::NONE).0
                    & (Keycode::SCANCODE_MASK | Keycode::EXTENDED_MASK))
                    != 0
                    && x11_scancode_is_remappable(scancode)
                {
                    // Not a character key and the scancode is safe to remap
                    // (DEBUG_KEYBOARD: "Changing scancode, was %d (%s), now %d (%s)")
                    kb.key_layout[i as usize] = scancode;
                }
            }
        } else {
            // (DEBUG_SCANCODES: "Keyboard layout unknown, please report the following to the SDL forums/mailing list")

            // Determine key_layout - only works on US QWERTY layout
            for i in min_keycode..=max_keycode {
                let scancode = self.x11_key_code_to_sdl_scancode(&kb, i as KeyCode);
                // (DEBUG_SCANCODES logs the key code, keysym and scancode)
                kb.key_layout[i as usize] = scancode;
            }
        }
        self.with_data(|d| d.keyboard = kb);

        self.x11_update_keymap(false);

        let _ = Scancode::APPLICATION.set_name(Some("Menu"));

        self.x11_reconcile_keyboard_state();

        Ok(())
    }

    /// Translation of `X11_GetXkbVirtualModifierMask()`.
    fn x11_get_xkb_virtual_modifier_mask(&self, kb: &KeyboardData, vmod_name: &str) -> u32 {
        let mut mod_mask: u32 = 0;

        let vmod = self.intern_atom(vmod_name, true);
        let desc = kb.xkb.desc_ptr;
        // SAFETY: the description came from XkbGetMap and its names and
        // server map were filled in by X11_UpdateKeymap() (a missing one
        // reads as no modifier; upstream dereferences it).
        unsafe {
            if vmod != None
                && !desc.is_null()
                && !(*desc).names.is_null()
                && !(*desc).server.is_null()
            {
                for i in 0..XkbNumVirtualMods {
                    if vmod == (*(*desc).names).vmods[i] {
                        mod_mask = (*(*desc).server).vmods[i] as u32;
                        break;
                    }
                }
            }
        }

        mod_mask
    }

    /// Translation of `X11_GetXModifierMask()`.
    fn x11_get_xmodifier_mask(&self, kb: &KeyboardData, scancode: Scancode) -> u32 {
        let display = self.display;
        let mut mod_mask: u32 = 0;

        // SAFETY: the display is open; the modifier map holds 8 rows of
        // max_keypermod key codes and is freed here.
        unsafe {
            let xmods = (self.x.XGetModifierMapping)(display);
            let n = (*xmods).max_keypermod as usize;
            for i in 3..8 {
                for j in 0..n {
                    let kc = *(*xmods).modifiermap.add(i * n + j);
                    if kb.key_layout[kc as usize] == scancode {
                        mod_mask = 1 << i;
                        break;
                    }
                }
            }
            (self.x.XFreeModifiermap)(xmods);
        }

        mod_mask
    }

    /// Build the keymaps from the server's keyboard mapping and make the
    /// current group's the active one. Translation of `X11_UpdateKeymap()`.
    pub(crate) fn x11_update_keymap(&self, send_event: bool) {
        let display = self.display;
        let mut kb = self.with_data(|d| std::mem::take(&mut d.keyboard));

        let keymap_to_set = if let (true, Some(xkb)) = (kb.xkb_enabled, &self.x.xkb) {
            // SAFETY: XkbStateRec is plain data.
            let mut state: XkbStateRec = unsafe { std::mem::zeroed() };

            for keymap in kb.xkb.keymaps.iter_mut() {
                *keymap = Option::None;
            }

            // SAFETY: the display is open; the description came from XkbGetMap.
            unsafe {
                (xkb.XkbGetNames)(display, XkbVirtualModNamesMask, kb.xkb.desc_ptr);
                (xkb.XkbGetUpdatedMap)(
                    display,
                    XkbAllClientInfoMask | XkbVirtualModsMask,
                    kb.xkb.desc_ptr,
                );

                if (xkb.XkbGetState)(display, XkbUseCoreKbd, &mut state) == Success {
                    kb.xkb.current_group = state.group as u32;
                }
            }

            kb.alt_mask = self.x11_get_xkb_virtual_modifier_mask(&kb, "Alt");
            if kb.alt_mask == 0 {
                kb.alt_mask = self.x11_get_xkb_virtual_modifier_mask(&kb, "Meta");
            }
            kb.gui_mask = self.x11_get_xkb_virtual_modifier_mask(&kb, "Super");
            kb.level3_mask = self.x11_get_xkb_virtual_modifier_mask(&kb, "LevelThree");
            kb.level5_mask = self.x11_get_xkb_virtual_modifier_mask(&kb, "LevelFive");
            kb.numlock_mask = self.x11_get_xkb_virtual_modifier_mask(&kb, "NumLock");
            kb.scrolllock_mask = self.x11_get_xkb_virtual_modifier_mask(&kb, "ScrollLock");

            // (SDL_CreateKeymap() can't fail here)
            let mut keymaps: [Keymap; XkbNumKbdGroups] = Default::default();

            // Only the shift, alt, level 3, level 5 and caps lock modifiers affect SDL keymaps.
            let valid_mod_mask =
                ShiftMask | LockMask | kb.alt_mask | kb.level3_mask | kb.level5_mask;

            let desc = kb.xkb.desc_ptr;
            // SAFETY: the description came from XkbGetMap with its client map.
            let (min_key_code, max_key_code) =
                unsafe { ((*desc).min_key_code as u32, (*desc).max_key_code as u32) };
            for xkeycode in min_key_code..max_key_code {
                let scancode = kb.key_layout[xkeycode as usize];
                if scancode == Scancode::UNKNOWN {
                    continue;
                }

                for (group, keymap) in keymaps.iter_mut().enumerate() {
                    let mut effective_group = group as u32;
                    // SAFETY: the client map covers the description's key codes.
                    let (max_key_group, key_group_info) = unsafe {
                        (
                            XkbKeyNumGroups(desc, xkeycode),
                            XkbKeyGroupInfo(desc, xkeycode),
                        )
                    };

                    if max_key_group != 0 && effective_group >= max_key_group as u32 {
                        let action = XkbOutOfRangeGroupAction(key_group_info);

                        match action {
                            XkbClampIntoRange => {
                                effective_group = max_key_group as u32 - 1;
                            }
                            XkbRedirectIntoRange => {
                                effective_group = XkbOutOfRangeGroupNumber(key_group_info) as u32;
                                if effective_group >= max_key_group as u32 {
                                    effective_group = 0;
                                }
                            }
                            _ => {
                                effective_group %= max_key_group as u32;
                            }
                        }
                    }

                    // SAFETY: as above; the key type and its map are Xlib's.
                    let key_type = unsafe { &*XkbKeyKeyType(desc, xkeycode, effective_group) };
                    let map: &[XkbKTMapEntryRec] = if key_type.map.is_null() {
                        &[]
                    } else {
                        // SAFETY: the type has map_count entries.
                        unsafe {
                            std::slice::from_raw_parts(key_type.map, key_type.map_count as usize)
                        }
                    };

                    for level in 0..key_type.num_levels as u32 {
                        let keysym = self.x11_key_code_to_sym(
                            &kb,
                            xkeycode as KeyCode,
                            effective_group,
                            level,
                        );

                        if keysym != NoSymbol {
                            let mut key_added = false;

                            for entry in map {
                                if entry.active != 0 && entry.level as u32 == level {
                                    let xkb_mod_mask = entry.mods.mask as u32;
                                    if (xkb_mod_mask | valid_mod_mask) == valid_mod_mask {
                                        let mut sdl_mod_mask = Keymod::NONE;
                                        if xkb_mod_mask & ShiftMask != 0 {
                                            sdl_mod_mask |= Keymod::SHIFT;
                                        }
                                        if xkb_mod_mask & LockMask != 0 {
                                            sdl_mod_mask |= Keymod::CAPS;
                                        }
                                        if xkb_mod_mask & kb.alt_mask != 0 {
                                            sdl_mod_mask |= Keymod::ALT;
                                        }
                                        if xkb_mod_mask & kb.level3_mask != 0 {
                                            sdl_mod_mask |= Keymod::MODE;
                                        }
                                        if xkb_mod_mask & kb.level5_mask != 0 {
                                            sdl_mod_mask |= Keymod::LEVEL5;
                                        }

                                        x11_add_keymap_entry(
                                            keymap,
                                            xkeycode,
                                            keysym,
                                            scancode,
                                            sdl_mod_mask,
                                        );
                                        key_added = true;
                                    }
                                }
                            }

                            // Add the unmodified key for level 0.
                            if level == 0 && !key_added {
                                x11_add_keymap_entry(
                                    keymap,
                                    xkeycode,
                                    keysym,
                                    scancode,
                                    Keymod::NONE,
                                );
                            }
                        }
                    }
                }
            }

            for (slot, keymap) in kb.xkb.keymaps.iter_mut().zip(keymaps) {
                *slot = Some(keymap);
            }
            kb.xkb
                .keymaps
                .get(kb.xkb.current_group as usize)
                .cloned()
                .flatten()
        } else {
            let mut keymap = Keymap::new();

            if send_event {
                if !kb.core.keysym_map.is_null() {
                    // SAFETY: the map came from XGetKeyboardMapping.
                    unsafe {
                        (self.x.XFree)(kb.core.keysym_map.cast());
                    }
                }
                // SAFETY: the display is open; the out-parameters are valid.
                unsafe {
                    (self.x.XDisplayKeycodes)(
                        display,
                        &mut kb.core.min_keycode,
                        &mut kb.core.max_keycode,
                    );
                    kb.core.keysym_map = (self.x.XGetKeyboardMapping)(
                        display,
                        kb.core.min_keycode as KeyCode,
                        kb.core.max_keycode - kb.core.min_keycode,
                        &mut kb.core.keysyms_per_key,
                    );
                }
            }

            for xkeycode in kb.core.min_keycode..=kb.core.max_keycode {
                let scancode = kb.key_layout[xkeycode as usize & 0xff];
                if scancode == Scancode::UNKNOWN {
                    continue;
                }

                let keysym = self.x11_key_code_to_sym(&kb, xkeycode as KeyCode, 0, 0);
                if keysym != NoSymbol {
                    x11_add_keymap_entry(
                        &mut keymap,
                        xkeycode as u32,
                        keysym,
                        scancode,
                        Keymod::NONE,
                    );
                }
            }

            kb.alt_mask = Mod1Mask; // Alt or Meta
            kb.gui_mask = Mod4Mask; // Super
            kb.level3_mask = Mod5Mask; // Note: Not a typo, Mod5 = level 3 shift, and Mod3 = level 5 shift.
            kb.level5_mask = Mod3Mask;
            kb.numlock_mask = self.x11_get_xmodifier_mask(&kb, Scancode::NUMLOCKCLEAR);
            kb.scrolllock_mask = self.x11_get_xmodifier_mask(&kb, Scancode::SCROLLLOCK);

            Some(keymap)
        };
        self.with_data(|d| d.keyboard = kb);

        keyboard::set_keymap(keymap_to_set, send_event);
    }

    /// Translation of `X11_QuitKeyboard()`.
    pub(crate) fn x11_quit_keyboard(&self) {
        if let Some(utf8) = &self.x.utf8 {
            let im = self.with_data(|d| std::mem::replace(&mut d.im, std::ptr::null_mut()));
            // SAFETY: the input method is ours; the callback was registered
            // with this device as its client data.
            unsafe {
                if !im.is_null() {
                    (utf8.XCloseIM)(im);
                } else {
                    (utf8.XUnregisterIMInstantiateCallback)(
                        self.display,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        Some(im_instantiate_handler),
                        self as *const X11Video as XPointer,
                    );
                }
            }
        }

        let mut kb = self.with_data(|d| std::mem::take(&mut d.keyboard));
        if kb.xkb_enabled {
            for keymap in kb.xkb.keymaps.iter_mut() {
                *keymap = Option::None;
            }

            if let Some(xkb) = &self.x.xkb {
                // SAFETY: the description came from XkbGetMap and is freed once.
                unsafe {
                    (xkb.XkbFreeKeyboard)(kb.xkb.desc_ptr, 0, True);
                }
            }
            kb.xkb.desc_ptr = std::ptr::null_mut();
        } else if !kb.core.keysym_map.is_null() {
            // SAFETY: the map came from XGetKeyboardMapping.
            unsafe {
                (self.x.XFree)(kb.core.keysym_map.cast());
            }
            kb.core.keysym_map = std::ptr::null_mut();
        }
        self.with_data(|d| d.keyboard = kb);
    }

    /// Translation of `X11_ClearComposition()` for a window.
    pub(crate) fn x11_clear_composition(&self, window: WindowID) {
        if with_x11_window(window, |_, data| clear_composition(data)).unwrap_or(false) {
            keyboard::send_editing_text("", 0, 0);
        }
    }

    /// Translation of `X11_CreateInputContext()`.
    pub(crate) fn x11_create_input_context(&self, data: &mut X11WindowData) {
        let Some(utf8) = &self.x.utf8 else {
            return;
        };
        let im = self.with_data(|d| d.im);

        if !im.is_null() {
            let hint = hints::get(hints::IME_IMPLEMENTED_UI);
            let client_data = &*data.ic_client as *const IcClientData as XPointer;

            let destroyed_callback = XIMCallback {
                client_data,
                callback: Some(ic_destroyed_handler),
            };

            if hint.is_some_and(|h| h.contains("composition")) {
                let draw_callback = XIMCallback {
                    client_data,
                    callback: Some(preedit_draw_callback_proc),
                };

                let start_callback = XIMCallback {
                    client_data,
                    callback: preedit_start_callback_proc(),
                };

                let done_callback = XIMCallback {
                    client_data,
                    callback: Some(preedit_done_callback),
                };

                let caret_callback = XIMCallback {
                    client_data,
                    callback: Some(preedit_caret_callback_proc),
                };

                // SAFETY: the callbacks are copied by Xlib; the lists end
                // with NULL; the nested list is freed after use.
                unsafe {
                    let attr = (utf8.XVaCreateNestedList)(
                        0,
                        XNPreeditStartCallback.as_ptr(),
                        &start_callback as *const XIMCallback,
                        XNPreeditDoneCallback.as_ptr(),
                        &done_callback as *const XIMCallback,
                        XNPreeditDrawCallback.as_ptr(),
                        &draw_callback as *const XIMCallback,
                        XNPreeditCaretCallback.as_ptr(),
                        &caret_callback as *const XIMCallback,
                        std::ptr::null::<c_char>(),
                    );
                    if !attr.is_null() {
                        data.ic = (utf8.XCreateIC)(
                            im,
                            XNInputStyle.as_ptr(),
                            (XIMPreeditCallbacks | XIMStatusCallbacks) as c_ulong,
                            XNPreeditAttributes.as_ptr(),
                            attr,
                            XNClientWindow.as_ptr(),
                            data.xwindow,
                            XNDestroyCallback.as_ptr(),
                            &destroyed_callback as *const XIMCallback,
                            std::ptr::null::<c_char>(),
                        );
                        (self.x.XFree)(attr);
                    }
                }
            }
            if data.ic.is_null() {
                // SAFETY: as above.
                data.ic = unsafe {
                    (utf8.XCreateIC)(
                        im,
                        XNInputStyle.as_ptr(),
                        (XIMPreeditNothing | XIMStatusNothing) as c_ulong,
                        XNClientWindow.as_ptr(),
                        data.xwindow,
                        XNDestroyCallback.as_ptr(),
                        &destroyed_callback as *const XIMCallback,
                        std::ptr::null::<c_char>(),
                    )
                };
            }
            data.xim_spot.x = -1;
            data.xim_spot.y = -1;
        }
    }

    /// Translation of `X11_DestroyInputContext()`.
    pub(crate) fn x11_destroy_input_context(&self, data: &mut X11WindowData) {
        if !data.ic.is_null() {
            if let Some(utf8) = &self.x.utf8 {
                // SAFETY: the input context is the window's, destroyed once.
                unsafe {
                    (utf8.XDestroyIC)(data.ic);
                }
            }
            data.ic = std::ptr::null_mut();
        }
        data.preedit_text = Option::None;
        data.preedit_feedback = Vec::new();
    }

    /// Translation of `X11_ResetXIM()`.
    fn x11_reset_xim(&self, window: WindowID) {
        let Some(utf8) = &self.x.utf8 else {
            return;
        };
        let ic = with_x11_window(window, |_, data| data.ic).unwrap_or(std::ptr::null_mut());

        if !ic.is_null() {
            // Clear any partially entered dead keys
            // SAFETY: the input context is the window's; the contents are
            // freed with XFree.
            unsafe {
                let contents = (utf8.Xutf8ResetIC)(ic);
                if !contents.is_null() {
                    (self.x.XFree)(contents.cast());
                }
            }
        }
    }

    /// Translation of `X11_StartTextInput()`.
    pub(crate) fn x11_start_text_input(
        &self,
        window: WindowID,
        _props: Option<&Properties>,
    ) -> Result<()> {
        self.x11_reset_xim(window);

        self.x11_update_text_input_area(window)
    }

    /// Translation of `X11_StopTextInput()`.
    pub(crate) fn x11_stop_text_input(&self, window: WindowID) -> Result<()> {
        self.x11_reset_xim(window);
        Ok(())
    }

    /// Translation of `X11_UpdateTextInputArea()`.
    pub(crate) fn x11_update_text_input_area(&self, window: WindowID) -> Result<()> {
        let Some(utf8) = &self.x.utf8 else {
            return Ok(());
        };
        let _ = with_x11_window(window, |w, data| {
            if !data.ic.is_null() {
                let spot = XPoint {
                    x: (w.text_input_rect.x + w.text_input_cursor) as i16,
                    y: (w.text_input_rect.y + w.text_input_rect.h) as i16,
                };
                if spot.x != data.xim_spot.x || spot.y != data.xim_spot.y {
                    // SAFETY: the list ends with NULL; the input context is
                    // the window's; the nested list is freed after use.
                    unsafe {
                        let attr = (utf8.XVaCreateNestedList)(
                            0,
                            XNSpotLocation.as_ptr(),
                            &spot as *const XPoint,
                            std::ptr::null::<c_char>(),
                        );
                        if !attr.is_null() {
                            (utf8.XSetICValues)(
                                data.ic,
                                XNPreeditAttributes.as_ptr(),
                                attr,
                                std::ptr::null::<c_char>(),
                            );
                            (self.x.XFree)(attr);
                        }
                    }
                    data.xim_spot = spot;
                }
            }
        });
        Ok(())
    }

    /// Translation of `X11_HasScreenKeyboardSupport()`.
    pub(crate) fn x11_has_screen_keyboard_support(&self) -> bool {
        self.use_steam_screen_keyboard
    }

    /// Translation of `X11_ShowScreenKeyboard()`.
    pub(crate) fn x11_show_screen_keyboard(&self, window: WindowID, props: Option<&Properties>) {
        if self.use_steam_screen_keyboard {
            /* For more documentation of the URL parameters, see:
             * https://partner.steamgames.com/doc/api/ISteamUtils#ShowFloatingGamepadTextInput
             */
            const K_E_FLOATING_GAMEPAD_TEXT_INPUT_MODE_MODE_SINGLE_LINE: i32 = 0; // Enter dismisses the keyboard
            const K_E_FLOATING_GAMEPAD_TEXT_INPUT_MODE_MODE_MULTIPLE_LINES: i32 = 1; // User needs to explicitly dismiss the keyboard
            const K_E_FLOATING_GAMEPAD_TEXT_INPUT_MODE_MODE_EMAIL: i32 = 2; // Keyboard is displayed in a special mode that makes it easier to enter emails
            const K_E_FLOATING_GAMEPAD_TEXT_INPUT_MODE_MODE_NUMERIC: i32 = 3; // Numeric keypad is shown

            let mode = match text_input_type(props) {
                TextInputType::TextEmail => K_E_FLOATING_GAMEPAD_TEXT_INPUT_MODE_MODE_EMAIL,
                TextInputType::Number
                | TextInputType::NumberPasswordHidden
                | TextInputType::NumberPasswordVisible => {
                    K_E_FLOATING_GAMEPAD_TEXT_INPUT_MODE_MODE_NUMERIC
                }
                _ => {
                    if text_input_multiline(props) {
                        K_E_FLOATING_GAMEPAD_TEXT_INPUT_MODE_MODE_MULTIPLE_LINES
                    } else {
                        K_E_FLOATING_GAMEPAD_TEXT_INPUT_MODE_MODE_SINGLE_LINE
                    }
                }
            };
            let rect = with_window(window, |w| w.text_input_rect).unwrap_or_default();
            let mut deeplink = format!(
                "steam://open/keyboard?XPosition={}&YPosition={}&Width={}&Height={}&Mode={}",
                rect.x, rect.y, rect.w, rect.h, mode
            );
            // (SDL_snprintf into a 128-byte buffer)
            deeplink.truncate(127);
            let _ = crate::misc::open_url(&deeplink);
            send_screen_keyboard_shown();
        }
    }

    /// Translation of `X11_HideScreenKeyboard()`.
    pub(crate) fn x11_hide_screen_keyboard(&self, _window: WindowID) {
        if self.use_steam_screen_keyboard {
            let _ = crate::misc::open_url("steam://close/keyboard");
            send_screen_keyboard_hidden();
        }
    }
}

/// Translation of `X11_AddKeymapEntry()`.
fn x11_add_keymap_entry(
    keymap: &mut Keymap,
    xkeycode: u32,
    xkeysym: KeySym,
    sdl_scancode: Scancode,
    sdl_mod_mask: Keymod,
) {
    let mut keycode = get_key_code_from_key_sym(xkeysym as u32, xkeycode, sdl_mod_mask);

    if keycode.0 == 0 {
        keycode = match sdl_scancode {
            Scancode::RETURN => Keycode::RETURN,
            Scancode::ESCAPE => Keycode::ESCAPE,
            Scancode::BACKSPACE => Keycode::BACKSPACE,
            Scancode::DELETE => Keycode::DELETE,
            _ => Keycode::from_scancode(sdl_scancode),
        };
    }

    keymap.set_entry(sdl_scancode, sdl_mod_mask, keycode);
}

/// The data part of `X11_ClearComposition()`: whether an empty editing
/// event has to be sent.
fn clear_composition(data: &mut X11WindowData) -> bool {
    if data.preedit_length > 0 {
        if let Some(text) = &mut data.preedit_text {
            text.clear();
        }
        data.preedit_length = 0;
    }

    if data.ime_needs_clear_composition {
        data.ime_needs_clear_composition = false;
        return true;
    }
    false
}

/// What `X11_SendEditingEvent()` sends.
enum EditingEvent {
    Clear,
    Text(String, i32, i32),
}

/// The data part of `X11_SendEditingEvent()`.
fn editing_event(data: &mut X11WindowData) -> EditingEvent {
    if data.preedit_length == 0 {
        return EditingEvent::Clear;
    }

    let mut in_highlight = false;
    let mut start: i32 = -1;
    let mut length: i32 = 0;
    let mut i: i32 = 0;
    while i < data.preedit_length {
        let feedback = data.preedit_feedback.get(i as usize).copied().unwrap_or(0);
        if feedback & (XIMReverse | XIMHighlight) != 0 {
            if start < 0 {
                start = i;
                in_highlight = true;
            }
        } else if in_highlight {
            // Found the end of the highlight
            break;
        }
        i += 1;
    }
    if in_highlight {
        length = i - start;
    } else {
        start = data.preedit_cursor.clamp(0, data.preedit_length);
    }
    data.ime_needs_clear_composition = true;
    EditingEvent::Text(data.preedit_text.clone().unwrap_or_default(), start, length)
}

/// Translation of `X11_SendEditingEvent()` for a window.
fn x11_send_editing_event(window: WindowID) {
    match with_x11_window(window, |_, data| editing_event(data)) {
        Ok(EditingEvent::Clear) => {
            if with_x11_window(window, |_, data| clear_composition(data)).unwrap_or(false) {
                keyboard::send_editing_text("", 0, 0);
            }
        }
        Ok(EditingEvent::Text(text, start, length)) => {
            keyboard::send_editing_text(&text, start, length)
        }
        Err(_) => {}
    }
}

/// The byte offset of character `n` of `s` (its end if shorter): stepping
/// with `SDL_StepUTF8()`.
fn char_offset(s: &str, n: i32) -> usize {
    s.char_indices()
        .nth(n.max(0) as usize)
        .map_or(s.len(), |(i, _)| i)
}

/// Translation of `im_instantiate_handler()`.
unsafe extern "C" fn im_instantiate_handler(
    _display: *mut Display,
    client_data: XPointer,
    _call_data: XPointer,
) {
    // SAFETY: the client data is the device that registered the callback,
    // alive until it unregisters it.
    let videodata = unsafe { &*(client_data as *const X11Video) };

    if videodata.x11_init_im() {
        if let Some(utf8) = &videodata.x.utf8 {
            // SAFETY: as registered.
            unsafe {
                (utf8.XUnregisterIMInstantiateCallback)(
                    videodata.display,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    Some(im_instantiate_handler),
                    client_data,
                );
            }
        }
    }
}

/// Translation of `im_destroyed_handler()`.
unsafe extern "C" fn im_destroyed_handler(_im: XIM, client_data: XPointer, _call_data: XPointer) {
    // SAFETY: the client data is the device that opened the input method.
    let videodata = unsafe { &*(client_data as *const X11Video) };

    videodata.with_data(|d| d.im = std::ptr::null_mut());
    if let Some(utf8) = &videodata.x.utf8 {
        // SAFETY: the handler is a valid extern fn; the device unregisters it
        // in X11_QuitKeyboard().
        unsafe {
            (utf8.XRegisterIMInstantiateCallback)(
                videodata.display,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                Some(im_instantiate_handler),
                client_data,
            );
        }
    }
}

/// Translation of `preedit_start_callback()`.
extern "C" fn preedit_start_callback(
    _xic: XIC,
    _client_data: XPointer,
    _call_data: XPointer,
) -> c_int {
    // No limit on preedit text length
    -1
}

/// [`preedit_start_callback`] as an `XIMProc`, as upstream casts it (Xlib
/// calls the start callback as returning an `int`).
fn preedit_start_callback_proc() -> XIMProc {
    type StartProc = extern "C" fn(XIC, XPointer, XPointer) -> c_int;
    // SAFETY: both are C function pointers taking three pointers; Xlib
    // calls this one with the start callback's signature.
    Some(unsafe {
        std::mem::transmute::<StartProc, unsafe extern "C" fn(XIM, XPointer, XPointer)>(
            preedit_start_callback,
        )
    })
}

/// Translation of `preedit_done_callback()`.
unsafe extern "C" fn preedit_done_callback(
    _xic: XIM,
    _client_data: XPointer,
    _call_data: XPointer,
) {
}

/// Translation of `preedit_draw_callback()`.
fn preedit_draw_callback(window: WindowID, call_data: &mut XIMPreeditDrawCallbackStruct) {
    let updated = with_x11_window(window, |_, data| {
        let chg_first = call_data.chg_first.clamp(0, data.preedit_length);
        let chg_length = call_data
            .chg_length
            .clamp(0, data.preedit_length - chg_first);

        let mut start = data
            .preedit_text
            .as_deref()
            .map_or(0, |t| char_offset(t, chg_first));
        if chg_length > 0 {
            // Delete text in range
            if let Some(text) = &mut data.preedit_text {
                let end = start + char_offset(&text[start..], chg_length);

                if end > start {
                    text.replace_range(start..end, "");
                    // FIXME (upstream): chg_length is clamped so that this
                    // is never true: the feedback isn't moved.
                    if (chg_first + chg_length) > data.preedit_length {
                        let first = chg_first as usize;
                        let count = (data.preedit_length - chg_first - chg_length) as usize;
                        let from = first + chg_length as usize;
                        data.preedit_feedback.copy_within(from..from + count, first);
                    }
                }
                start = start.min(text.len());
            }
            data.preedit_length -= chg_length;
        }

        if !call_data.text.is_null() {
            // SAFETY: the input method passes a valid text.
            let text = unsafe { &mut *call_data.text };
            // Insert text in range
            crate::sdl_assert!(text.encoding_is_wchar == 0);

            // FIXME (upstream): a NULL multi-byte string is passed to
            // SDL_strlen(); it is taken as empty here.
            let string: &[u8] = if text.string.is_null() {
                &[]
            } else {
                // SAFETY: a NUL-terminated multi-byte string.
                unsafe { CStr::from_ptr(text.string) }.to_bytes()
            };
            let string = String::from_utf8_lossy(string).into_owned();

            // The text length isn't calculated as directed by the spec, recalculate it now
            if !text.string.is_null() {
                text.length = string.chars().count() as u16;
            }

            let old = data.preedit_text.as_deref().unwrap_or("");
            let at = start.min(old.len());
            let mut preedit_text = String::with_capacity(old.len() + string.len());
            preedit_text.push_str(&old[..at]);
            preedit_text.push_str(&string);
            preedit_text.push_str(&old[at..]);

            let text_feedback: &[XIMFeedback] = if text.feedback.is_null() {
                &[]
            } else {
                // SAFETY: the text has `length` feedback entries.
                unsafe { std::slice::from_raw_parts(text.feedback, text.length as usize) }
            };
            let pre_size = chg_first as usize;
            let post_size = (data.preedit_length as usize).saturating_sub(pre_size);
            let mut feedback: Vec<XIMFeedback> =
                Vec::with_capacity(data.preedit_length as usize + text.length as usize);
            feedback.extend_from_slice(
                &data.preedit_feedback[..pre_size.min(data.preedit_feedback.len())],
            );
            feedback.resize(pre_size, 0);
            feedback.extend_from_slice(text_feedback);
            // FIXME (upstream): a NULL feedback array is copied from; it
            // reads as no highlighting here.
            feedback.resize(pre_size + text.length as usize, 0);
            let post_end = (pre_size + post_size).min(data.preedit_feedback.len());
            if pre_size < post_end {
                feedback.extend_from_slice(&data.preedit_feedback[pre_size..post_end]);
            }

            data.preedit_text = Some(preedit_text);
            data.preedit_feedback = feedback;

            data.preedit_length += text.length as c_int;
        }

        data.preedit_cursor = call_data.caret;

        // (DEBUG_XIM logs the change and the pre-edit text)
    });

    if updated.is_ok() {
        x11_send_editing_event(window);
    }
}

/// [`preedit_draw_callback`] as an `XIMProc`.
unsafe extern "C" fn preedit_draw_callback_proc(
    _xic: XIM,
    client_data: XPointer,
    call_data: XPointer,
) {
    // SAFETY: the client data is the window's IcClientData (alive as long as
    // its input context); the call data is a draw callback structure.
    unsafe {
        let client = &*(client_data as *const IcClientData);
        preedit_draw_callback(
            client.window,
            &mut *(call_data as *mut XIMPreeditDrawCallbackStruct),
        );
    }
}

/// Translation of `preedit_caret_callback()`.
fn preedit_caret_callback(window: WindowID, call_data: &XIMPreeditCaretCallbackStruct) {
    match call_data.direction {
        XIMAbsolutePosition => {
            let changed = with_x11_window(window, |_, data| {
                if call_data.position != data.preedit_cursor {
                    data.preedit_cursor = call_data.position;
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
            if changed {
                x11_send_editing_event(window);
            }
        }
        XIMDontChange => {}
        _ => {
            // Not currently supported
        }
    }
}

/// [`preedit_caret_callback`] as an `XIMProc`.
unsafe extern "C" fn preedit_caret_callback_proc(
    _xic: XIM,
    client_data: XPointer,
    call_data: XPointer,
) {
    // SAFETY: as for preedit_draw_callback_proc(), with a caret structure.
    unsafe {
        let client = &*(client_data as *const IcClientData);
        preedit_caret_callback(
            client.window,
            &*(call_data as *const XIMPreeditCaretCallbackStruct),
        );
    }
}

/// Translation of `ic_destroyed_handler()`.
unsafe extern "C" fn ic_destroyed_handler(_ic: XIM, client_data: XPointer, _call_data: XPointer) {
    // SAFETY: the client data is the window's IcClientData; its device
    // outlives the window.
    let client = unsafe { &*(client_data as *const IcClientData) };
    // SAFETY: as above.
    let videodata = unsafe { &*client.video };
    let _ = with_x11_window(client.window, |_, data| {
        data.ic = std::ptr::null_mut();
        videodata.x11_destroy_input_context(data);

        // If the IM was already recreated, create a new IC.
        if !videodata.with_data(|d| d.im).is_null() {
            videodata.x11_create_input_context(data);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remappable() {
        assert!(!x11_scancode_is_remappable(Scancode::LSHIFT));
        assert!(x11_scancode_is_remappable(Scancode::A));
    }

    #[test]
    fn char_offsets() {
        assert_eq!(char_offset("aé b", 0), 0);
        assert_eq!(char_offset("aé b", 2), 3);
        assert_eq!(char_offset("aé b", 9), 5);
    }
}
