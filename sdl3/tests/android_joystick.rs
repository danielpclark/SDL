// Tests of the Android joystick and haptic drivers in an app.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function makes a window and traces:
//!
//! * the joysticks and gamepads there are at the start (the emulator's
//!   keyboard, `qwerty2`, whose d-pad makes it one), their names, IDs,
//!   GUIDs (but their CRC, from the device's descriptor), types and
//!   mappings;
//! * the haptic devices (the system's vibrator, `VIBRATOR_SERVICE`): its
//!   features, the simple rumble and a left/right effect played and
//!   stopped, and what the system's vibrator was asked to play (the
//!   runner's `vibrations`);
//! * a gamepad plugged in (the runner's `gamepad add`: a uinput device made
//!   by the device's `uinput` command, with eleven buttons, six axes, a hat
//!   and force feedback): its events, its joystick's and gamepad's names,
//!   IDs, GUID, type and the mapping SDL_gamepad.c's Android guess makes
//!   from its button and axis masks; opened: its buttons, axes and
//!   capabilities (rumble: it has a vibrator), each button pressed and
//!   released, each axis moved and back (an axis's motion event reaches
//!   the app with the next input: Android batches motion events while
//!   nothing asks for a frame), the hat in each direction (the d-pad's
//!   buttons), with their events; rumble (the input reader's vibration of
//!   the device, then its stop once the rumble's time is up), the trigger
//!   rumble, the LED and the sensors (none: unsupported), the haptic of
//!   the joystick (none);
//! * its buttons while it's closed (keyboard keys: the south button's
//!   Return, the east one's Escape; their repeats, as many as the time
//!   the key is held, left out) and an axis (nothing);
//! * the gamepad unplugged while it's open (its events, disconnected, its
//!   rumble failing), then plugged in and unplugged once more;
//! * a DualSense plugged in before the launch (the runner's `devices=`:
//!   a uhid device made by the device's `hid` command, which the kernel's
//!   hid-playstation driver makes a gamepad with motion sensors, an RGB
//!   lightbar and force feedback, Android one input device with lights,
//!   sensors and a vibrator, API 31; among the joysticks at the start):
//!   its capabilities, its lightbar set (the lights session; the color
//!   the driver's last output report set, which the runner reads once
//!   it's removed), its accelerometer and gyroscope enabled (the device's
//!   sensor manager's listener) and their first events' data from an
//!   input report, its rumble (the device's vibrators: vibrating, then
//!   stopped; the motors' peak and last values), then unplugged while
//!   it's open; then plugged in while the app runs (`dualsense add`),
//!   where it has neither lights nor sensors: Android tells of the input
//!   device before its motion sensors' node is part of it, and the
//!   joystick handler keeps what it found then.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, run by the
//! runner's `--run-apk`) on the same emulator (API 35, Google APIs,
//! x86-64). Elsewhere this program does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app, with a DualSense plugged in before
/// the launch.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 22] = *b"app devices=dualsense\0";

#[cfg(target_os = "android")]
mod android_support;

#[cfg(target_os = "android")]
fn main() -> std::process::ExitCode {
    android_support::start();
    let status = android_support::checked(app::main);
    android_support::finish(status)
}

