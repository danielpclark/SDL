// Rust translation of src/video/x11/SDL_x11events.c and SDL_x11events.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Reading and dispatching X events: window state changes from the window
//! manager, focus, keyboard and mouse input (for the core protocol; XInput2
//! has `xinput2.rs`), the clipboard and drag and drop protocols, and waiting
//! for events.
//!
//! The dispatcher works on the window data in short steps: each step reads
//! or updates the window (and its [`X11WindowData`]) and the SDL calls
//! (window events, mouse and keyboard events) come between them, with
//! nothing borrowed.

use std::ffi::{c_char, c_int, c_long, c_uchar, c_uint, c_ulong, c_void, CStr};
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::keyboard::KeyboardData;
use super::modes::with_x11_display;
use super::sys::*;
use super::video::{atom_name, X11Video};
use super::window::{
    with_x11_window, x11_read_property, PendingFocus, PENDING_FOCUS_TIME,
    X11_PENDING_OP_FULLSCREEN, X11_PENDING_OP_MAXIMIZE, X11_PENDING_OP_MINIMIZE,
    X11_PENDING_OP_MOVE, X11_PENDING_OP_RESIZE, X11_PENDING_OP_RESTORE,
    X11_SIZE_MOVE_EVENTS_WAIT_FOR_BORDERS,
};
use super::x11dyn::X11Syms;
use super::xfixes::{x11_get_xfixes_selection_notify_event, X11_BARRIER_HANDLED_BY_EVENT};
use crate::core::unix::{io_ready, IoReadyFlags};
use crate::error::{Error, Result};
use crate::events::keyboard::{self, Keycode, Keymod, Scancode};
use crate::events::mouse::{self, MouseID, MouseWheelDirection};
use crate::events::window::{
    send_clipboard_update, send_display_event, send_drop_complete, send_drop_file,
    send_drop_position, send_drop_text, send_window_event, WindowFlags,
};
use crate::events::{EventType, KeyboardID, WindowID};
use crate::hints;
use crate::video::core::{update_fullscreen_mode, update_window_grab, with_window};
use crate::video::sysvideo::{DisplayMode, FlashOperation, FullscreenOp, HitTestResult};
use crate::video::Point;

const _NET_WM_MOVERESIZE_SIZE_TOPLEFT: c_long = 0;
const _NET_WM_MOVERESIZE_SIZE_TOP: c_long = 1;
const _NET_WM_MOVERESIZE_SIZE_TOPRIGHT: c_long = 2;
const _NET_WM_MOVERESIZE_SIZE_RIGHT: c_long = 3;
const _NET_WM_MOVERESIZE_SIZE_BOTTOMRIGHT: c_long = 4;
const _NET_WM_MOVERESIZE_SIZE_BOTTOM: c_long = 5;
const _NET_WM_MOVERESIZE_SIZE_BOTTOMLEFT: c_long = 6;
const _NET_WM_MOVERESIZE_SIZE_LEFT: c_long = 7;
const _NET_WM_MOVERESIZE_MOVE: c_long = 8;

/// The atoms of a list of targets (`(Atom *)p.data`, `p.count`).
///
/// # Safety
///
/// `data` must hold `count` atoms (or be NULL).
unsafe fn atom_list<'a>(data: *const c_uchar, count: c_ulong) -> &'a [Atom] {
    if data.is_null() || count == 0 {
        return &[];
    }
    // SAFETY: the caller's contract; format-32 property data is an array of
    // longs (atoms), aligned by Xlib.
    unsafe { std::slice::from_raw_parts(data as *const Atom, count as usize) }
}

/// Find text-uri-list in a list of targets and return it's atom
/// if available, else return None. Translation of `X11_PickTarget()`.
fn x11_pick_target(x: &X11Syms, disp: *mut Display, list: &[Atom]) -> Atom {
    let text_uri_request = super::video::intern_atom(x, disp, "text/uri-list", false);
    let mut request: Atom = None;
    let mut preferred: Atom = None;

    for &atom in list {
        if request == text_uri_request {
            break;
        }
        let name = atom_name(x, disp, atom).unwrap_or_default();
        // Preferred MIME targets
        if name == "text/uri-list" || name == "text/plain;charset=utf-8" || name == "UTF8_STRING" {
            if preferred == None {
                preferred = atom;
            }
            request = atom;
        }
        // Fallback MIME targets
        if (name == "text/plain" || name == "TEXT") && request == None {
            request = atom;
        }
    }

    // The type 'text/uri-list' is preferred over all others.
    if preferred != None && request != text_uri_request {
        request = preferred;
    }
    request
}

/// Wrapper for X11_PickTarget for a maximum of three targets, a special
/// case in the Xdnd protocol. Translation of `X11_PickTargetFromAtoms()`.
fn x11_pick_target_from_atoms(
    x: &X11Syms,
    disp: *mut Display,
    a0: Atom,
    a1: Atom,
    a2: Atom,
) -> Atom {
    let mut atom = Vec::with_capacity(3);
    if a0 != None {
        atom.push(a0);
    }
    if a1 != None {
        atom.push(a1);
    }
    if a2 != None {
        atom.push(a2);
    }
    x11_pick_target(x, disp, &atom)
}

/// Translation of `struct KeyRepeatCheckData`.
struct KeyRepeatCheckData {
    event: *const XEvent,
    found: bool,
}

/// Translation of `X11_KeyRepeatCheckIfEvent()`.
unsafe extern "C" fn x11_key_repeat_check_if_event(
    _display: *mut Display,
    chkev: *mut XEvent,
    arg: XPointer,
) -> Bool {
    // SAFETY: `arg` is the KeyRepeatCheckData of x11_key_repeat(), whose
    // event outlives the call; Xlib passes a valid event.
    unsafe {
        let d = &mut *(arg as *mut KeyRepeatCheckData);
        let chkev = &*chkev;
        let event = &*d.event;
        if chkev.get_type() == KeyPress
            && chkev.key().keycode == event.key().keycode
            && chkev.key().time.wrapping_sub(event.key().time) < 2
        {
            d.found = true;
        }
    }
    False
}

/// Check to see if this is a repeated key.
/// (idea shamelessly lifted from GII -- thanks guys! :)
/// Translation of `X11_KeyRepeat()`.
fn x11_key_repeat(x: &X11Syms, display: *mut Display, event: &XEvent) -> bool {
    let mut dummyev = XEvent::zeroed();
    let mut d = KeyRepeatCheckData {
        event,
        found: false,
    };
    // SAFETY: the display is open; the predicate gets `d`, alive for the
    // call.
    unsafe {
        if (x.XPending)(display) != 0 {
            (x.XCheckIfEvent)(
                display,
                &mut dummyev,
                Some(x11_key_repeat_check_if_event),
                &mut d as *mut KeyRepeatCheckData as XPointer,
            );
        }
    }
    d.found
}

/// Whether an X button is a wheel button, setting the ticks it scrolls.
/// Translation of `X11_IsWheelEvent()`.
pub(crate) fn x11_is_wheel_event(button: c_int, xticks: &mut c_int, yticks: &mut c_int) -> bool {
    /* according to the xlib docs, no specific mouse wheel events exist.
    However, the defacto standard is that the vertical wheel is X buttons
    4 (up) and 5 (down) and a horizontal wheel is 6 (left) and 7 (right). */

    // Xlib defines "Button1" through 5, so we just use literals here.
    match button {
        4 => {
            *yticks = 1;
            true
        }
        5 => {
            *yticks = -1;
            true
        }
        6 => {
            *xticks = 1;
            true
        }
        7 => {
            *xticks = -1;
            true
        }
        _ => false,
    }
}

/// A callback that sees every X event (an `XEvent *`) before SDL
/// processes it; returning `false` drops the event. Translation of
/// `SDL_X11EventHook` (the closure replaces its `userdata`).
pub type X11EventHook = Arc<dyn Fn(*mut c_void) -> bool + Send + Sync>;

// An X11 event hook
static G_X11_EVENT_HOOK: Mutex<Option<X11EventHook>> = Mutex::new(Option::None);

/// Set (or clear, with `None`) a callback for every X event, called with
/// the `XEvent *` before SDL processes it; it returns `false` to have SDL
/// ignore the event. Translation of `SDL_SetX11EventHook()`.
pub fn set_x11_event_hook(callback: Option<X11EventHook>) {
    *G_X11_EVENT_HOOK.lock().unwrap_or_else(|e| e.into_inner()) = callback;
}

