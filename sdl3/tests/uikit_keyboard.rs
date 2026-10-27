// Tests of the UIKit video driver's keyboard: text input through the
// hidden text field, the on-screen keyboard, the hardware keys' presses
// and the game controllers' keyboards.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as
//! `uikit_main` is. Its main function makes a window and traces the
//! keyboards, the key, text and on-screen keyboard events, and the state
//! of the window's hidden text field (its class, whether it's the first
//! responder, its keyboard type, return key, autocapitalization,
//! autocorrection, spell checking, secure entry, whether the return key
//! enables itself, its content type and its text), through:
//!
//! * the view controller's key commands (the arrows and Escape), given to
//!   its handler;
//! * text input started and stopped, with the simulator's keyboard
//!   (GameController's coalesced one, which comes a moment after the
//!   video subsystem starts);
//! * text typed in the field, in the program (`insertText:` and
//!   `deleteBackward`, which UIKit turns into the field's delegate's
//!   calls and its change notification, as for typing): letters, a
//!   capital, characters outside ASCII and outside the Basic Multilingual
//!   Plane, deleting;
//! * the return key (the field's delegate's `textFieldShouldReturn:`),
//!   with and without `SDL_HINT_RETURN_KEY_HIDES_IME`;
//! * text input started with each text input type, each capitalization
//!   (for the plain and the name types, whose defaults differ),
//!   autocorrection and multiline on and off (and multiline's default
//!   with the return key hint), properties changed while text input is
//!   active, and the one-time code's text, which isn't reset;
//! * the text input area, with the keyboard's notifications posted in the
//!   program (a keyboard 300 and 400 points high: the view moves up when
//!   the area would be under it, with the window windowed and
//!   fullscreen; hidden; and the shown and hidden events), and while text
//!   input is stopped;
//! * the view's press handlers, given stand-ins for `UIPress`, `UIKey`
//!   and `UIPressesEvent` (a key's HID usage, and the presses without a
//!   key: the arrows, select, menu, play/pause and another), with a
//!   keyboard (which takes them over) and without;
//! * a game controller's keyboard: a stand-in `GCKeyboard` (with its
//!   input) posted with `GCKeyboardDidConnectNotification`, whose key
//!   handler, set by the driver, the program calls (a key, Shift and a
//!   key, a key held while text is typed), then
//!   `GCKeyboardDidDisconnectNotification`; and the simulator's own
//!   (`GCKeyboard.coalescedKeyboard`), its handler and queue, posted as
//!   disconnected, so that the presses and the text field go on without
//!   a keyboard (backspaces for what's deleted, the text reset when it
//!   gets short), then as connected again;
//! * real keys and text, through idb (the runner's `key` and `text`
//!   requests, when it drives idb: else those checks are skipped, `note:
//!   skipping, idb unavailable`, see below): keys with text input
//!   stopped and started, text, Backspace, an arrow and Return. The view
//!   is made opaque first, as for the touches.
//!
//! Then the window is destroyed. Timestamps aren't traced; the keyboards'
//! IDs, which are addresses, are named by their first appearance (`k1`,
//! `k2`; `global` for 0).
//!
//! What is deterministic: UIKit shows its on-screen keyboard on the
//! simulator (GameController has its coalesced keyboard from the start),
//! and that keyboard's notifications, which make its shown and hidden
//! events and set its height, come some while the text field becomes
//! the first responder or resigns and some whenever the run loop gets to
//! them, differently on a newly booted simulator and on one whose
//! hardware keyboard has typed in a text field. So the program first has
//! idb type a key (A, not traced) in its text field, after which they
//! come the same way on both, and then turns the event pump off
//! (`sdl3::video::set_ios_event_pump()`) while it checks what it does
//! in-process: the run loop doesn't run, and only the notifications that
//! come at once (and those the program posts) are traced. Once the pump
//! is on again (for idb's keys), the on-screen keyboard's events and
//! state aren't traced; and since the coalesced keyboard's key events
//! come from its own queue and the rest from the main thread, each of the
//! two is traced in its own order. Without idb, the trace isn't compared.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5).
//! Elsewhere this program does nothing.

#[cfg(not(target_os = "ios"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "ios")]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "ios")]
mod uikit_support;

#[cfg(target_os = "ios")]
fn main() {
    ios::main()
}

#[cfg(target_os = "ios")]
mod ios {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::{c_char, c_void, CStr};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::keyboard;
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, KeyboardID, WindowID};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::properties::Properties;
    use sdl3::video::{self, textinput, Rect, Window};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit, idb, request};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
