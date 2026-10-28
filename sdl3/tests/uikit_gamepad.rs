// Tests of the MFi (GameController) joystick driver and the CoreMotion
// sensor driver on UIKit: the simulator's controllers, an on-screen virtual
// controller and stand-in controllers, the game controllers' interaction
// with the windows, and the sensors.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! GameController reports its controllers to an app (on its main run
//! loop), so this is a program of its own (`harness = false`), run as an
//! app on the iOS Simulator by `tools/ios_run.py` (the `__DATA,__sdl3_runner`
//! section below), as `uikit_main` is. Its main function traces (with the
//! main run loop running between the steps):
//!
//! * a window's and its view's game controller interactions
//!   (`GCEventInteraction`s for gamepads) before the gamepads start, while
//!   they run (and for a window made then, in place of the first: one
//!   window per display), and after they quit;
//! * the joysticks the driver finds (the simulator's built-in "Gamepad"):
//!   their names, GUIDs, vendor and product, types, mappings, and opened as
//!   gamepads their axes, buttons, capabilities, sensors, power, SF Symbols
//!   names, LED, rumble and sensor results, and the player index (which is
//!   the `GCController`'s);
//! * a `GCVirtualController` (the on-screen controller, "Apple Touch
//!   Controller") connected, opened and disconnected (setting its
//!   elements does nothing: only snapshots take values);
//! * stand-in controllers (`+[GCController controllerWithExtendedGamepad]`
//!   and `controllerWithMicroGamepad`, snapshots whose elements can be
//!   set) posted as connected (`GCControllerDidConnectNotification`, twice)
//!   and disconnected while open: the events and the gamepad's state as
//!   their buttons, sticks, trigger and d-pad are set, and what an open
//!   gamepad whose controller is gone answers;
//! * the gamepads with `SDL_JOYSTICK_MFI` off;
//! * CoreMotion's sensors (none on the simulator) and opening one.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5).
//! Elsewhere this program does nothing.

#[cfg(not(target_os = "ios"))]
fn main() {}