/// The current event hook (cloned, so it is called with nothing locked).
fn event_hook() -> Option<X11EventHook> {
    G_X11_EVENT_HOOK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Translation of `X11_UpdateSystemKeyModifiers()`.
fn x11_update_system_key_modifiers(x: &X11Syms, display: *mut Display, kb: &mut KeyboardData) {
    if kb.xkb_enabled {
        if let Some(xkb) = &x.xkb {
            // SAFETY: XkbStateRec is plain data.
            let mut xkb_state: XkbStateRec = unsafe { std::mem::zeroed() };
            // SAFETY: the display is open; the state is an out-parameter.
            if unsafe { (xkb.XkbGetState)(display, XkbUseCoreKbd, &mut xkb_state) } == Success {
                kb.pressed_modifiers = xkb_state.base_mods as u32;
                kb.locked_modifiers = (xkb_state.latched_mods | xkb_state.locked_mods) as u32;
            }
        }
    } else {
        let mut junk_window: Window = 0;
        let mut xx: c_int = 0;
        let mut yy: c_int = 0;
        let mut mod_mask: c_uint = 0;

        // SAFETY: the display is open; the out-parameters are valid (the
        // window and coordinate pairs alias, as upstream).
        unsafe {
            let jw: *mut Window = &mut junk_window;
            let px: *mut c_int = &mut xx;
            let py: *mut c_int = &mut yy;
            (x.XQueryPointer)(
                display,
                DefaultRootWindow(display),
                jw,
                jw,
                px,
                py,
                px,
                py,
                &mut mod_mask,
            );
        }
        kb.pressed_modifiers =
            mod_mask & (ShiftMask | ControlMask | Mod1Mask | Mod3Mask | Mod4Mask | Mod5Mask);
        kb.locked_modifiers = mod_mask & (LockMask | kb.numlock_mask | kb.scrolllock_mask);
    }
}

/// Translation of `X11_ReconcileModifiers()`: returns the modifier state
/// to pass to `SDL_SetModState()` (called by the caller, with nothing
/// borrowed).
fn x11_reconcile_modifiers(kb: &mut KeyboardData, key_pressed: bool) -> Keymod {
    /* Handle explicit pressed modifier state. This will correct the modifier state
     * if common modifier keys were remapped and the modifiers presumed to be set
     * during a key press event were incorrect, if the modifier was set to the
     * pressed state via means other than pressing the physical key, or if the
     * modifier state was set by a keypress before the corresponding key event
     * was received.
     */
    let shift = ShiftMask;
    let control = ControlMask;
    if key_pressed {
        for (mask, kmod) in [
            (shift, Keymod::SHIFT),
            (control, Keymod::CTRL),
            (kb.alt_mask, Keymod::ALT),
            (kb.gui_mask, Keymod::GUI),
        ] {
            if kb.pressed_modifiers & mask != 0
                && kb.sdl_physically_pressed_modifiers.intersects(kmod)
            {
                kb.sdl_pressed_modifiers &= !kmod;
                kb.sdl_pressed_modifiers |= kb.sdl_physically_pressed_modifiers & kmod;
            }
        }
    } else {
        for (mask, kmod) in [
            (shift, Keymod::SHIFT),
            (control, Keymod::CTRL),
            (kb.alt_mask, Keymod::ALT),
            (kb.gui_mask, Keymod::GUI),
            (kb.level3_mask, Keymod::MODE),
            (kb.level5_mask, Keymod::LEVEL5),
        ] {
            if kb.pressed_modifiers & mask != 0 {
                if !kb.sdl_pressed_modifiers.intersects(kmod) {
                    kb.sdl_pressed_modifiers |= kmod;
                }
            } else {
                kb.sdl_pressed_modifiers &= !kmod;
            }
        }
    }

    /* If a latch or lock was activated by a keypress, the latch/lock will
     * be tied to the specific left/right key that initiated it. Otherwise,
     * the ambiguous left/right combo is used.
     *
     * The modifier will remain active until the latch/lock is released by
     * the system.
     */
    for (mask, kmod) in [
        (shift, Keymod::SHIFT),
        (control, Keymod::CTRL),
        (kb.alt_mask, Keymod::ALT),
        (kb.gui_mask, Keymod::GUI),
    ] {
        if kb.locked_modifiers & mask != 0 {
            if kb.sdl_pressed_modifiers.intersects(kmod) {
                kb.sdl_locked_modifiers &= !kmod;
                kb.sdl_locked_modifiers |= kb.sdl_pressed_modifiers & kmod;
            } else if !kb.sdl_locked_modifiers.intersects(kmod) {
                kb.sdl_locked_modifiers |= kmod;
            }
        } else {
            kb.sdl_locked_modifiers &= !kmod;
        }
    }

    // (the level 3 and level 5 shifts, then the locks:)
    // Capslock, Numlock, and Scrolllock can only be locked, not pressed.
    for (mask, kmod) in [
        (kb.level3_mask, Keymod::MODE),
        (kb.level5_mask, Keymod::LEVEL5),
        (LockMask, Keymod::CAPS),
        (kb.numlock_mask, Keymod::NUM),
        (kb.scrolllock_mask, Keymod::SCROLL),
    ] {
        if kb.locked_modifiers & mask != 0 {
            kb.sdl_locked_modifiers |= kmod;
        } else {
            kb.sdl_locked_modifiers &= !kmod;
        }
    }

    kb.sdl_pressed_modifiers | kb.sdl_locked_modifiers
}

/// Translation of `X11_HandleModifierKeys()`, given the key code of the
/// scancode (`SDL_GetKeyFromScancode(scancode, SDL_KMOD_NONE, false)`):
/// returns the modifier state to set, if any.
fn x11_handle_modifier_keys(
    x: &X11Syms,
    display: *mut Display,
    kb: &mut KeyboardData,
    keycode: Keycode,
    pressed: bool,
) -> Option<Keymod> {
    /* SDL clients expect modifier state to be activated at the same time as the
     * source keypress, so we set pressed modifier state with the usual modifier
     * keys here, as the explicit modifier event won't arrive until after the
     * keypress event. If this is wrong, it will be corrected when the explicit
     * modifier state is checked.
     */
    let kmod = match keycode {
        Keycode::LSHIFT => Keymod::LSHIFT,
        Keycode::RSHIFT => Keymod::RSHIFT,
        Keycode::LCTRL => Keymod::LCTRL,
        Keycode::RCTRL => Keymod::RCTRL,
        Keycode::LALT => Keymod::LALT,
        Keycode::RALT => Keymod::RALT,
        Keycode::LGUI => Keymod::LGUI,
        Keycode::RGUI => Keymod::RGUI,
        Keycode::MODE => Keymod::MODE,
        Keycode::LEVEL5_SHIFT => Keymod::LEVEL5,
        Keycode::CAPSLOCK | Keycode::NUMLOCKCLEAR | Keycode::SCROLLLOCK => {
            // XKB provides the latched/locked state explicitly.
            if kb.xkb_enabled {
                /* For locking modifier keys, query the lock state directly, or we may have to wait until the next
                 * key press event to know if a lock was actually activated from the key event.
                 */
                let mut cur_mask = kb.locked_modifiers;
                x11_update_system_key_modifiers(x, display, kb);

                for mask in [LockMask, kb.numlock_mask, kb.scrolllock_mask] {
                    if kb.locked_modifiers & mask != 0 {
                        cur_mask |= mask;
                    } else {
                        cur_mask &= !mask;
                    }
                }

                kb.locked_modifiers = cur_mask;
            }
            Keymod::NONE
        }
        _ => return Option::None,
    };

    if pressed {
        kb.sdl_pressed_modifiers |= kmod;
        kb.sdl_physically_pressed_modifiers |= kmod;
    } else {
        kb.sdl_pressed_modifiers &= !kmod;
        kb.sdl_physically_pressed_modifiers &= !kmod;
    }

    Some(x11_reconcile_modifiers(kb, true))
}

/// The last error code seen by `BadWindowErrorHandler()` (`x11_last_error`).
static X11_LAST_ERROR: AtomicU8 = AtomicU8::new(0);

/// Translation of `BadWindowErrorHandler()`.
unsafe extern "C" fn bad_window_error_handler(d: *mut Display, e: *mut XErrorEvent) -> c_int {
    // SAFETY: Xlib passes a valid error event.
    let e = unsafe { &*e };
    X11_LAST_ERROR.store(e.error_code, Ordering::Relaxed);

    // Ignore BadWindow in cases where it's not fatal.
    if e.error_code as c_int != BadWindow as c_int {
        let mut err_msg = [0 as c_char; 128];
        if let Some(x) = super::x11dyn::loaded_symbols() {
            // SAFETY: the display is the erroring one; the buffer's size is
            // passed.
            unsafe {
                (x.XGetErrorText)(
                    d,
                    e.error_code as c_int,
                    err_msg.as_mut_ptr(),
                    err_msg.len() as c_int,
                );
            }
        }
        // SAFETY: XGetErrorText NUL-terminates (the buffer starts zeroed).
        let msg = unsafe { CStr::from_ptr(err_msg.as_ptr()) }.to_string_lossy();
        crate::error!(
            crate::log::Category::Video,
            "X failed request: {} ({}), major opcode: {}",
            e.error_code,
            msg,
            e.request_code
        );
    }
    0
}

/// Translation of `isMapNotify()` (of `SDL_x11events.c`): a MapNotify for
/// the unmap event's window and serial.
unsafe extern "C" fn is_map_notify(_display: *mut Display, ev: *mut XEvent, arg: XPointer) -> Bool {
    // SAFETY: `arg` is the XUnmapEvent being handled; Xlib passes a valid
    // event.
    unsafe {
        let unmap = &*(arg as *const XUnmapEvent);
        let ev = &*ev;
        (ev.get_type() == MapNotify
            && ev.map().window == unmap.window
            && ev.map().serial == unmap.serial) as Bool
    }
}

/// Translation of `isReparentNotify()`.
unsafe extern "C" fn is_reparent_notify(
    _display: *mut Display,
    ev: *mut XEvent,
    arg: XPointer,
) -> Bool {
    // SAFETY: as above.
    unsafe {
        let unmap = &*(arg as *const XUnmapEvent);
        let ev = &*ev;
        (ev.get_type() == ReparentNotify
            && ev.reparent().window == unmap.window
            && ev.reparent().serial == unmap.serial) as Bool
    }
}

/// Translation of `IsHighLatin1()`.
fn is_high_latin1(string: &[u8]) -> bool {
    string.iter().any(|&ch| ch >= 0x80)
}

/// Translation of `XLookupStringAsUTF8()`: the text (as UTF-8) and the
/// keysym of a key event.
fn x_lookup_string_as_utf8(
    x: &X11Syms,
    event_struct: &mut XKeyEvent,
    keysym_return: &mut KeySym,
) -> Vec<u8> {
    let mut buffer_return = [0 as c_char; 64];
    // SAFETY: the event is a key event; the buffer's size (less the room
    // for the terminator) is passed.
    let result = unsafe {
        (x.XLookupString)(
            event_struct,
            buffer_return.as_mut_ptr(),
            (buffer_return.len() - 1) as c_int,
            keysym_return,
            std::ptr::null_mut(),
        )
    };
    let len = result.clamp(0, (buffer_return.len() - 1) as c_int) as usize;
    let bytes: Vec<u8> = buffer_return[..len].iter().map(|&c| c as u8).collect();
    if is_high_latin1(&bytes) {
        match crate::stdlib::iconv::iconv_string("UTF-8", "ISO-8859-1", &bytes) {
            Ok(mut utf8_text) => {
                // (SDL_strlcpy into the 63-byte buffer)
                let end = utf8_text
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(utf8_text.len());
                utf8_text.truncate(end.min(buffer_return.len() - 2));
                utf8_text
            }
            Err(_) => Vec::new(),
        }
    } else {
        bytes
    }
}

/// Translation of `X11_GetEventTimestamp()`.
pub(crate) fn x11_get_event_timestamp(_time: c_ulong) -> Duration {
    // FIXME: Get the event time in the SDL tick time base
    Duration::from_nanos(crate::timer::ticks_ns())
}

impl X11Video {
    /// The SDL window of an X window. Translation of `X11_FindWindow()`.
    pub(crate) fn x11_find_window(&self, window: Window) -> Option<WindowID> {
        self.with_data(|d| {
            d.windowlist
                .iter()
                .find(|e| e.xwindow == window)
                .map(|e| e.window)
        })
    }

    /// Translation of `X11_HandleGenericEvent()`.
    fn x11_handle_generic_event(&self, xev: &mut XEvent) {
        let Some(xkb) = &self.x.xkb else {
            return;
        };
        // event is a union, so cookie == &event, but this is type safe.
        let cookie: *mut XGenericEventCookie = xev.cookie_mut();
        // SAFETY: the display is open; the cookie is the event's.
        if unsafe { (xkb.XGetEventData)(self.display, cookie) } != 0 {
            let hook = event_hook();
            if hook.is_none_or_true(xev) {
                // SAFETY: the cookie's data is valid until XFreeEventData.
                self.x11_handle_xinput2_event(unsafe { &*cookie });
            }
            // SAFETY: as above.
            unsafe {
                (xkb.XFreeEventData)(self.display, cookie);
            }
        }
    }

    /// `X11_ReconcileModifiers()` on the device's keyboard state.
    fn reconcile_modifiers(&self, key_pressed: bool) {
        let modstate = self.with_data(|d| x11_reconcile_modifiers(&mut d.keyboard, key_pressed));
        keyboard::set_mod_state(modstate);
    }

    /// `X11_HandleModifierKeys()` on the device's keyboard state.
    fn handle_modifier_keys(&self, scancode: Scancode, pressed: bool) {
        let keycode = keyboard::key_from_scancode(scancode, Keymod::NONE, false);
        let modstate = self.with_data(|d| {
            x11_handle_modifier_keys(&self.x, self.display, &mut d.keyboard, keycode, pressed)
        });
        if let Some(modstate) = modstate {
            keyboard::set_mod_state(modstate);
        }
    }

    /// Translation of `X11_ReconcileKeyboardState()`.
    pub(crate) fn x11_reconcile_keyboard_state(&self) {
        let display = self.display;
        let mut keys = [0 as c_char; 32];

        // Rebuild the modifier state in case it changed while focus was lost.
        self.with_data(|d| x11_update_system_key_modifiers(&self.x, display, &mut d.keyboard));
        self.reconcile_modifiers(false);

        // Keep caps, num, and scroll, but clear the others until we have updated key state.
        let key_layout = self.with_data(|d| {
            let kb = &mut d.keyboard;
            kb.sdl_pressed_modifiers = Keymod::NONE;
            kb.sdl_physically_pressed_modifiers = Keymod::NONE;
            kb.sdl_locked_modifiers &= Keymod::CAPS | Keymod::NUM | Keymod::SCROLL;
            kb.pressed_modifiers = 0;
            kb.locked_modifiers &= LockMask | kb.numlock_mask | kb.scrolllock_mask;
            kb.key_layout
        });

        // SAFETY: the display is open; the buffer has the 32 bytes XQueryKeymap writes.
        unsafe {
            (self.x.XQueryKeymap)(display, keys.as_mut_ptr());
        }

        let keystate = keyboard::keyboard_state();
        for (keycode, &scancode) in key_layout.iter().enumerate() {
            let x11_key_pressed = (keys[keycode / 8] as u8 & (1 << (keycode % 8))) != 0;
            let sdl_key_pressed = keystate[scancode.0 as usize];

            if x11_key_pressed && !sdl_key_pressed {
                // Only update modifier state for keys that are pressed in another application
                match keyboard::key_from_scancode(scancode, Keymod::NONE, false) {
                    Keycode::LCTRL
                    | Keycode::RCTRL
                    | Keycode::LSHIFT
                    | Keycode::RSHIFT
                    | Keycode::LALT
                    | Keycode::RALT
                    | Keycode::LGUI
                    | Keycode::RGUI
                    | Keycode::MODE
                    | Keycode::LEVEL5_SHIFT => {
                        self.handle_modifier_keys(scancode, true);
                        keyboard::send_keyboard_key_ignore_modifiers(
                            Duration::ZERO,
                            keyboard::GLOBAL_KEYBOARD_ID,
                            keycode as i32,
                            scancode,
                            true,
                        );
                    }
                    _ => {}
                }
            } else if !x11_key_pressed && sdl_key_pressed {
                self.handle_modifier_keys(scancode, false);
                keyboard::send_keyboard_key_ignore_modifiers(
                    Duration::ZERO,
                    keyboard::GLOBAL_KEYBOARD_ID,
                    keycode as i32,
                    scancode,
                    false,
                );
            }
        }

        // Update the latched/locked state for modifiers other than Caps, Num, and Scroll lock.
        self.with_data(|d| x11_update_system_key_modifiers(&self.x, display, &mut d.keyboard));
        self.reconcile_modifiers(true);
    }

    /// Translation of `X11_DispatchFocusIn()`.
    fn x11_dispatch_focus_in(&self, window: WindowID) {
        // (DEBUG_XEVENTS: "window 0x%lx: Dispatching FocusIn")
        let _ = keyboard::set_keyboard_focus(Some(window));
        self.x11_reconcile_keyboard_state();
        let Ok((ic, flashing_window)) = with_x11_window(window, |_, d| (d.ic, d.flashing_window))
        else {
            return;
        };
        if !ic.is_null() {
            if let Some(utf8) = &self.x.utf8 {
                // SAFETY: the input context is the window's.
                unsafe {
                    (utf8.XSetICFocus)(ic);
                }
            }
        }
        if flashing_window {
            let _ = self.x11_flash_window(window, FlashOperation::Cancel);
        }
    }

    /// Translation of `X11_DispatchFocusOut()`.
    fn x11_dispatch_focus_out(&self, window: WindowID) {
        // (DEBUG_XEVENTS: "window 0x%lx: Dispatching FocusOut")
        /* If another window has already processed a focus in, then don't try to
         * remove focus here.  Doing so will incorrectly remove focus from that
         * window, and the focus lost event for this window will have already
         * been dispatched anyway. */
        if Some(window) == keyboard::keyboard_focus() {
            let _ = keyboard::set_keyboard_focus(Option::None);
        }
        let ic = with_x11_window(window, |_, d| d.ic).unwrap_or(std::ptr::null_mut());
        if !ic.is_null() {
            if let Some(utf8) = &self.x.utf8 {
                // SAFETY: the input context is the window's.
                unsafe {
                    (utf8.XUnsetICFocus)(ic);
                }
            }
        }
    }

    /// Translation of `X11_DispatchMapNotify()`.
    fn x11_dispatch_map_notify(&self, window: WindowID) {
        send_window_event(window, EventType::WINDOW_SHOWN, 0, 0);
        let _ = with_x11_window(window, |_, d| d.was_shown = true);

        // This may be sent when restoring a minimized window.
        let flags = with_window(window, |w| w.flags()).unwrap_or_default();
        if flags.contains(WindowFlags::MINIMIZED) {
            send_window_event(window, EventType::WINDOW_RESTORED, 0, 0);
            send_window_event(window, EventType::WINDOW_EXPOSED, 0, 0);
        }

        let flags = with_window(window, |w| w.flags()).unwrap_or_default();
        if flags.contains(WindowFlags::INPUT_FOCUS) {
            update_window_grab(window);
        }
    }

    /// Translation of `X11_DispatchUnmapNotify()`.
    fn x11_dispatch_unmap_notify(&self, window: WindowID) {
        // This may be sent when minimizing a window.
        let is_hiding = with_window(window, |w| w.is_hiding).unwrap_or(false);
        if !is_hiding {
            send_window_event(window, EventType::WINDOW_MINIMIZED, 0, 0);
            send_window_event(window, EventType::WINDOW_OCCLUDED, 0, 0);
        } else {
            send_window_event(window, EventType::WINDOW_HIDDEN, 0, 0);
        }
    }

    /// Send a `_NET_WM_MOVERESIZE` request (the common part of
    /// `DispatchWindowMove()` and `InitiateWindowResize()`).
    fn send_move_resize(&self, window: WindowID, point: &Point, direction: c_long) {
        let display = self.display;
        let Ok((wx, wy, xwindow)) = with_x11_window(window, |w, d| (w.core.x, w.core.y, d.xwindow))
        else {
            return;
        };
        let mut evt = XEvent::zeroed();

        // !!! FIXME: we need to regrab this if necessary when the drag is done.
        // SAFETY: the display is open.
        unsafe {
            (self.x.XUngrabPointer)(display, 0);
            (self.x.XFlush)(display);
        }

        {
            let c = evt.client_mut();
            c.type_ = ClientMessage;
            c.window = xwindow;
            c.message_type = self.atoms()._NET_WM_MOVERESIZE;
            c.format = 32;
            c.data.l[0] = (wx as usize).wrapping_add(point.x as usize) as c_long;
            c.data.l[1] = (wy as usize).wrapping_add(point.y as usize) as c_long;
            c.data.l[2] = direction;
            c.data.l[3] = Button1 as c_long;
            c.data.l[4] = 0;
        }
        // SAFETY: the display is open; the event is a client message.
        unsafe {
            (self.x.XSendEvent)(
                display,
                DefaultRootWindow(display),
                False,
                SubstructureRedirectMask | SubstructureNotifyMask,
                &mut evt,
            );

            (self.x.XSync)(display, 0);
        }
    }

    /// Translation of `DispatchWindowMove()`.
    fn dispatch_window_move(&self, window: WindowID, point: &Point) {
        self.send_move_resize(window, point, _NET_WM_MOVERESIZE_MOVE);
    }

    /// Translation of `ScheduleWindowMove()`.
    fn schedule_window_move(&self, window: WindowID, point: &Point) {
        let _ = with_x11_window(window, |_, data| {
            data.pending_move = true;
            data.pending_move_point = *point;
        });
    }

    /// Translation of `InitiateWindowResize()`.
    fn initiate_window_resize(&self, window: WindowID, point: &Point, direction: c_long) {
        if !(_NET_WM_MOVERESIZE_SIZE_TOPLEFT..=_NET_WM_MOVERESIZE_SIZE_LEFT).contains(&direction) {
            return;
        }
        self.send_move_resize(window, point, direction);
    }

    /// Run the window's hit test at a point, updating the cursor; `false`
    /// if the window has none. Translation of `X11_ProcessHitTest()`.
    pub(crate) fn x11_process_hit_test(
        &self,
        window: WindowID,
        x: f32,
        y: f32,
        force_new_result: bool,
    ) -> bool {
        let point = Point {
            x: x as i32,
            y: y as i32,
        };
        let Some(rc) = crate::video::window::Window::from_raw(window).hit_test(point) else {
            return false;
        };
        let Ok(previous) = with_x11_window(window, |_, d| d.hit_test_result) else {
            return false;
        };
        if !force_new_result && rc == previous {
            return true;
        }
        self.x11_set_hit_test_cursor(rc);
        let _ = with_x11_window(window, |_, d| d.hit_test_result = rc);
        true
    }

    /// Translation of `X11_TriggerHitTestAction()`.
    pub(crate) fn x11_trigger_hit_test_action(&self, window: WindowID, x: f32, y: f32) -> bool {
        let Ok((has_hit_test, flags, hit_test_result)) = with_x11_window(window, |w, d| {
            (w.hit_test.is_some(), w.flags(), d.hit_test_result)
        }) else {
            return false;
        };

        if has_hit_test {
            let point = Point {
                x: x as i32,
                y: y as i32,
            };
            const DIRECTIONS: [c_long; 8] = [
                _NET_WM_MOVERESIZE_SIZE_TOPLEFT,
                _NET_WM_MOVERESIZE_SIZE_TOP,
                _NET_WM_MOVERESIZE_SIZE_TOPRIGHT,
                _NET_WM_MOVERESIZE_SIZE_RIGHT,
                _NET_WM_MOVERESIZE_SIZE_BOTTOMRIGHT,
                _NET_WM_MOVERESIZE_SIZE_BOTTOM,
                _NET_WM_MOVERESIZE_SIZE_BOTTOMLEFT,
                _NET_WM_MOVERESIZE_SIZE_LEFT,
            ];

            return match hit_test_result {
                HitTestResult::Draggable => {
                    /* Some window managers get in a bad state when a move event starts while input is transitioning
                    to the SDL window. This can happen when clicking on a drag region of an unfocused window
                    where the same mouse down event will trigger a drag event and a window activate. */
                    if flags.contains(WindowFlags::INPUT_FOCUS) {
                        self.dispatch_window_move(window, &point);
                    } else {
                        self.schedule_window_move(window, &point);
                    }
                    true
                }

                HitTestResult::ResizeTopLeft
                | HitTestResult::ResizeTop
                | HitTestResult::ResizeTopRight
                | HitTestResult::ResizeRight
                | HitTestResult::ResizeBottomRight
                | HitTestResult::ResizeBottom
                | HitTestResult::ResizeBottomLeft
                | HitTestResult::ResizeLeft => {
                    let index = hit_test_result as usize - HitTestResult::ResizeTopLeft as usize;
                    self.initiate_window_resize(window, &point, DIRECTIONS[index]);
                    true
                }

                _ => false,
            };
        }

        false
    }

    /// Translation of `X11_UpdateUserTime()`.
    fn x11_update_user_time(&self, window: WindowID, latest: c_ulong) {
        let atom = self.atoms()._NET_WM_USER_TIME;
        let _ = with_x11_window(window, |_, data| {
            if latest != 0 && latest != data.user_time {
                let value: c_long = latest as c_long;
                // SAFETY: the display is open and the window ours; the value
                // is one long.
                unsafe {
                    (self.x.XChangeProperty)(
                        self.display,
                        data.xwindow,
                        atom,
                        XA_CARDINAL,
                        32,
                        PropModeReplace,
                        &value as *const c_long as *const c_uchar,
                        1,
                    );
                }
                // (DEBUG_XEVENTS: "window 0x%lx: updating _NET_WM_USER_TIME to %lu")
                data.user_time = latest;
            }
        });
    }

    /// Translation of `X11_HandleClipboardEvent()`.
    fn x11_handle_clipboard_event(&self, xevent: &XEvent) {
        let display = self.display;
        let x = &self.x;
        let atoms = self.atoms();

        crate::sdl_assert!(self.with_data(|d| d.clipboard_window) != None);
        crate::sdl_assert!(xevent.any().window == self.with_data(|d| d.clipboard_window));

        match xevent.get_type() {
            // Copy the selection from our own CUTBUFFER to the requested property
            SelectionRequest => {
                let req = *xevent.selectionrequest();
                let mut sevent = XEvent::zeroed();
                let xa_targets = atoms.TARGETS;

                // (DEBUG_XEVENTS: "window CLIPBOARD: SelectionRequest (requestor = 0x%lx, target = 0x%lx, mime_type = %s)")

                /* If the requesting window was already destroyed, XChangeProperty can generate a BadWindow
                 * error. Register an error handler to catch this, and prevent it from being fatal.
                 */
                X11_LAST_ERROR.store(0, Ordering::Relaxed);
                // SAFETY: the handler is a valid extern fn.
                let prev_handler = unsafe { (x.XSetErrorHandler)(Some(bad_window_error_handler)) };

                let (callback, mime_types) = self.with_data(|d| {
                    let clipboard = if req.selection == XA_PRIMARY {
                        &d.primary_selection
                    } else {
                        &d.clipboard
                    };
                    (clipboard.callback.clone(), clipboard.mime_types.clone())
                });

                {
                    let s = sevent.selection_mut();
                    s.type_ = SelectionNotify;
                    s.selection = req.selection;
                    s.target = None;
                    s.property = None; // tell them no by default
                    s.requestor = req.requestor;
                    s.time = req.time;
                }

                /* !!! FIXME: We were probably storing this on the root window
                because an SDL window might go away...? but we don't have to do
                this now (or ever, really). */

                if req.target == xa_targets {
                    let mut supported_formats: Vec<Atom> = Vec::with_capacity(mime_types.len() + 1);
                    supported_formats.push(xa_targets);
                    for mime_type in &mime_types {
                        supported_formats
                            .push(super::video::intern_atom(x, display, mime_type, false));
                    }
                    // SAFETY: the display is open; the data is an array of atoms.
                    unsafe {
                        (x.XChangeProperty)(
                            display,
                            req.requestor,
                            req.property,
                            XA_ATOM,
                            32,
                            PropModeReplace,
                            supported_formats.as_ptr() as *const c_uchar,
                            supported_formats.len() as c_int,
                        );
                    }
                    let s = sevent.selection_mut();
                    s.property = req.property;
                    s.target = xa_targets;
                } else if let Some(callback) = callback {
                    for mime_type in &mime_types {
                        if super::video::intern_atom(x, display, mime_type, false) != req.target {
                            continue;
                        }

                        // FIXME: We don't support the X11 INCR protocol for large clipboards. Do we want that? - Yes, yes we do.
                        if let Some(seln_data) = callback(mime_type) {
                            // SAFETY: the display is open; the data has
                            // seln_data.len() bytes.
                            unsafe {
                                (x.XChangeProperty)(
                                    display,
                                    req.requestor,
                                    req.property,
                                    req.target,
                                    8,
                                    PropModeReplace,
                                    seln_data.as_ptr(),
                                    seln_data.len() as c_int,
                                );
                            }
                            let s = sevent.selection_mut();
                            s.property = req.property;
                            s.target = req.target;
                        }
                        break;
                    }
                }
                // SAFETY: the display is open; the event is a SelectionNotify.
                unsafe {
                    (x.XSendEvent)(display, req.requestor, False, 0, &mut sevent);
                    (x.XSync)(display, False);

                    (x.XSetErrorHandler)(prev_handler);
                }
            }

            SelectionNotify => {
                let xsel = *xevent.selection();
                // (DEBUG_XEVENTS: "window CLIPBOARD: SelectionNotify (requestor = 0x%lx, target = %s, property = %s)")
                if xsel.target == atoms.TARGETS && xsel.property == atoms.SDL_FORMATS {
                    /* the new mime formats are the SDL_FORMATS property as an array of Atoms */
                    let mut atom: Atom = None;
                    let mut data: *mut c_uchar = std::ptr::null_mut();
                    let mut format_property: c_int = 0;
                    let mut length: c_ulong = 0;
                    let mut bytes_left: c_ulong = 0;

                    let window = self.get_window();
                    // SAFETY: the display is open; the out-parameters are valid.
                    unsafe {
                        (x.XGetWindowProperty)(
                            display,
                            window,
                            atoms.SDL_FORMATS,
                            0,
                            200,
                            0,
                            XA_ATOM,
                            &mut atom,
                            &mut format_property,
                            &mut length,
                            &mut bytes_left,
                            &mut data,
                        );
                    }

                    // SAFETY: the property holds `length` atoms.
                    let patoms = unsafe { atom_list(data, length) };
                    let new_mime_types: Vec<String> = patoms
                        .iter()
                        .map(|&a| atom_name(x, display, a).unwrap_or_default())
                        .collect();

                    send_clipboard_update(false, new_mime_types);

                    // Clear the internal selection source data, as it was invalided after updating the clipboard.
                    let old = self.with_data(|d| std::mem::take(&mut d.clipboard));
                    drop(old);

                    if !data.is_null() {
                        // SAFETY: the property data came from Xlib.
                        unsafe {
                            (x.XFree)(data.cast());
                        }
                    }
                }

                self.with_data(|d| d.selection_waiting = false);
            }

            SelectionClear => {
                let xa_clipboard = atoms.CLIPBOARD;
                // (DEBUG_XEVENTS: "window CLIPBOARD: SelectionClear (requestor = 0x%lx, target = 0x%lx)")
                let selection = xevent.selectionclear().selection;

                let taken = self.with_data(|d| {
                    let clipboard = if selection == XA_PRIMARY {
                        Some(&mut d.primary_selection)
                    } else if xa_clipboard != None && selection == xa_clipboard {
                        Some(&mut d.clipboard)
                    } else {
                        Option::None
                    };
                    match clipboard {
                        Some(clipboard) if clipboard.callback.is_some() => {
                            Some(std::mem::take(clipboard))
                        }
                        _ => Option::None,
                    }
                });
                if let Some(clipboard) = taken {
                    if clipboard.sequence != 0 {
                        crate::video::clipboard::cancel_clipboard_data(clipboard.sequence);
                    }
                    // (otherwise dropping it frees the userdata)
                    drop(clipboard);
                }
            }

            PropertyNotify => {
                let name_of_atom = atom_name(x, display, xevent.property().atom);

                if let Some(name) = &name_of_atom {
                    if name.starts_with("SDL_SELECTION")
                        && xevent.property().state == PropertyNewValue
                    {
                        self.with_data(|d| d.selection_incr_waiting = false);
                    }
                }
            }

            _ => {}
        }
    }

    /// Translation of `X11_HandleKeyEvent()`.
    pub(crate) fn x11_handle_key_event(
        &self,
        window: WindowID,
        keyboard_id: KeyboardID,
        xevent: &mut XEvent,
    ) {
        let display = self.display;
        let x = &self.x;
        let keycode = xevent.key().keycode;
        let mut keysym: KeySym = NoSymbol;
        let mut text: Vec<u8> = Vec::new();
        let mut status: Status = 0;
        let mut handled_by_ime = false;
        let pressed = xevent.get_type() == KeyPress;
        let (scancode, xkb_enabled) = self.with_data(|d| {
            (
                d.keyboard.key_layout[keycode as usize & 0xff],
                d.keyboard.xkb_enabled,
            )
        });
        let timestamp = x11_get_event_timestamp(xevent.key().time);

        // (DEBUG_XEVENTS: "window 0x%lx %s (X11 keycode = 0x%X)")
        // (DEBUG_SCANCODES would report unrecognized keys here)

        // XKB updates the modifiers explicitly via a state event.
        if !xkb_enabled {
            let state = xevent.key().state;
            self.with_data(|d| {
                let kb = &mut d.keyboard;
                kb.pressed_modifiers =
                    state & (ShiftMask | ControlMask | Mod1Mask | Mod3Mask | Mod4Mask | Mod5Mask);
                kb.locked_modifiers = state & (LockMask | kb.numlock_mask | kb.scrolllock_mask);
            });
        }

        if keyboard::text_input_active(window) {
            // filter events catches XIM events and sends them to the correct handler
            // SAFETY: the event is valid; the IME callbacks run with nothing
            // borrowed.
            if unsafe { (x.XFilterEvent)(xevent, None) } != 0 {
                // (DEBUG_XEVENTS: "Filtered event type = %d display = %p window = 0x%lx")
                handled_by_ime = true;
            }

            if !handled_by_ime {
                let ic = with_x11_window(window, |_, d| d.ic).unwrap_or(std::ptr::null_mut());
                match (&x.utf8, ic.is_null(), xevent.get_type() == KeyPress) {
                    (Some(utf8), false, true) => {
                        let mut buf = [0 as c_char; 64];
                        // SAFETY: the input context is the window's; the
                        // buffer's size (less the terminator) is passed.
                        let text_length = unsafe {
                            (utf8.Xutf8LookupString)(
                                ic,
                                xevent.key_mut(),
                                buf.as_mut_ptr(),
                                (buf.len() - 1) as c_int,
                                &mut keysym,
                                &mut status,
                            )
                        };
                        let len = text_length.clamp(0, (buf.len() - 1) as c_int) as usize;
                        text = buf[..len].iter().map(|&c| c as u8).collect();
                    }
                    _ => {
                        text = x_lookup_string_as_utf8(x, xevent.key_mut(), &mut keysym);
                    }
                }
            }
        }

        if !handled_by_ime {
            if pressed {
                self.handle_modifier_keys(scancode, true);
                keyboard::send_keyboard_key_ignore_modifiers(
                    timestamp,
                    keyboard_id,
                    keycode as i32,
                    scancode,
                    true,
                );

                if !text.is_empty() && !keyboard::mod_state().intersects(Keymod::CTRL | Keymod::ALT)
                {
                    // (text[text_length] = '\0')
                    if let Some(end) = text.iter().position(|&b| b == 0) {
                        text.truncate(end);
                    }
                    self.x11_clear_composition(window);
                    keyboard::send_keyboard_text(&String::from_utf8_lossy(&text));
                }
            } else {
                if x11_key_repeat(x, display, xevent) {
                    // We're about to get a repeated key down, ignore the key up
                    return;
                }

                self.handle_modifier_keys(scancode, false);
                keyboard::send_keyboard_key_ignore_modifiers(
                    timestamp,
                    keyboard_id,
                    keycode as i32,
                    scancode,
                    false,
                );
            }
        }

        if pressed {
            self.x11_update_user_time(window, xevent.key().time);
        }
    }

    /// Translation of `X11_HandleButtonPress()`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn x11_handle_button_press(
        &self,
        window: WindowID,
        mouse_id: MouseID,
        mut button: c_int,
        x: f32,
        y: f32,
        time: c_ulong,
        serial: c_ulong,
    ) {
        let mut xticks: c_int = 0;
        let mut yticks: c_int = 0;
        let timestamp = x11_get_event_timestamp(time);

        // (DEBUG_XEVENTS: "window 0x%lx: ButtonPress (X11 button = %d)")

        let (relative_mode, mx, my) = mouse::with_mouse(|m| (m.relative_mode, m.x, m.y));
        if !relative_mode && (x != mx || y != my) {
            self.x11_process_hit_test(window, x, y, false);
            mouse::send_mouse_motion(timestamp, Some(window), mouse_id, false, x, y);
        }

        if x11_is_wheel_event(button, &mut xticks, &mut yticks) {
            mouse::send_mouse_wheel(
                timestamp,
                Some(window),
                mouse_id,
                -xticks as f32,
                yticks as f32,
                MouseWheelDirection::Normal,
            );
        } else {
            if button > 7 {
                /* X button values 4-7 are used for scrolling, so X1 is 8, X2 is 9, ...
                => subtract (8-SDL_BUTTON_X1) to get value SDL expects */
                button -= 8 - mouse::BUTTON_X1 as c_int;
            }
            if button == Button1 as c_int && self.x11_trigger_hit_test_action(window, x, y) {
                send_window_event(window, EventType::WINDOW_HIT_TEST, 0, 0);
                return; // don't pass this event on to app.
            }
            let ignore_serial = with_x11_window(window, |_, windowdata| {
                if windowdata.last_focus_event_time != 0 {
                    const X11_FOCUS_CLICK_TIMEOUT: u64 = 10;
                    if crate::timer::ticks_ms()
                        < windowdata.last_focus_event_time + X11_FOCUS_CLICK_TIMEOUT
                        && !hints::get_bool(hints::MOUSE_FOCUS_CLICKTHROUGH, false)
                    {
                        // Ignore all press events with this serial.
                        windowdata.ignore_button_press_serial = serial;
                    }
                    windowdata.last_focus_event_time = 0;
                }
                windowdata.ignore_button_press_serial
            })
            .unwrap_or(0);
            if serial != ignore_serial {
                mouse::send_mouse_button(timestamp, Some(window), mouse_id, button as u8, true);
            }
        }
        self.x11_update_user_time(window, time);
    }

    /// Translation of `X11_HandleButtonRelease()`.
    pub(crate) fn x11_handle_button_release(
        &self,
        window: WindowID,
        mouse_id: MouseID,
        mut button: c_int,
        time: c_ulong,
    ) {
        // The X server sends a Release event for each Press for wheels. Ignore them.
        let mut xticks: c_int = 0;
        let mut yticks: c_int = 0;
        let timestamp = x11_get_event_timestamp(time);

        // (DEBUG_XEVENTS: "window 0x%lx: ButtonRelease (X11 button = %d)")
        if !x11_is_wheel_event(button, &mut xticks, &mut yticks) {
            if button > 7 {
                // see explanation at case ButtonPress
                button -= 8 - mouse::BUTTON_X1 as c_int;
            }
            mouse::send_mouse_button(timestamp, Some(window), mouse_id, button as u8, false);

            if with_x11_window(window, |_, d| d.pending_grab).unwrap_or(false) {
                let _ = self.x11_set_window_mouse_grab(window, true);
            }
        }
    }

    /// Read the window manager's frame extents. Translation of
    /// `X11_GetBorderValues()`.
    pub(crate) fn x11_get_border_values(&self, window: WindowID) {
        let display = self.display;
        let mut type_: Atom = 0;
        let mut format: c_int = 0;
        let mut nitems: c_ulong = 0;
        let mut bytes_after: c_ulong = 0;
        let mut property: *mut c_uchar = std::ptr::null_mut();

        let Ok((xwindow, borderless)) = with_x11_window(window, |w, d| {
            (d.xwindow, w.flags().contains(WindowFlags::BORDERLESS))
        }) else {
            return;
        };

        // Some compositors will send extents even when the border hint is turned off. Ignore them in this case.
        if !borderless {
            // SAFETY: the display is open; the out-parameters are valid.
            let status = unsafe {
                (self.x.XGetWindowProperty)(
                    display,
                    xwindow,
                    self.atoms()._NET_FRAME_EXTENTS,
                    0,
                    16,
                    0,
                    XA_CARDINAL,
                    &mut type_,
                    &mut format,
                    &mut nitems,
                    &mut bytes_after,
                    &mut property,
                )
            };
            if status == Success {
                if type_ != None && nitems == 4 {
                    // SAFETY: the property holds 4 longs.
                    let v = unsafe { std::slice::from_raw_parts(property as *const c_long, 4) };
                    let _ = with_x11_window(window, |_, data| {
                        data.border_left = v[0] as c_int;
                        data.border_right = v[1] as c_int;
                        data.border_top = v[2] as c_int;
                        data.border_bottom = v[3] as c_int;
                    });
                }
                // SAFETY: the property data came from Xlib.
                unsafe {
                    (self.x.XFree)(property.cast());
                }

                // (DEBUG_XEVENTS: "New _NET_FRAME_EXTENTS: left=%d right=%d, top=%d, bottom=%d")
            }
        } else {
            let _ = with_x11_window(window, |_, data| {
                data.border_left = 0;
                data.border_top = 0;
                data.border_right = 0;
                data.border_bottom = 0;
            });
        }
    }

    /// Translation of `X11_EmitConfigureNotifyEvents()`.
    pub(crate) fn x11_emit_configure_notify_events(
        &self,
        window: WindowID,
        xevent: &XConfigureEvent,
    ) {
        let Ok((size_move_event_flags, moved, resized)) = with_x11_window(window, |_, data| {
            let moved = xevent.x != data.last_xconfigure.x || xevent.y != data.last_xconfigure.y;
            let resized = xevent.width != data.last_xconfigure.width
                || xevent.height != data.last_xconfigure.height;
            (data.size_move_event_flags, moved, resized)
        }) else {
            return;
        };

        if size_move_event_flags == 0 {
            if moved {
                let _ = with_x11_window(window, |_, data| {
                    data.pending_operation &= !X11_PENDING_OP_MOVE
                });
            }
            let (x, y) =
                crate::video::window::global_to_relative_for_window(window, xevent.x, xevent.y);
            send_window_event(window, EventType::WINDOW_MOVED, x, y);

            let children = with_window(window, |w| w.children.clone()).unwrap_or_default();
            for w in children {
                // Don't update hidden child popup windows, their relative position doesn't change
                let (popup, hidden) = with_window(w, |w| {
                    (w.is_popup(), w.flags().contains(WindowFlags::HIDDEN))
                })
                .unwrap_or((false, true));
                if popup && !hidden {
                    self.x11_update_window_position(w, true);
                }
            }

            if resized {
                let _ = with_x11_window(window, |_, data| {
                    data.pending_operation &= !X11_PENDING_OP_RESIZE
                });
            }
            send_window_event(
                window,
                EventType::WINDOW_RESIZED,
                xevent.width,
                xevent.height,
            );
        }

        let _ = with_x11_window(window, |_, data| data.last_xconfigure = *xevent);
    }

    /// Translation of `X11_DispatchEvent()`.
    fn x11_dispatch_event(&self, xevent: &mut XEvent) {
        let display = self.display;
        let x = &self.x;
        let atoms = self.atoms();

        // filter events catches XIM events and sends them to the correct handler
        // Key press/release events are filtered in X11_HandleKeyEvent()
        if xevent.get_type() != KeyPress && xevent.get_type() != KeyRelease {
            // SAFETY: the event is valid.
            if unsafe { (x.XFilterEvent)(xevent, None) } != 0 {
                // (DEBUG_XEVENTS: "Filtered event type = %d display = %p window = 0x%lx")
                return;
            }
        }

        if xevent.get_type() == GenericEvent {
            self.x11_handle_generic_event(xevent);
            return;
        }

        // Calling the event hook for generic events happens in X11_HandleGenericEvent(), where the event data is available
        if let Some(hook) = event_hook() {
            if !hook(xevent as *mut XEvent as *mut c_void) {
                return;
            }
        }

        let xrandr_event_base = self.with_data(|d| d.xrandr_event_base);
        if xrandr_event_base != 0 && xevent.get_type() == xrandr_event_base + RRNotify {
            self.x11_handle_xrandr_event(xevent);
        }

        // (DEBUG_XEVENTS: "X11 event type = %d display = %p window = 0x%lx")

        if x.xfixes.is_some() && xevent.get_type() == x11_get_xfixes_selection_notify_event() {
            // SAFETY: an XFixesSelectionNotify event.
            let ev = unsafe { *xevent.cast::<XFixesSelectionNotifyEvent>() };

            // (DEBUG_XEVENTS: "window CLIPBOARD: XFixesSelectionNotify (selection = %s)")

            if ev.subtype == XFixesSetSelectionOwnerNotify {
                if ev.selection != atoms.CLIPBOARD {
                    return;
                }

                // SAFETY: the display is open.
                if unsafe { (x.XGetSelectionOwner)(display, ev.selection) }
                    == self.with_data(|d| d.clipboard_window)
                {
                    return;
                }

                /* when here we're notified that the clipboard had an external change, we request the
                 * available mime types by asking for a conversion to the TARGETS format. We should get a
                 * SelectionNotify event later, and when treating these results, we will push a ClipboardUpdated
                 * event
                 */

                let window = self.get_window();
                // SAFETY: the display is open; the window is the clipboard window.
                unsafe {
                    (x.XConvertSelection)(
                        display,
                        atoms.CLIPBOARD,
                        atoms.TARGETS,
                        atoms.SDL_FORMATS,
                        window,
                        CurrentTime,
                    );
                }
            }

            return;
        }

        let clipboard_window = self.with_data(|d| d.clipboard_window);
        if clipboard_window != None && clipboard_window == xevent.any().window {
            self.x11_handle_clipboard_event(xevent);
            return;
        }

        // xsettings internally filters events for the windows it watches
        self.x11_handle_xsettings_event(xevent);

        let Some(window) = self.x11_find_window(xevent.any().window) else {
            self.x11_dispatch_windowless_event(xevent);
            return;
        };

        match xevent.get_type() {
            // Gaining mouse coverage?
            EnterNotify => {
                let crossing = *xevent.crossing();
                // (DEBUG_XEVENTS: "window 0x%lx: EnterNotify! (%d,%d,%d)", NotifyGrab/NotifyUngrab)
                mouse::set_mouse_focus(Some(window));

                mouse::with_mouse(|m| {
                    m.last_x = crossing.x as f32;
                    m.last_y = crossing.y as f32;
                });

                {
                    // Only create the barriers if we have input focus
                    if let Ok((active, focus, rect)) = with_x11_window(window, |w, d| {
                        (
                            d.pointer_barrier_active,
                            w.flags().contains(WindowFlags::INPUT_FOCUS),
                            d.barrier_rect,
                        )
                    }) {
                        if active && focus {
                            let _ = self.x11_confine_cursor_with_flags(
                                window,
                                Some(&rect),
                                X11_BARRIER_HANDLED_BY_EVENT,
                            );
                        }
                    }
                }

                if !mouse::with_mouse(|m| m.relative_mode) {
                    mouse::send_mouse_motion(
                        Duration::ZERO,
                        Some(window),
                        mouse::GLOBAL_MOUSE_ID,
                        false,
                        crossing.x as f32,
                        crossing.y as f32,
                    );
                }

                // We ungrab in LeaveNotify, so we may need to grab again here, but not if captured, as the capture can be lost.
                let flags = with_window(window, |w| w.flags()).unwrap_or_default();
                if !flags.contains(WindowFlags::MOUSE_CAPTURE) {
                    update_window_grab(window);
                }

                let (last_x, last_y) = mouse::with_mouse(|m| (m.last_x, m.last_y));
                self.x11_process_hit_test(window, last_x, last_y, true);
            }
            // Losing mouse coverage?
            LeaveNotify => {
                let crossing = *xevent.crossing();
                // (DEBUG_XEVENTS: "window 0x%lx: LeaveNotify! (%d,%d,%d)", NotifyGrab/NotifyUngrab)
                if !mouse::with_mouse(|m| m.relative_mode) {
                    mouse::send_mouse_motion(
                        Duration::ZERO,
                        Some(window),
                        mouse::GLOBAL_MOUSE_ID,
                        false,
                        crossing.x as f32,
                        crossing.y as f32,
                    );
                }

                if crossing.mode != NotifyGrab
                    && crossing.mode != NotifyUngrab
                    && crossing.detail != NotifyInferior
                {
                    /* In order for interaction with the window decorations and menu to work properly
                    on Mutter, we need to ungrab the keyboard when the mouse leaves. */
                    let flags = with_window(window, |w| w.flags()).unwrap_or_default();
                    if !flags.contains(WindowFlags::FULLSCREEN) {
                        let _ = self.x11_set_window_keyboard_grab(window, false);
                    }

                    mouse::set_mouse_focus(Option::None);
                }
            }

            // Gaining input focus?
            FocusIn => {
                let focus = *xevent.focus();
                if focus.mode == NotifyGrab || focus.mode == NotifyUngrab {
                    // Someone is handling a global hotkey, ignore it
                    // (DEBUG_XEVENTS: "window 0x%lx: FocusIn (NotifyGrab/NotifyUngrab, ignoring)")
                    return;
                }

                if focus.detail == NotifyInferior || focus.detail == NotifyPointer {
                    // (DEBUG_XEVENTS: "window 0x%lx: FocusIn (NotifyInferior/NotifyPointer, ignoring)")
                    return;
                }
                // (DEBUG_XEVENTS: "window 0x%lx: FocusIn!")
                if self.with_data(|d| d.last_mode_change_deadline) == 0 {
                    // no recent mode changes
                    let _ = with_x11_window(window, |_, data| {
                        data.pending_focus = PendingFocus::None;
                        data.pending_focus_time = 0;
                    });
                    self.x11_dispatch_focus_in(window);
                } else {
                    let _ = with_x11_window(window, |_, data| {
                        data.pending_focus = PendingFocus::In;
                        data.pending_focus_time = crate::timer::ticks_ms() + PENDING_FOCUS_TIME;
                    });
                }
                let _ = with_x11_window(window, |_, data| {
                    data.last_focus_event_time = crate::timer::ticks_ms()
                });
            }

            // Losing input focus?
            FocusOut => {
                let focus = *xevent.focus();
                if focus.mode == NotifyGrab || focus.mode == NotifyUngrab {
                    // Someone is handling a global hotkey, ignore it
                    // (DEBUG_XEVENTS: "window 0x%lx: FocusOut (NotifyGrab/NotifyUngrab, ignoring)")
                    return;
                }
                if focus.detail == NotifyInferior || focus.detail == NotifyPointer {
                    /* We still have focus if a child gets focus. We also don't
                    care about the position of the pointer when the keyboard
                    focus changed. */
                    // (DEBUG_XEVENTS: "window 0x%lx: FocusOut (NotifyInferior/NotifyPointer, ignoring)")
                    return;
                }
                // (DEBUG_XEVENTS: "window 0x%lx: FocusOut!")
                if self.with_data(|d| d.last_mode_change_deadline) == 0 {
                    // no recent mode changes
                    let _ = with_x11_window(window, |_, data| {
                        data.pending_focus = PendingFocus::None;
                        data.pending_focus_time = 0;
                    });
                    self.x11_dispatch_focus_out(window);
                } else {
                    let _ = with_x11_window(window, |_, data| {
                        data.pending_focus = PendingFocus::Out;
                        data.pending_focus_time = crate::timer::ticks_ms() + PENDING_FOCUS_TIME;
                    });
                }

                // Disable confinement if it is activated.
                if with_x11_window(window, |_, d| d.pointer_barrier_active).unwrap_or(false) {
                    let _ = self.x11_confine_cursor_with_flags(
                        window,
                        Option::None,
                        X11_BARRIER_HANDLED_BY_EVENT,
                    );
                }
            }

            // Have we been iconified?
            UnmapNotify => {
                let mut ev = XEvent::zeroed();

                // (DEBUG_XEVENTS: "window 0x%lx: UnmapNotify!")

                let unmap: *mut XUnmapEvent = xevent.unmap_mut();
                // SAFETY: the display is open; the predicates read the unmap
                // event, alive for the calls.
                let reparented = unsafe {
                    (x.XCheckIfEvent)(
                        display,
                        &mut ev,
                        Some(is_reparent_notify),
                        unmap as XPointer,
                    )
                } != 0;
                if reparented {
                    // SAFETY: as above.
                    unsafe {
                        (x.XCheckIfEvent)(display, &mut ev, Some(is_map_notify), unmap as XPointer);
                    }
                } else {
                    self.x11_dispatch_unmap_notify(window);
                }

                // Disable confinement if the window gets hidden.
                if with_x11_window(window, |_, d| d.pointer_barrier_active).unwrap_or(false) {
                    let _ = self.x11_confine_cursor_with_flags(
                        window,
                        Option::None,
                        X11_BARRIER_HANDLED_BY_EVENT,
                    );
                }
            }

            // Have we been restored?
            MapNotify => {
                // (DEBUG_XEVENTS: "window 0x%lx: MapNotify!")
                self.x11_dispatch_map_notify(window);

                // Enable confinement if it was activated.
                if let Ok((true, rect)) =
                    with_x11_window(window, |_, d| (d.pointer_barrier_active, d.barrier_rect))
                {
                    let _ = self.x11_confine_cursor_with_flags(
                        window,
                        Some(&rect),
                        X11_BARRIER_HANDLED_BY_EVENT,
                    );
                }
            }

            // Have we been resized or moved?
            ConfigureNotify => {
                // (DEBUG_XEVENTS: "window 0x%lx: ConfigureNotify! (position: %d,%d, size: %dx%d)")
                // Real configure notify events are relative to the parent, synthetic events are absolute.
                if xevent.configure().send_event == 0 {
                    let mut num_children: c_uint = 0;
                    let mut child_return: Window = 0;
                    let mut root: Window = 0;
                    let mut parent: Window = 0;
                    let mut children: *mut Window = std::ptr::null_mut();
                    /* Translate these coordinates back to relative to root.
                     *
                     * XTranslateCoordinates can generate a BadWindow error if called during a racy
                     * reparenting operation, so a non-fatal error handler is required.
                     */
                    X11_LAST_ERROR.store(0, Ordering::Relaxed);
                    // SAFETY: the handler is a valid extern fn; the display
                    // is open; the out-parameters are valid.
                    unsafe {
                        let prev_handler = (x.XSetErrorHandler)(Some(bad_window_error_handler));

                        let conf = xevent.configure_mut();
                        (x.XQueryTree)(
                            display,
                            conf.window,
                            &mut root,
                            &mut parent,
                            &mut children,
                            &mut num_children,
                        );
                        // FIXME (upstream): the list of children from
                        // XQueryTree() is never freed.
                        let (cx, cy) = (conf.x, conf.y);
                        (x.XTranslateCoordinates)(
                            conf.display,
                            parent,
                            DefaultRootWindow(conf.display),
                            cx,
                            cy,
                            &mut conf.x,
                            &mut conf.y,
                            &mut child_return,
                        );

                        // Make sure the error callback was called if XTranslateCoordinates() failed.
                        (x.XSync)(display, False);
                        (x.XSetErrorHandler)(prev_handler);
                    }

                    // If XTranslateCoordinates failed due to a BadWindow error, nothing more to do here.
                    if X11_LAST_ERROR.load(Ordering::Relaxed) as c_int == BadWindow as c_int {
                        return;
                    }
                }

                /* Some window managers send ConfigureNotify before PropertyNotify when changing state (Xfce and
                 * fvwm are known to do this), which is backwards from other window managers, as well as what is
                 * expected by SDL and its clients. Defer emitting the size/move events until the corresponding
                 * PropertyNotify arrives for consistency.
                 */
                // FIXME (upstream): this reads xevent->xproperty.window of a
                // ConfigureNotify (the same offset as xconfigure.event).
                let property_window = xevent.property().window;
                let net_wm_state = self.x11_get_net_wm_state(window, property_window);
                let flags = with_window(window, |w| w.flags()).unwrap_or_default();
                let changed = WindowFlags(net_wm_state.0 ^ flags.0);
                let conf = *xevent.configure();
                let emit_after = with_x11_window(window, |_, data| {
                    if changed.intersects(WindowFlags::FULLSCREEN | WindowFlags::MAXIMIZED) {
                        data.pending_xconfigure = conf;
                        data.emit_size_move_after_property_notify = true;
                    }
                    data.emit_size_move_after_property_notify
                })
                .unwrap_or(false);

                if !emit_after {
                    self.x11_emit_configure_notify_events(window, &conf);
                }

                self.x11_handle_configure(window, &conf);
            }

            // Have we been requested to quit (or another client message?)
            ClientMessage => {
                self.x11_handle_client_message(window, xevent);
            }

            // Do we need to refresh ourselves?
            Expose => {
                // (DEBUG_XEVENTS: "window 0x%lx: Expose (count = %d)")
                send_window_event(window, EventType::WINDOW_EXPOSED, 0, 0);
            }

            /* Use XInput2 instead of the xevents API if possible, for:
            - KeyPress
            - KeyRelease
            - MotionNotify
            - ButtonPress
            - ButtonRelease
            XInput2 has more precise information, e.g., to distinguish different input devices. */
            KeyPress | KeyRelease => {
                let mut keyboard_id = keyboard::GLOBAL_KEYBOARD_ID;
                let xinput2_keyboard_enabled =
                    with_x11_window(window, |_, d| d.xinput2_keyboard_enabled).unwrap_or(false);
                let (last_key_serial, last_keyboard) =
                    self.with_data(|d| (d.xinput_last_key_serial, d.xinput_last_keyboard_device));
                if xinput2_keyboard_enabled {
                    // This input is being handled by XInput2.
                    return;
                } else if xevent.key().serial == last_key_serial {
                    // Use the device ID from the XInput2 event if the serials match.
                    keyboard_id = last_keyboard;
                }

                self.x11_handle_key_event(window, keyboard_id, xevent);
            }

            MotionNotify => {
                let (mouse_enabled, mouse_grabbed) =
                    with_x11_window(window, |_, d| (d.xinput2_mouse_enabled, d.mouse_grabbed))
                        .unwrap_or((false, false));
                if super::xinput2::x11_xinput2_handles_motion_for_window(
                    mouse_enabled,
                    mouse_grabbed,
                ) {
                    // This input is being handled by XInput2
                    return;
                }

                if !mouse::with_mouse(|m| m.relative_mode) {
                    let motion = *xevent.motion();
                    // (DEBUG_MOTION: "window 0x%lx: X11 motion: %d,%d")

                    self.x11_process_hit_test(window, motion.x as f32, motion.y as f32, false);
                    mouse::send_mouse_motion(
                        Duration::ZERO,
                        Some(window),
                        mouse::GLOBAL_MOUSE_ID,
                        false,
                        motion.x as f32,
                        motion.y as f32,
                    );
                }
            }

            ButtonPress => {
                let b = *xevent.button();
                let mouse_enabled =
                    with_x11_window(window, |_, d| d.xinput2_mouse_enabled).unwrap_or(false);
                if mouse_enabled && b.serial == self.with_data(|d| d.xinput_last_button_serial) {
                    // This input event was handled by XInput2.
                    return;
                }

                self.x11_handle_button_press(
                    window,
                    mouse::GLOBAL_MOUSE_ID,
                    b.button as c_int,
                    b.x as f32,
                    b.y as f32,
                    b.time,
                    b.serial,
                );
            }

            ButtonRelease => {
                let b = *xevent.button();
                let mouse_enabled =
                    with_x11_window(window, |_, d| d.xinput2_mouse_enabled).unwrap_or(false);
                if mouse_enabled && b.serial == self.with_data(|d| d.xinput_last_button_serial) {
                    // This input event was handled by XInput2.
                    return;
                }

                self.x11_handle_button_release(
                    window,
                    mouse::GLOBAL_MOUSE_ID,
                    b.button as c_int,
                    b.time,
                );
            }

            PropertyNotify => {
                self.x11_handle_property_notify(window, xevent);
            }

            SelectionNotify => {
                self.x11_handle_selection_notify(window, xevent);
            }

            _ => {
                // (DEBUG_XEVENTS: "window 0x%lx: Unhandled event %d")
            }
        }
    }

    /// The part of `X11_DispatchEvent()` for events that aren't for one of
    /// our windows.
    fn x11_dispatch_windowless_event(&self, xevent: &mut XEvent) {
        let display = self.display;
        let x = &self.x;

        // The window for KeymapNotify, etc events is 0
        let (xkb_enabled, xkb_event) =
            self.with_data(|d| (d.keyboard.xkb_enabled, d.keyboard.xkb.event));
        if xkb_enabled && xevent.get_type() == xkb_event {
            // SAFETY: an XKB event (all XkbEvent members fit in an XEvent).
            let any = unsafe { *xevent.cast::<XkbAnyEvent>() };
            match any.xkb_type {
                XkbStateNotify => {
                    // (DEBUG_XEVENTS: "window 0x%lx: XkbStateNotify!")
                    // SAFETY: as above.
                    let state = unsafe { *xevent.cast::<XkbStateNotifyEvent>() };
                    let keymap = self.with_data(|d| {
                        let xkb = &mut d.keyboard.xkb;
                        if (state.changed as c_ulong & XkbGroupStateMask) != 0
                            && state.group as u32 != xkb.current_group
                        {
                            xkb.current_group = state.group as u32;
                            Some(
                                xkb.keymaps
                                    .get(xkb.current_group as usize)
                                    .cloned()
                                    .flatten(),
                            )
                        } else {
                            Option::None
                        }
                    });
                    if let Some(keymap) = keymap {
                        keyboard::set_keymap(keymap, true);
                    }

                    if (state.changed as c_ulong & XkbModifierStateMask) != 0 {
                        self.with_data(|d| {
                            d.keyboard.pressed_modifiers = state.base_mods;
                            d.keyboard.locked_modifiers = state.latched_mods | state.locked_mods;
                        });
                        self.reconcile_modifiers(false);
                    }
                }

                XkbMapNotify | XkbNewKeyboardNotify => {
                    // (DEBUG_XEVENTS: "window 0x%lx: XkbMapNotify!" / "XkbNewKeyboardNotify!")
                    if let Some(xkb) = &x.xkb {
                        // SAFETY: the event is an XKB map (or new keyboard)
                        // notification, read as the `map` member.
                        unsafe {
                            let map = xevent as *mut XEvent as *mut XkbMapNotifyEvent;
                            (xkb.XkbRefreshKeyboardMapping)(map);
                        }
                    }

                    // Don't redundantly rebuild the keymap if this is a duplicate event.
                    let rebuild = self.with_data(|d| {
                        if any.serial != d.keyboard.xkb.last_map_serial {
                            d.keyboard.xkb.last_map_serial = any.serial;
                            true
                        } else {
                            false
                        }
                    });
                    if rebuild {
                        self.x11_update_keymap(true);
                    }
                }

                _ => {}
            }
        } else if xevent.get_type() == KeymapNotify {
            // (DEBUG_XEVENTS: "window 0x%lx: KeymapNotify!")
            if keyboard::keyboard_focus().is_some() {
                if !xkb_enabled {
                    self.x11_update_keymap(true);
                }
                self.x11_reconcile_keyboard_state();
            }
        } else if xevent.get_type() == MappingNotify {
            let request = xevent.mapping().request;

            if request == MappingPointer {
                // (DEBUG_XEVENTS: "window 0x%lx: MappingNotify!")
                self.x11_xinput2_update_pointer_mapping();
            } else if !xkb_enabled {
                // Has the keyboard layout changed?
                // (DEBUG_XEVENTS: "window 0x%lx: MappingNotify!")
                if request == MappingKeyboard || request == MappingModifier {
                    // SAFETY: a MappingNotify event.
                    unsafe {
                        (x.XRefreshKeyboardMapping)(xevent.mapping_mut());
                    }
                }

                self.x11_update_keymap(true);
            }
        } else if xevent.get_type() == PropertyNotify {
            if let Some(name_of_atom) = atom_name(x, display, xevent.property().atom) {
                if name_of_atom.starts_with("_ICC_PROFILE") {
                    let windows: Vec<_> = self.with_data(|d| d.windowlist.clone());
                    for entry in windows {
                        // SAFETY: the display is open; attrib is an out-parameter.
                        let screennum = unsafe {
                            let mut attrib: XWindowAttributes = std::mem::zeroed();
                            (x.XGetWindowAttributes)(display, entry.xwindow, &mut attrib);
                            (x.XScreenNumberOfScreen)(attrib.screen)
                        };
                        if screennum == 0 && name_of_atom == "_ICC_PROFILE" {
                            send_window_event(
                                entry.window,
                                EventType::WINDOW_ICCPROF_CHANGED,
                                0,
                                0,
                            );
                        } else if name_of_atom.len() > "_ICC_PROFILE_".len()
                            && name_of_atom.starts_with("_ICC_PROFILE_")
                        {
                            let iccscreennum = crate::stdlib::string::strtol(
                                &name_of_atom["_ICC_PROFILE_".len()..],
                                10,
                            )
                            .0 as c_int;

                            if screennum == iccscreennum {
                                send_window_event(
                                    entry.window,
                                    EventType::WINDOW_ICCPROF_CHANGED,
                                    0,
                                    0,
                                );
                            }
                        }
                    }
                } else if name_of_atom == "_NET_WORKAREA" {
                    for display_id in crate::video::display::displays().unwrap_or_default() {
                        send_display_event(
                            display_id,
                            EventType::DISPLAY_USABLE_BOUNDS_CHANGED,
                            0,
                            0,
                        );
                    }
                } else if name_of_atom == "GAMESCOPE_DISPLAY_IS_EXTERNAL"
                    || name_of_atom == "GAMESCOPE_SDR_ON_HDR_CONTENT_BRIGHTNESS"
                {
                    self.x11_check_displays_moved(display);
                }
            }
        }
    }

    /// The `ClientMessage` case of `X11_DispatchEvent()`.
    fn x11_handle_client_message(&self, window: WindowID, xevent: &mut XEvent) {
        // (upstream's function-level static `xdnd_version`)
        static XDND_VERSION: AtomicI32 = AtomicI32::new(0);
        let display = self.display;
        let x = &self.x;
        let atoms = self.atoms();
        let client = *xevent.client();
        let l = client.data.l;
        let Ok(xwindow) = with_x11_window(window, |_, d| d.xwindow) else {
            return;
        };

        if client.message_type == atoms.XdndEnter {
            let use_list = (l[1] & 1) != 0;
            let xdnd_source = l[0] as Window;
            XDND_VERSION.store((l[1] >> 24) as i32, Ordering::Relaxed);
            // (DEBUG_XEVENTS: source window, protocol version, more than 3 data types)

            let xdnd_req = if use_list {
                // fetch conversion targets
                // SAFETY: the display is open; the property is freed here.
                unsafe {
                    let p = x11_read_property(x, display, xdnd_source, atoms.XdndTypeList);
                    // pick one
                    let req = x11_pick_target(x, display, atom_list(p.data, p.count));
                    (x.XFree)(p.data.cast());
                    req
                }
            } else {
                // pick from list of three
                x11_pick_target_from_atoms(x, display, l[2] as Atom, l[3] as Atom, l[4] as Atom)
            };
            let _ = with_x11_window(window, |_, data| {
                data.xdnd_source = xdnd_source;
                data.xdnd_req = xdnd_req;
            });
        } else if client.message_type == atoms.XdndLeave {
            // (DEBUG_XEVENTS: "XID of source window : 0x%lx")
            send_drop_complete(Some(window));
        } else if client.message_type == atoms.XdndPosition {
            // (DEBUG_XEVENTS: the action requested by the user)
            {
                // Drag and Drop position
                let root_x = (l[2] >> 16) as c_int;
                let root_y = (l[2] & 0xffff) as c_int;
                let mut window_x: c_int = 0;
                let mut window_y: c_int = 0;
                let mut child_return: Window = 0;
                // Translate from root to current window position
                // SAFETY: the display is open; the out-parameters are valid.
                unsafe {
                    (x.XTranslateCoordinates)(
                        display,
                        DefaultRootWindow(display),
                        xwindow,
                        root_x,
                        root_y,
                        &mut window_x,
                        &mut window_y,
                        &mut child_return,
                    );
                }

                send_drop_position(Some(window), window_x as f32, window_y as f32);
            }

            // reply with status
            let xdnd_req = with_x11_window(window, |_, d| d.xdnd_req).unwrap_or(None);
            let mut m = XEvent::zeroed();
            {
                let c = m.client_mut();
                c.type_ = ClientMessage;
                c.display = client.display;
                c.window = l[0] as Window;
                c.message_type = atoms.XdndStatus;
                c.format = 32;
                c.data.l[0] = xwindow as c_long;
                c.data.l[1] = (xdnd_req != None) as c_long;
                c.data.l[2] = 0; // specify an empty rectangle
                c.data.l[3] = 0;
                c.data.l[4] = atoms.XdndActionCopy as c_long; // we only accept copying anyway
            }

            // SAFETY: the display is open; the event is a client message.
            unsafe {
                (x.XSendEvent)(display, l[0] as Window, False, NoEventMask, &mut m);
                (x.XFlush)(display);
            }
        } else if client.message_type == atoms.XdndDrop {
            let xdnd_req = with_x11_window(window, |_, d| d.xdnd_req).unwrap_or(None);
            if xdnd_req == None {
                // say again - not interested!
                let mut m = XEvent::zeroed();
                {
                    let c = m.client_mut();
                    c.type_ = ClientMessage;
                    c.display = client.display;
                    c.window = l[0] as Window;
                    c.message_type = atoms.XdndFinished;
                    c.format = 32;
                    c.data.l[0] = xwindow as c_long;
                    c.data.l[1] = 0;
                    c.data.l[2] = None as c_long; // fail!
                }
                // SAFETY: as above.
                unsafe {
                    (x.XSendEvent)(display, l[0] as Window, False, NoEventMask, &mut m);
                }
            } else {
                // convert
                let time = if XDND_VERSION.load(Ordering::Relaxed) >= 1 {
                    l[2] as Time
                } else {
                    CurrentTime
                };
                // SAFETY: the display is open; the window is ours.
                unsafe {
                    (x.XConvertSelection)(
                        display,
                        atoms.XdndSelection,
                        xdnd_req,
                        atoms.PRIMARY,
                        xwindow,
                        time,
                    );
                }
            }
        } else if client.message_type == atoms.WM_PROTOCOLS
            && client.format == 32
            && l[0] as Atom == atoms._NET_WM_PING
        {
            // SAFETY: the display is open.
            let root = unsafe { DefaultRootWindow(display) };

            // (DEBUG_XEVENTS: "window 0x%lx: _NET_WM_PING")
            xevent.client_mut().window = root;
            // SAFETY: the display is open; the event is the ping, readdressed.
            unsafe {
                (x.XSendEvent)(
                    display,
                    root,
                    False,
                    SubstructureRedirectMask | SubstructureNotifyMask,
                    xevent,
                );
            }
        } else if client.message_type == atoms.WM_PROTOCOLS
            && client.format == 32
            && l[0] as Atom == atoms.WM_DELETE_WINDOW
        {
            // (DEBUG_XEVENTS: "window 0x%lx: WM_DELETE_WINDOW")
            send_window_event(window, EventType::WINDOW_CLOSE_REQUESTED, 0, 0);
        } else if client.message_type == atoms.WM_PROTOCOLS
            && client.format == 32
            && l[0] as Atom == atoms._NET_WM_SYNC_REQUEST
        {
            // (DEBUG_XEVENTS: "window %p: _NET_WM_SYNC_REQUEST")
            self.x11_handle_sync_request(window, &client);
        }
    }

    /// The `PropertyNotify` case of `X11_DispatchEvent()`.
    fn x11_handle_property_notify(&self, window: WindowID, xevent: &XEvent) {
        let display = self.display;
        let x = &self.x;
        let atoms = self.atoms();
        let prop = *xevent.property();

        // (DEBUG_XEVENTS dumps the property's name and value here)

        /* Take advantage of this moment to make sure user_time has a
        valid timestamp from the X server, so if we later try to
        raise/restore this window, _NET_ACTIVE_WINDOW can have a
        non-zero timestamp, even if there's never been a mouse or
        key press to this window so far. Note that we don't try to
        set _NET_WM_USER_TIME here, though. That's only for legit
        user interaction with the window. */
        let Ok(xwindow) = with_x11_window(window, |_, data| {
            if data.user_time == 0 {
                data.user_time = prop.time;
            }
            data.xwindow
        }) else {
            return;
        };

        if prop.atom == atoms._NET_WM_STATE {
            /* Get the new state from the window manager.
             * Compositing window managers can alter visibility of windows
             * without ever mapping / unmapping them, so we handle that here,
             * because they use the NETWM protocol to notify us of changes.
             */
            let flags = self.x11_get_net_wm_state(window, prop.window);
            let window_flags = with_window(window, |w| w.flags()).unwrap_or_default();
            let changed = WindowFlags(flags.0 ^ window_flags.0);

            if changed.contains(WindowFlags::HIDDEN) && !flags.contains(WindowFlags::HIDDEN) {
                self.x11_dispatch_map_notify(window);
            }

            let is_popup = with_window(window, |w| w.is_popup()).unwrap_or(false);
            if !is_popup {
                if changed.contains(WindowFlags::FULLSCREEN) {
                    let _ = with_x11_window(window, |_, d| {
                        d.pending_operation &= !X11_PENDING_OP_FULLSCREEN
                    });

                    if flags.contains(WindowFlags::FULLSCREEN) {
                        if !flags.contains(WindowFlags::MINIMIZED) {
                            let commit = with_x11_window(window, |w, d| {
                                w.current_fullscreen_mode != d.requested_fullscreen_mode
                            })
                            .unwrap_or(false);

                            // Ensure the maximized flag is cleared before entering fullscreen.
                            send_window_event(window, EventType::WINDOW_RESTORED, 0, 0);
                            send_window_event(window, EventType::WINDOW_ENTER_FULLSCREEN, 0, 0);
                            if commit {
                                /* This was initiated by the compositor, or the mode was changed between the request and the window
                                 * becoming fullscreen. Switch to the application requested mode if necessary.
                                 */
                                let _ = with_window(window, |w| {
                                    w.current_fullscreen_mode = w.requested_fullscreen_mode
                                });
                                let _ = update_fullscreen_mode(window, FullscreenOp::Update, true);
                            } else {
                                let _ = update_fullscreen_mode(window, FullscreenOp::Enter, false);
                            }
                        }
                    } else {
                        send_window_event(window, EventType::WINDOW_LEAVE_FULLSCREEN, 0, 0);
                        let _ = update_fullscreen_mode(window, FullscreenOp::Leave, false);

                        let _ = with_x11_window(window, |_, d| {
                            d.requested_fullscreen_mode = DisplayMode::default()
                        });

                        // Need to restore or update any limits changed while the window was fullscreen.
                        self.x11_set_window_min_max(window, flags.contains(WindowFlags::MAXIMIZED));

                        // Toggle the borders if they were forced on while creating a borderless fullscreen window.
                        let _ = with_x11_window(window, |_, data| {
                            if data.fullscreen_borders_forced_on {
                                data.toggle_borders = true;
                                data.fullscreen_borders_forced_on = false;
                            }
                        });
                    }

                    let toggle = with_x11_window(window, |w, data| {
                        let has_borders = data.border_top != 0
                            || data.border_left != 0
                            || data.border_bottom != 0
                            || data.border_right != 0;
                        if flags.contains(WindowFlags::FULLSCREEN) && has_borders {
                            /* If the window is entering fullscreen and the borders are
                             * non-zero sized, turn off size events until the borders are
                             * shut off to avoid bogus window sizes and positions, and
                             * note that the old borders were non-zero for restoration.
                             */
                            data.size_move_event_flags |= X11_SIZE_MOVE_EVENTS_WAIT_FOR_BORDERS;
                            data.previous_borders_nonzero = true;
                        } else if !flags.contains(WindowFlags::FULLSCREEN)
                            && data.previous_borders_nonzero
                            && !has_borders
                        {
                            /* If the window is leaving fullscreen and the current borders
                             * are zero sized, but weren't when entering fullscreen, turn
                             * off size events until the borders come back to avoid bogus
                             * window sizes and positions.
                             */
                            data.size_move_event_flags |= X11_SIZE_MOVE_EVENTS_WAIT_FOR_BORDERS;
                            data.previous_borders_nonzero = false;
                        } else {
                            data.size_move_event_flags = 0;
                            data.previous_borders_nonzero = false;

                            if !w.flags().contains(WindowFlags::FULLSCREEN) && data.toggle_borders {
                                data.toggle_borders = false;
                                return Some(!w.flags().contains(WindowFlags::BORDERLESS));
                            }
                        }
                        Option::None
                    })
                    .ok()
                    .flatten();
                    if let Some(bordered) = toggle {
                        self.x11_set_window_bordered(window, bordered);
                    }
                }
                if changed.contains(WindowFlags::MAXIMIZED)
                    && flags.contains(WindowFlags::MAXIMIZED)
                    && !flags.contains(WindowFlags::MINIMIZED)
                {
                    let _ = with_x11_window(window, |_, d| {
                        d.pending_operation &= !X11_PENDING_OP_MAXIMIZE
                    });
                    if changed.contains(WindowFlags::MINIMIZED) {
                        let _ = with_x11_window(window, |_, d| {
                            d.pending_operation &= !X11_PENDING_OP_RESTORE
                        });
                        // If coming out of minimized, send a restore event before sending maximized.
                        send_window_event(window, EventType::WINDOW_RESTORED, 0, 0);
                    }
                    send_window_event(window, EventType::WINDOW_MAXIMIZED, 0, 0);
                }
                if changed.contains(WindowFlags::MINIMIZED)
                    && flags.contains(WindowFlags::MINIMIZED)
                {
                    let _ = with_x11_window(window, |_, d| {
                        d.pending_operation &= !X11_PENDING_OP_MINIMIZE
                    });
                    send_window_event(window, EventType::WINDOW_MINIMIZED, 0, 0);
                }
                if !flags.intersects(WindowFlags::MAXIMIZED | WindowFlags::MINIMIZED) {
                    let _ = with_x11_window(window, |_, d| {
                        d.pending_operation &= !X11_PENDING_OP_RESTORE
                    });
                    send_window_event(window, EventType::WINDOW_RESTORED, 0, 0);

                    // Apply any pending state if restored.
                    if !flags.contains(WindowFlags::FULLSCREEN) {
                        let _ = with_x11_window(window, |w, data| {
                            if data.pending_position {
                                data.pending_position = false;
                                data.pending_operation |= X11_PENDING_OP_MOVE;
                                data.expected.x = w.pending.x - data.border_left;
                                data.expected.y = w.pending.y - data.border_top;
                                // SAFETY: the display is open and the window ours.
                                unsafe {
                                    (x.XMoveWindow)(
                                        display,
                                        data.xwindow,
                                        data.expected.x,
                                        data.expected.y,
                                    );
                                }
                            }
                            if data.pending_size {
                                data.pending_size = false;
                                data.pending_operation |= X11_PENDING_OP_RESIZE;
                                data.expected.w = w.pending.w;
                                data.expected.h = w.pending.h;
                                // SAFETY: as above.
                                unsafe {
                                    (x.XResizeWindow)(
                                        display,
                                        data.xwindow,
                                        w.pending.w as c_uint,
                                        w.pending.h as c_uint,
                                    );
                                }
                            }
                        });
                    }
                }
                let pending = with_x11_window(window, |_, data| {
                    data.emit_size_move_after_property_notify
                        .then_some(data.pending_xconfigure)
                })
                .ok()
                .flatten();
                if let Some(pending_xconfigure) = pending {
                    self.x11_emit_configure_notify_events(window, &pending_xconfigure);
                    let _ = with_x11_window(window, |_, d| {
                        d.emit_size_move_after_property_notify = false
                    });
                }
                if flags.contains(WindowFlags::INPUT_FOCUS) {
                    let pending_move = with_x11_window(window, |_, d| {
                        d.pending_move.then_some(d.pending_move_point)
                    })
                    .ok()
                    .flatten();
                    if let Some(point) = pending_move {
                        self.dispatch_window_move(window, &point);
                        let _ = with_x11_window(window, |_, d| d.pending_move = false);
                    }
                }
            }
            if changed.contains(WindowFlags::OCCLUDED) {
                send_window_event(
                    window,
                    if flags.contains(WindowFlags::OCCLUDED) {
                        EventType::WINDOW_OCCLUDED
                    } else {
                        EventType::WINDOW_EXPOSED
                    },
                    0,
                    0,
                );
            }
        } else if prop.atom == atoms.WM_STATE {
            /* Support for ICCCM-compliant window managers (like i3) that change
            WM_STATE to WithdrawnState without sending UnmapNotify or updating
            _NET_WM_STATE when moving windows to invisible workspaces. */
            let mut type_: Atom = 0;
            let mut format: c_int = 0;
            let mut nitems: c_ulong = 0;
            let mut bytes_after: c_ulong = 0;
            let mut prop_data: *mut c_uchar = std::ptr::null_mut();

            // SAFETY: the display is open; the out-parameters are valid.
            let status = unsafe {
                (x.XGetWindowProperty)(
                    display,
                    xwindow,
                    atoms.WM_STATE,
                    0,
                    2,
                    False,
                    atoms.WM_STATE,
                    &mut type_,
                    &mut format,
                    &mut nitems,
                    &mut bytes_after,
                    &mut prop_data,
                )
            };
            if status == Success {
                if nitems > 0 {
                    // WM_STATE: 0=Withdrawn, 1=Normal, 3=Iconic
                    // FIXME (upstream): the format-32 value (a long) is read
                    // as its first 32 bits, wrong on big-endian 64-bit
                    // systems.
                    // SAFETY: the property holds at least one long.
                    let state = unsafe { *(prop_data as *const u32) };
                    let minimized =
                        with_window(window, |w| w.flags().contains(WindowFlags::MINIMIZED))
                            .unwrap_or(false);

                    if state == 0 || state == 3 {
                        // Withdrawn or Iconic
                        if !minimized {
                            send_window_event(window, EventType::WINDOW_MINIMIZED, 0, 0);
                            send_window_event(window, EventType::WINDOW_OCCLUDED, 0, 0);
                        }
                    } else if state == 1 {
                        // NormalState
                        if minimized {
                            send_window_event(window, EventType::WINDOW_RESTORED, 0, 0);
                            send_window_event(window, EventType::WINDOW_EXPOSED, 0, 0);
                        }
                    }
                }
                // SAFETY: the property data came from Xlib.
                unsafe {
                    (x.XFree)(prop_data.cast());
                }
            }
        } else if prop.atom == atoms.XKLAVIER_STATE {
            /* Hack for Ubuntu 12.04 (etc) that doesn't send MappingNotify
            events when the keyboard layout changes (for example,
            changing from English to French on the menubar's keyboard
            icon). Since it changes the XKLAVIER_STATE property, we
            notice and reinit our keymap here. This might not be the
            right approach, but it seems to work. */
            self.x11_update_keymap(true);
        } else if prop.atom == atoms._NET_FRAME_EXTENTS {
            self.x11_get_border_values(window);
            let toggle = with_x11_window(window, |w, data| {
                if data.size_move_event_flags != 0 {
                    /* Events are disabled when leaving fullscreen until the borders appear to avoid
                     * incorrect size/position events on compositing window managers.
                     */
                    data.size_move_event_flags &= !X11_SIZE_MOVE_EVENTS_WAIT_FOR_BORDERS;
                }
                if !w.flags().contains(WindowFlags::FULLSCREEN) && data.toggle_borders {
                    data.toggle_borders = false;
                    return Some(!w.flags().contains(WindowFlags::BORDERLESS));
                }
                Option::None
            })
            .ok()
            .flatten();
            if let Some(bordered) = toggle {
                self.x11_set_window_bordered(window, bordered);
            }
        }
    }

    /// The `SelectionNotify` case of `X11_DispatchEvent()` (drag and drop
    /// data).
    fn x11_handle_selection_notify(&self, window: WindowID, xevent: &XEvent) {
        let display = self.display;
        let x = &self.x;
        let atoms = self.atoms();
        let target = xevent.selection().target;
        // (DEBUG_XEVENTS: "window 0x%lx: SelectionNotify (requestor = 0x%lx, target = 0x%lx)")
        let Ok((xdnd_req, xwindow, xdnd_source)) =
            with_x11_window(window, |_, d| (d.xdnd_req, d.xwindow, d.xdnd_source))
        else {
            return;
        };
        if target == xdnd_req {
            // read data
            // SAFETY: the display is open; the property is freed below.
            let p = unsafe { x11_read_property(x, display, xwindow, atoms.PRIMARY) };

            if p.format == 8 {
                if let Some(name) = atom_name(x, display, target) {
                    let data: &[u8] = if p.data.is_null() {
                        &[]
                    } else {
                        // SAFETY: format-8 data of `count` bytes.
                        unsafe { std::slice::from_raw_parts(p.data, p.count as usize) }
                    };
                    // (SDL_strtok_r: the data is a C string)
                    let data = &data[..data.iter().position(|&b| b == 0).unwrap_or(data.len())];
                    for token in data
                        .split(|&b| b == b'\r' || b == b'\n')
                        .filter(|t| !t.is_empty())
                    {
                        let token = String::from_utf8_lossy(token);
                        if name == "text/plain;charset=utf-8"
                            || name == "UTF8_STRING"
                            || name == "text/plain"
                            || name == "TEXT"
                        {
                            send_drop_text(Some(window), &token);
                        } else if name == "text/uri-list" {
                            if let Some(file) = crate::utils::uri_to_local(&token) {
                                let file = String::from_utf8_lossy(&file);
                                send_drop_file(Some(window), Option::None, &file);
                            }
                        }
                    }
                }
                send_drop_complete(Some(window));
            }
            // SAFETY: the property data came from Xlib.
            unsafe {
                (x.XFree)(p.data.cast());
            }

            // send reply
            let mut m = XEvent::zeroed();
            {
                let c = m.client_mut();
                c.type_ = ClientMessage;
                c.display = display;
                c.window = xdnd_source;
                c.message_type = atoms.XdndFinished;
                c.format = 32;
                c.data.l[0] = xwindow as c_long;
                c.data.l[1] = 1;
                c.data.l[2] = atoms.XdndActionCopy as c_long;
            }
            // SAFETY: the display is open; the event is a client message.
            unsafe {
                (x.XSendEvent)(display, xdnd_source, False, NoEventMask, &mut m);

                (x.XSync)(display, False);
            }
        }
    }

    /// Translation of `X11_HandleFocusChanges()`.
    fn x11_handle_focus_changes(&self) {
        let windows = self.with_data(|d| d.windowlist.clone());
        for entry in windows {
            let pending =
                with_x11_window(entry.window, |_, d| (d.pending_focus, d.pending_focus_time));
            if let Ok((pending_focus, pending_focus_time)) = pending {
                if pending_focus != PendingFocus::None {
                    let now = crate::timer::ticks_ms();
                    if now >= pending_focus_time {
                        if pending_focus == PendingFocus::In {
                            self.x11_dispatch_focus_in(entry.window);
                        } else {
                            self.x11_dispatch_focus_out(entry.window);
                        }
                        let _ = with_x11_window(entry.window, |_, d| {
                            d.pending_focus = PendingFocus::None
                        });
                    }
                }
            }
        }
    }

    /// Wake up a thread waiting for events (an `_SDL_WAKEUP` client message
    /// to the window, through the request display). Translation of
    /// `X11_SendWakeupEvent()`.
    pub(crate) fn x11_send_wakeup_event(&self, window: WindowID) {
        let req_display = self.request_conn.display;
        let Ok(xwindow) = with_x11_window(window, |_, d| d.xwindow) else {
            return;
        };
        let mut event = XEvent::zeroed();

        {
            let c = event.client_mut();
            c.type_ = ClientMessage;
            c.display = req_display;
            c.send_event = True;
            c.message_type = self.atoms()._SDL_WAKEUP;
            c.format = 8;
        }

        // SAFETY: the request display is open; the event is a client message.
        unsafe {
            (self.x.XSendEvent)(req_display, xwindow, False, NoEventMask, &mut event);
            /* XSendEvent returns a status and it could be BadValue or BadWindow. If an
            error happens it is an SDL's internal error and there is nothing we can do here. */
            (self.x.XFlush)(req_display);
        }
    }

    /// Wait for an event for up to `timeout_ns` (negative: forever) and
    /// dispatch it. Translation of `X11_WaitEventTimeout()`.
    pub(crate) fn x11_wait_event_timeout(&self, timeout_ns: i64) -> i32 {
        let display = self.display;
        let mut xevent = XEvent::zeroed();

        // Flush and poll to grab any events already read and queued
        // SAFETY: the display is open.
        unsafe {
            (self.x.XFlush)(display);
        }
        if x11_poll_event(&self.x, display, &mut xevent) {
            // Fall through
        } else if timeout_ns == 0 {
            return 0;
        } else {
            // Use SDL_IOR_NO_RETRY to ensure SIGINT will break us out of our wait
            // SAFETY: the display is open.
            let fd = unsafe { ConnectionNumber(display) };
            let err = io_ready(fd, IoReadyFlags::READ | IoReadyFlags::NO_RETRY, timeout_ns);
            if err > 0 {
                if !x11_poll_event(&self.x, display, &mut xevent) {
                    /* Someone may have beat us to reading the fd. Return 1 here to
                     * trigger the normal spurious wakeup logic in the event core. */
                    return 1;
                }
            } else if err == 0 {
                // Timeout
                return 0;
            } else {
                // Error returned from poll()/select()

                if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                    /* If the wait was interrupted by a signal, we may have generated a
                     * SDL_EVENT_QUIT event. Let the caller know to call SDL_PumpEvents(). */
                    return 1;
                } else {
                    return err;
                }
            }
        }

        self.x11_dispatch_event(&mut xevent);
        1
    }

    /// Translation of `X11_PumpEvents()`.
    pub(crate) fn x11_pump_events(&self) {
        let mut xevent = XEvent::zeroed();

        /* Check if a display had the mode changed and is waiting for a window to asynchronously become
         * fullscreen. If there is no fullscreen window past the elapsed timeout, revert the mode switch.
         */
        for display_id in crate::video::display::displays().unwrap_or_default() {
            let revert = with_x11_display(display_id, |display, data| {
                if data.mode_switch_deadline_ns != 0 {
                    if display.fullscreen_window.is_some() {
                        data.mode_switch_deadline_ns = 0;
                    } else if crate::timer::ticks_ns() >= data.mode_switch_deadline_ns {
                        return true;
                    }
                }
                false
            })
            .unwrap_or(false);
            if revert {
                crate::error!(
                    crate::log::Category::Video,
                    "Time out elapsed after mode switch on display {} with no window becoming fullscreen; reverting",
                    display_id
                );
                let _ =
                    crate::video::display::set_display_mode_for_display(display_id, Option::None);
                with_x11_display(display_id, |_, data| data.mode_switch_deadline_ns = 0);
            }
        }

        self.with_data(|data| {
            if data.last_mode_change_deadline != 0
                && crate::timer::ticks_ms() >= data.last_mode_change_deadline
            {
                data.last_mode_change_deadline = 0; // assume we're done.
            }
        });

        // Update activity every 30 seconds to prevent screensaver
        if !crate::video::core::screen_saver_enabled() {
            let now = crate::timer::ticks_ms();
            self.with_data(|data| {
                if data.screensaver_activity == 0 || now >= data.screensaver_activity + 30000 {
                    // SAFETY: the display is open.
                    unsafe {
                        (self.x.XResetScreenSaver)(self.display);
                    }

                    #[cfg(not(target_os = "android"))]
                    crate::core::linux::dbus::screensaver_tickle();

                    data.screensaver_activity = now;
                }
            });
        }

        // Keep processing pending events
        while x11_poll_event(&self.x, self.display, &mut xevent) {
            self.x11_dispatch_event(&mut xevent);
        }

        // FIXME: Only need to do this when there are pending focus changes
        self.x11_handle_focus_changes();

        // FIXME: Only need to do this when there are flashing windows
        let windows = self.with_data(|d| d.windowlist.clone());
        for entry in windows {
            let flash_cancel_time =
                with_x11_window(entry.window, |_, d| d.flash_cancel_time).unwrap_or(0);
            if flash_cancel_time != 0 && crate::timer::ticks_ms() >= flash_cancel_time {
                let _ = self.x11_flash_window(entry.window, FlashOperation::Cancel);
            }
        }

        if self.with_data(|d| d.xinput_hierarchy_changed) {
            self.x11_xinput2_update_devices();
            self.with_data(|d| d.xinput_hierarchy_changed = false);
        }
    }

    /// Translation of `X11_SuspendScreenSaver()`.
    pub(crate) fn x11_suspend_screen_saver(&self, suspend: bool) -> Result<()> {
        #[cfg(not(target_os = "android"))]
        {
            if crate::core::linux::dbus::screensaver_inhibit(suspend) {
                return Ok(());
            }

            if suspend {
                crate::core::linux::dbus::screensaver_tickle();
            }
        }

        if let Some(xss) = &self.x.xss {
            let mut dummy: c_int = 0;
            let mut dummy2: c_int = 0;
            let mut major_version: c_int = 0;
            let mut minor_version: c_int = 0;
            // X11_XScreenSaverSuspend was introduced in MIT-SCREEN-SAVER 1.1
            // SAFETY: the display is open; the out-parameters are valid.
            unsafe {
                if (xss.XScreenSaverQueryExtension)(self.display, &mut dummy, &mut dummy2) == 0
                    || (xss.XScreenSaverQueryVersion)(
                        self.display,
                        &mut major_version,
                        &mut minor_version,
                    ) == 0
                    || major_version < 1
                    || (major_version == 1 && minor_version < 1)
                {
                    return Err(Error::unsupported());
                }

                (xss.XScreenSaverSuspend)(self.display, suspend as Bool);
                (self.x.XResetScreenSaver)(self.display);
            }
            return Ok(());
        }
        Err(Error::unsupported())
    }
}