coalesced keyboard: no
start: keyboards: (none); has keyboard=no
screen keyboard support: yes
active: event: KEYBOARD_ADDED(which=k1)
active: keyboards: k1 name=\"Keyboard\"; has keyboard=yes
create: ok
created: event: W:WINDOW_SAFE_AREA_CHANGED(0,0)
created: event: W:WINDOW_MOUSE_ENTER(0,0)
created: event: W:WINDOW_FOCUS_GAINED(0,0)
created: event: W:WINDOW_SHOWN(0,0)
created: event: W:WINDOW_PIXEL_SIZE_CHANGED(393,852)
text field: SDLUITextField (a UITextField) hidden=yes delegate=view controller subviews=1
view controller: text field delegate=yes
text field: can paste=no
created: field: first responder=no keyboard type=0 return key=0 capitalization=2 autocorrection=0 spell checking=0 secure=no return key automatically=no content type=nil text=spaces(64)
created: text input active=no screen keyboard shown=no
created: view frame=0,0 393x852
key commands: 5
key command: input=up modifiers=0x0 action=handleCommand:
key command up: event: KEY_DOWN(which=global scancode=82 key=0x40000052 mod=0x0 raw=0 down=1 repeat=0 w=W)
key command up: event: KEY_UP(which=global scancode=82 key=0x40000052 mod=0x0 raw=0 down=0 repeat=0 w=W)
key command: input=down modifiers=0x0 action=handleCommand:
key command down: event: KEY_DOWN(which=global scancode=81 key=0x40000051 mod=0x0 raw=0 down=1 repeat=0 w=W)
key command down: event: KEY_UP(which=global scancode=81 key=0x40000051 mod=0x0 raw=0 down=0 repeat=0 w=W)
key command: input=left modifiers=0x0 action=handleCommand:
key command left: event: KEY_DOWN(which=global scancode=80 key=0x40000050 mod=0x0 raw=0 down=1 repeat=0 w=W)
key command left: event: KEY_UP(which=global scancode=80 key=0x40000050 mod=0x0 raw=0 down=0 repeat=0 w=W)
key command: input=right modifiers=0x0 action=handleCommand:
key command right: event: KEY_DOWN(which=global scancode=79 key=0x4000004f mod=0x0 raw=0 down=1 repeat=0 w=W)
key command right: event: KEY_UP(which=global scancode=79 key=0x4000004f mod=0x0 raw=0 down=0 repeat=0 w=W)
key command: input=escape modifiers=0x0 action=handleCommand:
key command escape: event: KEY_DOWN(which=global scancode=41 key=0x1b mod=0x0 raw=0 down=1 repeat=0 w=W)
key command escape: event: KEY_UP(which=global scancode=41 key=0x1b mod=0x0 raw=0 down=0 repeat=0 w=W)
key command \"x\": events: (none)
start: ok
start: event: SCREEN_KEYBOARD_SHOWN
start: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
start: text input active=yes screen keyboard shown=yes
start: later: events: (none)
start: later: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
start: later: text input active=yes screen keyboard shown=yes
start: later: view frame=0,0 393x852
start again: ok
start again: event: SCREEN_KEYBOARD_HIDDEN
insert \"hi\": event: KEY_DOWN(which=global scancode=11 key=0x68 mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"hi\": event: KEY_UP(which=global scancode=11 key=0x68 mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"hi\": event: KEY_DOWN(which=global scancode=12 key=0x69 mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"hi\": event: KEY_UP(which=global scancode=12 key=0x69 mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"hi\": event: TEXT_INPUT(text=\"hi\" w=W)
insert \"hi\": text=\"                                                                hi\"
insert \"A\": event: KEY_DOWN(which=global scancode=225 key=0x400000e1 mod=0x1 raw=0 down=1 repeat=0 w=W)
insert \"A\": event: KEY_DOWN(which=global scancode=4 key=0x61 mod=0x1 raw=0 down=1 repeat=0 w=W)
insert \"A\": event: KEY_UP(which=global scancode=4 key=0x61 mod=0x1 raw=0 down=0 repeat=0 w=W)
insert \"A\": event: KEY_UP(which=global scancode=225 key=0x400000e1 mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"A\": event: TEXT_INPUT(text=\"A\" w=W)
insert \"A\": text=\"                                                                hiA\"
insert \"é😀\": event: KEY_DOWN(which=global scancode=400 key=0x0 mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"é😀\": event: KEY_UP(which=global scancode=400 key=0x0 mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"é😀\": event: KEY_DOWN(which=global scancode=401 key=0x0 mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"é😀\": event: KEY_UP(which=global scancode=401 key=0x0 mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"é😀\": event: KEY_DOWN(which=global scancode=402 key=0x0 mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"é😀\": event: KEY_UP(which=global scancode=402 key=0x0 mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"é😀\": event: TEXT_INPUT(text=\"é😀\" w=W)
insert \"é😀\": text=\"                                                                hiAé😀\"
delete 1: events: (none)
delete 1: text=\"                                                                hiAé\"
delete 2: events: (none)
delete 2: text=\"                                                                hiA\"
should return: yes
should return: event: KEY_DOWN(which=global scancode=40 key=0xd mod=0x0 raw=0 down=1 repeat=0 w=W)
should return: event: KEY_UP(which=global scancode=40 key=0xd mod=0x0 raw=0 down=0 repeat=0 w=W)
should return: text input active=yes screen keyboard shown=no
should return (hides the IME): yes
should return (hides the IME): event: KEY_DOWN(which=global scancode=40 key=0xd mod=0x0 raw=0 down=1 repeat=0 w=W)
should return (hides the IME): event: KEY_UP(which=global scancode=40 key=0xd mod=0x0 raw=0 down=0 repeat=0 w=W)
should return (hides the IME): text input active=no screen keyboard shown=no
should return (hides the IME): field: first responder=no keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
should return (stopped): yes
should return (stopped): event: KEY_DOWN(which=global scancode=40 key=0xd mod=0x0 raw=0 down=1 repeat=0 w=W)
should return (stopped): event: KEY_UP(which=global scancode=40 key=0xd mod=0x0 raw=0 down=0 repeat=0 w=W)
stop again: ok
stop again: events: (none)
stop again: text input active=no screen keyboard shown=no
props type 0: start: ok
props type 0: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props type 0: event: SCREEN_KEYBOARD_SHOWN
type 0: stop: ok
type 0: field: first responder=no keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
type 0: event: SCREEN_KEYBOARD_HIDDEN
props type 1: start: ok
props type 1: field: first responder=yes keyboard type=0 return key=0 capitalization=1 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"name\" text=spaces(64)
props type 1: event: SCREEN_KEYBOARD_SHOWN
type 1: stop: ok
type 1: field: first responder=no keyboard type=0 return key=0 capitalization=1 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"name\" text=spaces(64)
type 1: event: SCREEN_KEYBOARD_HIDDEN
props type 2: start: ok
props type 2: field: first responder=yes keyboard type=7 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"email\" text=spaces(64)
props type 2: event: SCREEN_KEYBOARD_SHOWN
type 2: stop: ok
type 2: field: first responder=no keyboard type=7 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"email\" text=spaces(64)
type 2: event: SCREEN_KEYBOARD_HIDDEN
props type 3: start: ok
props type 3: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"username\" text=spaces(64)
props type 3: event: SCREEN_KEYBOARD_SHOWN
type 3: stop: ok
type 3: field: first responder=no keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"username\" text=spaces(64)
type 3: event: SCREEN_KEYBOARD_HIDDEN
props type 4: start: ok
props type 4: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=yes return key automatically=yes content type=\"password\" text=spaces(64)
props type 4: event: SCREEN_KEYBOARD_SHOWN
type 4: stop: ok
type 4: field: first responder=no keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=yes return key automatically=yes content type=\"password\" text=spaces(64)
type 4: event: SCREEN_KEYBOARD_HIDDEN
props type 5: start: ok
props type 5: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"password\" text=spaces(64)
props type 5: event: SCREEN_KEYBOARD_SHOWN
type 5: stop: ok
type 5: field: first responder=no keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"password\" text=spaces(64)
type 5: event: SCREEN_KEYBOARD_HIDDEN
props type 6: start: ok
props type 6: field: first responder=yes keyboard type=8 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props type 6: event: SCREEN_KEYBOARD_SHOWN
type 6: stop: ok
type 6: field: first responder=no keyboard type=8 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
type 6: event: SCREEN_KEYBOARD_HIDDEN
props type 7: start: ok
props type 7: field: first responder=yes keyboard type=4 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=yes return key automatically=yes content type=\"one-time-code\" text=\"\"
props type 7: event: SCREEN_KEYBOARD_SHOWN
type 7: stop: ok
type 7: field: first responder=no keyboard type=4 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=yes return key automatically=yes content type=\"one-time-code\" text=\"\"
type 7: event: SCREEN_KEYBOARD_HIDDEN
props type 8: start: ok
props type 8: field: first responder=yes keyboard type=4 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"one-time-code\" text=\"\"
props type 8: event: SCREEN_KEYBOARD_SHOWN
type 8: stop: ok
type 8: field: first responder=no keyboard type=4 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"one-time-code\" text=\"\"
type 8: event: SCREEN_KEYBOARD_HIDDEN
props type 9: start: ok
props type 9: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=\"\"
props type 9: event: SCREEN_KEYBOARD_SHOWN
type 9: stop: ok
type 9: field: first responder=no keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
type 9: event: SCREEN_KEYBOARD_HIDDEN
props capitalization 0: start: ok
props capitalization 0: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props capitalization 0: event: SCREEN_KEYBOARD_SHOWN
props name, capitalization 0: start: ok
props name, capitalization 0: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"name\" text=spaces(64)
props name, capitalization 0: event: SCREEN_KEYBOARD_HIDDEN
props name, capitalization 0: event: SCREEN_KEYBOARD_SHOWN
props capitalization 1: start: ok
props capitalization 1: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props capitalization 1: event: SCREEN_KEYBOARD_HIDDEN
props capitalization 1: event: SCREEN_KEYBOARD_SHOWN
props name, capitalization 1: start: ok
props name, capitalization 1: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"name\" text=spaces(64)
props name, capitalization 1: event: SCREEN_KEYBOARD_HIDDEN
props name, capitalization 1: event: SCREEN_KEYBOARD_SHOWN
props capitalization 2: start: ok
props capitalization 2: field: first responder=yes keyboard type=0 return key=0 capitalization=1 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props capitalization 2: event: SCREEN_KEYBOARD_HIDDEN
props capitalization 2: event: SCREEN_KEYBOARD_SHOWN
props name, capitalization 2: start: ok
props name, capitalization 2: field: first responder=yes keyboard type=0 return key=0 capitalization=1 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"name\" text=spaces(64)
props name, capitalization 2: event: SCREEN_KEYBOARD_HIDDEN
props name, capitalization 2: event: SCREEN_KEYBOARD_SHOWN
props capitalization 3: start: ok
props capitalization 3: field: first responder=yes keyboard type=0 return key=0 capitalization=3 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props capitalization 3: event: SCREEN_KEYBOARD_HIDDEN
props capitalization 3: event: SCREEN_KEYBOARD_SHOWN
props name, capitalization 3: start: ok
props name, capitalization 3: field: first responder=yes keyboard type=0 return key=0 capitalization=3 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"name\" text=spaces(64)
props name, capitalization 3: event: SCREEN_KEYBOARD_HIDDEN
props name, capitalization 3: event: SCREEN_KEYBOARD_SHOWN
props capitalization 4: start: ok
props capitalization 4: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props capitalization 4: event: SCREEN_KEYBOARD_HIDDEN
props capitalization 4: event: SCREEN_KEYBOARD_SHOWN
props name, capitalization 4: start: ok
props name, capitalization 4: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"name\" text=spaces(64)
props name, capitalization 4: event: SCREEN_KEYBOARD_HIDDEN
props name, capitalization 4: event: SCREEN_KEYBOARD_SHOWN
props name: start: ok
props name: field: first responder=yes keyboard type=0 return key=0 capitalization=1 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"name\" text=spaces(64)
props name: event: SCREEN_KEYBOARD_HIDDEN
props name: event: SCREEN_KEYBOARD_SHOWN
props autocorrect on: start: ok
props autocorrect on: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props autocorrect on: event: SCREEN_KEYBOARD_HIDDEN
props autocorrect on: event: SCREEN_KEYBOARD_SHOWN
props autocorrect off: start: ok
props autocorrect off: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=1 spell checking=1 secure=no return key automatically=yes content type=nil text=spaces(64)
props autocorrect off: event: SCREEN_KEYBOARD_HIDDEN
props autocorrect off: event: SCREEN_KEYBOARD_SHOWN
props multiline on: start: ok
props multiline on: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props multiline on: event: SCREEN_KEYBOARD_HIDDEN
props multiline on: event: SCREEN_KEYBOARD_SHOWN
props multiline off: start: ok
props multiline off: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=no content type=nil text=spaces(64)
props multiline off: event: SCREEN_KEYBOARD_HIDDEN
props multiline off: event: SCREEN_KEYBOARD_SHOWN
props multiline unset (the return key hides the IME): start: ok
props multiline unset (the return key hides the IME): field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=no content type=nil text=spaces(64)
props multiline unset (the return key hides the IME): event: SCREEN_KEYBOARD_HIDDEN
props multiline unset (the return key hides the IME): event: SCREEN_KEYBOARD_SHOWN
props multiline on (the return key hides the IME): start: ok
props multiline on (the return key hides the IME): field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props multiline on (the return key hides the IME): event: SCREEN_KEYBOARD_HIDDEN
props multiline on (the return key hides the IME): event: SCREEN_KEYBOARD_SHOWN
props stopped: event: SCREEN_KEYBOARD_HIDDEN
props email, active: start: ok
props email, active: field: first responder=yes keyboard type=7 return key=0 capitalization=0 autocorrection=1 spell checking=1 secure=no return key automatically=yes content type=\"email\" text=spaces(64)
props email, active: event: SCREEN_KEYBOARD_SHOWN
props password, while active: start: ok
props password, while active: field: first responder=yes keyboard type=0 return key=0 capitalization=1 autocorrection=2 spell checking=2 secure=yes return key automatically=no content type=\"password\" text=spaces(64)
props password, while active: event: SCREEN_KEYBOARD_HIDDEN
props, while active: text input active=yes screen keyboard shown=no
props: start without properties: ok
props: start without properties: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
props: stopped: events: (none)
props one-time code: start: ok
props one-time code: field: first responder=yes keyboard type=4 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=yes return key automatically=yes content type=\"one-time-code\" text=\"\"
props one-time code: event: SCREEN_KEYBOARD_SHOWN
insert \"1\" (one-time code): event: KEY_DOWN(which=global scancode=30 key=0x31 mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"1\" (one-time code): event: KEY_UP(which=global scancode=30 key=0x31 mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"1\" (one-time code): event: TEXT_INPUT(text=\"1\" w=W)
insert \"1\" (one-time code): text=\"1\"
insert \"23\" (one-time code): events: (none)
insert \"23\" (one-time code): text=\"123\"
insert \"4\" (one-time code): events: (none)
insert \"4\" (one-time code): text=\"1234\"
delete (one-time code): events: (none)
delete (one-time code): text=\"123\"
one-time code: stopped: field: first responder=no keyboard type=4 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=yes return key automatically=yes content type=\"one-time-code\" text=\"123\"
one-time code: stopped: event: SCREEN_KEYBOARD_HIDDEN
props one-time code again: start: ok
props one-time code again: field: first responder=yes keyboard type=4 return key=0 capitalization=0 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=\"one-time-code\" text=\"123\"
props one-time code again: event: SCREEN_KEYBOARD_SHOWN
props text after the one-time code: start: ok
props text after the one-time code: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=\"123\"
props text after the one-time code: event: SCREEN_KEYBOARD_HIDDEN
props text after the one-time code: event: SCREEN_KEYBOARD_SHOWN
insert \"z\" (after the one-time code): event: KEY_DOWN(which=global scancode=31 key=0x32 mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"z\" (after the one-time code): event: KEY_UP(which=global scancode=31 key=0x32 mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"z\" (after the one-time code): event: KEY_DOWN(which=global scancode=32 key=0x33 mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"z\" (after the one-time code): event: KEY_UP(which=global scancode=32 key=0x33 mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"z\" (after the one-time code): event: KEY_DOWN(which=global scancode=29 key=0x7a mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"z\" (after the one-time code): event: KEY_UP(which=global scancode=29 key=0x7a mod=0x0 raw=0 down=0 repeat=0 w=W)
insert \"z\" (after the one-time code): event: TEXT_INPUT(text=\"23z\" w=W)
insert \"z\" (after the one-time code): text=\"123z\"
text after the one-time code: stopped: field: first responder=no keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
text after the one-time code: stopped: event: SCREEN_KEYBOARD_HIDDEN
start: ok
start (plain): field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
start (plain), stopped: event: SCREEN_KEYBOARD_SHOWN
start (plain), stopped: event: SCREEN_KEYBOARD_HIDDEN
area: start: ok
area: start: event: SCREEN_KEYBOARD_SHOWN
area 10,700 100x40: ok
area 10,700 100x40: view frame=0,0 393x852
keyboard will show (300): view frame=0,-188 393x852
keyboard will show (300): event: W:WINDOW_SAFE_AREA_CHANGED(0,0)
keyboard did show: events: (none)
keyboard did show: text input active=yes screen keyboard shown=yes
area 10,100 100x40: ok
area 10,100 100x40: view frame=0,0 393x852
area 0,800 50x50: ok
area 0,800 50x50: view frame=0,-298 393x852
area: 0,800 50x50 cursor=0
fullscreen: ok
fullscreen: view frame=0,-298 393x852
fullscreen: event: W:WINDOW_ENTER_FULLSCREEN(0,0)
fullscreen: event: W:WINDOW_SAFE_AREA_CHANGED(0,0)
windowed: ok
windowed: view frame=0,-298 393x852
windowed: event: W:WINDOW_LEAVE_FULLSCREEN(0,0)
windowed: event: W:WINDOW_SAFE_AREA_CHANGED(0,0)
keyboard will show (400): view frame=0,-398 393x852
area none: ok
area none: view frame=0,0 393x852
area 0,800 50x50 again: view frame=0,-398 393x852
keyboard will hide: view frame=0,0 393x852
keyboard will hide: event: W:WINDOW_SAFE_AREA_CHANGED(0,0)
keyboard did hide: event: SCREEN_KEYBOARD_HIDDEN
keyboard did hide: text input active=yes screen keyboard shown=no
keyboard did show again: event: SCREEN_KEYBOARD_SHOWN
keyboard did show again: text input active=yes screen keyboard shown=yes
area: stop: ok
area: stop: event: SCREEN_KEYBOARD_HIDDEN
area 0,700 50x50, stopped: view frame=0,0 393x852
keyboard will show (300), stopped: view frame=0,-198 393x852
keyboard will hide, stopped: view frame=0,0 393x852
area: done: event: W:WINDOW_SAFE_AREA_CHANGED(0,0)
press a (with a keyboard) began: events: (none)
press a (with a keyboard) changed: events: (none)
press a (with a keyboard) ended: events: (none)
press a (with a keyboard) began and cancelled: events: (none)
gckeyboard: coalesced: yes handler=yes queue=org.libsdl.input.keyboard
gckeyboard connected: event: KEYBOARD_ADDED(which=k2)
gckeyboard connected: keyboards: k1 name=\"Keyboard\", k2 name=\"Keyboard\"; has keyboard=yes
gckeyboard: handler=yes queue=yes
gckeyboard a pressed: event: KEY_DOWN(which=k2 scancode=4 key=0x61 mod=0x0 raw=0 down=1 repeat=0 w=W)
gckeyboard a released: event: KEY_UP(which=k2 scancode=4 key=0x61 mod=0x0 raw=0 down=0 repeat=0 w=W)
gckeyboard shift b pressed: event: KEY_DOWN(which=k2 scancode=225 key=0x400000e1 mod=0x1 raw=0 down=1 repeat=0 w=W)
gckeyboard shift b pressed: event: KEY_DOWN(which=k2 scancode=5 key=0x62 mod=0x1 raw=0 down=1 repeat=0 w=W)
gckeyboard shift b released: event: KEY_UP(which=k2 scancode=5 key=0x62 mod=0x1 raw=0 down=0 repeat=0 w=W)
gckeyboard shift b released: event: KEY_UP(which=k2 scancode=225 key=0x400000e1 mod=0x0 raw=0 down=0 repeat=0 w=W)
text input: ok
gckeyboard text input: event: SCREEN_KEYBOARD_SHOWN
gckeyboard c pressed (text input): event: KEY_DOWN(which=k2 scancode=6 key=0x63 mod=0x0 raw=0 down=1 repeat=0 w=W)
insert \"c\" (a key pressed): event: TEXT_INPUT(text=\"c\" w=W)
insert \"c\" (a key pressed): text=\"                                                                c\"
gckeyboard c released (text input): event: KEY_UP(which=k2 scancode=6 key=0x63 mod=0x0 raw=0 down=0 repeat=0 w=W)
gckeyboard text input stopped: event: SCREEN_KEYBOARD_HIDDEN
gckeyboard disconnected: event: KEYBOARD_REMOVED(which=k2)
gckeyboard: handler=no
gckeyboard disconnected: keyboards: k1 name=\"Keyboard\"; has keyboard=yes
coalesced keyboard disconnected: event: KEYBOARD_REMOVED(which=k1)
coalesced keyboard disconnected: keyboards: (none); has keyboard=no
coalesced keyboard: handler=no
press a began: event: KEY_DOWN(which=global scancode=4 key=0x61 mod=0x0 raw=0 down=1 repeat=0 w=W)
press a changed: events: (none)
press a ended: event: KEY_UP(which=global scancode=4 key=0x61 mod=0x0 raw=0 down=0 repeat=0 w=W)
press a began and cancelled: event: KEY_DOWN(which=global scancode=4 key=0x61 mod=0x0 raw=0 down=1 repeat=0 w=W)
press a began and cancelled: event: KEY_UP(which=global scancode=4 key=0x61 mod=0x0 raw=0 down=0 repeat=0 w=W)
press return began: event: KEY_DOWN(which=global scancode=40 key=0xd mod=0x0 raw=0 down=1 repeat=0 w=W)
press return changed: events: (none)
press return ended: event: KEY_UP(which=global scancode=40 key=0xd mod=0x0 raw=0 down=0 repeat=0 w=W)
press return began and cancelled: event: KEY_DOWN(which=global scancode=40 key=0xd mod=0x0 raw=0 down=1 repeat=0 w=W)
press return began and cancelled: event: KEY_UP(which=global scancode=40 key=0xd mod=0x0 raw=0 down=0 repeat=0 w=W)
press up arrow began: event: KEY_DOWN(which=global scancode=82 key=0x40000052 mod=0x0 raw=0 down=1 repeat=0 w=W)
press up arrow changed: events: (none)
press up arrow ended: event: KEY_UP(which=global scancode=82 key=0x40000052 mod=0x0 raw=0 down=0 repeat=0 w=W)
press up arrow began and cancelled: event: KEY_DOWN(which=global scancode=82 key=0x40000052 mod=0x0 raw=0 down=1 repeat=0 w=W)
press up arrow began and cancelled: event: KEY_UP(which=global scancode=82 key=0x40000052 mod=0x0 raw=0 down=0 repeat=0 w=W)
press down arrow began: event: KEY_DOWN(which=global scancode=81 key=0x40000051 mod=0x0 raw=0 down=1 repeat=0 w=W)
press down arrow changed: events: (none)
press down arrow ended: event: KEY_UP(which=global scancode=81 key=0x40000051 mod=0x0 raw=0 down=0 repeat=0 w=W)
press down arrow began and cancelled: event: KEY_DOWN(which=global scancode=81 key=0x40000051 mod=0x0 raw=0 down=1 repeat=0 w=W)
press down arrow began and cancelled: event: KEY_UP(which=global scancode=81 key=0x40000051 mod=0x0 raw=0 down=0 repeat=0 w=W)
press left arrow began: event: KEY_DOWN(which=global scancode=80 key=0x40000050 mod=0x0 raw=0 down=1 repeat=0 w=W)
press left arrow changed: events: (none)
press left arrow ended: event: KEY_UP(which=global scancode=80 key=0x40000050 mod=0x0 raw=0 down=0 repeat=0 w=W)
press left arrow began and cancelled: event: KEY_DOWN(which=global scancode=80 key=0x40000050 mod=0x0 raw=0 down=1 repeat=0 w=W)
press left arrow began and cancelled: event: KEY_UP(which=global scancode=80 key=0x40000050 mod=0x0 raw=0 down=0 repeat=0 w=W)
press right arrow began: event: KEY_DOWN(which=global scancode=79 key=0x4000004f mod=0x0 raw=0 down=1 repeat=0 w=W)
press right arrow changed: events: (none)
press right arrow ended: event: KEY_UP(which=global scancode=79 key=0x4000004f mod=0x0 raw=0 down=0 repeat=0 w=W)
press right arrow began and cancelled: event: KEY_DOWN(which=global scancode=79 key=0x4000004f mod=0x0 raw=0 down=1 repeat=0 w=W)
press right arrow began and cancelled: event: KEY_UP(which=global scancode=79 key=0x4000004f mod=0x0 raw=0 down=0 repeat=0 w=W)
press select began: event: KEY_DOWN(which=global scancode=40 key=0xd mod=0x0 raw=0 down=1 repeat=0 w=W)
press select changed: events: (none)
press select ended: event: KEY_UP(which=global scancode=40 key=0xd mod=0x0 raw=0 down=0 repeat=0 w=W)
press select began and cancelled: event: KEY_DOWN(which=global scancode=40 key=0xd mod=0x0 raw=0 down=1 repeat=0 w=W)
press select began and cancelled: event: KEY_UP(which=global scancode=40 key=0xd mod=0x0 raw=0 down=0 repeat=0 w=W)
press menu began: event: KEY_DOWN(which=global scancode=41 key=0x1b mod=0x0 raw=0 down=1 repeat=0 w=W)
press menu changed: events: (none)
press menu ended: event: KEY_UP(which=global scancode=41 key=0x1b mod=0x0 raw=0 down=0 repeat=0 w=W)
press menu began and cancelled: event: KEY_DOWN(which=global scancode=41 key=0x1b mod=0x0 raw=0 down=1 repeat=0 w=W)
press menu began and cancelled: event: KEY_UP(which=global scancode=41 key=0x1b mod=0x0 raw=0 down=0 repeat=0 w=W)
press play/pause began: event: KEY_DOWN(which=global scancode=72 key=0x40000048 mod=0x0 raw=0 down=1 repeat=0 w=W)
press play/pause changed: events: (none)
press play/pause ended: event: KEY_UP(which=global scancode=72 key=0x40000048 mod=0x0 raw=0 down=0 repeat=0 w=W)
press play/pause began and cancelled: event: KEY_DOWN(which=global scancode=72 key=0x40000048 mod=0x0 raw=0 down=1 repeat=0 w=W)
press play/pause began and cancelled: event: KEY_UP(which=global scancode=72 key=0x40000048 mod=0x0 raw=0 down=0 repeat=0 w=W)
press page up began: events: (none)
press page up changed: events: (none)
press page up ended: events: (none)
press page up began and cancelled: events: (none)
no keyboard: start: ok
no keyboard: start: event: SCREEN_KEYBOARD_SHOWN
no keyboard: start: field: first responder=yes keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
insert \"ab\" (no keyboard): event: TEXT_INPUT(text=\"ab\" w=W)
insert \"ab\" (no keyboard): text=\"                                                                ab\"
insert \"Hé😀\" (no keyboard): event: TEXT_INPUT(text=\"Hé😀\" w=W)
insert \"Hé😀\" (no keyboard): text=\"                                                                abHé😀\"
delete 1 (no keyboard): event: KEY_DOWN(which=global scancode=42 key=0x8 mod=0x0 raw=0 down=1 repeat=0 w=W)
delete 1 (no keyboard): event: KEY_UP(which=global scancode=42 key=0x8 mod=0x0 raw=0 down=0 repeat=0 w=W)
delete 1 (no keyboard): text=\"                                                                abHé\"
delete 2 (no keyboard): event: KEY_DOWN(which=global scancode=42 key=0x8 mod=0x0 raw=0 down=1 repeat=0 w=W)
delete 2 (no keyboard): event: KEY_UP(which=global scancode=42 key=0x8 mod=0x0 raw=0 down=0 repeat=0 w=W)
delete 2 (no keyboard): text=\"                                                                abH\"
delete (no keyboard) x10: backspace downs=10 ups=10 other events=0 text=spaces(57)
delete (no keyboard) x50: backspace downs=50 ups=50 other events=0 text=spaces(7)
delete (no keyboard) x10: backspace downs=7 ups=7 other events=0 text=\"\"
insert \"x\" (no keyboard): event: TEXT_INPUT(text=\"x\" w=W)
insert \"x\" (no keyboard): text=\"x\"
no keyboard: stop: ok
no keyboard: stop: event: SCREEN_KEYBOARD_HIDDEN
no keyboard: stop: field: first responder=no keyboard type=0 return key=0 capitalization=2 autocorrection=2 spell checking=2 secure=no return key automatically=yes content type=nil text=spaces(64)
coalesced keyboard connected: event: KEYBOARD_ADDED(which=k1)
coalesced keyboard connected: keyboards: k1 name=\"Keyboard\"; has keyboard=yes
coalesced keyboard: handler=yes
pump on: keyboards: k1 name=\"Keyboard\"; has keyboard=yes
idb: opaque: events: (none)
idb: key 4: event: KEY_DOWN(which=k1 scancode=4 key=0x61 mod=0x0 raw=0 down=1 repeat=0 w=W)
idb: key 4: event: KEY_UP(which=k1 scancode=4 key=0x61 mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: key 82: event: KEY_DOWN(which=k1 scancode=82 key=0x40000052 mod=0x0 raw=0 down=1 repeat=0 w=W)
idb: key 82: event: KEY_UP(which=k1 scancode=82 key=0x40000052 mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: start: ok
idb: start: events: (none)
idb: start: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=1 spell checking=1 secure=no return key automatically=yes content type=nil text=spaces(64)
idb: key 4 (text input): event: KEY_DOWN(which=k1 scancode=4 key=0x61 mod=0x0 raw=0 down=1 repeat=0 w=W)
idb: key 4 (text input): event: KEY_UP(which=k1 scancode=4 key=0x61 mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: key 4 (text input): event: TEXT_INPUT(text=\"a\" w=W)
idb: text=\"                                                                a\"
idb: text \"Hi!\" (text input): event: KEY_DOWN(which=k1 scancode=225 key=0x400000e1 mod=0x1 raw=0 down=1 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: KEY_DOWN(which=k1 scancode=11 key=0x68 mod=0x1 raw=0 down=1 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: KEY_UP(which=k1 scancode=11 key=0x68 mod=0x1 raw=0 down=0 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: KEY_UP(which=k1 scancode=225 key=0x400000e1 mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: KEY_DOWN(which=k1 scancode=12 key=0x69 mod=0x0 raw=0 down=1 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: KEY_UP(which=k1 scancode=12 key=0x69 mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: KEY_DOWN(which=k1 scancode=225 key=0x400000e1 mod=0x1 raw=0 down=1 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: KEY_DOWN(which=k1 scancode=30 key=0x31 mod=0x1 raw=0 down=1 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: KEY_UP(which=k1 scancode=30 key=0x31 mod=0x1 raw=0 down=0 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: KEY_UP(which=k1 scancode=225 key=0x400000e1 mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: text \"Hi!\" (text input): event: TEXT_INPUT(text=\"H\" w=W)
idb: text \"Hi!\" (text input): event: TEXT_INPUT(text=\"i\" w=W)
idb: text \"Hi!\" (text input): event: TEXT_INPUT(text=\"!\" w=W)
idb: text=\"                                                                aHi!\"
idb: key 42 (text input): event: KEY_DOWN(which=k1 scancode=42 key=0x8 mod=0x0 raw=0 down=1 repeat=0 w=W)
idb: key 42 (text input): event: KEY_UP(which=k1 scancode=42 key=0x8 mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: text=\"                                                                aHi\"
idb: key 82 (text input): event: KEY_DOWN(which=k1 scancode=82 key=0x40000052 mod=0x0 raw=0 down=1 repeat=0 w=W)
idb: key 82 (text input): event: KEY_UP(which=k1 scancode=82 key=0x40000052 mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: key 40 (text input): event: KEY_DOWN(which=k1 scancode=40 key=0xd mod=0x0 raw=0 down=1 repeat=0 w=W)
idb: key 40 (text input): event: KEY_UP(which=k1 scancode=40 key=0xd mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: key 40 (text input): event: KEY_DOWN(which=global scancode=40 key=0xd mod=0x0 raw=0 down=1 repeat=0 w=W)
idb: key 40 (text input): event: KEY_UP(which=global scancode=40 key=0xd mod=0x0 raw=0 down=0 repeat=0 w=W)
idb: text input active=yes
idb: field: first responder=yes keyboard type=0 return key=0 capitalization=0 autocorrection=1 spell checking=1 secure=no return key automatically=yes content type=nil text=\"                                                                aHi\"
idb: stop: ok
idb: stop: events: (none)
destroyed
destroyed: event: W:WINDOW_DESTROYED(0,0)
destroyed: keyboards: k1 name=\"Keyboard\"; has keyboard=yes
quit
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    fn yes_no(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    fn ok<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".into(),
            Err(e) => format!("error: {e}"),
        }
    }

    /// A number as C's `%g` prints it.
    fn g(v: f64) -> String {
        if v == 0.0 {
            return if v.is_sign_negative() { "-0" } else { "0" }.into();
        }
        let exponent = v.abs().log10().floor() as i32;
        if (-5..6).contains(&exponent) {
            let decimals = (5 - exponent).max(0) as usize;
            let s = format!("{v:.decimals$}");
            if s.contains('.') {
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                s
            }
        } else {
            let s = format!("{v:.5e}");
            let (mantissa, exp) = s.split_once('e').unwrap();
            let mantissa = if mantissa.contains('.') {
                mantissa.trim_end_matches('0').trim_end_matches('.')
            } else {
                mantissa
            };
            let exp: i32 = exp.parse().unwrap();
            format!(
                "{mantissa}e{}{:02}",
                if exp < 0 { '-' } else { '+' },
                exp.abs()
            )
        }
    }

    /// A text as the trace quotes it: bytes below 0x20, 0x7f, quotes and
    /// backslashes escaped.
    fn quoted(text: &str) -> String {
        let mut out = String::from("\"");
        for ch in text.chars() {
            if (ch as u32) < 0x20 || ch == '\x7f' || ch == '"' || ch == '\\' {
                out.push_str(&format!("\\x{:02x}", ch as u32));
            } else {
                out.push(ch);
            }
        }
        out.push('"');
        out
    }

    // ---- the objects the translated handlers are given

    /// What a `UIPress` (with its `UIKey`, an `SDLTestKey`) and a
    /// `UIPressesEvent` tell the view (an `SDLTestPress`).
    #[derive(Clone, Copy, Default)]
    struct Press {
        key: usize,
        press_type: isize,
        stamp: f64,
    }

    /// A `GCKeyboard` and its `GCKeyboardInput` (an `SDLTestKeyboard`),
    /// keeping the handler and the queue the driver sets.
    #[derive(Clone, Copy, Default)]
    struct Kb {
        input: usize,
        handler: usize,
        queue_set: bool,
    }

    thread_local! {
        static PRESSES: RefCell<HashMap<usize, Press>> = RefCell::new(HashMap::new());
        static KEY_CODES: RefCell<HashMap<usize, isize>> = RefCell::new(HashMap::new());
        static KBS: RefCell<HashMap<usize, Kb>> = RefCell::new(HashMap::new());
    }

    fn press(this: Id) -> Press {
        PRESSES.with(|p| {
            p.borrow()
                .get(&(this as usize))
                .copied()
                .unwrap_or_default()
        })
    }

    fn kb<R>(this: Id, change: impl FnOnce(&mut Kb) -> R) -> R {
        KBS.with(|k| change(k.borrow_mut().entry(this as usize).or_default()))
    }

    extern "C" fn key_code(this: Id, _: objc::Sel) -> isize {
        KEY_CODES.with(|k| k.borrow().get(&(this as usize)).copied().unwrap_or(0))
    }
    extern "C" fn press_key(this: Id, _: objc::Sel) -> Id {
        press(this).key as Id
    }
    extern "C" fn press_type(this: Id, _: objc::Sel) -> isize {
        press(this).press_type
    }
    extern "C" fn press_timestamp(this: Id, _: objc::Sel) -> f64 {
        press(this).stamp
    }
    extern "C" fn keyboard_input(this: Id, _: objc::Sel) -> Id {
        kb(this, |k| k.input) as Id
    }
    extern "C" fn set_key_changed_handler(this: Id, _: objc::Sel, handler: Id) {
        let copy = objc::block_copy(handler);
        let old = kb(this, |k| std::mem::replace(&mut k.handler, copy as usize));
        objc::block_release(old as Id);
    }
    extern "C" fn key_changed_handler(this: Id, _: objc::Sel) -> Id {
        kb(this, |k| k.handler) as Id
    }
    extern "C" fn set_handler_queue(this: Id, _: objc::Sel, queue: Id) {
        kb(this, |k| k.queue_set = !queue.is_null());
    }

    /// Declare `SDLTestKey`, `SDLTestPress` and `SDLTestKeyboard`.
    fn declare_classes() {
        // SAFETY: the implementations have the encodings' signatures.
        unsafe {
            objc::declare_class(
                "SDLTestKey",
                "NSObject",
                &[("keyCode", key_code as *const c_void, "q@:")],
            );
            objc::declare_class(
                "SDLTestPress",
                "NSObject",
                &[
                    ("key", press_key as *const c_void, "@@:"),
                    ("type", press_type as *const c_void, "q@:"),
                    ("timestamp", press_timestamp as *const c_void, "d@:"),
                ],
            );
            objc::declare_class(
                "SDLTestKeyboard",
                "NSObject",
                &[
                    ("keyboardInput", keyboard_input as *const c_void, "@@:"),
                    (
                        "setKeyChangedHandler:",
                        set_key_changed_handler as *const c_void,
                        "v@:@?",
                    ),
                    (
                        "keyChangedHandler",
                        key_changed_handler as *const c_void,
                        "@?@:",
                    ),
                    (
                        "setHandlerQueue:",
                        set_handler_queue as *const c_void,
                        "v@:@",
                    ),
                ],
            );
        }
    }

    /// `[cls new]` (kept for the life of the program).
    fn new_object(cls: &str) -> Id {
        // SAFETY: NSObject's class method (owned, never released).
        unsafe { objc::send0(objc::class(cls), "new") }
    }

    /// A press of a key (`code`, its HID usage), or of a button of a type
    /// (`code` -1: no key).
    fn press_of(code: isize, press_type: isize) -> Id {
        let p = new_object("SDLTestPress");
        let key = if code >= 0 {
            let key = new_object("SDLTestKey");
            KEY_CODES.with(|k| k.borrow_mut().insert(key as usize, code));
            key as usize
        } else {
            0
        };
        PRESSES.with(|presses| {
            presses.borrow_mut().insert(
                p as usize,
                Press {
                    key,
                    press_type,
                    stamp: 1.0,
                },
            )
        });
        p
    }

    // ---- labels: the keyboards' IDs, which are addresses, by first
    // appearance

    static KEYBOARDS: Mutex<Vec<KeyboardID>> = Mutex::new(Vec::new());

    fn kb_label(id: KeyboardID) -> String {
        if id == 0 {
            return "global".into();
        }
        let mut l = KEYBOARDS.lock().unwrap();
        let i = match l.iter().position(|&k| k == id) {
            Some(i) => i,
            None => {
                l.push(id);
                l.len() - 1
            }
        };
        format!("k{}", i + 1)
    }

    static WINDOW: Mutex<WindowID> = Mutex::new(0);

    fn wl(id: WindowID) -> &'static str {
        if id == *WINDOW.lock().unwrap() {
            "W"
        } else if id == 0 {
            "0"
        } else {
            "?"
        }
    }

    /// Once the run loop runs again: the on-screen keyboard's events, which
    /// come when its notifications do.
    static ASYNC_KEYBOARD: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    /// An event as the trace names it (the C program's names), `None` for
    /// the mice's device events (and the on-screen keyboard's, once
    /// they're asynchronous).
    fn describe(event: &Event) -> Option<String> {
        let ty = event.event_type();
        let n = event_name(ty);
        if (ty == events::EventType::SCREEN_KEYBOARD_SHOWN
            || ty == events::EventType::SCREEN_KEYBOARD_HIDDEN)
            && ASYNC_KEYBOARD.load(std::sync::atomic::Ordering::Relaxed)
        {
            return None;
        }
        Some(match event {
            Event::MouseDevice(_) => return None,
            Event::KeyboardDevice(e) => format!("{n}(which={})", kb_label(e.which)),
            Event::Key(e) => format!(
                "{n}(which={} scancode={} key={:#x} mod={:#x} raw={} down={} repeat={} w={})",
                kb_label(e.which),
                e.scancode.0,
                e.key.0,
                e.modifiers.0,
                e.raw,
                e.down as i32,
                e.repeat as i32,
                wl(e.window_id)
            ),
            Event::TextInput(e) => format!("{n}(text={} w={})", quoted(&e.text), wl(e.window_id)),
            Event::TextEditing(e) => format!(
                "{n}(text={} start={} length={} w={})",
                quoted(&e.text),
                e.start,
                e.length,
                wl(e.window_id)
            ),
            Event::Display(d) => format!("{n}({},{})", d.display_id, d.data1),
            Event::Window(w) => format!("{}:{n}({},{})", wl(w.window_id), w.data1, w.data2),
            _ => n,
        })
    }

    /// Pump, and the queued events.
    fn take_queued(out: &mut Vec<String>) {
        events::pump();
        while let Some(event) = events::poll() {
            out.extend(describe(&event));
        }
    }

    /// Pump for `ms` milliseconds (10 ms steps).
    fn settle(out: &mut Vec<String>, ms: u64) {
        take_queued(out);
        for _ in 0..ms / 10 {
            std::thread::sleep(Duration::from_millis(10));
            take_queued(out);
        }
    }

    fn seen_count(seen: &[String], what: &str) -> usize {
        seen.iter().filter(|e| e.contains(what)).count()
    }

    fn trace_seen(label: &str, seen: &[String]) {
        if seen.is_empty() {
            t(format!("{label}: events: (none)"));
        }
        for e in seen {
            t(format!("{label}: event: {e}"));
        }
    }

    fn trace_events(label: &str, ms: u64) {
        let mut seen = Vec::new();
        settle(&mut seen, ms);
        trace_seen(label, &seen);
    }

    /// The events the handlers queued at once.
    fn now_events(label: &str) {
        trace_events(label, 0);
    }

    // ---- what the trace tells of the app's state

    /// The window, its view controller, its view and its text field.
    struct App {
        w: Window,
        vc: Id,
        view: Id,
        tf: Id,
    }

    fn keyboards(label: &str) {
        let items: Vec<String> = keyboard::keyboards()
            .into_iter()
            .map(|id| {
                format!(
                    "{} name={}",
                    kb_label(id),
                    quoted(&keyboard::keyboard_name(id).unwrap_or_default())
                )
            })
            .collect();
        t(format!(
            "{label}: keyboards: {}; has keyboard={}",
            if items.is_empty() {
                "(none)".into()
            } else {
                items.join(", ")
            },
            yes_no(keyboard::has_keyboard())
        ));
    }

    /// An `NSString`'s length (UTF-16 code units).
    fn ns_length(s: Id) -> usize {
        // SAFETY: an NSString (or nil).
        unsafe { objc::send0(s, "length") }
    }

    fn field_text(app: &App) -> String {
        // SAFETY: a UITextField property.
        let text: Id = unsafe { objc::send0(app.tf, "text") };
        let s = objc::string(text).unwrap_or_default();
        if !s.is_empty() && s.chars().all(|c| c == ' ') {
            format!("spaces({})", ns_length(text))
        } else {
            quoted(&s)
        }
    }

    fn field(label: &str, app: &App) {
        let tf = app.tf;
        // SAFETY: UITextField's and UIResponder's properties.
        unsafe {
            let first_responder: bool = objc::send0(tf, "isFirstResponder");
            let keyboard_type: isize = objc::send0(tf, "keyboardType");
            let return_key: isize = objc::send0(tf, "returnKeyType");
            let capitalization: isize = objc::send0(tf, "autocapitalizationType");
            let autocorrection: isize = objc::send0(tf, "autocorrectionType");
            let spell_checking: isize = objc::send0(tf, "spellCheckingType");
            let secure: bool = objc::send0(tf, "isSecureTextEntry");
            let return_automatically: bool = objc::send0(tf, "enablesReturnKeyAutomatically");
            let content: Id = objc::send0(tf, "textContentType");
            t(format!(
                "{label}: field: first responder={} keyboard type={keyboard_type} return key={return_key} \
                 capitalization={capitalization} autocorrection={autocorrection} spell checking={spell_checking} \
                 secure={} return key automatically={} content type={} text={}",
                yes_no(first_responder),
                yes_no(secure),
                yes_no(return_automatically),
                objc::string(content).map_or("nil".into(), |c| quoted(&c)),
                field_text(app)
            ));
        }
    }

    fn text_state(label: &str, app: &App) {
        t(format!(
            "{label}: text input active={} screen keyboard shown={}",
            yes_no(app.w.text_input_active().unwrap()),
            yes_no(app.w.screen_keyboard_shown().unwrap())
        ));
    }

    /// `CGRect`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    fn frame(label: &str, app: &App) {
        // SAFETY: a UIView property.
        let f: CGRect = unsafe { objc::send0(app.view, "frame") };
        t(format!(
            "{label}: view frame={},{} {}x{}",
            g(f.x),
            g(f.y),
            g(f.w),
            g(f.h)
        ));
    }

    // ---- the text field

    fn insert(label: &str, app: &App, text: &str) {
        // SAFETY: UITextField's UIKeyInput method.
        unsafe {
            let () = objc::send1(app.tf, "insertText:", objc::ns_string(text));
        }
        let l = format!("insert {label}");
        now_events(&l);
        t(format!("{l}: text={}", field_text(app)));
    }

    fn delete_backward(label: &str, app: &App) {
        // SAFETY: UITextField's UIKeyInput method.
        unsafe {
            let () = objc::send0(app.tf, "deleteBackward");
        }
        let l = format!("delete {label}");
        now_events(&l);
        t(format!("{l}: text={}", field_text(app)));
    }

    /// Deleting `count` times: how many backspaces were sent, and what
    /// was left.
    fn delete_many(label: &str, app: &App, count: usize) {
        let mut seen = Vec::new();
        for _ in 0..count {
            // SAFETY: as above.
            unsafe {
                let () = objc::send0(app.tf, "deleteBackward");
            }
            take_queued(&mut seen);
        }
        let other = seen.len() - seen_count(&seen, "scancode=42 ");
        t(format!(
            "delete {label} x{count}: backspace downs={} ups={} other events={other} text={}",
            seen_count(&seen, "KEY_DOWN(which=global scancode=42 "),
            seen_count(&seen, "KEY_UP(which=global scancode=42 "),
            field_text(app)
        ));
    }

    // ---- the text input properties

    fn props_case(
        label: &str,
        app: &App,
        ty: Option<i64>,
        capitalization: Option<i64>,
        autocorrect: Option<bool>,
        multiline: Option<bool>,
    ) {
        let props = Properties::new();
        if let Some(ty) = ty {
            props
                .set(textinput::PROP_TEXTINPUT_TYPE_NUMBER, ty)
                .unwrap();
        }
        if let Some(c) = capitalization {
            props
                .set(textinput::PROP_TEXTINPUT_CAPITALIZATION_NUMBER, c)
                .unwrap();
        }
        if let Some(a) = autocorrect {
            props
                .set(textinput::PROP_TEXTINPUT_AUTOCORRECT_BOOLEAN, a)
                .unwrap();
        }
        if let Some(m) = multiline {
            props
                .set(textinput::PROP_TEXTINPUT_MULTILINE_BOOLEAN, m)
                .unwrap();
        }
        let l = format!("props {label}");
        t(format!(
            "{l}: start: {}",
            ok(&app.w.start_text_input_with_properties(Some(&props)))
        ));
        drop(props);
        field(&l, app);
        now_events(&l);
    }

    /// Stop text input (its result isn't traced).
    fn stop(app: &App) {
        let _ = app.w.stop_text_input();
    }

    // ---- the keyboard's notifications, posted

    #[link(name = "UIKit", kind = "framework")]
    unsafe extern "C" {
        static UIKeyboardWillShowNotification: Id;
        static UIKeyboardDidShowNotification: Id;
        static UIKeyboardWillHideNotification: Id;
        static UIKeyboardDidHideNotification: Id;
        static UIKeyboardFrameEndUserInfoKey: Id;
        static UIKeyInputUpArrow: Id;
        static UIKeyInputDownArrow: Id;
        static UIKeyInputLeftArrow: Id;
        static UIKeyInputRightArrow: Id;
        static UIKeyInputEscape: Id;
    }

    #[link(name = "GameController", kind = "framework")]
    unsafe extern "C" {
        static GCKeyboardDidConnectNotification: Id;
        static GCKeyboardDidDisconnectNotification: Id;
    }

    unsafe extern "C" {
        fn dispatch_queue_get_label(queue: Id) -> *const c_char;
    }

    fn post_keyboard(name: Id, y: f64, height: f64) {
        // SAFETY: NSValue's UIKit class method, NSDictionary's and the
        // notification center's methods.
        unsafe {
            let value: Id = objc::send1(
                objc::class("NSValue"),
                "valueWithCGRect:",
                CGRect {
                    x: 0.0,
                    y,
                    w: 393.0,
                    h: height,
                },
            );
            let info: Id = objc::send2(
                objc::class("NSDictionary"),
                "dictionaryWithObject:forKey:",
                value,
                UIKeyboardFrameEndUserInfoKey,
            );
            let center: Id = objc::send0(objc::class("NSNotificationCenter"), "defaultCenter");
            let () = objc::send3(
                center,
                "postNotificationName:object:userInfo:",
                name,
                std::ptr::null_mut::<c_void>(),
                info,
            );
        }
    }

    // ---- the presses

    fn presses(label: &str, app: &App, p: Id) {
        // SAFETY: NSSet's class method (autoreleased) and the view's press
        // handlers.
        unsafe {
            let set: Id = objc::send1(objc::class("NSSet"), "setWithObject:", p);
            let () = objc::send2(app.view, "pressesBegan:withEvent:", set, p);
            now_events(&format!("press {label} began"));
            let () = objc::send2(app.view, "pressesChanged:withEvent:", set, p);
            now_events(&format!("press {label} changed"));
            let () = objc::send2(app.view, "pressesEnded:withEvent:", set, p);
            now_events(&format!("press {label} ended"));
            let () = objc::send2(app.view, "pressesBegan:withEvent:", set, p);
            let () = objc::send2(app.view, "pressesCancelled:withEvent:", set, p);
            now_events(&format!("press {label} began and cancelled"));
        }
    }

    // ---- idb

    /// Ask the runner for keys or text, then wait for `count` events
    /// naming `until` (for at most a minute: idb starts its companion the
    /// first time), and a second more.
    fn idb_request(label: &str, what: &str, until: &str, count: usize) {
        let mut seen = Vec::new();
        println!("uikit_keyboard: {:.3} s: request {what}", elapsed());
        request(what);
        let end = Instant::now() + Duration::from_secs(60);
        while seen_count(&seen, until) < count && Instant::now() < end {
            take_queued(&mut seen);
            std::thread::sleep(Duration::from_millis(10));
        }
        settle(&mut seen, 1000);
        println!("uikit_keyboard: {:.3} s: {label} done", elapsed());
        // (the keyboard's events come from its queue, the others from the
        // main thread: each in their order)
        let (mut sorted, others): (Vec<String>, Vec<String>) =
            seen.into_iter().partition(|e| e.contains("(which=k1 "));
        sorted.extend(others);
        trace_seen(&format!("idb: {label}"), &sorted);
    }

    fn key_input_name(input: Id) -> String {
        // SAFETY: UIKit's constant strings, compared as upstream does.
        unsafe {
            if input == UIKeyInputUpArrow {
                "up".into()
            } else if input == UIKeyInputDownArrow {
                "down".into()
            } else if input == UIKeyInputLeftArrow {
                "left".into()
            } else if input == UIKeyInputRightArrow {
                "right".into()
            } else if input == UIKeyInputEscape {
                "escape".into()
            } else {
                quoted(&objc::string(input).unwrap_or_default())
            }
        }
    }

    /// The window's `UIWindow` (its property, a number here).
    fn uiwindow_of(w: &Window) -> Id {
        w.properties()
            .ok()
            .and_then(|p| p.get_number(video::window::PROP_WINDOW_UIKIT_WINDOW_POINTER))
            .map_or(std::ptr::null_mut(), |p| p as usize as Id)
    }

    /// The objects of an `NSArray`.
    fn items(array: Id) -> Vec<Id> {
        if array.is_null() {
            return Vec::new();
        }
        // SAFETY: an NSArray's count and objects.
        unsafe {
            let count: usize = objc::send0(array, "count");
            (0..count)
                .map(|i| objc::send1(array, "objectAtIndex:", i))
                .collect()
        }
    }

    fn is_kind_of(obj: Id, cls: &str) -> bool {
        // SAFETY: NSObject's method.
        unsafe { objc::send1(obj, "isKindOfClass:", objc::class(cls)) }
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    /// `textFieldShouldReturn:`, as the text field asks its delegate.
    fn should_return(app: &App) -> bool {
        // SAFETY: the view controller's UITextFieldDelegate method.
        unsafe { objc::send1(app.vc, "textFieldShouldReturn:", app.tf) }
    }

    fn post(name: Id, object: Id) {
        objc::post_notification_for(name, object);
    }

    /// A key handler's call, `^(GCKeyboardInput *, GCControllerButtonInput
    /// *, GCKeyCode, BOOL)`.
    fn key_changed(input: Id, handler: Id, code: isize, pressed: bool) {
        // SAFETY: the driver's handler, a block of that signature.
        unsafe {
            objc::call_block4(
                handler,
                input,
                std::ptr::null_mut::<c_void>(),
                code,
                pressed,
            )
        }
    }

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_keyboard: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_keyboard: {:.3} s: main", elapsed());
        declare_classes();
        let r = init::init(InitFlags::VIDEO);
        t(format!("init video: {}", ok(&r)));
        r.unwrap();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap()
        ));
        // SAFETY: GCKeyboard's class property.
        let coalesced: Id = unsafe { objc::send0(objc::class("GCKeyboard"), "coalescedKeyboard") };
        t(format!(
            "coalesced keyboard: {}",
            yes_no(!coalesced.is_null())
        ));
        keyboards("start");
        t(format!(
            "screen keyboard support: {}",
            yes_no(textinput::has_screen_keyboard_support())
        ));

        // The app becomes active around when the main function starts
        {
            let mut seen = Vec::new();
            let end = Instant::now() + Duration::from_secs(30);
            while application_state() != 0 && Instant::now() < end {
                take_queued(&mut seen);
                std::thread::sleep(Duration::from_millis(10));
            }
            settle(&mut seen, 100);
            // (the application events come when they come)
            for e in seen.iter().filter(|e| e.starts_with("KEY")) {
                t(format!("active: event: {e}"));
            }
        }
        keyboards("active");

        let r = Window::create("keyboard", 320, 480, WindowFlags::NONE);
        t(format!("create: {}", ok(&r)));
        let w = r.unwrap();
        *WINDOW.lock().unwrap() = w.id();
        trace_events("created", 500);
        // SAFETY: UIWindow's and UIViewController's properties.
        let (vc, view) = unsafe {
            let vc: Id = objc::send0(uiwindow_of(&w), "rootViewController");
            (vc, objc::send0::<Id>(vc, "view"))
        };
        // SAFETY: a UIView property.
        let subviews = items(unsafe { objc::send0(view, "subviews") });
        let tf = subviews
            .iter()
            .copied()
            .rfind(|&s| is_kind_of(s, "UITextField"))
            .unwrap_or(std::ptr::null_mut());
        let app = App { w, vc, view, tf };
        // SAFETY: UIView's and UITextField's properties, and UIResponder's
        // method.
        unsafe {
            let hidden: bool = objc::send0(tf, "isHidden");
            let delegate: Id = objc::send0(tf, "delegate");
            t(format!(
                "text field: {} (a {}) hidden={} delegate={} subviews={}",
                objc::class_name(tf),
                objc::superclass_name(tf),
                yes_no(hidden),
                if delegate == vc {
                    "view controller"
                } else {
                    "other"
                },
                subviews.len()
            ));
            t(format!(
                "view controller: text field delegate={}",
                yes_no(objc::conforms_to(vc, "UITextFieldDelegate"))
            ));
            let paste: bool = objc::send2(
                tf,
                "canPerformAction:withSender:",
                objc::sel("paste:"),
                std::ptr::null_mut::<c_void>(),
            );
            t(format!("text field: can paste={}", yes_no(paste)));
        }
        field("created", &app);
        text_state("created", &app);
        frame("created", &app);

        // The on-screen keyboard's notifications come differently until
        // the simulator's hardware keyboard has typed in a text field: a
        // key typed through idb while the text field is the first
        // responder (not traced) makes them come the same way on any
        // simulator. Then the run loop doesn't run (the event pump is off)
        // for the checks made in the program, so that only the
        // notifications that come at once, and those posted in it, do.
        let idb = idb();
        if idb.is_ok() {
            let mut warmup = Vec::new();
            let _ = app.w.start_text_input();
            settle(&mut warmup, 1000);
            println!(
                "uikit_keyboard: {:.3} s: request key 4 (the hardware keyboard's mode)",
                elapsed()
            );
            request("key 4");
            let end = Instant::now() + Duration::from_secs(60);
            while seen_count(&warmup, "KEY_UP") < 1 && Instant::now() < end {
                take_queued(&mut warmup);
                std::thread::sleep(Duration::from_millis(10));
            }
            settle(&mut warmup, 1000);
            let _ = app.w.stop_text_input();
            settle(&mut warmup, 1500);
            println!(
                "uikit_keyboard: {:.3} s: the hardware keyboard's mode ({} events)",
                elapsed(),
                warmup.len()
            );
        }
        video::set_ios_event_pump(false);

        // The key commands
        // SAFETY: the view controller's key commands, UIKeyCommand's
        // properties, and the view controller's handler.
        unsafe {
            let commands = items(objc::send0(vc, "keyCommands"));
            t(format!("key commands: {}", commands.len()));
            for c in commands {
                let input: Id = objc::send0(c, "input");
                let modifiers: isize = objc::send0(c, "modifierFlags");
                let action: objc::Sel = objc::send0(c, "action");
                t(format!(
                    "key command: input={} modifiers={:#x} action={}",
                    key_input_name(input),
                    modifiers,
                    objc::sel_name(action)
                ));
                let () = objc::send1(vc, "handleCommand:", c);
                now_events(&format!("key command {}", key_input_name(input)));
            }
            let x: Id = objc::send3(
                objc::class("UIKeyCommand"),
                "keyCommandWithInput:modifierFlags:action:",
                objc::ns_string("x"),
                0isize,
                objc::sel("handleCommand:"),
            );
            let () = objc::send1(vc, "handleCommand:", x);
            now_events("key command \"x\"");
        }

        // Text input, with the hardware keyboard (the simulator's)
        t(format!("start: {}", ok(&app.w.start_text_input())));
        now_events("start");
        field("start", &app);
        text_state("start", &app);
        trace_events("start: later", 1500);
        field("start: later", &app);
        text_state("start: later", &app);
        frame("start: later", &app);
        t(format!("start again: {}", ok(&app.w.start_text_input())));
        now_events("start again");

        insert("\"hi\"", &app, "hi");
        insert("\"A\"", &app, "A");
        insert("\"é😀\"", &app, "é😀");
        delete_backward("1", &app);
        delete_backward("2", &app);

        // The return key
        t(format!("should return: {}", yes_no(should_return(&app))));
        now_events("should return");
        text_state("should return", &app);
        hints::set(hints::RETURN_KEY_HIDES_IME, "1").unwrap();
        t(format!(
            "should return (hides the IME): {}",
            yes_no(should_return(&app))
        ));
        now_events("should return (hides the IME)");
        text_state("should return (hides the IME)", &app);
        field("should return (hides the IME)", &app);
        t(format!(
            "should return (stopped): {}",
            yes_no(should_return(&app))
        ));
        now_events("should return (stopped)");
        let unset = || {
            hints::set_with_priority(hints::RETURN_KEY_HIDES_IME, None, hints::Priority::Normal)
                .unwrap();
        };
        unset();
        t(format!("stop again: {}", ok(&app.w.stop_text_input())));
        now_events("stop again");
        text_state("stop again", &app);

        // The text input properties
        for ty in 0..=9 {
            let l = format!("type {ty}");
            props_case(&l, &app, Some(ty), None, None, None);
            t(format!("{l}: stop: {}", ok(&app.w.stop_text_input())));
            field(&l, &app);
            now_events(&l);
        }
        for cap in 0..=4 {
            props_case(
                &format!("capitalization {cap}"),
                &app,
                None,
                Some(cap),
                None,
                None,
            );
            stop(&app);
            props_case(
                &format!("name, capitalization {cap}"),
                &app,
                Some(textinput::TextInputType::TextName as i64),
                Some(cap),
                None,
                None,
            );
            stop(&app);
        }
        props_case(
            "name",
            &app,
            Some(textinput::TextInputType::TextName as i64),
            None,
            None,
            None,
        );
        stop(&app);
        props_case("autocorrect on", &app, None, None, Some(true), None);
        stop(&app);
        props_case("autocorrect off", &app, None, None, Some(false), None);
        stop(&app);
        props_case("multiline on", &app, None, None, None, Some(true));
        stop(&app);
        props_case("multiline off", &app, None, None, None, Some(false));
        stop(&app);
        hints::set(hints::RETURN_KEY_HIDES_IME, "1").unwrap();
        props_case(
            "multiline unset (the return key hides the IME)",
            &app,
            None,
            None,
            None,
            None,
        );
        stop(&app);
        props_case(
            "multiline on (the return key hides the IME)",
            &app,
            None,
            None,
            None,
            Some(true),
        );
        stop(&app);
        unset();
        now_events("props stopped");
        // Properties changed while text input is active
        props_case(
            "email, active",
            &app,
            Some(textinput::TextInputType::TextEmail as i64),
            None,
            Some(false),
            None,
        );
        props_case(
            "password, while active",
            &app,
            Some(textinput::TextInputType::TextPasswordHidden as i64),
            Some(textinput::Capitalization::Words as i64),
            Some(true),
            Some(false),
        );
        text_state("props, while active", &app);
        t(format!(
            "props: start without properties: {}",
            ok(&app.w.start_text_input())
        ));
        field("props: start without properties", &app);
        stop(&app);
        now_events("props: stopped");
        // The one-time code: its text
        props_case(
            "one-time code",
            &app,
            Some(textinput::TextInputType::NumberPasswordHidden as i64),
            None,
            None,
            None,
        );
        insert("\"1\" (one-time code)", &app, "1");
        insert("\"23\" (one-time code)", &app, "23");
        insert("\"4\" (one-time code)", &app, "4");
        delete_backward("(one-time code)", &app);
        stop(&app);
        field("one-time code: stopped", &app);
        now_events("one-time code: stopped");
        props_case(
            "one-time code again",
            &app,
            Some(textinput::TextInputType::NumberPasswordVisible as i64),
            None,
            None,
            None,
        );
        stop(&app);
        props_case(
            "text after the one-time code",
            &app,
            Some(textinput::TextInputType::Text as i64),
            None,
            None,
            None,
        );
        insert("\"z\" (after the one-time code)", &app, "z");
        stop(&app);
        field("text after the one-time code: stopped", &app);
        now_events("text after the one-time code: stopped");
        t(format!("start: {}", ok(&app.w.start_text_input())));
        field("start (plain)", &app);
        stop(&app);
        now_events("start (plain), stopped");

        // The text input area, and the keyboard's notifications
        // SAFETY: UIKit's constant strings.
        let (will_show, did_show, will_hide, did_hide) = unsafe {
            (
                UIKeyboardWillShowNotification,
                UIKeyboardDidShowNotification,
                UIKeyboardWillHideNotification,
                UIKeyboardDidHideNotification,
            )
        };
        t(format!("area: start: {}", ok(&app.w.start_text_input())));
        now_events("area: start");
        let area = |x, y, w, h, cursor| {
            app.w
                .set_text_input_area(Some(&Rect { x, y, w, h }), cursor)
        };
        t(format!(
            "area 10,700 100x40: {}",
            ok(&area(10, 700, 100, 40, 5))
        ));
        frame("area 10,700 100x40", &app);
        post_keyboard(will_show, 552.0, 300.0);
        frame("keyboard will show (300)", &app);
        now_events("keyboard will show (300)");
        post_keyboard(did_show, 552.0, 300.0);
        now_events("keyboard did show");
        text_state("keyboard did show", &app);
        t(format!(
            "area 10,100 100x40: {}",
            ok(&area(10, 100, 100, 40, 0))
        ));
        frame("area 10,100 100x40", &app);
        t(format!(
            "area 0,800 50x50: {}",
            ok(&area(0, 800, 50, 50, 0))
        ));
        frame("area 0,800 50x50", &app);
        {
            let (got, cursor) = app.w.text_input_area().unwrap();
            t(format!(
                "area: {},{} {}x{} cursor={cursor}",
                got.x, got.y, got.w, got.h
            ));
        }
        t(format!("fullscreen: {}", ok(&app.w.set_fullscreen(true))));
        frame("fullscreen", &app);
        trace_events("fullscreen", 300);
        t(format!("windowed: {}", ok(&app.w.set_fullscreen(false))));
        frame("windowed", &app);
        trace_events("windowed", 300);
        post_keyboard(will_show, 452.0, 400.0);
        frame("keyboard will show (400)", &app);
        t(format!(
            "area none: {}",
            ok(&app.w.set_text_input_area(None, 0))
        ));
        frame("area none", &app);
        let _ = area(0, 800, 50, 50, 0);
        frame("area 0,800 50x50 again", &app);
        post_keyboard(will_hide, 852.0, 0.0);
        frame("keyboard will hide", &app);
        now_events("keyboard will hide");
        post_keyboard(did_hide, 852.0, 0.0);
        now_events("keyboard did hide");
        text_state("keyboard did hide", &app);
        post_keyboard(did_show, 552.0, 300.0);
        now_events("keyboard did show again");
        text_state("keyboard did show again", &app);
        t(format!("area: stop: {}", ok(&app.w.stop_text_input())));
        now_events("area: stop");
        let _ = area(0, 700, 50, 50, 0);
        frame("area 0,700 50x50, stopped", &app);
        post_keyboard(will_show, 552.0, 300.0);
        frame("keyboard will show (300), stopped", &app);
        post_keyboard(will_hide, 852.0, 0.0);
        frame("keyboard will hide, stopped", &app);
        let _ = app.w.set_text_input_area(None, 0);
        now_events("area: done");

        // The presses, with the keyboard
        presses("a (with a keyboard)", &app, press_of(4, 0));

        // A game controller's keyboard (GCKeyboard), connected through its
        // notification (the simulator's own, now connected)
        // SAFETY: GCKeyboard's class property.
        let coalesced: Id = unsafe { objc::send0(objc::class("GCKeyboard"), "coalescedKeyboard") };
        // SAFETY: GCKeyboard's and GCKeyboardInput's properties.
        unsafe {
            let input: Id = objc::send0(coalesced, "keyboardInput");
            let handler: Id = objc::send0(input, "keyChangedHandler");
            let queue: Id = objc::send0(coalesced, "handlerQueue");
            t(format!(
                "gckeyboard: coalesced: {} handler={} queue={}",
                yes_no(!coalesced.is_null()),
                yes_no(!handler.is_null()),
                if queue.is_null() {
                    "nil".into()
                } else {
                    CStr::from_ptr(dispatch_queue_get_label(queue))
                        .to_string_lossy()
                        .into_owned()
                }
            ));
        }
        let kbd = new_object("SDLTestKeyboard");
        let kbd_input = new_object("SDLTestKeyboard");
        kb(kbd, |k| k.input = kbd_input as usize);
        // SAFETY: GameController's constant strings.
        let (connect, disconnect) = unsafe {
            (
                GCKeyboardDidConnectNotification,
                GCKeyboardDidDisconnectNotification,
            )
        };
        post(connect, kbd);
        now_events("gckeyboard connected");
        keyboards("gckeyboard connected");
        let handler = || kb(kbd_input, |k| k.handler) as Id;
        t(format!(
            "gckeyboard: handler={} queue={}",
            yes_no(!handler().is_null()),
            yes_no(kb(kbd, |k| k.queue_set))
        ));
        key_changed(kbd_input, handler(), 4, true);
        now_events("gckeyboard a pressed");
        key_changed(kbd_input, handler(), 4, false);
        now_events("gckeyboard a released");
        key_changed(kbd_input, handler(), 225, true);
        key_changed(kbd_input, handler(), 5, true);
        now_events("gckeyboard shift b pressed");
        key_changed(kbd_input, handler(), 5, false);
        key_changed(kbd_input, handler(), 225, false);
        now_events("gckeyboard shift b released");
        t(format!("text input: {}", ok(&app.w.start_text_input())));
        now_events("gckeyboard text input");
        key_changed(kbd_input, handler(), 6, true);
        now_events("gckeyboard c pressed (text input)");
        insert("\"c\" (a key pressed)", &app, "c");
        key_changed(kbd_input, handler(), 6, false);
        now_events("gckeyboard c released (text input)");
        stop(&app);
        now_events("gckeyboard text input stopped");
        post(disconnect, kbd);
        now_events("gckeyboard disconnected");
        t(format!(
            "gckeyboard: handler={}",
            yes_no(!handler().is_null())
        ));
        keyboards("gckeyboard disconnected");

        // The simulator's keyboard, disconnected (as far as SDL can tell)
        let coalesced_handler = || -> bool {
            // SAFETY: GCKeyboard's and GCKeyboardInput's properties.
            unsafe {
                let input: Id = objc::send0(coalesced, "keyboardInput");
                !objc::send0::<Id>(input, "keyChangedHandler").is_null()
            }
        };
        if !coalesced.is_null() {
            post(disconnect, coalesced);
        }
        now_events("coalesced keyboard disconnected");
        keyboards("coalesced keyboard disconnected");
        t(format!(
            "coalesced keyboard: handler={}",
            yes_no(coalesced_handler())
        ));

        // The presses, without a keyboard
        presses("a", &app, press_of(4, 0));
        presses("return", &app, press_of(40, 0));
        presses("up arrow", &app, press_of(-1, 0));
        presses("down arrow", &app, press_of(-1, 1));
        presses("left arrow", &app, press_of(-1, 2));
        presses("right arrow", &app, press_of(-1, 3));
        presses("select", &app, press_of(-1, 4));
        presses("menu", &app, press_of(-1, 5));
        presses("play/pause", &app, press_of(-1, 6));
        presses("page up", &app, press_of(-1, 30));

        // The text field, without a keyboard
        t(format!(
            "no keyboard: start: {}",
            ok(&app.w.start_text_input())
        ));
        now_events("no keyboard: start");
        field("no keyboard: start", &app);
        insert("\"ab\" (no keyboard)", &app, "ab");
        insert("\"Hé😀\" (no keyboard)", &app, "Hé😀");
        delete_backward("1 (no keyboard)", &app);
        delete_backward("2 (no keyboard)", &app);
        delete_many("(no keyboard)", &app, 10);
        delete_many("(no keyboard)", &app, 50);
        delete_many("(no keyboard)", &app, 10);
        insert("\"x\" (no keyboard)", &app, "x");
        t(format!(
            "no keyboard: stop: {}",
            ok(&app.w.stop_text_input())
        ));
        now_events("no keyboard: stop");
        field("no keyboard: stop", &app);

        // The simulator's keyboard, back
        if !coalesced.is_null() {
            post(connect, coalesced);
        }
        now_events("coalesced keyboard connected");
        keyboards("coalesced keyboard connected");
        t(format!(
            "coalesced keyboard: handler={}",
            yes_no(coalesced_handler())
        ));

        // The run loop again: the on-screen keyboard's notifications that
        // were waiting, and those to come
        video::set_ios_event_pump(true);
        ASYNC_KEYBOARD.store(true, std::sync::atomic::Ordering::Relaxed);
        settle(&mut Vec::new(), 2000);
        keyboards("pump on");

        // Real keys and text, through idb
        match &idb {
            Ok(()) => {
                // (the simulator passes input through a window with nothing
                // drawn in it)
                // SAFETY: UIColor's class method and a UIView property.
                unsafe {
                    let black: Id = objc::send0(objc::class("UIColor"), "blackColor");
                    let () = objc::send1(view, "setBackgroundColor:", black);
                }
                trace_events("idb: opaque", 300);
                idb_request("key 4", "key 4", "KEY_UP", 1);
                idb_request("key 82", "key 82", "KEY_UP", 1);
                // (no autocapitalization or autocorrection: the text is
                // what's typed)
                let props = Properties::new();
                props
                    .set(
                        textinput::PROP_TEXTINPUT_CAPITALIZATION_NUMBER,
                        textinput::Capitalization::None as i64,
                    )
                    .unwrap();
                props
                    .set(textinput::PROP_TEXTINPUT_AUTOCORRECT_BOOLEAN, false)
                    .unwrap();
                t(format!(
                    "idb: start: {}",
                    ok(&app.w.start_text_input_with_properties(Some(&props)))
                ));
                drop(props);
                now_events("idb: start");
                field("idb: start", &app);
                idb_request("key 4 (text input)", "key 4", "KEY_UP", 1);
                t(format!("idb: text={}", field_text(&app)));
                idb_request("text \"Hi!\" (text input)", "text Hi!", "TEXT_INPUT", 3);
                t(format!("idb: text={}", field_text(&app)));
                idb_request("key 42 (text input)", "key 42", "KEY_UP", 1);
                t(format!("idb: text={}", field_text(&app)));
                idb_request("key 82 (text input)", "key 82", "KEY_UP", 1);
                idb_request("key 40 (text input)", "key 40", "KEY_UP(which=k1", 1);
                t(format!(
                    "idb: text input active={}",
                    yes_no(app.w.text_input_active().unwrap())
                ));
                field("idb", &app);
                t(format!("idb: stop: {}", ok(&app.w.stop_text_input())));
                now_events("idb: stop");
            }
            Err(why) => println!("note: skipping, idb unavailable: {why}"),
        }

        app.w.destroy();
        t("destroyed");
        trace_events("destroyed", 300);
        keyboards("destroyed");

        init::quit();
        t("quit");

        // (without idb's key, the simulator may be in the on-screen
        // keyboard's mode, where the trace is upstream's only by chance)
        if idb.is_ok() {
            check_trace(&TRACE.lock().unwrap(), EXPECTED);
        }
    }
}