/// The runner's word: run as an app.
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
    use std::ffi::c_void;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType, JoystickID};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::joystick::gamepad::{self, Gamepad, GamepadAxis, GamepadButton};
    use sdl3::joystick::{
        self, JoystickConnectionState, PROP_JOYSTICK_CAP_MONO_LED_BOOLEAN,
        PROP_JOYSTICK_CAP_PLAYER_LED_BOOLEAN, PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN,
        PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN,
    };
    use sdl3::power::PowerState;
    use sdl3::sensor::{self, Sensor, SensorType};
    use sdl3::video::Window;

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    const EXPECTED: &str = "\
app state: active
start: GCController controllers: (none)
init video: ok
window A: ok
window A: interactions: window=0 view=0
video: events: (none)
init gamepad: ok
window A: interactions: window=1 view=0
init: events: JOYSTICK_ADDED(#1 Gamepad), GAMEPAD_ADDED(#1 Gamepad)
init: joysticks: #1 Gamepad
init: GCController controllers: Gamepad (MFi, player 0)
#1: name=Gamepad path=(null) guid=0500a5d0ac05000004000000fa256d04 vendor=0x05ac product=0x0004 version=9722 type=1
#1: gamepad=yes type=standard real type=standard player=0 virtual=no
#1: mapping=0500a5d0ac05000004000000fa256d04,Gamepad,a:b0,b:b1,x:b3,y:b4,start:b2,leftstick:b10,rightstick:b13,leftshoulder:b9,rightshoulder:b12,dpup:b8,dpdown:b5,dpleft:b6,dpright:b7,leftx:a11,lefty:a12~,rightx:a16,righty:a17~,lefttrigger:+a13,righttrigger:+a18,
#1: open: name=Gamepad axes=19 buttons=15 hats=0 balls=0 touchpads=0
#1: caps: rumble=no trigger_rumble=no rgb_led=no mono_led=no player_led=no
#1: sensors: accel=no gyro=no
#1: power=unknown percent=-1 connection=unknown
#1: buttons: a, b, x, y, start, leftstick, rightstick, leftshoulder, rightshoulder, dpup, dpdown, dpleft, dpright
#1: button symbols: a=a.circle, b=b.circle, x=x.circle, y=y.circle, start=line.horizontal.3.circle, leftstick=l.joystick.down, rightstick=r.joystick.press.down, leftshoulder=l1.rectangle.roundedbottom, rightshoulder=r1.rectangle.roundedbottom, dpup=dpad.up.fill, dpdown=dpad.down.fill, dpleft=dpad.left.fill, dpright=dpad.right.fill
#1: axes: leftx, lefty, rightx, righty, lefttrigger, righttrigger
#1: axis symbols: leftx=l.joystick, lefty=l.joystick, rightx=r.joystick, righty=r.joystick, lefttrigger=l2.rectangle.roundedtop, righttrigger=r2.rectangle.roundedtop
#1: led: error: That operation is not supported
#1: rumble: error: That operation is not supported
#1: rumble triggers: error: That operation is not supported
#1: accel on: error: No such sensor on this device
#1: gyro on: error: No such sensor on this device
built-in: state: lefty=-1, righty=-1
#1: player index set to 2: 2 (for id 2)
player index: GCController controllers: Gamepad (MFi, player 2)
#1: closed
closed: GCController controllers: Gamepad (MFi, player -1)
built-in: events: (none)
window A destroyed
window B: ok
window B: interactions: window=0 view=1
virtual controller: connecting
virtual controller: events: JOYSTICK_ADDED(#2 Apple Touch Controller), GAMEPAD_ADDED(#2 Apple Touch Controller)
virtual controller: GCController controllers: Apple Touch Controller (Touch Controller, player 0), Gamepad (MFi, player -1)
#2: name=Apple Touch Controller path=(null) guid=05001547ac05000004000000a1006d04 vendor=0x05ac product=0x0004 version=161 type=1
#2: gamepad=yes type=standard real type=standard player=0 virtual=no
#2: mapping=05001547ac05000004000000a1006d04,Apple Touch Controller,a:b0,b:b1,x:b3,y:b4,start:b2,leftshoulder:b9,rightshoulder:b11,dpup:b8,dpdown:b5,dpleft:b6,dpright:b7,leftx:a0,lefty:a1~,rightx:a2,righty:a3~,lefttrigger:b10,righttrigger:b12,
#2: open: name=Apple Touch Controller axes=4 buttons=13 hats=0 balls=0 touchpads=0
#2: caps: rumble=no trigger_rumble=no rgb_led=no mono_led=no player_led=no
#2: sensors: accel=no gyro=no
#2: power=unknown percent=-1 connection=unknown
#2: buttons: a, b, x, y, start, leftshoulder, rightshoulder, dpup, dpdown, dpleft, dpright
#2: button symbols: a=a.circle, b=b.circle, x=x.circle, y=y.circle, start=line.horizontal.3.circle, leftshoulder=l1.rectangle.roundedbottom, rightshoulder=r1.rectangle.roundedbottom, dpup=dpad.up.fill, dpdown=dpad.down.fill, dpleft=dpad.left.fill, dpright=dpad.right.fill
#2: axes: leftx, lefty, rightx, righty, lefttrigger, righttrigger
#2: axis symbols: leftx=l.joystick, lefty=l.joystick, rightx=r.joystick, righty=r.joystick, lefttrigger=l2.rectangle.roundedtop, righttrigger=r2.rectangle.roundedtop
#2: led: error: That operation is not supported
#2: rumble: error: That operation is not supported
#2: rumble triggers: error: That operation is not supported
#2: accel on: error: No such sensor on this device
#2: gyro on: error: No such sensor on this device
virtual controller: state: lefty=-1, righty=-1
virtual controller: A set: events: (none)
virtual controller: A set: state: lefty=-1, righty=-1
virtual controller: A unset: events: (none)
snapshot: vendor=ExtendedGamepad category=MFi extended=yes
posted GCControllerDidConnectNotification (snapshot)
snapshot connected: events: JOYSTICK_ADDED(#3 ExtendedGamepad), GAMEPAD_ADDED(#3 ExtendedGamepad)
snapshot: joysticks: #1 Gamepad, #2 Apple Touch Controller, #3 ExtendedGamepad
#3: name=ExtendedGamepad path=(null) guid=05007da8ac05000004000000a31f6d04 vendor=0x05ac product=0x0004 version=8099 type=1
#3: gamepad=yes type=standard real type=standard player=1 virtual=no
#3: mapping=05007da8ac05000004000000a31f6d04,ExtendedGamepad,a:b4,b:b5,x:b9,y:b10,back:b8,guide:b6,start:b7,leftstick:b17,rightstick:b25,leftshoulder:b16,rightshoulder:b24,dpup:b14,dpdown:b11,dpleft:b12,dpright:b13,paddle1:b19,paddle2:b21,paddle3:b20,paddle4:b22,leftx:a18,lefty:a19~,rightx:a28,righty:a29~,lefttrigger:+a20,righttrigger:+a30,
#3: open: name=ExtendedGamepad axes=31 buttons=27 hats=0 balls=0 touchpads=0
#3: caps: rumble=no trigger_rumble=no rgb_led=no mono_led=no player_led=no
#3: sensors: accel=no gyro=no
#3: power=unknown percent=-1 connection=unknown
#3: buttons: a, b, x, y, back, guide, start, leftstick, rightstick, leftshoulder, rightshoulder, dpup, dpdown, dpleft, dpright, paddle1, paddle2, paddle3, paddle4
#3: button symbols: a=a.circle, b=b.circle, x=x.circle, y=y.circle, back=ellipsis.circle, guide=house.circle, start=line.horizontal.3.circle, leftstick=l.joystick.down, rightstick=r.joystick.press.down, leftshoulder=l1.rectangle.roundedbottom, rightshoulder=r1.rectangle.roundedbottom, dpup=dpad.up.fill, dpdown=dpad.down.fill, dpleft=dpad.left.fill, dpright=dpad.right.fill, paddle1=m1.button.horizontal, paddle2=m2.button.horizontal, paddle3=m3.button.horizontal, paddle4=m4.button.horizontal
#3: axes: leftx, lefty, rightx, righty, lefttrigger, righttrigger
#3: axis symbols: leftx=l.joystick, lefty=l.joystick, rightx=r.joystick, righty=r.joystick, lefttrigger=l2.rectangle.roundedtop, righttrigger=r2.rectangle.roundedtop
#3: led: error: That operation is not supported
#3: rumble: error: That operation is not supported
#3: rumble triggers: error: That operation is not supported
#3: accel on: error: No such sensor on this device
#3: gyro on: error: No such sensor on this device
snapshot: state: lefty=-1, righty=-1
snapshot: A pressed: events: JOYSTICK_AXIS_MOTION(#3 axis=4 value=0), JOYSTICK_AXIS_MOTION(#3 axis=4 value=32767), GAMEPAD_BUTTON_DOWN(#3 a), JOYSTICK_BUTTON_DOWN(#3 button=4)
snapshot: A pressed: state: a, lefty=-1, righty=-1
snapshot: A released: events: JOYSTICK_AXIS_MOTION(#3 axis=4 value=0), GAMEPAD_BUTTON_UP(#3 a), JOYSTICK_BUTTON_UP(#3 button=4)
snapshot: left stick and right trigger: events: GAMEPAD_AXIS_MOTION(#3 leftx=0), JOYSTICK_AXIS_MOTION(#3 axis=18 value=0), GAMEPAD_AXIS_MOTION(#3 leftx=16383), JOYSTICK_AXIS_MOTION(#3 axis=18 value=16383), GAMEPAD_AXIS_MOTION(#3 lefty=-1), JOYSTICK_AXIS_MOTION(#3 axis=19 value=0), GAMEPAD_AXIS_MOTION(#3 lefty=8190), JOYSTICK_AXIS_MOTION(#3 axis=19 value=-8191), GAMEPAD_AXIS_MOTION(#3 righttrigger=0), JOYSTICK_AXIS_MOTION(#3 axis=30 value=0), GAMEPAD_AXIS_MOTION(#3 righttrigger=24575), JOYSTICK_AXIS_MOTION(#3 axis=30 value=24575), JOYSTICK_BUTTON_DOWN(#3 button=26)
snapshot: left stick and right trigger: state: leftx=16383, lefty=8190, righty=-1, righttrigger=24575
snapshot: d-pad up and menu: events: JOYSTICK_AXIS_MOTION(#3 axis=7 value=0), JOYSTICK_AXIS_MOTION(#3 axis=7 value=32767), JOYSTICK_AXIS_MOTION(#3 axis=14 value=0), JOYSTICK_AXIS_MOTION(#3 axis=14 value=32767), GAMEPAD_AXIS_MOTION(#3 leftx=0), JOYSTICK_AXIS_MOTION(#3 axis=18 value=0), GAMEPAD_AXIS_MOTION(#3 lefty=-1), JOYSTICK_AXIS_MOTION(#3 axis=19 value=0), GAMEPAD_AXIS_MOTION(#3 righttrigger=0), JOYSTICK_AXIS_MOTION(#3 axis=30 value=0), GAMEPAD_BUTTON_DOWN(#3 start), JOYSTICK_BUTTON_DOWN(#3 button=7), GAMEPAD_BUTTON_DOWN(#3 dpup), JOYSTICK_BUTTON_DOWN(#3 button=14), JOYSTICK_BUTTON_UP(#3 button=26)
snapshot: d-pad up and menu: state: start, dpup, lefty=-1, righty=-1
snapshot: released: events: JOYSTICK_AXIS_MOTION(#3 axis=7 value=0), JOYSTICK_AXIS_MOTION(#3 axis=14 value=0), GAMEPAD_BUTTON_UP(#3 start), JOYSTICK_BUTTON_UP(#3 button=7), GAMEPAD_BUTTON_UP(#3 dpup), JOYSTICK_BUTTON_UP(#3 button=14)
snapshot: released: state: lefty=-1, righty=-1
posted GCControllerDidConnectNotification (snapshot again)
snapshot again: events: (none)
snapshot again: joysticks: #1 Gamepad, #2 Apple Touch Controller, #3 ExtendedGamepad
micro snapshot: vendor=MicroGamepad category=Siri Remote micro=yes
posted GCControllerDidConnectNotification (micro snapshot)
micro snapshot connected: events: JOYSTICK_ADDED(#4 MicroGamepad), GAMEPAD_ADDED(#4 MicroGamepad)
#4: name=MicroGamepad path=(null) guid=050050daac05000004000000961b6d04 vendor=0x05ac product=0x0004 version=7062 type=1
#4: gamepad=yes type=standard real type=standard player=3 virtual=no
#4: mapping=050050daac05000004000000961b6d04,MicroGamepad,a:b0,b:b1,x:b2,leftx:a0,lefty:a1~,
#4: open: name=MicroGamepad axes=2 buttons=3 hats=0 balls=0 touchpads=0
#4: caps: rumble=no trigger_rumble=no rgb_led=no mono_led=no player_led=no
#4: sensors: accel=yes gyro=yes
#4: power=unknown percent=-1 connection=unknown
#4: buttons: a, b, x
#4: button symbols: dpup=dpad.up.fill, dpdown=dpad.down.fill, dpleft=dpad.left.fill, dpright=dpad.right.fill
#4: axes: leftx, lefty
#4: axis symbols: (none)
#4: led: error: That operation is not supported
#4: rumble: error: That operation is not supported
#4: rumble triggers: error: That operation is not supported
#4: accel on: ok
#4: gyro on: ok
#4: closed
posted GCControllerDidDisconnectNotification (micro snapshot)
micro snapshot disconnected: events: GAMEPAD_REMOVED(#4 MicroGamepad), JOYSTICK_REMOVED(#4 MicroGamepad)
posted GCControllerDidDisconnectNotification (snapshot)
snapshot disconnected: events: GAMEPAD_AXIS_MOTION(#3 lefty=0), GAMEPAD_AXIS_MOTION(#3 righty=0), GAMEPAD_REMOVED(#3 ExtendedGamepad), JOYSTICK_REMOVED(#3 ExtendedGamepad)
snapshot disconnected: joysticks: #1 Gamepad, #2 Apple Touch Controller
snapshot: connected=no
snapshot: rumble: error: Controller is no longer connected
snapshot: led: ok
snapshot: accel on: error: No such sensor on this device
snapshot: closed
virtual controller: closed
virtual controller: disconnecting
virtual controller disconnected: events: GAMEPAD_REMOVED(#2 Apple Touch Controller), JOYSTICK_REMOVED(#2 Apple Touch Controller)
virtual controller disconnected: joysticks: #1 Gamepad
virtual controller disconnected: GCController controllers: Gamepad (MFi, player -1)
quit gamepad
window B: interactions: window=0 view=1
quit: events: JOYSTICK_REMOVED(#1 Gamepad), JOYSTICK_REMOVED(#1 Gamepad)
init gamepad (SDL_JOYSTICK_MFI=0): ok
MFi off: events: (none)
MFi off: joysticks: (none)
window B: interactions: window=0 view=1
quit gamepad
window B: interactions: window=0 view=1
CMMotionManager: accelerometer=no gyro=no magnetometer=no device motion=no
init sensor: ok
sensors: 0
open sensor 0: Sensor 0 not found
sensor 0: name=(null) type=-1
sensors: events: (none)
quit sensor
done
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

    fn ns(s: Id) -> String {
        objc::string(s).unwrap_or_else(|| "nil".into())
    }

    fn opt(s: Option<&str>) -> &str {
        s.unwrap_or("(null)")
    }

    /// "A, B, C" or "(none)".
    fn list(items: &[String]) -> String {
        if items.is_empty() {
            "(none)".into()
        } else {
            items.join(", ")
        }
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after: u8) -> i32;
    }

    // (linked, as the C program links it)
    #[link(name = "GameController", kind = "framework")]
    unsafe extern "C" {
        static GCInputButtonA: Id;
        static GCInputButtonB: Id;
        static GCInputButtonX: Id;
        static GCInputButtonY: Id;
        static GCInputLeftThumbstick: Id;
        static GCInputRightThumbstick: Id;
        static GCInputLeftShoulder: Id;
        static GCInputRightShoulder: Id;
        static GCInputLeftTrigger: Id;
        static GCInputRightTrigger: Id;
        static GCControllerDidConnectNotification: Id;
        static GCControllerDidDisconnectNotification: Id;
    }

    /// `GCUIEventTypeGamepad`.
    const GC_UI_EVENT_TYPE_GAMEPAD: usize = 1;

    /// Run the main run loop for `ms` milliseconds.
    fn run_loop(ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            // SAFETY: the main thread's run loop, in its default mode.
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 0) };
        }
    }

    /// The joysticks seen, in the order first seen ("#N" in the trace),
    /// with their names then.
    static SEEN: Mutex<Vec<(JoystickID, String)>> = Mutex::new(Vec::new());

    fn label_of(id: JoystickID) -> usize {
        let mut seen = SEEN.lock().unwrap();
        if let Some(i) = seen.iter().position(|(s, _)| *s == id) {
            return i + 1;
        }
        let name = joystick::joystick_name_for_id(id)
            .ok()
            .flatten()
            .unwrap_or_else(|| "?".into());
        seen.push((id, name));
        seen.len()
    }

    fn name_of(id: JoystickID) -> String {
        let l = label_of(id);
        SEEN.lock().unwrap()[l - 1].1.clone()
    }

    fn seen_count() -> usize {
        SEEN.lock().unwrap().len()
    }

    fn axis_name(axis: u8) -> String {
        opt(GamepadAxis::all()
            .nth(axis as usize)
            .and_then(|a| a.as_str()))
        .to_string()
    }

    fn button_name(button: u8) -> String {
        opt(GamepadButton::from_i32(button as i32).as_str()).to_string()
    }

    fn power_name(s: PowerState) -> &'static str {
        match s {
            PowerState::Error => "error",
            PowerState::Unknown => "unknown",
            PowerState::OnBattery => "on battery",
            PowerState::NoBattery => "no battery",
            PowerState::Charging => "charging",
            PowerState::Charged => "charged",
        }
    }

    fn connection_name(s: JoystickConnectionState) -> &'static str {
        match s {
            JoystickConnectionState::Invalid => "invalid",
            JoystickConnectionState::Unknown => "unknown",
            JoystickConnectionState::Wired => "wired",
            JoystickConnectionState::Wireless => "wireless",
        }
    }

    /// Run the run loop for `ms`, then the joystick, gamepad and sensor
    /// events (pumped); the update-complete ones aren't traced (their
    /// number depends on the timing).
    fn events(label: &str, ms: u64) {
        run_loop(ms);
        events::pump();
        let mut seen = Vec::new();
        while let Some(event) = events::poll() {
            let ty = event.event_type();
            if ty == EventType::JOYSTICK_UPDATE_COMPLETE || ty == EventType::GAMEPAD_UPDATE_COMPLETE
            {
                continue;
            }
            let n = event_name(ty);
            let line = match event {
                Event::JoyDevice(e) => format!("{n}(#{} {})", label_of(e.which), name_of(e.which)),
                Event::GamepadDevice(e) => {
                    format!("{n}(#{} {})", label_of(e.which), name_of(e.which))
                }
                Event::JoyAxis(e) => format!(
                    "{n}(#{} axis={} value={})",
                    label_of(e.which),
                    e.axis,
                    e.value
                ),
                Event::JoyHat(e) => format!(
                    "{n}(#{} hat={} value={})",
                    label_of(e.which),
                    e.hat,
                    e.value
                ),
                Event::JoyButton(e) => {
                    format!("{n}(#{} button={})", label_of(e.which), e.button)
                }
                Event::JoyBattery(e) => format!(
                    "{n}(#{} state={} percent={})",
                    label_of(e.which),
                    power_name(e.state),
                    e.percent
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
                Event::JoyBall(_)
                | Event::GamepadTouchpad(_)
                | Event::GamepadSensor(_)
                | Event::Sensor(_) => n,
                _ => continue,
            };
            seen.push(line);
        }
        t(format!("{label}: events: {}", list(&seen)));
    }

    /// `[array count]` and `[array objectAtIndex:i]`.
    fn items(array: Id) -> Vec<Id> {
        // SAFETY: NSArray's methods (nil has no items).
        unsafe {
            let count: usize = objc::send0(array, "count");
            (0..count)
                .map(|i| objc::send1(array, "objectAtIndex:", i))
                .collect()
        }
    }

    /// The `GCEventInteraction`s for gamepads on a view.
    fn count_interactions(view: Id) -> usize {
        let class = objc::class("GCEventInteraction");
        if class.is_null() {
            return 0;
        }
        // SAFETY: UIView's interactions, and the interactions' class and
        // property.
        unsafe {
            let all: Id = objc::send0(view, "interactions");
            items(all)
                .into_iter()
                .filter(|&i| {
                    let kind: bool = objc::send1(i, "isKindOfClass:", class);
                    kind && objc::send0::<usize>(i, "handledEventTypes") == GC_UI_EVENT_TYPE_GAMEPAD
                })
                .count()
        }
    }

    /// The window's `UIWindow` (its property, a number here).
    fn uiwindow_of(w: &Window) -> Id {
        w.properties()
            .ok()
            .and_then(|p| p.get_number(sdl3::video::window::PROP_WINDOW_UIKIT_WINDOW_POINTER))
            .map_or(std::ptr::null_mut(), |p| p as usize as Id)
    }

    fn interactions(label: &str, w: &Window) {
        let window = uiwindow_of(w);
        // SAFETY: UIWindow's root view controller and its view.
        let view: Id = unsafe {
            let vc: Id = objc::send0(window, "rootViewController");
            objc::send0(vc, "view")
        };
        t(format!(
            "{label}: interactions: window={} view={}",
            count_interactions(window),
            count_interactions(view)
        ));
    }

    fn gc_controllers() -> Vec<Id> {
        // SAFETY: GCController's class property.
        items(unsafe { objc::send0(objc::class("GCController"), "controllers") })
    }

    fn controllers(label: &str) {
        let all: Vec<String> = gc_controllers()
            .into_iter()
            .map(|c| {
                // SAFETY: GCController's properties.
                let (vendor, category, player): (Id, Id, isize) = unsafe {
                    (
                        objc::send0(c, "vendorName"),
                        objc::send0(c, "productCategory"),
                        objc::send0(c, "playerIndex"),
                    )
                };
                format!("{} ({}, player {player})", ns(vendor), ns(category))
            })
            .collect();
        t(format!("{label}: GCController controllers: {}", list(&all)));
    }

    fn result(label: &str, r: sdl3::error::Result<()>) {
        match r {
            Ok(()) => t(format!("{label}: ok")),
            Err(e) => t(format!("{label}: error: {}", e.message())),
        }
    }

    /// What SDL says of a joystick before it's opened.
    fn describe_id(id: JoystickID) {
        let l = label_of(id);
        t(format!(
            "#{l}: name={} path={} guid={} vendor={:#06x} product={:#06x} version={} type={}",
            opt(joystick::joystick_name_for_id(id).ok().flatten().as_deref()),
            opt(joystick::joystick_path_for_id(id).ok().as_deref()),
            joystick::joystick_guid_for_id(id),
            joystick::joystick_vendor_for_id(id),
            joystick::joystick_product_for_id(id),
            joystick::joystick_product_version_for_id(id),
            joystick::joystick_type_for_id(id) as i32
        ));
        t(format!(
            "#{l}: gamepad={} type={} real type={} player={} virtual={}",
            yes_no(gamepad::is_gamepad(id)),
            opt(gamepad::gamepad_type_for_id(id).as_str()),
            opt(gamepad::real_gamepad_type_for_id(id).as_str()),
            joystick::joystick_player_index_for_id(id),
            yes_no(joystick::is_joystick_virtual(id))
        ));
        t(format!(
            "#{l}: mapping={}",
            opt(gamepad::gamepad_mapping_for_id(id).as_deref())
        ));
    }

    /// What SDL says of an open gamepad.
    fn describe_gamepad(gp: &Gamepad) {
        let l = label_of(gp.id());
        let j = gp.joystick().unwrap();
        t(format!(
            "#{l}: open: name={} axes={} buttons={} hats={} balls={} touchpads={}",
            opt(gp.name().ok().flatten().as_deref()),
            j.num_axes().unwrap(),
            j.num_buttons().unwrap(),
            j.num_hats().unwrap(),
            j.num_balls().unwrap(),
            gp.num_touchpads()
        ));
        let p = gp.properties().unwrap();
        let cap = |name: &str| yes_no(p.get_bool(name).unwrap_or(false));
        t(format!(
            "#{l}: caps: rumble={} trigger_rumble={} rgb_led={} mono_led={} player_led={}",
            cap(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN),
            cap(PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN),
            cap(PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN),
            cap(PROP_JOYSTICK_CAP_MONO_LED_BOOLEAN),
            cap(PROP_JOYSTICK_CAP_PLAYER_LED_BOOLEAN)
        ));
        t(format!(
            "#{l}: sensors: accel={} gyro={}",
            yes_no(gp.has_sensor(SensorType::Accel)),
            yes_no(gp.has_sensor(SensorType::Gyro))
        ));
        let (state, percent) = gp.power_info();
        t(format!(
            "#{l}: power={} percent={percent} connection={}",
            power_name(state),
            connection_name(gp.connection_state())
        ));
        let mut buttons = Vec::new();
        let mut symbols = Vec::new();
        for b in GamepadButton::all() {
            if gp.has_button(b) {
                buttons.push(opt(b.as_str()).to_string());
            }
            if let Some(sym) = gp.apple_sf_symbols_name_for_button(b) {
                symbols.push(format!("{}={sym}", opt(b.as_str())));
            }
        }
        t(format!("#{l}: buttons: {}", list(&buttons)));
        t(format!("#{l}: button symbols: {}", list(&symbols)));
        let mut axes = Vec::new();
        let mut axis_symbols = Vec::new();
        for a in GamepadAxis::all() {
            if gp.has_axis(a) {
                axes.push(opt(a.as_str()).to_string());
            }
            if let Some(sym) = gp.apple_sf_symbols_name_for_axis(a) {
                axis_symbols.push(format!("{}={sym}", opt(a.as_str())));
            }
        }
        t(format!("#{l}: axes: {}", list(&axes)));
        t(format!("#{l}: axis symbols: {}", list(&axis_symbols)));
        result(&format!("#{l}: led"), gp.set_led(0, 64, 255));
        result(&format!("#{l}: rumble"), gp.rumble(0x4000, 0x8000, 100));
        result(
            &format!("#{l}: rumble triggers"),
            gp.rumble_triggers(0x4000, 0x8000, 100),
        );
        result(
            &format!("#{l}: accel on"),
            gp.set_sensor_enabled(SensorType::Accel, true),
        );
        result(
            &format!("#{l}: gyro on"),
            gp.set_sensor_enabled(SensorType::Gyro, true),
        );
    }

    /// The gamepad's state: its buttons down and its axes not at rest.
    fn state(label: &str, gp: &Gamepad) {
        let mut down = Vec::new();
        for b in GamepadButton::all() {
            if gp.button(b) {
                down.push(opt(b.as_str()).to_string());
            }
        }
        for a in GamepadAxis::all() {
            let v = gp.axis(a);
            if v != 0 {
                down.push(format!("{}={v}", opt(a.as_str())));
            }
        }
        t(format!("{label}: state: {}", list(&down)));
    }

    fn joysticks(label: &str) {
        let all: Vec<String> = joystick::joysticks()
            .into_iter()
            .map(|id| format!("#{} {}", label_of(id), name_of(id)))
            .collect();
        t(format!("{label}: joysticks: {}", list(&all)));
    }

    /// The first joystick not among the first `seen` seen, after waiting up
    /// to five seconds.
    fn wait_for_new(seen: usize) -> Option<JoystickID> {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            let old: Vec<JoystickID> = SEEN.lock().unwrap()[..seen]
                .iter()
                .map(|(id, _)| *id)
                .collect();
            if let Some(found) = joystick::joysticks()
                .into_iter()
                .find(|id| !old.contains(id))
            {
                run_loop(300);
                return Some(found);
            }
            run_loop(10);
        }
        None
    }

    fn open_and_describe(id: JoystickID) -> Option<Gamepad> {
        describe_id(id);
        match Gamepad::open(id) {
            Ok(gp) => {
                describe_gamepad(&gp);
                Some(gp)
            }
            Err(e) => {
                t(format!("#{}: open: error: {}", label_of(id), e.message()));
                None
            }
        }
    }

    // The elements of a GCExtendedGamepad (setting them works on snapshots).

    fn set_value(input: Id, value: f32) {
        // SAFETY: GCControllerButtonInput's (or GCControllerAxisInput's)
        // method.
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

    /// Drive a stand-in controller (a snapshot GCController, posted as
    /// connected) through its extended gamepad's elements.
    fn drive(label: &str, gp: &Gamepad, g: Id) {
        set_value(element(g, "buttonA"), 1.0);
        let l = format!("{label}: A pressed");
        events(&l, 100);
        state(&l, gp);
        set_value(element(g, "buttonA"), 0.0);
        events(&format!("{label}: A released"), 100);
        set_xy(element(g, "leftThumbstick"), 0.5, -0.25);
        set_value(element(g, "rightTrigger"), 0.75);
        let l = format!("{label}: left stick and right trigger");
        events(&l, 100);
        state(&l, gp);
        set_xy(element(g, "leftThumbstick"), 0.0, 0.0);
        set_value(element(g, "rightTrigger"), 0.0);
        set_xy(element(g, "dpad"), 0.0, 1.0);
        set_value(element(g, "buttonMenu"), 1.0);
        let l = format!("{label}: d-pad up and menu");
        events(&l, 100);
        state(&l, gp);
        set_xy(element(g, "dpad"), 0.0, 0.0);
        set_value(element(g, "buttonMenu"), 0.0);
        let l = format!("{label}: released");
        events(&l, 100);
        state(&l, gp);
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
        println!("uikit_gamepad: run_app returned {status}, which it never does");
        exit(1);
    }

    /// Whether GameController has its virtual controller (iOS 15; else the
    /// program skips, unless `SDL3_TEST_REQUIRE` names `gamecontroller`).
    fn virtual_controller_available() -> bool {
        if !objc::class("GCVirtualController").is_null() {
            return true;
        }
        let why = "no GCVirtualController class";
        let required = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
        if required
            .split(',')
            .map(str::trim)
            .any(|c| c == "gamecontroller" || c == "all")
        {
            panic!("required capability gamecontroller unavailable: {why}");
        }
        eprintln!("note: skipping, gamecontroller unavailable: {why}");
        false
    }

    fn main_function() {
        println!("uikit_gamepad: {:.3} s: main", elapsed());
        if !virtual_controller_available() {
            return;
        }
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            run_loop(10);
        }
        t(format!(
            "app state: {}",
            if application_state() == 0 {
                "active"
            } else {
                "not active"
            }
        ));
        controllers("start");

        let r = init::init(InitFlags::VIDEO);
        t(format!(
            "init video: {}",
            r.as_ref()
                .map_or_else(|e| e.message().to_string(), |()| "ok".into())
        ));
        let a = Window::create("uikit_gamepad", 0, 0, WindowFlags::NONE);
        t(format!(
            "window A: {}",
            a.as_ref()
                .map_or_else(|e| e.message().to_string(), |_| "ok".into())
        ));
        let a = a.unwrap();
        run_loop(300);
        interactions("window A", &a);
        events("video", 100);

        let r = init::init(InitFlags::GAMEPAD);
        t(format!(
            "init gamepad: {}",
            r.as_ref()
                .map_or_else(|e| e.message().to_string(), |()| "ok".into())
        ));
        interactions("window A", &a);
        events("init", 1000);
        joysticks("init");
        controllers("init");

        // The built-in controller(s)
        for id in joystick::joysticks() {
            if let Some(gp) = open_and_describe(id) {
                state("built-in", &gp);
                let _ = gp.set_player_index(2);
                t(format!(
                    "#{}: player index set to 2: {} (for id {})",
                    label_of(id),
                    gp.player_index(),
                    joystick::joystick_player_index_for_id(id)
                ));
                controllers("player index");
                drop(gp);
                t(format!("#{}: closed", label_of(id)));
                controllers("closed");
            }
        }
        events("built-in", 100);

        // A window made while the joysticks run (in place of the first:
        // one window per display)
        a.destroy();
        t("window A destroyed");
        run_loop(300);
        let r = Window::create("uikit_gamepad B", 0, 0, WindowFlags::NONE);
        t(format!(
            "window B: {}",
            r.as_ref()
                .map_or_else(|e| e.message().to_string(), |_| "ok".into())
        ));
        let a = r.unwrap();
        run_loop(300);
        interactions("window B", &a);

        // The on-screen virtual controller
        // SAFETY: GameController's constants, NSArray's and NSSet's class
        // methods, GCVirtualControllerConfiguration's and
        // GCVirtualController's initializers (owned) and methods.
        let vc: Id = unsafe {
            let elements = [
                GCInputButtonA,
                GCInputButtonB,
                GCInputButtonX,
                GCInputButtonY,
                GCInputLeftThumbstick,
                GCInputRightThumbstick,
                GCInputLeftShoulder,
                GCInputRightShoulder,
                GCInputLeftTrigger,
                GCInputRightTrigger,
            ];
            let array: Id = objc::send2(
                objc::class("NSArray"),
                "arrayWithObjects:count:",
                elements.as_ptr(),
                elements.len(),
            );
            let set: Id = objc::send1(objc::class("NSSet"), "setWithArray:", array);
            let config: Id = objc::send0(objc::class("GCVirtualControllerConfiguration"), "alloc");
            let config: Id = objc::send0(config, "init");
            let () = objc::send1(config, "setElements:", set);
            let vc: Id = objc::send0(objc::class("GCVirtualController"), "alloc");
            let vc: Id = objc::send1(vc, "initWithConfiguration:", config);
            let () = objc::send1(
                vc,
                "connectWithReplyHandler:",
                std::ptr::null_mut::<c_void>(),
            );
            vc
        };
        t(format!(
            "virtual controller: {}",
            if vc.is_null() { "none" } else { "connecting" }
        ));
        let before = seen_count();
        let vid = wait_for_new(before);
        events("virtual controller", 100);
        controllers("virtual controller");
        let mut vgp = None;
        if let Some(vid) = vid {
            vgp = open_and_describe(vid);
            // SAFETY: GCVirtualController's controller and its profile.
            let extended: Id = unsafe {
                let controller: Id = objc::send0(vc, "controller");
                objc::send0(controller, "extendedGamepad")
            };
            if let Some(gp) = vgp.as_ref().filter(|_| !extended.is_null()) {
                state("virtual controller", gp);
                set_value(element(extended, "buttonA"), 1.0);
                events("virtual controller: A set", 100);
                state("virtual controller: A set", gp);
                set_value(element(extended, "buttonA"), 0.0);
                events("virtual controller: A unset", 100);
            }
        } else {
            t("virtual controller: no joystick");
        }

        // A snapshot controller (an extended gamepad) posted as connected
        // SAFETY: GCController's class method and properties.
        let snap: Id =
            unsafe { objc::send0(objc::class("GCController"), "controllerWithExtendedGamepad") };
        // SAFETY: as above.
        let (vendor, category, extended): (Id, Id, Id) = unsafe {
            (
                objc::send0(snap, "vendorName"),
                objc::send0(snap, "productCategory"),
                objc::send0(snap, "extendedGamepad"),
            )
        };
        t(format!(
            "snapshot: vendor={} category={} extended={}",
            ns(vendor),
            ns(category),
            yes_no(!extended.is_null())
        ));
        let before_snap = seen_count();
        // SAFETY: GameController's constant.
        objc::post_notification_for(unsafe { GCControllerDidConnectNotification }, snap);
        t("posted GCControllerDidConnectNotification (snapshot)");
        events("snapshot connected", 100);
        joysticks("snapshot");
        let sid = wait_for_new(before_snap);
        let mut sgp = None;
        if let Some(sid) = sid {
            sgp = open_and_describe(sid);
            if let Some(gp) = &sgp {
                state("snapshot", gp);
                drive("snapshot", gp, extended);
            }
        }
        // SAFETY: as above.
        objc::post_notification_for(unsafe { GCControllerDidConnectNotification }, snap);
        t("posted GCControllerDidConnectNotification (snapshot again)");
        events("snapshot again", 100);
        joysticks("snapshot again");

        // A micro gamepad snapshot
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
            ns(vendor),
            ns(category),
            yes_no(!micro_gamepad.is_null())
        ));
        let before_micro = seen_count();
        // SAFETY: GameController's constant.
        objc::post_notification_for(unsafe { GCControllerDidConnectNotification }, micro);
        t("posted GCControllerDidConnectNotification (micro snapshot)");
        events("micro snapshot connected", 100);
        match wait_for_new(before_micro) {
            Some(mid) => {
                if let Some(gp) = open_and_describe(mid) {
                    drop(gp);
                    t(format!("#{}: closed", label_of(mid)));
                }
            }
            None => t("micro snapshot: no joystick"),
        }
        // SAFETY: as above.
        objc::post_notification_for(unsafe { GCControllerDidDisconnectNotification }, micro);
        t("posted GCControllerDidDisconnectNotification (micro snapshot)");
        events("micro snapshot disconnected", 100);

        // The snapshot disconnected while open
        // SAFETY: as above.
        objc::post_notification_for(unsafe { GCControllerDidDisconnectNotification }, snap);
        t("posted GCControllerDidDisconnectNotification (snapshot)");
        events("snapshot disconnected", 100);
        joysticks("snapshot disconnected");
        if let Some(gp) = sgp {
            t(format!("snapshot: connected={}", yes_no(gp.connected())));
            result("snapshot: rumble", gp.rumble(0x4000, 0x8000, 100));
            result("snapshot: led", gp.set_led(0, 64, 255));
            result(
                "snapshot: accel on",
                gp.set_sensor_enabled(SensorType::Accel, true),
            );
            drop(gp);
            t("snapshot: closed");
        }

        // The virtual controller disconnected
        if let Some(gp) = vgp {
            drop(gp);
            t("virtual controller: closed");
        }
        if !vc.is_null() {
            // SAFETY: GCVirtualController's method.
            unsafe { objc::send0::<()>(vc, "disconnect") };
            t("virtual controller: disconnecting");
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end
                && vid.is_some_and(|id| joystick::joystick_name_for_id(id).is_ok())
            {
                run_loop(10);
            }
            run_loop(300);
        }
        events("virtual controller disconnected", 100);
        joysticks("virtual controller disconnected");
        controllers("virtual controller disconnected");

        // The driver off
        init::quit_subsystem(InitFlags::GAMEPAD);
        t("quit gamepad");
        interactions("window B", &a);
        events("quit", 100);
        hints::set(hints::JOYSTICK_MFI, "0").unwrap();
        let r = init::init(InitFlags::GAMEPAD);
        t(format!(
            "init gamepad (SDL_JOYSTICK_MFI=0): {}",
            r.as_ref()
                .map_or_else(|e| e.message().to_string(), |()| "ok".into())
        ));
        events("MFi off", 500);
        joysticks("MFi off");
        interactions("window B", &a);
        init::quit_subsystem(InitFlags::GAMEPAD);
        hints::reset(hints::JOYSTICK_MFI);
        t("quit gamepad");
        interactions("window B", &a);

        // Sensors
        // SAFETY: CMMotionManager's alloc and init, and its properties.
        let (accel, gyro, magnetometer, device_motion): (bool, bool, bool, bool) = unsafe {
            let mm: Id = objc::send0(objc::class("CMMotionManager"), "alloc");
            let mm: Id = objc::send0(mm, "init");
            (
                objc::send0(mm, "isAccelerometerAvailable"),
                objc::send0(mm, "isGyroAvailable"),
                objc::send0(mm, "isMagnetometerAvailable"),
                objc::send0(mm, "isDeviceMotionAvailable"),
            )
        };
        t(format!(
            "CMMotionManager: accelerometer={} gyro={} magnetometer={} device motion={}",
            yes_no(accel),
            yes_no(gyro),
            yes_no(magnetometer),
            yes_no(device_motion)
        ));
        let r = init::init(InitFlags::SENSOR);
        t(format!(
            "init sensor: {}",
            r.as_ref()
                .map_or_else(|e| e.message().to_string(), |()| "ok".into())
        ));
        let ids = sensor::sensors();
        t(format!("sensors: {}", ids.len()));
        for (i, &id) in ids.iter().enumerate() {
            let s = Sensor::open(id);
            t(format!(
                "sensor {i}: name={} type={} non-portable={} open={}",
                opt(sensor::sensor_name_for_id(id).ok().flatten().as_deref()),
                sensor::sensor_type_for_id(id).unwrap_or(SensorType::Invalid) as i32,
                sensor::sensor_non_portable_type_for_id(id).unwrap_or(-1),
                s.as_ref()
                    .map_or_else(|e| e.message().to_string(), |_| "ok".into())
            ));
        }
        let none = Sensor::open(0);
        t(format!(
            "open sensor 0: {}",
            none.as_ref()
                .map_or_else(|e| e.message().to_string(), |_| "ok".into())
        ));
        t(format!(
            "sensor 0: name={} type={}",
            opt(sensor::sensor_name_for_id(0).ok().flatten().as_deref()),
            sensor::sensor_type_for_id(0).unwrap_or(SensorType::Invalid) as i32
        ));
        events("sensors", 100);
        init::quit_subsystem(InitFlags::SENSOR);
        t("quit sensor");

        a.destroy();
        init::quit();
        t("done");

        check_trace(&TRACE.lock().unwrap(), EXPECTED);
    }
}