/// Translation of `isAnyEvent()`.
unsafe extern "C" fn is_any_event(
    _display: *mut Display,
    _ev: *mut XEvent,
    _arg: XPointer,
) -> Bool {
    True
}

/// Translation of `X11_PollEvent()`.
fn x11_poll_event(x: &X11Syms, display: *mut Display, event: &mut XEvent) -> bool {
    // SAFETY: the display is open; the event is an out-parameter.
    unsafe { (x.XCheckIfEvent)(display, event, Some(is_any_event), std::ptr::null_mut()) != 0 }
}

/// Calling an optional event hook (`!g_X11EventHook || g_X11EventHook(...)`).
trait HookExt {
    fn is_none_or_true(&self, xev: &mut XEvent) -> bool;
}

impl HookExt for Option<X11EventHook> {
    fn is_none_or_true(&self, xev: &mut XEvent) -> bool {
        match self {
            Some(hook) => hook(xev as *mut XEvent as *mut c_void),
            Option::None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_buttons() {
        let (mut x, mut y) = (0, 0);
        assert!(x11_is_wheel_event(4, &mut x, &mut y));
        assert_eq!((x, y), (0, 1));
        assert!(x11_is_wheel_event(7, &mut x, &mut y));
        assert_eq!((x, y), (-1, 1));
        assert!(!x11_is_wheel_event(1, &mut x, &mut y));
        assert!(!x11_is_wheel_event(8, &mut x, &mut y));
    }

    #[test]
    fn modifiers_reconcile() {
        let mut kb = KeyboardData {
            alt_mask: Mod1Mask,
            gui_mask: Mod4Mask,
            level3_mask: Mod5Mask,
            level5_mask: Mod3Mask,
            numlock_mask: Mod2Mask,
            ..KeyboardData::default()
        };
        kb.pressed_modifiers = ShiftMask;
        kb.locked_modifiers = LockMask | Mod2Mask;
        let m = x11_reconcile_modifiers(&mut kb, false);
        assert!(m.contains(Keymod::SHIFT));
        assert!(m.contains(Keymod::CAPS));
        assert!(m.contains(Keymod::NUM));
        assert!(!m.intersects(Keymod::CTRL));

        // a physically pressed left shift wins over the ambiguous pair
        kb.sdl_physically_pressed_modifiers = Keymod::LSHIFT;
        let m = x11_reconcile_modifiers(&mut kb, true);
        assert!(m.contains(Keymod::LSHIFT));
        assert!(!m.contains(Keymod::RSHIFT));
    }

    #[test]
    fn latin1() {
        assert!(!is_high_latin1(b"abc"));
        assert!(is_high_latin1(&[b'a', 0xe9]));
    }
}
