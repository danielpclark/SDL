// Tests of the UIKit video driver's input on visionOS: the view's
// presses, text input, and a game controller.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the visionOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as
//! `uikit_visionos` is. Its main function makes a window and traces the
//! keyboards (the simulator's coalesced `GCKeyboard` is posted as
//! disconnected, so that the presses are the view's), the joysticks (the
//! simulator's "Gamepad"), and the key, text and game controller events
//! of:
//!
//! * the view's press handlers, given stand-ins for `UIPress`, `UIKey` and
//!   `UIPressesEvent`: the presses without a key (the arrows, select,
//!   menu, play/pause, another), which are Up, Down, Left, Right, Return,
//!   Escape and Pause, the other nothing, while no joystick is open, and
//!   a key, its HID usage;
//! * text input started (the hidden text field becomes the first
//!   responder), text typed in the field in-process (`insertText:`,
//!   `deleteBackward`) and text input stopped;
//! * a game controller (the MFi driver, the only joystick driver on
//!   visionOS, which upstream builds without HIDAPI): a stand-in,
//!   `+[GCController controllerWithExtendedGamepad]`'s snapshot, posted
//!   as connected, opened as a gamepad (its name, GUID, mapping, axes,
//!   buttons and hats), while it's open the presses again (a joystick is
//!   open: the presses without a key are nothing), its d-pad, A and X
//!   buttons and the menu button set, then closed (the presses are keys
//!   again) and posted as disconnected.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an Apple Vision Pro with
//! visionOS 2.5). Joysticks are named by their first appearance (`#1`...),
//! and the mice's device events and the joysticks' update events are left
//! out. Elsewhere this program does nothing.

#[cfg(not(target_os = "visionos"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "visionos")]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(target_os = "visionos")]
mod uikit_support;

#[cfg(target_os = "visionos")]
fn main() {
    visionos::main()
}

