// Tests of the UIKit video driver's input on tvOS: the Siri Remote's
// presses and swipes, text input, and the remote as a joystick.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the tvOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as
//! `uikit_tvos` is. Its main function makes a window and traces the
//! keyboards (the simulator's coalesced `GCKeyboard`, when there is one,
//! is posted as disconnected, so that the presses are the view's), and
//! the key, text and game controller events of:
//!
//! * the view's press handlers, given stand-ins for `UIPress`, `UIKey` and
//!   `UIPressesEvent`: the Siri Remote's buttons (the arrows, select,
//!   menu, play/pause, another) and a key, which are Up, Down, Left,
//!   Right, Return, Escape (menu: back) and Pause, the other button
//!   nothing, the key its HID usage, while no remote is opened as a
//!   joystick;
//! * the view's swipe handler (`swipeGesture:`), given stand-ins for its
//!   `UISwipeGestureRecognizer`s: a swipe ended in each direction (an
//!   arrow key pressed and released), and one that began (nothing);
//! * text input started (the hidden text field becomes the first
//!   responder), text typed in the field in-process (`insertText:`,
//!   `deleteBackward`) and text input stopped;
//! * the Siri Remote as a joystick (`SDL_HINT_TV_REMOTE_AS_JOYSTICK`, on
//!   by default): a stand-in, `+[GCController
//!   controllerWithMicroGamepad]`'s snapshot (product category "Siri
//!   Remote"), posted as connected, opened as a gamepad (its name, GUID,
//!   mapping, axes, buttons and hats, and its micro gamepad's rotation and
//!   absolute d-pad, which SDL sets), while it's open the presses and
//!   swipes again (the remote's are its now: nothing), its d-pad, A and X
//!   buttons and the menu button (the pause button: B, which tvOS
//!   otherwise keeps to itself) set, then closed (the presses are keys
//!   again), opened with `SDL_HINT_APPLE_TV_REMOTE_ALLOW_ROTATION`, and
//!   posted as disconnected; and another posted with the hint off (no
//!   joystick).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an Apple TV 4K (3rd
//! generation) with tvOS 18.5). Joysticks are named by their first
//! appearance (`#1`...), and the mice's device events and the joysticks'
//! update events are left out. Elsewhere this program does nothing.

#[cfg(not(target_os = "tvos"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "tvos")]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "tvos")]
mod uikit_support;

#[cfg(target_os = "tvos")]
fn main() {
    tvos::main()
}