#[cfg(target_os = "android")]
mod app {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventType, JoystickID};
    use sdl3::guid::Guid;
    use sdl3::haptic::{self, Haptic, HapticEffect, HapticLeftRight};
    use sdl3::init::{self, InitFlags};
    use sdl3::joystick::gamepad::{self, Gamepad};
    use sdl3::joystick::{self, Joystick, PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN};
    use sdl3::sensor::SensorType;
    use sdl3::video::Window;

    use super::android_support::{check_trace, event_name, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
start: joysticks: 2 gamepads: 2
start: joystick J1: name=qwerty2 vendor=0x0627 product=0x0001 version=0x7851 type=1 player=0 guid=0500xxxx270600000100000051780000
start: gamepad J1: name=qwerty2 type=standard mapping=GUID,*,a:b0,b:b4,start:b6,dpup:b11,dpdown:b12,dpleft:b13,dpright:b14,crc:dcc8,
start: joystick J2: name=DualSense Wireless Controller vendor=0x054c product=0x0ce6 version=0xffff type=1 player=1 guid=0500xxxx4c050000e60c0000ffff3f00
start: gamepad J2: name=PS5 Controller type=ps5 mapping=GUID,PS5 Controller,a:b0,b:b1,back:b4,dpdown:b12,dpleft:b13,dpright:b14,dpup:b11,guide:b5,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b2,y:b3,
haptics: 1 mouse haptic=no
haptic 0: name=VIBRATOR_SERVICE
haptic: open: ok
haptic: name=VIBRATOR_SERVICE features=0x800 effects=1 playing=1 axes=0 rumble supported=yes
haptic: init rumble: ok
haptic: play rumble: ok
haptic: stop rumble: ok
haptic: effect supported=yes
haptic: create effect: id=0
haptic: run effect: ok
haptic: effect status: error: Haptic: Device does not support status queries.
haptic: stop effect: ok
haptic: gain: error: Haptic: Device does not support setting gain.
haptic: autocenter: error: Haptic: Device does not support setting autocenter.
haptic: pause: error: Haptic: Device does not support setting pausing.
haptic: resume: ok
haptic: stop effects: ok
haptic: closed
logcat: vibration: TOUCH cancelled_by_user Step=150ms(amplitude=0.55)
logcat: vibration: TOUCH cancelled_by_user Step=200ms(amplitude=0.50)
create: ok
logcat: gamepad: added
added: event: JOYSTICK_ADDED(which=J3)
added: event: GAMEPAD_ADDED(which=J3)
added: new joysticks: 1
added: joystick J3: name=SDL3 Test Gamepad vendor=0x1209 product=0x5d3a version=0x7fff type=1 player=2 guid=0500xxxx091200003a5d0000ff7f3f80
added: gamepad J3: name=SDL3 Test Gamepad type=standard mapping=GUID,*,a:b0,b:b1,x:b2,y:b3,back:b4,guide:b5,start:b6,leftstick:b7,rightstick:b8,leftshoulder:b9,rightshoulder:b10,dpup:b11,dpdown:b12,dpleft:b13,dpright:b14,leftx:a0,lefty:a1,rightx:a2,righty:a3,lefttrigger:a4,righttrigger:a5,crc:ec2b,
open: ok
open: axes=6 buttons=15 hats=0 rumble=yes trigger rumble=no rgb led=no accel=no gyro=no player=2 connected=yes
open: events: (none)
BTN_SOUTH down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=0)
BTN_SOUTH down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=0)
BTN_SOUTH up: event: GAMEPAD_BUTTON_UP(which=J3 button=0)
BTN_SOUTH up: event: JOYSTICK_BUTTON_UP(which=J3 button=0)
BTN_EAST down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=1)
BTN_EAST down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=1)
BTN_EAST up: event: GAMEPAD_BUTTON_UP(which=J3 button=1)
BTN_EAST up: event: JOYSTICK_BUTTON_UP(which=J3 button=1)
BTN_NORTH down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=2)
BTN_NORTH down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=2)
BTN_NORTH up: event: GAMEPAD_BUTTON_UP(which=J3 button=2)
BTN_NORTH up: event: JOYSTICK_BUTTON_UP(which=J3 button=2)
BTN_WEST down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=3)
BTN_WEST down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=3)
BTN_WEST up: event: GAMEPAD_BUTTON_UP(which=J3 button=3)
BTN_WEST up: event: JOYSTICK_BUTTON_UP(which=J3 button=3)
BTN_TL down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=9)
BTN_TL down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=9)
BTN_TL up: event: GAMEPAD_BUTTON_UP(which=J3 button=9)
BTN_TL up: event: JOYSTICK_BUTTON_UP(which=J3 button=9)
BTN_TR down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=10)
BTN_TR down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=10)
BTN_TR up: event: GAMEPAD_BUTTON_UP(which=J3 button=10)
BTN_TR up: event: JOYSTICK_BUTTON_UP(which=J3 button=10)
BTN_SELECT down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=4)
BTN_SELECT down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=4)
BTN_SELECT up: event: GAMEPAD_BUTTON_UP(which=J3 button=4)
BTN_SELECT up: event: JOYSTICK_BUTTON_UP(which=J3 button=4)
BTN_START down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=6)
BTN_START down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=6)
BTN_START up: event: GAMEPAD_BUTTON_UP(which=J3 button=6)
BTN_START up: event: JOYSTICK_BUTTON_UP(which=J3 button=6)
BTN_MODE down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=5)
BTN_MODE down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=5)
BTN_MODE up: event: JOYSTICK_BUTTON_UP(which=J3 button=5)
BTN_MODE up: event: GAMEPAD_BUTTON_UP(which=J3 button=5)
BTN_THUMBL down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=7)
BTN_THUMBL down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=7)
BTN_THUMBL up: event: GAMEPAD_BUTTON_UP(which=J3 button=7)
BTN_THUMBL up: event: JOYSTICK_BUTTON_UP(which=J3 button=7)
BTN_THUMBR down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=8)
BTN_THUMBR down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=8)
BTN_THUMBR up: event: GAMEPAD_BUTTON_UP(which=J3 button=8)
BTN_THUMBR up: event: JOYSTICK_BUTTON_UP(which=J3 button=8)
ABS_X 16384: events: (none)
ABS_X 0: event: GAMEPAD_AXIS_MOTION(which=J3 axis=0 value=16384)
ABS_X 0: event: JOYSTICK_AXIS_MOTION(which=J3 axis=0 value=16384)
ABS_X 0: event: GAMEPAD_AXIS_MOTION(which=J3 axis=0 value=0)
ABS_X 0: event: JOYSTICK_AXIS_MOTION(which=J3 axis=0 value=0)
ABS_Y -32768: event: GAMEPAD_AXIS_MOTION(which=J3 axis=1 value=0)
ABS_Y -32768: event: JOYSTICK_AXIS_MOTION(which=J3 axis=1 value=0)
ABS_Y -32768: event: GAMEPAD_AXIS_MOTION(which=J3 axis=1 value=-32767)
ABS_Y -32768: event: JOYSTICK_AXIS_MOTION(which=J3 axis=1 value=-32767)
ABS_Y 0: event: GAMEPAD_AXIS_MOTION(which=J3 axis=1 value=0)
ABS_Y 0: event: JOYSTICK_AXIS_MOTION(which=J3 axis=1 value=0)
ABS_RX 32767: event: GAMEPAD_AXIS_MOTION(which=J3 axis=2 value=0)
ABS_RX 32767: event: JOYSTICK_AXIS_MOTION(which=J3 axis=2 value=0)
ABS_RX 32767: event: GAMEPAD_AXIS_MOTION(which=J3 axis=2 value=32767)
ABS_RX 32767: event: JOYSTICK_AXIS_MOTION(which=J3 axis=2 value=32767)
ABS_RX 0: event: GAMEPAD_AXIS_MOTION(which=J3 axis=2 value=0)
ABS_RX 0: event: JOYSTICK_AXIS_MOTION(which=J3 axis=2 value=0)
ABS_RY -16384: event: GAMEPAD_AXIS_MOTION(which=J3 axis=3 value=0)
ABS_RY -16384: event: JOYSTICK_AXIS_MOTION(which=J3 axis=3 value=0)
ABS_RY -16384: event: GAMEPAD_AXIS_MOTION(which=J3 axis=3 value=-16383)
ABS_RY -16384: event: JOYSTICK_AXIS_MOTION(which=J3 axis=3 value=-16383)
ABS_RY 0: event: GAMEPAD_AXIS_MOTION(which=J3 axis=3 value=0)
ABS_RY 0: event: JOYSTICK_AXIS_MOTION(which=J3 axis=3 value=0)
ABS_Z 255: event: GAMEPAD_AXIS_MOTION(which=J3 axis=4 value=16383)
ABS_Z 255: event: JOYSTICK_AXIS_MOTION(which=J3 axis=4 value=0)
ABS_Z 255: event: GAMEPAD_AXIS_MOTION(which=J3 axis=4 value=32767)
ABS_Z 255: event: JOYSTICK_AXIS_MOTION(which=J3 axis=4 value=32767)
ABS_Z 0: event: GAMEPAD_AXIS_MOTION(which=J3 axis=4 value=0)
ABS_Z 0: event: JOYSTICK_AXIS_MOTION(which=J3 axis=4 value=-32767)
ABS_RZ 128: events: (none)
ABS_RZ 0: event: GAMEPAD_AXIS_MOTION(which=J3 axis=5 value=16383)
ABS_RZ 0: event: JOYSTICK_AXIS_MOTION(which=J3 axis=5 value=0)
ABS_RZ 0: event: GAMEPAD_AXIS_MOTION(which=J3 axis=5 value=0)
ABS_RZ 0: event: JOYSTICK_AXIS_MOTION(which=J3 axis=5 value=-32767)
axes: 0 0 0 0 -32767 -32767
hat left: event: GAMEPAD_BUTTON_DOWN(which=J3 button=13)
hat left: event: JOYSTICK_BUTTON_DOWN(which=J3 button=13)
hat right: event: GAMEPAD_BUTTON_UP(which=J3 button=13)
hat right: event: JOYSTICK_BUTTON_UP(which=J3 button=13)
hat right: event: GAMEPAD_BUTTON_DOWN(which=J3 button=14)
hat right: event: JOYSTICK_BUTTON_DOWN(which=J3 button=14)
hat center x: event: GAMEPAD_BUTTON_UP(which=J3 button=14)
hat center x: event: JOYSTICK_BUTTON_UP(which=J3 button=14)
hat up: event: GAMEPAD_BUTTON_DOWN(which=J3 button=11)
hat up: event: JOYSTICK_BUTTON_DOWN(which=J3 button=11)
hat down: event: GAMEPAD_BUTTON_UP(which=J3 button=11)
hat down: event: JOYSTICK_BUTTON_UP(which=J3 button=11)
hat down: event: GAMEPAD_BUTTON_DOWN(which=J3 button=12)
hat down: event: JOYSTICK_BUTTON_DOWN(which=J3 button=12)
hat center y: event: GAMEPAD_BUTTON_UP(which=J3 button=12)
hat center y: event: JOYSTICK_BUTTON_UP(which=J3 button=12)
hat right up: event: GAMEPAD_BUTTON_DOWN(which=J3 button=11)
hat right up: event: JOYSTICK_BUTTON_DOWN(which=J3 button=11)
hat right up: event: GAMEPAD_BUTTON_DOWN(which=J3 button=14)
hat right up: event: JOYSTICK_BUTTON_DOWN(which=J3 button=14)
hat center: event: GAMEPAD_BUTTON_UP(which=J3 button=11)
hat center: event: JOYSTICK_BUTTON_UP(which=J3 button=11)
hat center: event: GAMEPAD_BUTTON_UP(which=J3 button=14)
hat center: event: JOYSTICK_BUTTON_UP(which=J3 button=14)
rumble: ok
logcat: gamepad: vibrating true
rumble: after its time
logcat: gamepad: vibrating false
rumble triggers: error: That operation is not supported
led: error: That operation is not supported
sensor: error: No such sensor on this device
haptic: joystick is haptic=no
haptic: open from joystick: error: Haptic: Joystick isn't a haptic device.
effects: events: (none)
closed
closed BTN_SOUTH down: event: KEY_DOWN(which=0 scancode=40 key=0xd mod=0x0 raw=96 repeat=0)
closed BTN_SOUTH up: event: KEY_UP(which=0 scancode=40 key=0xd mod=0x0 raw=96 repeat=0)
closed BTN_EAST down: event: KEY_DOWN(which=0 scancode=41 key=0x1b mod=0x0 raw=97 repeat=0)
closed BTN_EAST up: event: KEY_UP(which=0 scancode=41 key=0x1b mod=0x0 raw=97 repeat=0)
closed ABS_X: events: (none)
closed ABS_X back: events: (none)
open again
open again: events: (none)
logcat: gamepad: removed
removed: event: GAMEPAD_REMOVED(which=J3)
removed: event: JOYSTICK_REMOVED(which=J3)
removed: connected=no rumble: error: Rumble failed, device disconnected
removed: joysticks: 2 gamepads: 2
removed: joystick J1: name=qwerty2 vendor=0x0627 product=0x0001 version=0x7851 type=1 player=0 guid=0500xxxx270600000100000051780000
removed: gamepad J1: name=qwerty2 type=standard mapping=GUID,*,a:b0,b:b4,start:b6,dpup:b11,dpdown:b12,dpleft:b13,dpright:b14,crc:dcc8,
removed: joystick J2: name=DualSense Wireless Controller vendor=0x054c product=0x0ce6 version=0xffff type=1 player=1 guid=0500xxxx4c050000e60c0000ffff3f00
removed: gamepad J2: name=PS5 Controller type=ps5 mapping=GUID,PS5 Controller,a:b0,b:b1,back:b4,dpdown:b12,dpleft:b13,dpright:b14,dpup:b11,guide:b5,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b2,y:b3,
logcat: gamepad: added
added again: event: JOYSTICK_ADDED(which=J4)
added again: event: GAMEPAD_ADDED(which=J4)
added again: new joysticks: 1
added again: joystick J4: name=SDL3 Test Gamepad vendor=0x1209 product=0x5d3a version=0x7fff type=1 player=2 guid=0500xxxx091200003a5d0000ff7f3f80
added again: gamepad J4: name=SDL3 Test Gamepad type=standard mapping=GUID,*,a:b0,b:b1,x:b2,y:b3,back:b4,guide:b5,start:b6,leftstick:b7,rightstick:b8,leftshoulder:b9,rightshoulder:b10,dpup:b11,dpdown:b12,dpleft:b13,dpright:b14,leftx:a0,lefty:a1,rightx:a2,righty:a3,lefttrigger:a4,righttrigger:a5,crc:ec2b,
logcat: gamepad: removed
removed again: event: GAMEPAD_REMOVED(which=J4)
removed again: event: JOYSTICK_REMOVED(which=J4)
dualsense: open: ok
dualsense: axes=8 buttons=17 hats=0 rumble=yes trigger rumble=no rgb led=yes accel=yes gyro=yes player=1 connected=yes
dualsense: led: ok
dualsense: accel on: ok
dualsense: gyro on: ok
dualsense: enabled accel=yes gyro=yes rate=0.0
dualsense: accel: 4.903 -9.807 2.452
dualsense: gyro: 1.178 -2.356 3.534
dualsense: accel off: ok
dualsense: gyro off: ok
dualsense: rumble: ok
logcat: dualsense: vibrating true
dualsense: rumble: after its time
logcat: dualsense: vibrating false
logcat: dualsense: lightbar 255 0 0
logcat: dualsense: motors peak 191 191 last 0 0
logcat: dualsense: removed
dualsense removed: event: GAMEPAD_AXIS_MOTION(which=J2 axis=0 value=0)
dualsense removed: event: GAMEPAD_AXIS_MOTION(which=J2 axis=1 value=0)
dualsense removed: event: GAMEPAD_AXIS_MOTION(which=J2 axis=2 value=0)
dualsense removed: event: GAMEPAD_AXIS_MOTION(which=J2 axis=3 value=0)
dualsense removed: event: GAMEPAD_REMOVED(which=J2)
dualsense removed: event: JOYSTICK_REMOVED(which=J2)
dualsense removed: connected=no led: error: SetLED failed, device disconnected
logcat: dualsense: added
dualsense added: event: JOYSTICK_ADDED(which=J5)
dualsense added: event: GAMEPAD_ADDED(which=J5)
dualsense added: new joysticks: 1
dualsense added: joystick J5: name=DualSense Wireless Controller vendor=0x054c product=0x0ce6 version=0xffff type=1 player=1 guid=0500xxxx4c050000e60c0000ffff3f00
dualsense added: gamepad J5: name=PS5 Controller type=ps5 mapping=GUID,PS5 Controller,a:b0,b:b1,back:b4,dpdown:b12,dpleft:b13,dpright:b14,dpup:b11,guide:b5,leftshoulder:b9,leftstick:b7,lefttrigger:a4,leftx:a0,lefty:a1,rightshoulder:b10,rightstick:b8,righttrigger:a5,rightx:a2,righty:a3,start:b6,x:b2,y:b3,
dualsense added: axes=8 buttons=17 hats=0 rumble=yes trigger rumble=no rgb led=no accel=no gyro=no player=1 connected=yes
logcat: dualsense: lightbar 0 0 128
logcat: dualsense: motors peak 0 0 last 0 0
logcat: dualsense: removed
dualsense removed again: event: GAMEPAD_REMOVED(which=J5)
dualsense removed again: event: JOYSTICK_REMOVED(which=J5)
end: joysticks: 1 gamepads: 1
end: joystick J1: name=qwerty2 vendor=0x0627 product=0x0001 version=0x7851 type=1 player=0 guid=0500xxxx270600000100000051780000
end: gamepad J1: name=qwerty2 type=standard mapping=GUID,*,a:b0,b:b4,start:b6,dpup:b11,dpdown:b12,dpleft:b13,dpright:b14,crc:dcc8,
quit
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    fn yes(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    fn result<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        }
    }

    /// The joysticks' IDs in the order the trace first names them: `J1`,
    /// `J2`, ...
    static IDS: Mutex<Vec<JoystickID>> = Mutex::new(Vec::new());

    fn jname(id: JoystickID) -> String {
        if id == 0 {
            return "0".to_string();
        }
        let mut ids = IDS.lock().unwrap();
        let i = match ids.iter().position(|&i| i == id) {
            Some(i) => i,
            None => {
                ids.push(id);
                ids.len() - 1
            }
        };
        format!("J{}", i + 1)
    }

    /// A GUID's bytes in hexadecimal, its CRC (bytes 2 and 3, from the
    /// device's descriptor) as `xxxx`.
    fn guid_text(guid: Guid) -> String {
        let mut s = String::new();
        for (i, b) in guid.0.iter().enumerate() {
            if i == 2 || i == 3 {
                s.push_str("xx");
            } else {
                s.push_str(&format!("{b:02x}"));
            }
        }
        s
    }

    /// A mapping string with its GUID (its first field) as `GUID`.
    fn mapping_text(mapping: Option<String>) -> String {
        match mapping {
            Some(m) => match m.split_once(',') {
                Some((_, rest)) => format!("GUID,{rest}"),
                None => m,
            },
            None => "(null)".to_string(),
        }
    }

    /// An event as the trace names it (the C program's names).
    fn describe(event: &Event) -> String {
        let n = event_name(event.event_type());
        match event {
            Event::JoyDevice(e) => format!("{n}(which={})", jname(e.which)),
            Event::JoyAxis(e) => format!(
                "{n}(which={} axis={} value={})",
                jname(e.which),
                e.axis,
                e.value
            ),
            Event::JoyHat(e) => format!(
                "{n}(which={} hat={} value={})",
                jname(e.which),
                e.hat,
                e.value
            ),
            Event::JoyButton(e) => {
                format!("{n}(which={} button={})", jname(e.which), e.button)
            }
            Event::GamepadDevice(e) => format!("{n}(which={})", jname(e.which)),
            Event::GamepadAxis(e) => format!(
                "{n}(which={} axis={} value={})",
                jname(e.which),
                e.axis,
                e.value
            ),
            Event::GamepadButton(e) => {
                format!("{n}(which={} button={})", jname(e.which), e.button)
            }
            Event::Key(e) => format!(
                "{n}(which={} scancode={} key={:#x} mod={:#x} raw={} repeat={})",
                e.which, e.scancode.0, e.key.0, e.modifiers.0, e.raw, e.repeat as i32
            ),
            _ => n,
        }
    }

    /// Whether the trace keeps an event: the joysticks' and gamepads' (but
    /// their update completes, batteries and sensors), and the keys.
    fn traced(ty: EventType) -> bool {
        matches!(
            ty,
            EventType::JOYSTICK_AXIS_MOTION
                | EventType::JOYSTICK_HAT_MOTION
                | EventType::JOYSTICK_BUTTON_DOWN
                | EventType::JOYSTICK_BUTTON_UP
                | EventType::JOYSTICK_ADDED
                | EventType::JOYSTICK_REMOVED
                | EventType::GAMEPAD_AXIS_MOTION
                | EventType::GAMEPAD_BUTTON_DOWN
                | EventType::GAMEPAD_BUTTON_UP
                | EventType::GAMEPAD_ADDED
                | EventType::GAMEPAD_REMOVED
                | EventType::GAMEPAD_REMAPPED
                | EventType::KEY_DOWN
                | EventType::KEY_UP
        )
    }

    struct Events {
        seen: Vec<EventType>,
        lines: Vec<String>,
        /// The sensors' events: the sensor, its data.
        sensors: Vec<(i32, [f32; 3])>,
    }

    static EVENTS: Mutex<Events> = Mutex::new(Events {
        seen: Vec::new(),
        lines: Vec::new(),
        sensors: Vec::new(),
    });

    fn take_queued() {
        events::pump();
        while let Some(event) = events::poll() {
            let ty = event.event_type();
            let mut e = EVENTS.lock().unwrap();
            e.seen.push(ty);
            // (but the keys' repeats, as many as the time a key is held)
            if traced(ty) && !matches!(&event, Event::Key(k) if k.repeat) {
                e.lines.push(describe(&event));
            }
            if let Event::GamepadSensor(s) = &event {
                e.sensors.push((s.sensor, s.data));
            }
        }
    }

    fn saw_type(ty: EventType) -> bool {
        EVENTS.lock().unwrap().seen.contains(&ty)
    }

    /// Pump until an event of `ty` (none: `None`), up to 5 s, then `ms`
    /// milliseconds.
    fn settle(ty: Option<EventType>, ms: u64) {
        let end = Instant::now() + Duration::from_millis(5000);
        take_queued();
        while ty.is_some_and(|ty| !saw_type(ty)) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
            take_queued();
        }
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
            take_queued();
        }
        EVENTS.lock().unwrap().seen.clear();
    }

    fn clear_events() {
        let mut e = EVENTS.lock().unwrap();
        e.lines.clear();
        e.seen.clear();
    }

    /// The events since the last trace, one line each.
    fn trace_events(label: &str) {
        let lines = std::mem::take(&mut EVENTS.lock().unwrap().lines);
        if lines.is_empty() {
            t(format!("{label}: events: (none)"));
        }
        for line in lines {
            t(format!("{label}: event: {line}"));
        }
    }

    /// A request of the runner's, and the lines of its answer in the trace
    /// (as the C reference's job puts them there).
    fn ask(what: &str) {
        for line in request(what) {
            t(format!("logcat: {line}"));
        }
    }

    /// A joystick's description (as SDL has it without opening it).
    fn info(label: &str, id: JoystickID) {
        let name = joystick::joystick_name_for_id(id).ok().flatten();
        t(format!(
            "{label}: joystick {}: name={} vendor=0x{:04x} product=0x{:04x} version=0x{:04x} type={} player={} guid={}",
            jname(id),
            name.as_deref().unwrap_or("(null)"),
            joystick::joystick_vendor_for_id(id),
            joystick::joystick_product_for_id(id),
            joystick::joystick_product_version_for_id(id),
            joystick::joystick_type_for_id(id) as i32,
            joystick::joystick_player_index_for_id(id),
            guid_text(joystick::joystick_guid_for_id(id))
        ));
        if gamepad::is_gamepad(id) {
            let name = gamepad::gamepad_name_for_id(id).ok().flatten();
            t(format!(
                "{label}: gamepad {}: name={} type={} mapping={}",
                jname(id),
                name.as_deref().unwrap_or("(null)"),
                gamepad::gamepad_type_for_id(id)
                    .as_str()
                    .unwrap_or("(null)"),
                mapping_text(gamepad::gamepad_mapping_for_id(id))
            ));
        } else {
            t(format!("{label}: {}: not a gamepad", jname(id)));
        }
    }

    /// The joysticks there are.
    fn list(label: &str) {
        let ids = joystick::joysticks();
        t(format!(
            "{label}: joysticks: {} gamepads: {}",
            ids.len(),
            gamepad::gamepads().len()
        ));
        for id in ids {
            info(label, id);
        }
    }

    /// The gamepad's buttons, axes and capabilities, opened.
    fn opened(label: &str, pad: &Gamepad, joy: &Joystick) {
        let props = joy.properties().unwrap();
        let cap = |name: &str| yes(props.get_bool(name).unwrap_or(false));
        t(format!(
            "{label}: axes={} buttons={} hats={} rumble={} trigger rumble={} rgb led={} accel={} gyro={} player={} connected={}",
            joy.num_axes().unwrap_or(0),
            joy.num_buttons().unwrap_or(0),
            joy.num_hats().unwrap_or(0),
            cap(PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN),
            cap(joystick::PROP_JOYSTICK_CAP_TRIGGER_RUMBLE_BOOLEAN),
            cap(joystick::PROP_JOYSTICK_CAP_RGB_LED_BOOLEAN),
            yes(pad.has_sensor(SensorType::Accel)),
            yes(pad.has_sensor(SensorType::Gyro)),
            pad.player_index(),
            yes(pad.connected())
        ));
    }

    /// A gamepad request's events: the request, then the events until
    /// `ty`, then 150 ms.
    fn input(label: &str, args: &str, ty: Option<EventType>) {
        ask(&format!("gamepad {args}"));
        settle(ty, 150);
        trace_events(label);
    }

    const BUTTONS: [&str; 11] = [
        "BTN_SOUTH",
        "BTN_EAST",
        "BTN_NORTH",
        "BTN_WEST",
        "BTN_TL",
        "BTN_TR",
        "BTN_SELECT",
        "BTN_START",
        "BTN_MODE",
        "BTN_THUMBL",
        "BTN_THUMBR",
    ];

    const AXES: [(&str, i32); 6] = [
        ("ABS_X", 16384),
        ("ABS_Y", -32768),
        ("ABS_RX", 32767),
        ("ABS_RY", -16384),
        ("ABS_Z", 255),
        ("ABS_RZ", 128),
    ];

    /// The gamepad plugged in: its events and description, its ID.
    fn plug(label: &str) -> JoystickID {
        let before = joystick::joysticks();
        ask("gamepad add");
        settle(Some(EventType::GAMEPAD_ADDED), 300);
        trace_events(label);
        let added: Vec<JoystickID> = joystick::joysticks()
            .into_iter()
            .filter(|id| !before.contains(id))
            .collect();
        t(format!("{label}: new joysticks: {}", added.len()));
        for &id in &added {
            info(label, id);
        }
        added.first().copied().unwrap_or(0)
    }

    fn haptics() {
        let ids = haptic::haptics();
        t(format!(
            "haptics: {} mouse haptic={}",
            ids.len(),
            yes(haptic::is_mouse_haptic())
        ));
        for (i, &id) in ids.iter().enumerate() {
            let name = haptic::haptic_name_for_id(id).ok().flatten();
            t(format!(
                "haptic {i}: name={}",
                name.as_deref().unwrap_or("(null)")
            ));
        }
        let Some(&id) = ids.first() else {
            return;
        };
        let opened = Haptic::open(id);
        t(format!("haptic: open: {}", result(&opened)));
        let Ok(h) = opened else {
            return;
        };
        t(format!(
            "haptic: name={} features=0x{:x} effects={} playing={} axes={} rumble supported={}",
            h.name().ok().flatten().as_deref().unwrap_or("(null)"),
            h.features().map(|f| f.0).unwrap_or(0),
            h.max_effects().unwrap_or(0),
            h.max_effects_playing().unwrap_or(0),
            h.num_axes().unwrap_or(0),
            yes(h.rumble_supported())
        ));
        t(format!("haptic: init rumble: {}", result(&h.init_rumble())));
        t(format!(
            "haptic: play rumble: {}",
            result(&h.play_rumble(0.5, 200))
        ));
        t(format!("haptic: stop rumble: {}", result(&h.stop_rumble())));
        let effect = HapticEffect::LeftRight(HapticLeftRight {
            length: 150,
            large_magnitude: 0x6000,
            small_magnitude: 0x2000,
        });
        t(format!(
            "haptic: effect supported={}",
            yes(h.effect_supported(&effect))
        ));
        let created = h.create_effect(&effect);
        t(format!(
            "haptic: create effect: {}",
            match &created {
                Ok(id) => format!("id={id}"),
                Err(e) => format!("error: {e}"),
            }
        ));
        if let Ok(e) = created {
            t(format!(
                "haptic: run effect: {}",
                result(&h.run_effect(e, 1))
            ));
            t(format!(
                "haptic: effect status: {}",
                match h.effect_status(e) {
                    Ok(playing) => yes(playing).to_string(),
                    Err(err) => format!("error: {err}"),
                }
            ));
            t(format!(
                "haptic: stop effect: {}",
                result(&h.stop_effect(e))
            ));
            h.destroy_effect(e);
        }
        t(format!("haptic: gain: {}", result(&h.set_gain(50))));
        t(format!(
            "haptic: autocenter: {}",
            result(&h.set_autocenter(0))
        ));
        t(format!("haptic: pause: {}", result(&h.pause())));
        t(format!("haptic: resume: {}", result(&h.resume())));
        t(format!(
            "haptic: stop effects: {}",
            result(&h.stop_effects())
        ));
        drop(h);
        t("haptic: closed");
        // (the system's vibrator's: what was asked of it)
        ask("vibrations");
    }

    /// Whether a sensor's event came (`SensorType` as its number).
    fn saw_sensor(sensor: SensorType) -> bool {
        EVENTS
            .lock()
            .unwrap()
            .sensors
            .iter()
            .any(|&(s, _)| s == sensor as i32)
    }

    /// The first event's data of a sensor.
    fn first_sensor_data(label: &str, name: &str, sensor: SensorType) {
        let data = EVENTS
            .lock()
            .unwrap()
            .sensors
            .iter()
            .find(|&&(s, _)| s == sensor as i32)
            .map(|&(_, d)| d);
        match data {
            Some(d) => t(format!(
                "{label}: {name}: {:.3} {:.3} {:.3}",
                d[0], d[1], d[2]
            )),
            None => t(format!("{label}: {name}: none")),
        }
    }

    /// The DualSense plugged in before the launch (the runner's: a uhid
    /// device that the kernel's hid-playstation driver makes a gamepad
    /// with motion sensors, an RGB lightbar and force feedback): its
    /// lightbar, sensors and rumble, then unplugged while it's open; then
    /// plugged in again while the app runs.
    fn dualsense() {
        let Some(id) = joystick::joysticks().into_iter().find(|&id| {
            joystick::joystick_vendor_for_id(id) == 0x054c
                && joystick::joystick_product_for_id(id) == 0x0ce6
        }) else {
            t("dualsense: none");
            return;
        };
        let opened_pad = Gamepad::open(id);
        t(format!("dualsense: open: {}", result(&opened_pad)));
        let Ok(pad) = opened_pad else {
            return;
        };
        let joy = pad.joystick().expect("its joystick");
        opened("dualsense", &pad, &joy);

        t(format!(
            "dualsense: led: {}",
            result(&pad.set_led(255, 0, 0))
        ));
        t(format!(
            "dualsense: accel on: {}",
            result(&pad.set_sensor_enabled(SensorType::Accel, true))
        ));
        t(format!(
            "dualsense: gyro on: {}",
            result(&pad.set_sensor_enabled(SensorType::Gyro, true))
        ));
        t(format!(
            "dualsense: enabled accel={} gyro={} rate={:.1}",
            yes(pad.sensor_enabled(SensorType::Accel)),
            yes(pad.sensor_enabled(SensorType::Gyro)),
            pad.sensor_data_rate(SensorType::Accel)
        ));
        // (reports until both sensors' events come: the listener's
        // registration takes its time)
        EVENTS.lock().unwrap().sensors.clear();
        for _ in 0..25 {
            ask("dualsense sensors 1000 -2000 3000 4096 -8192 2048");
            settle(None, 200);
            if saw_sensor(SensorType::Accel) && saw_sensor(SensorType::Gyro) {
                break;
            }
        }
        first_sensor_data("dualsense", "accel", SensorType::Accel);
        first_sensor_data("dualsense", "gyro", SensorType::Gyro);
        t(format!(
            "dualsense: accel off: {}",
            result(&pad.set_sensor_enabled(SensorType::Accel, false))
        ));
        t(format!(
            "dualsense: gyro off: {}",
            result(&pad.set_sensor_enabled(SensorType::Gyro, false))
        ));

        t(format!(
            "dualsense: rumble: {}",
            result(&pad.rumble(0xc000, 0x4000, 1000))
        ));
        ask("dualsense vibrating yes");
        settle(None, 1500);
        t("dualsense: rumble: after its time");
        ask("dualsense vibrating no");
        clear_events();
        EVENTS.lock().unwrap().sensors.clear();

        // Unplugged while it's open: what its output reports set
        ask("dualsense remove");
        settle(Some(EventType::GAMEPAD_REMOVED), 300);
        trace_events("dualsense removed");
        t(format!(
            "dualsense removed: connected={} led: {}",
            yes(pad.connected()),
            result(&pad.set_led(0, 255, 0))
        ));
        drop(joy);
        drop(pad);

        // Plugged in while the app runs: Android tells of it before its
        // sensors and lights are part of it
        let before = joystick::joysticks();
        ask("dualsense add");
        settle(Some(EventType::GAMEPAD_ADDED), 1000);
        trace_events("dualsense added");
        let added: Vec<JoystickID> = joystick::joysticks()
            .into_iter()
            .filter(|id| !before.contains(id))
            .collect();
        t(format!("dualsense added: new joysticks: {}", added.len()));
        for &id in &added {
            info("dualsense added", id);
            if let Ok(pad) = Gamepad::open(id) {
                let joy = pad.joystick().expect("its joystick");
                opened("dualsense added", &pad, &joy);
            }
        }
        ask("dualsense remove");
        settle(Some(EventType::GAMEPAD_REMOVED), 300);
        trace_events("dualsense removed again");
    }

    pub(super) fn main() {
        t(format!(
            "init: {}",
            result(&init::init(
                InitFlags::VIDEO | InitFlags::GAMEPAD | InitFlags::HAPTIC
            ))
        ));
        list("start");
        haptics();

        let created = Window::create("SDL joystick", 320, 240, WindowFlags::RESIZABLE);
        t(format!("create: {}", result(&created)));
        let _w = created.expect("the window");
        // (until the activity's surface is ready and has the focus: the
        // joysticks' events are dropped while a window lacks it)
        settle(Some(EventType::WINDOW_FOCUS_GAINED), 2000);
        clear_events();

        // Plugged in
        let id = plug("added");
        let opened_pad = Gamepad::open(id);
        t(format!("open: {}", result(&opened_pad)));
        let pad = opened_pad.expect("the gamepad");
        let joy = pad.joystick().expect("its joystick");
        opened("open", &pad, &joy);
        settle(None, 100);
        trace_events("open");

        // Each button
        for button in BUTTONS {
            input(
                &format!("{button} down"),
                &format!("key {button} 1"),
                Some(EventType::JOYSTICK_BUTTON_DOWN),
            );
            input(
                &format!("{button} up"),
                &format!("key {button} 0"),
                Some(EventType::JOYSTICK_BUTTON_UP),
            );
        }

        // Each axis, and back
        for (axis, value) in AXES {
            input(
                &format!("{axis} {value}"),
                &format!("abs {axis} {value}"),
                Some(EventType::JOYSTICK_AXIS_MOTION),
            );
            input(
                &format!("{axis} 0"),
                &format!("abs {axis} 0"),
                Some(EventType::JOYSTICK_AXIS_MOTION),
            );
        }
        t(format!(
            "axes: {}",
            (0..joy.num_axes().unwrap_or(0))
                .map(|i| joy.axis(i).unwrap_or(0).to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ));

        // The hat: the d-pad's buttons
        for (label, args) in [
            ("hat left", "abs ABS_HAT0X -1"),
            ("hat right", "abs ABS_HAT0X 1"),
            ("hat center x", "abs ABS_HAT0X 0"),
            ("hat up", "abs ABS_HAT0Y -1"),
            ("hat down", "abs ABS_HAT0Y 1"),
            ("hat center y", "abs ABS_HAT0Y 0"),
            ("hat right up", "abs ABS_HAT0X 1 ABS_HAT0Y -1"),
            ("hat center", "abs ABS_HAT0X 0 ABS_HAT0Y 0"),
        ] {
            input(label, args, Some(EventType::JOYSTICK_BUTTON_UP));
        }

        // Rumble, the triggers' rumble, the LED, the sensors
        t(format!(
            "rumble: {}",
            result(&pad.rumble(0xc000, 0x4000, 2000))
        ));
        ask("gamepad vibrating yes");
        settle(None, 2500);
        t("rumble: after its time");
        ask("gamepad vibrating no");
        t(format!(
            "rumble triggers: {}",
            result(&pad.rumble_triggers(0x8000, 0x8000, 300))
        ));
        t(format!("led: {}", result(&pad.set_led(255, 0, 0))));
        t(format!(
            "sensor: {}",
            result(&pad.set_sensor_enabled(SensorType::Accel, true))
        ));
        t(format!(
            "haptic: joystick is haptic={}",
            yes(haptic::is_joystick_haptic(&joy))
        ));
        t(format!(
            "haptic: open from joystick: {}",
            result(&Haptic::open_from_joystick(&joy))
        ));
        settle(None, 100);
        trace_events("effects");

        // Closed: the keys are keyboard keys, the axes nothing
        drop(joy);
        drop(pad);
        t("closed");
        for button in ["BTN_SOUTH", "BTN_EAST"] {
            input(
                &format!("closed {button} down"),
                &format!("key {button} 1"),
                Some(EventType::KEY_DOWN),
            );
            input(
                &format!("closed {button} up"),
                &format!("key {button} 0"),
                Some(EventType::KEY_UP),
            );
        }
        input("closed ABS_X", "abs ABS_X 20000", None);
        input("closed ABS_X back", "abs ABS_X 0", None);

        // Unplugged while it's open
        let pad = Gamepad::open(id).expect("the gamepad again");
        t("open again");
        settle(None, 100);
        trace_events("open again");
        ask("gamepad remove");
        settle(Some(EventType::GAMEPAD_REMOVED), 300);
        trace_events("removed");
        t(format!(
            "removed: connected={} rumble: {}",
            yes(pad.connected()),
            result(&pad.rumble(0x1000, 0x1000, 100))
        ));
        drop(pad);
        list("removed");

        // Plugged in and unplugged again
        plug("added again");
        ask("gamepad remove");
        settle(Some(EventType::GAMEPAD_REMOVED), 300);
        trace_events("removed again");

        dualsense();
        list("end");

        init::quit();
        t("quit");

        check_trace(&TRACE.lock().unwrap(), EXPECTED);
    }
}