#[cfg(target_os = "visionos")]
mod visionos {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::keyboard;
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType, JoystickID, WindowID};
    use sdl3::init::{self, InitFlags};
    use sdl3::joystick;
    use sdl3::joystick::gamepad::{self, Gamepad, GamepadAxis, GamepadButton};
    use sdl3::video::Window;

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an Apple Vision Pro simulator
    /// with visionOS 2.5).
    const EXPECTED: &str = "\
init video and gamepad: ok
application state: 0
start: keyboards: Keyboard; has keyboard=yes
start: joysticks: #1 Gamepad
coalesced keyboard: yes
keyboard disconnected: events: KEYBOARD_REMOVED
keyboard disconnected: keyboards: (none); has keyboard=no
W: create: ok
W: created: events: W:WINDOW_MOUSE_ENTER(0,0), W:WINDOW_FOCUS_GAINED(0,0), W:WINDOW_SHOWN(0,0), W:WINDOW_PIXEL_SIZE_CHANGED(640,480)
W: view SDL_uikitview
no gamepad: press up arrow began: events: KEY_DOWN(global scancode=82 key=0x40000052 down=1 w=W)
no gamepad: press up arrow ended: events: KEY_UP(global scancode=82 key=0x40000052 down=0 w=W)
no gamepad: press down arrow began: events: KEY_DOWN(global scancode=81 key=0x40000051 down=1 w=W)
no gamepad: press down arrow ended: events: KEY_UP(global scancode=81 key=0x40000051 down=0 w=W)
no gamepad: press left arrow began: events: KEY_DOWN(global scancode=80 key=0x40000050 down=1 w=W)
no gamepad: press left arrow ended: events: KEY_UP(global scancode=80 key=0x40000050 down=0 w=W)
no gamepad: press right arrow began: events: KEY_DOWN(global scancode=79 key=0x4000004f down=1 w=W)
no gamepad: press right arrow ended: events: KEY_UP(global scancode=79 key=0x4000004f down=0 w=W)
no gamepad: press select began: events: KEY_DOWN(global scancode=40 key=0xd down=1 w=W)
no gamepad: press select ended: events: KEY_UP(global scancode=40 key=0xd down=0 w=W)
no gamepad: press menu began: events: KEY_DOWN(global scancode=41 key=0x1b down=1 w=W)
no gamepad: press menu ended: events: KEY_UP(global scancode=41 key=0x1b down=0 w=W)
no gamepad: press play/pause began: events: KEY_DOWN(global scancode=72 key=0x40000048 down=1 w=W)
no gamepad: press play/pause ended: events: KEY_UP(global scancode=72 key=0x40000048 down=0 w=W)
no gamepad: press page up began: events: (none)
no gamepad: press page up ended: events: (none)
no gamepad: press key a began: events: KEY_DOWN(global scancode=4 key=0x61 down=1 w=W)
no gamepad: press key a ended: events: KEY_UP(global scancode=4 key=0x61 down=0 w=W)
start text input: ok
text input started: events: SCREEN_KEYBOARD_SHOWN
text input active=yes
text field: SDLUITextField first responder=yes
insert \"xr\": events: KEY_DOWN(global scancode=27 key=0x78 down=1 w=W), KEY_UP(global scancode=27 key=0x78 down=0 w=W), KEY_DOWN(global scancode=21 key=0x72 down=1 w=W), KEY_UP(global scancode=21 key=0x72 down=0 w=W), TEXT_INPUT(xr w=W)
delete backward: events: KEY_DOWN(global scancode=42 key=0x8 down=1 w=W), KEY_UP(global scancode=42 key=0x8 down=0 w=W)
stop text input: ok
text input stopped: events: SCREEN_KEYBOARD_HIDDEN
text input active=no first responder=no
extended snapshot: vendor=ExtendedGamepad category=MFi extended=yes
gamepad connected: events: JOYSTICK_ADDED(#2), GAMEPAD_ADDED(#2)
#2: name=ExtendedGamepad guid=05007da8ac05000004000000a31f6d04 vendor=0x05ac product=0x0004 gamepad=yes type=1
#2: mapping=05007da8ac05000004000000a31f6d04,ExtendedGamepad,a:b4,b:b5,x:b9,y:b10,back:b8,guide:b6,start:b7,leftstick:b17,rightstick:b25,leftshoulder:b16,rightshoulder:b24,dpup:b14,dpdown:b11,dpleft:b12,dpright:b13,paddle1:b19,paddle2:b21,paddle3:b20,paddle4:b22,leftx:a18,lefty:a19~,rightx:a28,righty:a29~,lefttrigger:+a20,righttrigger:+a30,
#2: open: axes=31 buttons=27 hats=0
#2: buttons: a, b, x, y, back, guide, start, leftstick, rightstick, leftshoulder, rightshoulder, dpup, dpdown, dpleft, dpright, paddle1, paddle2, paddle3, paddle4
opened: events: (none)
gamepad open: press up arrow began: events: (none)
gamepad open: press up arrow ended: events: (none)
gamepad open: press down arrow began: events: (none)
gamepad open: press down arrow ended: events: (none)
gamepad open: press left arrow began: events: (none)
gamepad open: press left arrow ended: events: (none)
gamepad open: press right arrow began: events: (none)
gamepad open: press right arrow ended: events: (none)
gamepad open: press select began: events: (none)
gamepad open: press select ended: events: (none)
gamepad open: press menu began: events: (none)
gamepad open: press menu ended: events: (none)
gamepad open: press play/pause began: events: (none)
gamepad open: press play/pause ended: events: (none)
gamepad open: press page up began: events: (none)
gamepad open: press page up ended: events: (none)
gamepad open: press key a began: events: KEY_DOWN(global scancode=4 key=0x61 down=1 w=W)
gamepad open: press key a ended: events: KEY_UP(global scancode=4 key=0x61 down=0 w=W)
d-pad: events: JOYSTICK_AXIS_MOTION(#2 axis=11 value=0), JOYSTICK_AXIS_MOTION(#2 axis=11 value=32767), JOYSTICK_AXIS_MOTION(#2 axis=13 value=0), JOYSTICK_AXIS_MOTION(#2 axis=13 value=16383), GAMEPAD_BUTTON_DOWN(#2 dpdown), JOYSTICK_BUTTON_DOWN(#2 button=11), GAMEPAD_BUTTON_DOWN(#2 dpright), JOYSTICK_BUTTON_DOWN(#2 button=13)
button A: events: JOYSTICK_AXIS_MOTION(#2 axis=4 value=0), JOYSTICK_AXIS_MOTION(#2 axis=4 value=32767), JOYSTICK_AXIS_MOTION(#2 axis=11 value=0), JOYSTICK_AXIS_MOTION(#2 axis=13 value=0), GAMEPAD_BUTTON_DOWN(#2 a), JOYSTICK_BUTTON_DOWN(#2 button=4), GAMEPAD_BUTTON_UP(#2 dpdown), JOYSTICK_BUTTON_UP(#2 button=11), GAMEPAD_BUTTON_UP(#2 dpright), JOYSTICK_BUTTON_UP(#2 button=13)
button X: events: JOYSTICK_AXIS_MOTION(#2 axis=4 value=0), JOYSTICK_AXIS_MOTION(#2 axis=9 value=0), JOYSTICK_AXIS_MOTION(#2 axis=9 value=32767), GAMEPAD_BUTTON_UP(#2 a), JOYSTICK_BUTTON_UP(#2 button=4), GAMEPAD_BUTTON_DOWN(#2 x), JOYSTICK_BUTTON_DOWN(#2 button=9)
menu: events: JOYSTICK_AXIS_MOTION(#2 axis=7 value=0), JOYSTICK_AXIS_MOTION(#2 axis=7 value=32767), JOYSTICK_AXIS_MOTION(#2 axis=9 value=0), GAMEPAD_BUTTON_DOWN(#2 start), JOYSTICK_BUTTON_DOWN(#2 button=7), GAMEPAD_BUTTON_UP(#2 x), JOYSTICK_BUTTON_UP(#2 button=9)
released: events: JOYSTICK_AXIS_MOTION(#2 axis=7 value=0), GAMEPAD_BUTTON_UP(#2 start), JOYSTICK_BUTTON_UP(#2 button=7)
closed
gamepad closed: press up arrow began: events: KEY_DOWN(global scancode=82 key=0x40000052 down=1 w=W)
gamepad closed: press up arrow ended: events: KEY_UP(global scancode=82 key=0x40000052 down=0 w=W)
gamepad closed: press down arrow began: events: KEY_DOWN(global scancode=81 key=0x40000051 down=1 w=W)
gamepad closed: press down arrow ended: events: KEY_UP(global scancode=81 key=0x40000051 down=0 w=W)
gamepad closed: press left arrow began: events: KEY_DOWN(global scancode=80 key=0x40000050 down=1 w=W)
gamepad closed: press left arrow ended: events: KEY_UP(global scancode=80 key=0x40000050 down=0 w=W)
gamepad closed: press right arrow began: events: KEY_DOWN(global scancode=79 key=0x4000004f down=1 w=W)
gamepad closed: press right arrow ended: events: KEY_UP(global scancode=79 key=0x4000004f down=0 w=W)
gamepad closed: press select began: events: KEY_DOWN(global scancode=40 key=0xd down=1 w=W)
gamepad closed: press select ended: events: KEY_UP(global scancode=40 key=0xd down=0 w=W)
gamepad closed: press menu began: events: KEY_DOWN(global scancode=41 key=0x1b down=1 w=W)
gamepad closed: press menu ended: events: KEY_UP(global scancode=41 key=0x1b down=0 w=W)
gamepad closed: press play/pause began: events: KEY_DOWN(global scancode=72 key=0x40000048 down=1 w=W)
gamepad closed: press play/pause ended: events: KEY_UP(global scancode=72 key=0x40000048 down=0 w=W)
gamepad closed: press page up began: events: (none)
gamepad closed: press page up ended: events: (none)
gamepad closed: press key a began: events: KEY_DOWN(global scancode=4 key=0x61 down=1 w=W)
gamepad closed: press key a ended: events: KEY_UP(global scancode=4 key=0x61 down=0 w=W)
gamepad disconnected: events: GAMEPAD_REMOVED(#2), JOYSTICK_REMOVED(#2)
gamepad disconnected: joysticks: #1 Gamepad
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
    /// `UIPressesEvent` tell the view (an `SDLTestPress`).
    #[derive(Clone, Copy, Default)]
    struct Press {
        key: usize,
        press_type: isize,
    }

    thread_local! {
        static PRESSES: RefCell<HashMap<usize, Press>> = RefCell::new(HashMap::new());
        static KEY_CODES: RefCell<HashMap<usize, isize>> = RefCell::new(HashMap::new());
    }

    fn press(this: Id) -> Press {
        PRESSES.with(|p| {
            p.borrow()
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
    /// Declare `SDLTestKey` and `SDLTestPress`.
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

    fn element(gamepad: Id, name: &str) -> Id {
        // SAFETY: a GCExtendedGamepad property.
        unsafe { objc::send0(gamepad, name) }
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

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_visionos_input: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_visionos_input: {:.3} s: main", elapsed());
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
        // SAFETY: GCKeyboard's class method (visionOS 1).
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
        trace_events("W: created", 1000);
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

        presses_of("no gamepad", view);

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
            let () = objc::send1(tf, "insertText:", objc::ns_string("xr"));
        }
        trace_events("insert \"xr\"", 0);
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

        // A game controller: a stand-in extended gamepad
        // SAFETY: GCController's class method and properties.
        let controller: Id =
            unsafe { objc::send0(objc::class("GCController"), "controllerWithExtendedGamepad") };
        // SAFETY: as above.
        let (vendor, category, extended): (Id, Id, Id) = unsafe {
            (
                objc::send0(controller, "vendorName"),
                objc::send0(controller, "productCategory"),
                objc::send0(controller, "extendedGamepad"),
            )
        };
        t(format!(
            "extended snapshot: vendor={} category={} extended={}",
            objc::string(vendor).unwrap_or_else(|| "(null)".into()),
            objc::string(category).unwrap_or_else(|| "(null)".into()),
            yes_no(!extended.is_null())
        ));
        let seen = JOYSTICKS.lock().unwrap().len();
        post(c"GCControllerDidConnectNotification", controller);
        trace_events("gamepad connected", 100);
        let id = wait_for_new(seen);
        let mut gp = None;
        match id {
            Some(id) => {
                describe_joystick(id);
                gp = open_gamepad(id);
                trace_events("opened", 100);
            }
            None => t("gamepad: no joystick"),
        }
        presses_of("gamepad open", view);
        set_xy(element(extended, "dpad"), 0.5, -1.0);
        trace_events("d-pad", 100);
        set_xy(element(extended, "dpad"), 0.0, 0.0);
        set_value(element(extended, "buttonA"), 1.0);
        trace_events("button A", 100);
        set_value(element(extended, "buttonA"), 0.0);
        set_value(element(extended, "buttonX"), 1.0);
        trace_events("button X", 100);
        set_value(element(extended, "buttonX"), 0.0);
        set_value(element(extended, "buttonMenu"), 1.0);
        trace_events("menu", 100);
        set_value(element(extended, "buttonMenu"), 0.0);
        trace_events("released", 100);
        if let Some(gp) = gp.take() {
            drop(gp);
            t("closed");
        }
        presses_of("gamepad closed", view);
        post(c"GCControllerDidDisconnectNotification", controller);
        trace_events("gamepad disconnected", 100);
        joysticks("gamepad disconnected");

        w.destroy();
        trace_events("W: destroyed", 300);
        init::quit();
        t("quit");

        check_trace(&TRACE.lock().unwrap(), EXPECTED);
    }
}