#[cfg(target_os = "tvos")]
mod tvos {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::keyboard;
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType, JoystickID, WindowID};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::joystick;
    use sdl3::joystick::gamepad::{self, Gamepad, GamepadAxis, GamepadButton};
    use sdl3::video::Window;

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an Apple TV 4K (3rd generation)
    /// simulator with tvOS 18.5).
    const EXPECTED: &str = "\
init video and gamepad: ok
application state: 0
start: keyboards: Keyboard; has keyboard=yes
start: joysticks: #1 Gamepad
coalesced keyboard: yes
keyboard disconnected: events: KEYBOARD_REMOVED
keyboard disconnected: keyboards: (none); has keyboard=no
W: create: ok
W: created: events: W:WINDOW_SAFE_AREA_CHANGED(0,0), W:WINDOW_MOUSE_ENTER(0,0), W:WINDOW_FOCUS_GAINED(0,0), W:WINDOW_SHOWN(0,0), W:WINDOW_PIXEL_SIZE_CHANGED(1920,1080)
W: view SDL_uikitview
no remote: press up arrow began: events: KEY_DOWN(global scancode=82 key=0x40000052 down=1 w=W)
no remote: press up arrow ended: events: KEY_UP(global scancode=82 key=0x40000052 down=0 w=W)
no remote: press down arrow began: events: KEY_DOWN(global scancode=81 key=0x40000051 down=1 w=W)
no remote: press down arrow ended: events: KEY_UP(global scancode=81 key=0x40000051 down=0 w=W)
no remote: press left arrow began: events: KEY_DOWN(global scancode=80 key=0x40000050 down=1 w=W)
no remote: press left arrow ended: events: KEY_UP(global scancode=80 key=0x40000050 down=0 w=W)
no remote: press right arrow began: events: KEY_DOWN(global scancode=79 key=0x4000004f down=1 w=W)
no remote: press right arrow ended: events: KEY_UP(global scancode=79 key=0x4000004f down=0 w=W)
no remote: press select began: events: KEY_DOWN(global scancode=40 key=0xd down=1 w=W)
no remote: press select ended: events: KEY_UP(global scancode=40 key=0xd down=0 w=W)
no remote: press menu began: events: KEY_DOWN(global scancode=41 key=0x1b down=1 w=W)
no remote: press menu ended: events: KEY_UP(global scancode=41 key=0x1b down=0 w=W)
no remote: press play/pause began: events: KEY_DOWN(global scancode=72 key=0x40000048 down=1 w=W)
no remote: press play/pause ended: events: KEY_UP(global scancode=72 key=0x40000048 down=0 w=W)
no remote: press page up began: events: (none)
no remote: press page up ended: events: (none)
no remote: press key a began: events: KEY_DOWN(global scancode=4 key=0x61 down=1 w=W)
no remote: press key a ended: events: KEY_UP(global scancode=4 key=0x61 down=0 w=W)
no remote: swipe up: events: KEY_DOWN(global scancode=82 key=0x40000052 down=1 w=W), KEY_UP(global scancode=82 key=0x40000052 down=0 w=W)
no remote: swipe down: events: KEY_DOWN(global scancode=81 key=0x40000051 down=1 w=W), KEY_UP(global scancode=81 key=0x40000051 down=0 w=W)
no remote: swipe left: events: KEY_DOWN(global scancode=80 key=0x40000050 down=1 w=W), KEY_UP(global scancode=80 key=0x40000050 down=0 w=W)
no remote: swipe right: events: KEY_DOWN(global scancode=79 key=0x4000004f down=1 w=W), KEY_UP(global scancode=79 key=0x4000004f down=0 w=W)
no remote: swipe up (began): events: (none)
start text input: ok
text input started: events: SCREEN_KEYBOARD_SHOWN
text input active=yes
text field: NSKVONotifying_SDLUITextField first responder=yes
insert \"tv\": events: KEY_DOWN(global scancode=23 key=0x74 down=1 w=W), KEY_UP(global scancode=23 key=0x74 down=0 w=W), KEY_DOWN(global scancode=25 key=0x76 down=1 w=W), KEY_UP(global scancode=25 key=0x76 down=0 w=W), TEXT_INPUT(tv w=W)
delete backward: events: KEY_DOWN(global scancode=42 key=0x8 down=1 w=W), KEY_UP(global scancode=42 key=0x8 down=0 w=W)
stop text input: ok
text input stopped: events: SCREEN_KEYBOARD_HIDDEN
text input active=no first responder=no
micro snapshot: vendor=MicroGamepad category=Siri Remote micro=yes
micro connected: events: JOYSTICK_ADDED(#2), GAMEPAD_ADDED(#2)
#2: name=MicroGamepad guid=050050daac05000004000000961b6d04 vendor=0x05ac product=0x0004 gamepad=yes type=1
#2: mapping=050050daac05000004000000961b6d04,MicroGamepad,a:b0,b:b1,x:b2,leftx:a0,lefty:a1~,
#2: open: axes=2 buttons=3 hats=0
#2: buttons: a, b, x
opened: events: (none)
opened: allows rotation=no absolute d-pad=no
remote open: press up arrow began: events: (none)
remote open: press up arrow ended: events: (none)
remote open: press down arrow began: events: (none)
remote open: press down arrow ended: events: (none)
remote open: press left arrow began: events: (none)
remote open: press left arrow ended: events: (none)
remote open: press right arrow began: events: (none)
remote open: press right arrow ended: events: (none)
remote open: press select began: events: (none)
remote open: press select ended: events: (none)
remote open: press menu began: events: (none)
remote open: press menu ended: events: (none)
remote open: press play/pause began: events: (none)
remote open: press play/pause ended: events: (none)
remote open: press page up began: events: (none)
remote open: press page up ended: events: (none)
remote open: press key a began: events: KEY_DOWN(global scancode=4 key=0x61 down=1 w=W)
remote open: press key a ended: events: KEY_UP(global scancode=4 key=0x61 down=0 w=W)
remote open: swipe up: events: (none)
remote open: swipe down: events: (none)
remote open: swipe left: events: (none)
remote open: swipe right: events: (none)
remote open: swipe up (began): events: (none)
d-pad: events: GAMEPAD_AXIS_MOTION(#2 leftx=0), JOYSTICK_AXIS_MOTION(#2 axis=0 value=0), GAMEPAD_AXIS_MOTION(#2 leftx=16383), JOYSTICK_AXIS_MOTION(#2 axis=0 value=16383), GAMEPAD_AXIS_MOTION(#2 lefty=-1), JOYSTICK_AXIS_MOTION(#2 axis=1 value=0), GAMEPAD_AXIS_MOTION(#2 lefty=32766), JOYSTICK_AXIS_MOTION(#2 axis=1 value=-32767)
button A: events: GAMEPAD_AXIS_MOTION(#2 leftx=0), JOYSTICK_AXIS_MOTION(#2 axis=0 value=0), GAMEPAD_AXIS_MOTION(#2 lefty=-1), JOYSTICK_AXIS_MOTION(#2 axis=1 value=0), GAMEPAD_BUTTON_DOWN(#2 a), JOYSTICK_BUTTON_DOWN(#2 button=0)
button X: events: GAMEPAD_BUTTON_UP(#2 a), JOYSTICK_BUTTON_UP(#2 button=0), GAMEPAD_BUTTON_DOWN(#2 x), JOYSTICK_BUTTON_DOWN(#2 button=2)
menu: events: GAMEPAD_BUTTON_DOWN(#2 b), JOYSTICK_BUTTON_DOWN(#2 button=1), GAMEPAD_BUTTON_UP(#2 x), JOYSTICK_BUTTON_UP(#2 button=2)
released: events: GAMEPAD_BUTTON_UP(#2 b), JOYSTICK_BUTTON_UP(#2 button=1)
closed
remote closed: press up arrow began: events: KEY_DOWN(global scancode=82 key=0x40000052 down=1 w=W)
remote closed: press up arrow ended: events: KEY_UP(global scancode=82 key=0x40000052 down=0 w=W)
remote closed: press down arrow began: events: KEY_DOWN(global scancode=81 key=0x40000051 down=1 w=W)
remote closed: press down arrow ended: events: KEY_UP(global scancode=81 key=0x40000051 down=0 w=W)
remote closed: press left arrow began: events: KEY_DOWN(global scancode=80 key=0x40000050 down=1 w=W)
remote closed: press left arrow ended: events: KEY_UP(global scancode=80 key=0x40000050 down=0 w=W)
remote closed: press right arrow began: events: KEY_DOWN(global scancode=79 key=0x4000004f down=1 w=W)
remote closed: press right arrow ended: events: KEY_UP(global scancode=79 key=0x4000004f down=0 w=W)
remote closed: press select began: events: KEY_DOWN(global scancode=40 key=0xd down=1 w=W)
remote closed: press select ended: events: KEY_UP(global scancode=40 key=0xd down=0 w=W)
remote closed: press menu began: events: KEY_DOWN(global scancode=41 key=0x1b down=1 w=W)
remote closed: press menu ended: events: KEY_UP(global scancode=41 key=0x1b down=0 w=W)
remote closed: press play/pause began: events: KEY_DOWN(global scancode=72 key=0x40000048 down=1 w=W)
remote closed: press play/pause ended: events: KEY_UP(global scancode=72 key=0x40000048 down=0 w=W)
remote closed: press page up began: events: (none)
remote closed: press page up ended: events: (none)
remote closed: press key a began: events: KEY_DOWN(global scancode=4 key=0x61 down=1 w=W)
remote closed: press key a ended: events: KEY_UP(global scancode=4 key=0x61 down=0 w=W)
#2: open: axes=2 buttons=3 hats=0
#2: buttons: a, b, x
rotation hint: allows rotation=no absolute d-pad=no
micro disconnected: events: GAMEPAD_REMOVED(#2), JOYSTICK_REMOVED(#2)
micro disconnected: joysticks: #1 Gamepad
micro (not a joystick) connected: events: (none)
micro (not a joystick) connected: joysticks: #1 Gamepad
micro (not a joystick) disconnected: events: (none)
W: destroyed: events: W:WINDOW_DESTROYED(0,0)
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

    fn list(items: Vec<String>) -> String {
        if items.is_empty() {
            "(none)".into()
        } else {
            items.join(", ")
        }
    }

    fn ok<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".into(),
            Err(e) => format!("error: {e}"),
        }
    }

    // ---- the stand-ins the translated handlers are given

    /// What a `UIPress` (with its `UIKey`, an `SDLTestKey`) and its
    /// `UIPressesEvent` tell the view (an `SDLTestPress`), and what a
    /// `UISwipeGestureRecognizer` tells it (an `SDLTestSwipe`: the
    /// state, the direction).
    #[derive(Clone, Copy, Default)]
    struct Press {
        key: usize,
        press_type: isize,
    }

    thread_local! {
        static PRESSES: RefCell<HashMap<usize, Press>> = RefCell::new(HashMap::new());
        static KEY_CODES: RefCell<HashMap<usize, isize>> = RefCell::new(HashMap::new());
        static SWIPES: RefCell<HashMap<usize, (isize, usize)>> = RefCell::new(HashMap::new());
    }

    fn press(this: Id) -> Press {
        PRESSES.with(|p| {
            p.borrow()
                .get(&(this as usize))
                .copied()
                .unwrap_or_default()
        })
    }

    fn swipe(this: Id) -> (isize, usize) {
        SWIPES.with(|s| {
            s.borrow()
                .get(&(this as usize))
                .copied()
                .unwrap_or_default()
        })
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
    extern "C" fn press_timestamp(_this: Id, _: objc::Sel) -> f64 {
        1.0
    }
    extern "C" fn swipe_state(this: Id, _: objc::Sel) -> isize {
        swipe(this).0
    }
    extern "C" fn swipe_direction(this: Id, _: objc::Sel) -> usize {
        swipe(this).1
    }

    /// Declare `SDLTestKey`, `SDLTestPress` and `SDLTestSwipe`.
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
                "SDLTestSwipe",
                "NSObject",
                &[
                    ("state", swipe_state as *const c_void, "q@:"),
                    ("direction", swipe_direction as *const c_void, "Q@:"),
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
            presses
                .borrow_mut()
                .insert(p as usize, Press { key, press_type })
        });
        p
    }

    // ---- labels: the joysticks by first appearance

    static JOYSTICKS: Mutex<Vec<JoystickID>> = Mutex::new(Vec::new());

    fn label_of(id: JoystickID) -> usize {
        let mut seen = JOYSTICKS.lock().unwrap();
        if let Some(i) = seen.iter().position(|s| *s == id) {
            return i + 1;
        }
        seen.push(id);
        seen.len()
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

    fn axis_name(axis: u8) -> String {
        GamepadAxis::all()
            .nth(axis as usize)
            .and_then(|a| a.as_str())
            .unwrap_or("(null)")
            .to_string()
    }

    fn button_name(button: u8) -> String {
        GamepadButton::from_i32(button as i32)
            .as_str()
            .unwrap_or("(null)")
            .to_string()
    }

    /// An event as the trace names it (the C program's names), `None` for
    /// those left out (see the module's description).
    fn describe(event: &Event) -> Option<String> {
        let ty = event.event_type();
        let n = event_name(ty);
        if ty == EventType::JOYSTICK_UPDATE_COMPLETE || ty == EventType::GAMEPAD_UPDATE_COMPLETE {
            return None;
        }
        Some(match event {
            Event::MouseDevice(_) => return None,
            Event::Key(e) => format!(
                "{n}({} scancode={} key={:#x} down={} w={})",
                if e.which == 0 { "global" } else { "keyboard" },
                e.scancode.0,
                e.key.0,
                e.down as i32,
                wl(e.window_id)
            ),
            Event::TextInput(e) => format!("{n}({} w={})", e.text, wl(e.window_id)),
            Event::TextEditing(e) => format!(
                "{n}({} {} {} w={})",
                e.text,
                e.start,
                e.length,
                wl(e.window_id)
            ),
            Event::KeyboardDevice(_) => n,
            Event::JoyDevice(e) => format!("{n}(#{})", label_of(e.which)),
            Event::GamepadDevice(e) => format!("{n}(#{})", label_of(e.which)),
            Event::JoyAxis(e) => format!(
                "{n}(#{} axis={} value={})",
                label_of(e.which),
                e.axis,
                e.value
            ),
            Event::JoyButton(e) => format!("{n}(#{} button={})", label_of(e.which), e.button),
            Event::JoyHat(e) => format!(
                "{n}(#{} hat={} value={})",
                label_of(e.which),
                e.hat,
                e.value
            ),
            Event::GamepadAxis(e) => format!(
                "{n}(#{} {}={})",
                label_of(e.which),
                axis_name(e.axis),
                e.value
            ),
            Event::GamepadButton(e) => {
                format!("{n}(#{} {})", label_of(e.which), button_name(e.button))
            }
            Event::Window(w) => format!("{}:{n}({},{})", wl(w.window_id), w.data1, w.data2),
            _ => n,
        })
    }

    fn take_queued(out: &mut Vec<String>) {
        events::pump();
        while let Some(event) = events::poll() {
            out.extend(describe(&event));
        }
    }

    /// The events that come within `ms` milliseconds (10 ms steps; 0: those
    /// queued).
    fn trace_events(label: &str, ms: u64) {
        let mut seen = Vec::new();
        take_queued(&mut seen);
        for _ in 0..ms / 10 {
            std::thread::sleep(Duration::from_millis(10));
            take_queued(&mut seen);
        }
        t(format!("{label}: events: {}", list(seen)));
    }

    fn keyboards(label: &str) {
        let names: Vec<String> = keyboard::keyboards()
            .into_iter()
            .map(|id| keyboard::keyboard_name(id).unwrap_or_default())
            .collect();
        t(format!(
            "{label}: keyboards: {}; has keyboard={}",
            list(names),
            yes_no(keyboard::has_keyboard())
        ));
    }

    fn joysticks(label: &str) {
        let all: Vec<String> = joystick::joysticks()
            .into_iter()
            .map(|id| {
                format!(
                    "#{} {}",
                    label_of(id),
                    joystick::joystick_name_for_id(id)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| "(null)".into())
                )
            })
            .collect();
        t(format!("{label}: joysticks: {}", list(all)));
    }

    fn presses_of(label: &str, view: Id) {
        // UIPressType's values
        for (name, code, press_type) in [
            ("up arrow", -1, 0),
            ("down arrow", -1, 1),
            ("left arrow", -1, 2),
            ("right arrow", -1, 3),
            ("select", -1, 4),
            ("menu", -1, 5),
            ("play/pause", -1, 6),
            ("page up", -1, 30),
            ("key a", 4, 4),
        ] {
            let l = format!("{label}: press {name}");
            let p = press_of(code, press_type);
            // SAFETY: NSSet's class method (autoreleased) and the view's
            // press handlers.
            unsafe {
                let set: Id = objc::send1(objc::class("NSSet"), "setWithObject:", p);
                let () = objc::send2(view, "pressesBegan:withEvent:", set, p);
                trace_events(&format!("{l} began"), 0);
                let () = objc::send2(view, "pressesEnded:withEvent:", set, p);
                trace_events(&format!("{l} ended"), 0);
            }
        }
    }

    fn swipes(label: &str, view: Id) {
        // UISwipeGestureRecognizerDirection's and UIGestureRecognizerState's
        // values
        for (name, direction, state) in [
            ("up", 1usize << 2, 3isize),
            ("down", 1 << 3, 3),
            ("left", 1 << 1, 3),
            ("right", 1 << 0, 3),
            ("up (began)", 1 << 2, 1),
        ] {
            let s = new_object("SDLTestSwipe");
            SWIPES.with(|w| w.borrow_mut().insert(s as usize, (state, direction)));
            // SAFETY: the view's swipe handler.
            unsafe {
                let () = objc::send1(view, "swipeGesture:", s);
            }
            trace_events(&format!("{label}: swipe {name}"), 0);
        }
    }

    fn describe_joystick(id: JoystickID) {
        let l = label_of(id);
        t(format!(
            "#{l}: name={} guid={} vendor={:#06x} product={:#06x} gamepad={} type={}",
            joystick::joystick_name_for_id(id)
                .ok()
                .flatten()
                .unwrap_or_else(|| "(null)".into()),
            joystick::joystick_guid_for_id(id),
            joystick::joystick_vendor_for_id(id),
            joystick::joystick_product_for_id(id),
            yes_no(gamepad::is_gamepad(id)),
            gamepad::gamepad_type_for_id(id) as i32
        ));
        t(format!(
            "#{l}: mapping={}",
            gamepad::gamepad_mapping_for_id(id).unwrap_or_else(|| "(null)".into())
        ));
    }

    fn open_gamepad(id: JoystickID) -> Option<Gamepad> {
        let l = label_of(id);
        let gp = match Gamepad::open(id) {
            Ok(gp) => gp,
            Err(e) => {
                t(format!("#{l}: open: error: {e}"));
                return None;
            }
        };
        let j = gp.joystick().unwrap();
        t(format!(
            "#{l}: open: axes={} buttons={} hats={}",
            j.num_axes().unwrap(),
            j.num_buttons().unwrap(),
            j.num_hats().unwrap()
        ));
        let buttons: Vec<String> = GamepadButton::all()
            .filter(|&b| gp.has_button(b))
            .map(|b| b.as_str().unwrap_or("(null)").to_string())
            .collect();
        t(format!("#{l}: buttons: {}", list(buttons)));
        Some(gp)
    }

    fn micro_state(label: &str, controller: Id) {
        // SAFETY: GCController's and GCMicroGamepad's properties.
        unsafe {
            let micro: Id = objc::send0(controller, "microGamepad");
            let rotation: bool = objc::send0(micro, "allowsRotation");
            let absolute: bool = objc::send0(micro, "reportsAbsoluteDpadValues");
            t(format!(
                "{label}: allows rotation={} absolute d-pad={}",
                yes_no(rotation),
                yes_no(absolute)
            ));
        }
    }

    /// The first joystick not among the first `seen` labelled, after
    /// waiting up to five seconds.
    fn wait_for_new(seen: usize) -> Option<JoystickID> {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            let old: Vec<JoystickID> = JOYSTICKS.lock().unwrap()[..seen].to_vec();
            if let Some(found) = joystick::joysticks()
                .into_iter()
                .find(|id| !old.contains(id))
            {
                return Some(found);
            }
            std::thread::sleep(Duration::from_millis(10));
            events::pump();
        }
        None
    }

    fn set_value(input: Id, value: f32) {
        // SAFETY: GCControllerButtonInput's method.
        unsafe { objc::send1::<f32, ()>(input, "setValue:", value) }
    }

    fn set_xy(dpad: Id, x: f32, y: f32) {
        // SAFETY: GCControllerDirectionPad's method.
        unsafe { objc::send2::<f32, f32, ()>(dpad, "setValueForXAxis:yAxis:", x, y) }
    }

    fn element(micro: Id, name: &str) -> Id {
        // SAFETY: a GCMicroGamepad property.
        unsafe { objc::send0(micro, name) }
    }

    fn game_controller_string(name: &std::ffi::CStr) -> Id {
        // SAFETY: GameController's NSString constant (the driver loaded the
        // framework).
        unsafe {
            let p = libc_dlsym(name.as_ptr()) as *const Id;
            if p.is_null() {
                std::ptr::null_mut()
            } else {
                *p
            }
        }
    }

    unsafe extern "C" {
        #[link_name = "dlsym"]
        fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
    }

    /// `dlsym(RTLD_DEFAULT, symbol)`.
    unsafe fn libc_dlsym(symbol: *const std::ffi::c_char) -> *mut c_void {
        // (RTLD_DEFAULT is -2 on Apple platforms)
        // SAFETY: the caller's symbol name.
        unsafe { dlsym(-2isize as *mut c_void, symbol) }
    }

    fn post(name: &std::ffi::CStr, object: Id) {
        objc::post_notification_for(game_controller_string(name), object);
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    fn set_hint(name: &str, value: Option<&str>) {
        hints::set_with_priority(name, value, hints::Priority::Normal).unwrap();
    }

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_tvos_input: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_tvos_input: {:.3} s: main", elapsed());
        declare_classes();
        let r = init::init(InitFlags::VIDEO | InitFlags::GAMEPAD);
        t(format!("init video and gamepad: {}", ok(&r)));
        r.unwrap();
        let mut ignored = Vec::new();
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
        for _ in 0..50 {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
        t(format!("application state: {}", application_state()));
        keyboards("start");
        joysticks("start");
        // SAFETY: GCKeyboard's class method (tvOS 14).
        let coalesced: Id = unsafe { objc::send0(objc::class("GCKeyboard"), "coalescedKeyboard") };
        t(format!(
            "coalesced keyboard: {}",
            yes_no(!coalesced.is_null())
        ));
        if !coalesced.is_null() {
            post(c"GCKeyboardDidDisconnectNotification", coalesced);
            trace_events("keyboard disconnected", 0);
            keyboards("keyboard disconnected");
        }

        let r = Window::create("W", 640, 480, WindowFlags::NONE);
        t(format!("W: create: {}", ok(&r)));
        let w = r.unwrap();
        *WINDOW.lock().unwrap() = w.id();
        trace_events("W: created", 500);
        let uw = w
            .properties()
            .ok()
            .and_then(|p| p.get_number(sdl3::video::window::PROP_WINDOW_UIKIT_WINDOW_POINTER))
            .map_or(std::ptr::null_mut(), |p| p as usize as Id);
        // SAFETY: UIWindow's and UIViewController's properties.
        let view: Id = unsafe {
            let vc: Id = objc::send0(uw, "rootViewController");
            objc::send0(vc, "view")
        };
        t(format!("W: view {}", objc::class_name(view)));

        presses_of("no remote", view);
        swipes("no remote", view);

        // Text input: the hidden text field
        t(format!("start text input: {}", ok(&w.start_text_input())));
        trace_events("text input started", 300);
        t(format!(
            "text input active={}",
            yes_no(w.text_input_active().unwrap_or(false))
        ));
        let text_field_class = objc::class("UITextField");
        // SAFETY: UIView's subviews (an NSArray) and NSObject's method.
        let tf = unsafe {
            let subviews: Id = objc::send0(view, "subviews");
            let count: usize = objc::send0(subviews, "count");
            let mut found = std::ptr::null_mut();
            for i in 0..count {
                let sub: Id = objc::send1(subviews, "objectAtIndex:", i);
                if objc::send1::<_, bool>(sub, "isKindOfClass:", text_field_class) {
                    found = sub;
                }
            }
            found
        };
        let first_responder = |tf: Id| -> bool {
            // SAFETY: a UIResponder property (NO for nil).
            !tf.is_null() && unsafe { objc::send0::<bool>(tf, "isFirstResponder") }
        };
        t(format!(
            "text field: {} first responder={}",
            if tf.is_null() {
                "nil".to_string()
            } else {
                objc::class_name(tf)
            },
            yes_no(first_responder(tf))
        ));
        // SAFETY: UIKeyInput's methods of the text field (no-ops on nil).
        unsafe {
            let () = objc::send1(tf, "insertText:", objc::ns_string("tv"));
        }
        trace_events("insert \"tv\"", 0);
        // SAFETY: as above.
        unsafe {
            let () = objc::send0(tf, "deleteBackward");
        }
        trace_events("delete backward", 0);
        t(format!("stop text input: {}", ok(&w.stop_text_input())));
        trace_events("text input stopped", 300);
        t(format!(
            "text input active={} first responder={}",
            yes_no(w.text_input_active().unwrap_or(false)),
            yes_no(first_responder(tf))
        ));

        // The Siri Remote as a joystick: a stand-in micro gamepad
        // SAFETY: GCController's class method and properties.
        let micro: Id =
            unsafe { objc::send0(objc::class("GCController"), "controllerWithMicroGamepad") };
        // SAFETY: as above.
        let (vendor, category, micro_gamepad): (Id, Id, Id) = unsafe {
            (
                objc::send0(micro, "vendorName"),
                objc::send0(micro, "productCategory"),
                objc::send0(micro, "microGamepad"),
            )
        };
        t(format!(
            "micro snapshot: vendor={} category={} micro={}",
            objc::string(vendor).unwrap_or_else(|| "(null)".into()),
            objc::string(category).unwrap_or_else(|| "(null)".into()),
            yes_no(!micro_gamepad.is_null())
        ));
        let seen = JOYSTICKS.lock().unwrap().len();
        post(c"GCControllerDidConnectNotification", micro);
        trace_events("micro connected", 100);
        let id = wait_for_new(seen);
        let mut gp = None;
        match id {
            Some(id) => {
                describe_joystick(id);
                gp = open_gamepad(id);
                trace_events("opened", 100);
                micro_state("opened", micro);
            }
            None => t("micro: no joystick"),
        }
        presses_of("remote open", view);
        swipes("remote open", view);
        set_xy(element(micro_gamepad, "dpad"), 0.5, -1.0);
        trace_events("d-pad", 100);
        set_xy(element(micro_gamepad, "dpad"), 0.0, 0.0);
        set_value(element(micro_gamepad, "buttonA"), 1.0);
        trace_events("button A", 100);
        set_value(element(micro_gamepad, "buttonA"), 0.0);
        set_value(element(micro_gamepad, "buttonX"), 1.0);
        trace_events("button X", 100);
        set_value(element(micro_gamepad, "buttonX"), 0.0);
        set_value(element(micro_gamepad, "buttonMenu"), 1.0);
        trace_events("menu", 100);
        set_value(element(micro_gamepad, "buttonMenu"), 0.0);
        trace_events("released", 100);
        if let Some(gp) = gp.take() {
            drop(gp);
            t("closed");
        }
        presses_of("remote closed", view);
        set_hint(hints::APPLE_TV_REMOTE_ALLOW_ROTATION, Some("1"));
        if let Some(id) = id {
            let again = open_gamepad(id);
            micro_state("rotation hint", micro);
            drop(again);
        }
        set_hint(hints::APPLE_TV_REMOTE_ALLOW_ROTATION, None);
        post(c"GCControllerDidDisconnectNotification", micro);
        trace_events("micro disconnected", 100);
        joysticks("micro disconnected");

        // Not as a joystick
        set_hint(hints::TV_REMOTE_AS_JOYSTICK, Some("0"));
        // SAFETY: GCController's class method.
        let micro2: Id =
            unsafe { objc::send0(objc::class("GCController"), "controllerWithMicroGamepad") };
        post(c"GCControllerDidConnectNotification", micro2);
        trace_events("micro (not a joystick) connected", 300);
        joysticks("micro (not a joystick) connected");
        post(c"GCControllerDidDisconnectNotification", micro2);
        trace_events("micro (not a joystick) disconnected", 100);
        set_hint(hints::TV_REMOTE_AS_JOYSTICK, None);

        w.destroy();
        trace_events("W: destroyed", 300);
        init::quit();
        t("quit");

        check_trace(&TRACE.lock().unwrap(), EXPECTED);
    }
}
