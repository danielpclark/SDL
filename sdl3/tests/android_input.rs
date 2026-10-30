// Tests of the android video driver's input: touches, the pinch, the
// mouse, its cursors and relative mode, and pens.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function makes a window and traces the touch devices, the mice and
//! the events that come from:
//!
//! * real input, through the runner's `input` requests (`adb shell
//!   input`), each more than the double click's time after the last: a
//!   tap and a swipe (whose moves are whether there were any, and the
//!   events from there on their names: their number, and so where the
//!   swipe ends, are a matter of timing), a tap with
//!   `SDL_HINT_TOUCH_MOUSE_EVENTS` off, the mouse's tap and wheel (`input
//!   mouse scroll`), its tap with `SDL_HINT_MOUSE_TOUCH_EVENTS` off and in
//!   the relative mode, and the stylus's tap;
//! * what `input` can't make (it has one pointer, the mouse's primary
//!   button or none, and no pressure, hovering, eraser or captured
//!   pointer), made in the program: `MotionEvent.obtain()`'s events,
//!   dispatched to the focused view, the surface, on SDL's thread
//!   (`dispatchTouchEvent()`, `dispatchGenericMotionEvent()` for the
//!   hovering and the wheel, `dispatchCapturedPointerEvent()`: an app
//!   can't inject events into its window without the `INJECT_EVENTS`
//!   permission): two fingers (down, moved, up one after the
//!   other), a pinch (the surface's `ScaleGestureDetector`), a cancelled
//!   touch, a pressure above 1, the mouse hovering, its five buttons and
//!   its wheel, the captured pointer's motion and buttons (relative mode),
//!   and the stylus hovering, pressing with its pressure and its primary
//!   button, and its eraser;
//! * the cursors: a color cursor, hidden and shown, the default one and
//!   the 34 system cursors, each traced by the surface's pointer icon
//!   (`View.getPointerIcon()`, what `setPointerIcon()` got);
//! * the relative mode on and off: the pointer captured
//!   (`View.hasPointerCapture()`), and its cursor (hidden: an empty custom
//!   cursor).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, run by the
//! runner's `--run-apk`) on the same emulator (API 35, Google APIs, x86-64,
//! a Pixel 6's screen, cold booted). Elsewhere this program does nothing.

#[cfg(not(target_os = "android"))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(target_os = "android")]
#[used]
#[link_section = ".sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

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

    use sdl3::android;
    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, mouse, touch, Event, EventType, WindowID};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::{self, PixelFormat, Surface, Window};

    use super::android_support::jni::{self, JNIEnv, Jobject, Jvalue};
    use super::android_support::{check_trace, event_name, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
video driver: android
hints: touch mouse events=(null) mouse touch events=(null)
start: touch devices: -1 type=0 name=\"mouse_input\", -3 type=0 name=\"Virtual\", 3 type=0 name=\"virtio_input_multi_touch_11\", 4 type=0 name=\"virtio_input_multi_touch_10\", 5 type=0 name=\"virtio_input_multi_touch_9\", 6 type=0 name=\"virtio_input_multi_touch_8\", 7 type=0 name=\"virtio_input_multi_touch_7\", 8 type=0 name=\"virtio_input_multi_touch_6\", 9 type=0 name=\"virtio_input_multi_touch_5\", 10 type=0 name=\"virtio_input_multi_touch_4\", 11 type=0 name=\"virtio_input_multi_touch_3\", 12 type=0 name=\"virtio_input_multi_touch_2\", 13 type=0 name=\"virtio_input_multi_touch_1\"
start: mice=0 has mouse=no
create: ok
window: size=1080x2400
focused view: org.libsdl.app.SDLSurface
created: touch devices: -1 type=0 name=\"mouse_input\", -3 type=0 name=\"Virtual\", 3 type=0 name=\"virtio_input_multi_touch_11\", 4 type=0 name=\"virtio_input_multi_touch_10\", 5 type=0 name=\"virtio_input_multi_touch_9\", 6 type=0 name=\"virtio_input_multi_touch_8\", 7 type=0 name=\"virtio_input_multi_touch_7\", 8 type=0 name=\"virtio_input_multi_touch_6\", 9 type=0 name=\"virtio_input_multi_touch_5\", 10 type=0 name=\"virtio_input_multi_touch_4\", 11 type=0 name=\"virtio_input_multi_touch_3\", 12 type=0 name=\"virtio_input_multi_touch_2\", 13 type=0 name=\"virtio_input_multi_touch_1\"
created: mice=0 has mouse=no
created: pointer icon: PointerIcon{type=ARROW, hotspotX=0.0, hotspotY=0.0}
tap: event: MOUSE_MOTION(which=touch state=0x0 x=300.2780 y=600.2501 xrel=0.0000 yrel=0.0000 w=W)
tap: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=300.2780 y=600.2501 w=W)
tap: event: FINGER_DOWN(touch=-3 finger=1 x=0.2780 y=0.2501 dx=0.0000 dy=0.0000 p=1.0000 w=W)
tap: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=300.2780 y=600.2501 w=W)
tap: event: FINGER_UP(touch=-3 finger=1 x=0.2780 y=0.2501 dx=0.0000 dy=0.0000 p=0.0000 w=W)
swipe: event: MOUSE_MOTION(which=touch state=0x0 x=200.1853 y=800.3335 xrel=0.0000 yrel=0.0000 w=W)
swipe: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=200.1853 y=800.3335 w=W)
swipe: event: FINGER_DOWN(touch=-3 finger=1 x=0.1854 y=0.3335 dx=0.0000 dy=0.0000 p=1.0000 w=W)
swipe: event: MOUSE_BUTTON_UP
swipe: moves: yes
swipe: event: FINGER_UP
touches: touch devices: -1 type=0 name=\"mouse_input\", -3 type=0 name=\"Virtual\", 3 type=0 name=\"virtio_input_multi_touch_11\", 4 type=0 name=\"virtio_input_multi_touch_10\", 5 type=0 name=\"virtio_input_multi_touch_9\", 6 type=0 name=\"virtio_input_multi_touch_8\", 7 type=0 name=\"virtio_input_multi_touch_7\", 8 type=0 name=\"virtio_input_multi_touch_6\", 9 type=0 name=\"virtio_input_multi_touch_5\", 10 type=0 name=\"virtio_input_multi_touch_4\", 11 type=0 name=\"virtio_input_multi_touch_3\", 12 type=0 name=\"virtio_input_multi_touch_2\", 13 type=0 name=\"virtio_input_multi_touch_1\"
fingers down 1: event: MOUSE_MOTION(which=touch state=0x0 x=200.1853 y=1000.4168 xrel=0.0000 yrel=0.0000 w=W)
fingers down 1: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=200.1853 y=1000.4168 w=W)
fingers down 1: event: FINGER_DOWN(touch=-3 finger=1 x=0.1854 y=0.4168 dx=0.0000 dy=0.0000 p=1.0000 w=W)
fingers down 2: event: FINGER_DOWN(touch=-3 finger=2 x=0.7414 y=0.4168 dx=0.0000 dy=0.0000 p=1.0000 w=W)
fingers moved: event: MOUSE_MOTION(which=touch state=0x1 x=220.2039 y=1010.4210 xrel=20.0186 yrel=10.0042 w=W)
fingers moved: event: FINGER_MOTION(touch=-3 finger=1 x=0.2039 y=0.4210 dx=0.0185 dy=0.0042 p=1.0000 w=W)
fingers moved: event: FINGER_MOTION(touch=-3 finger=2 x=0.7229 y=0.4127 dx=-0.0185 dy=-0.0042 p=1.0000 w=W)
finger 1 up: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=220.2039 y=1010.4210 w=W)
finger 1 up: event: FINGER_UP(touch=-3 finger=1 x=0.2039 y=0.4210 dx=0.0000 dy=0.0000 p=1.0000 w=W)
finger 2 moved: event: FINGER_MOTION(touch=-3 finger=2 x=0.7044 y=0.4085 dx=-0.0185 dy=-0.0042 p=1.0000 w=W)
finger 2 up: event: FINGER_UP(touch=-3 finger=2 x=0.7044 y=0.4085 dx=0.0000 dy=0.0000 p=1.0000 w=W)
pinch down: event: MOUSE_MOTION(which=touch state=0x0 x=300.2780 y=1200.5002 xrel=0.0000 yrel=0.0000 w=W)
pinch down: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=300.2780 y=1200.5002 w=W)
pinch down: event: FINGER_DOWN(touch=-3 finger=1 x=0.2780 y=0.5002 dx=0.0000 dy=0.0000 p=1.0000 w=W)
pinch down: event: FINGER_DOWN(touch=-3 finger=2 x=0.7229 y=0.5002 dx=0.0000 dy=0.0000 p=1.0000 w=W)
pinch move 1: event: MOUSE_MOTION(which=touch state=0x1 x=250.2317 y=1200.5002 xrel=-50.0464 yrel=0.0000 w=W)
pinch move 1: event: FINGER_MOTION(touch=-3 finger=1 x=0.2317 y=0.5002 dx=-0.0463 dy=0.0000 p=1.0000 w=W)
pinch move 1: event: FINGER_MOTION(touch=-3 finger=2 x=0.7692 y=0.5002 dx=0.0463 dy=0.0000 p=1.0000 w=W)
pinch move 1: event: PINCH_BEGIN(scale=0.0000 span=580.5375,0.0000 focus=540.5004,1200.5002 w=W)
pinch move 1: event: PINCH_UPDATE(scale=1.0000 span=580.5375,0.0000 focus=540.5004,1200.5002 w=W)
pinch move 2: event: MOUSE_MOTION(which=touch state=0x1 x=200.1853 y=1200.5002 xrel=-50.0463 yrel=0.0000 w=W)
pinch move 2: event: FINGER_MOTION(touch=-3 finger=1 x=0.1854 y=0.5002 dx=-0.0463 dy=0.0000 p=1.0000 w=W)
pinch move 2: event: FINGER_MOTION(touch=-3 finger=2 x=0.8156 y=0.5002 dx=0.0463 dy=0.0000 p=1.0000 w=W)
pinch move 2: event: PINCH_UPDATE(scale=1.1724 span=680.6302,0.0000 focus=540.5004,1200.5002 w=W)
pinch move 3: event: MOUSE_MOTION(which=touch state=0x1 x=150.1390 y=1210.5044 xrel=-50.0463 yrel=10.0042 w=W)
pinch move 3: event: FINGER_MOTION(touch=-3 finger=1 x=0.1390 y=0.5044 dx=-0.0463 dy=0.0042 p=1.0000 w=W)
pinch move 3: event: FINGER_MOTION(touch=-3 finger=2 x=0.8619 y=0.4960 dx=0.0463 dy=-0.0042 p=1.0000 w=W)
pinch move 3: event: PINCH_UPDATE(scale=1.1474 span=780.7229,20.0083 focus=540.5004,1200.5002 w=W)
pinch up 2: event: FINGER_UP(touch=-3 finger=2 x=0.8619 y=0.4960 dx=0.0000 dy=0.0000 p=1.0000 w=W)
pinch up 2: event: PINCH_END(scale=0.0000 span=-1.0000,-1.0000 focus=-1.0000,-1.0000 w=W)
pinch up 1: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=150.1390 y=1210.5044 w=W)
pinch up 1: event: FINGER_UP(touch=-3 finger=1 x=0.1390 y=0.5044 dx=0.0000 dy=0.0000 p=1.0000 w=W)
cancel: event: MOUSE_MOTION(which=touch state=0x0 x=500.4634 y=500.2084 xrel=0.0000 yrel=0.0000 w=W)
cancel: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=500.4634 y=500.2084 w=W)
cancel: event: FINGER_DOWN(touch=-3 finger=1 x=0.4634 y=0.2084 dx=0.0000 dy=0.0000 p=0.5000 w=W)
cancel: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=500.4634 y=500.2084 w=W)
cancel: event: FINGER_CANCELED(touch=-3 finger=1 x=0.4634 y=0.2084 dx=0.0000 dy=0.0000 p=0.5000 w=W)
pressure: event: MOUSE_MOTION(which=touch state=0x0 x=100.0927 y=100.0417 xrel=0.0000 yrel=0.0000 w=W)
pressure: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=100.0927 y=100.0417 w=W)
pressure: event: FINGER_DOWN(touch=-3 finger=1 x=0.0927 y=0.0417 dx=0.0000 dy=0.0000 p=1.0000 w=W)
pressure: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=100.0927 y=100.0417 w=W)
pressure: event: FINGER_UP(touch=-3 finger=1 x=0.0927 y=0.0417 dx=0.0000 dy=0.0000 p=1.0000 w=W)
tap, touch mouse events off: event: FINGER_DOWN(touch=-3 finger=1 x=0.2780 y=0.2501 dx=0.0000 dy=0.0000 p=1.0000 w=W)
tap, touch mouse events off: event: FINGER_UP(touch=-3 finger=1 x=0.2780 y=0.2501 dx=0.0000 dy=0.0000 p=0.0000 w=W)
touch mouse events: touch devices: -1 type=0 name=\"mouse_input\", -3 type=0 name=\"Virtual\", 3 type=0 name=\"virtio_input_multi_touch_11\", 4 type=0 name=\"virtio_input_multi_touch_10\", 5 type=0 name=\"virtio_input_multi_touch_9\", 6 type=0 name=\"virtio_input_multi_touch_8\", 7 type=0 name=\"virtio_input_multi_touch_7\", 8 type=0 name=\"virtio_input_multi_touch_6\", 9 type=0 name=\"virtio_input_multi_touch_5\", 10 type=0 name=\"virtio_input_multi_touch_4\", 11 type=0 name=\"virtio_input_multi_touch_3\", 12 type=0 name=\"virtio_input_multi_touch_2\", 13 type=0 name=\"virtio_input_multi_touch_1\"
mouse tap: event: MOUSE_MOTION(which=0 state=0x0 x=400.0000 y=500.0000 xrel=299.9073 yrel=399.9583 w=W)
mouse tap: event: MOUSE_BUTTON_DOWN(which=0 button=0 clicks=1 x=400.0000 y=500.0000 w=W)
mouse tap: event: MOUSE_BUTTON_UP(which=0 button=0 clicks=1 x=400.0000 y=500.0000 w=W)
mouse tap: mice=0 has mouse=no
mouse scroll: event: MOUSE_WHEEL(which=0 x=-1.0000 y=2.0000 direction=0 mouse=400.0000,500.0000 integer=-1,2 w=W)
mouse touch events off: touch devices: 13 type=0 name=\"virtio_input_multi_touch_1\", -3 type=0 name=\"Virtual\", 3 type=0 name=\"virtio_input_multi_touch_11\", 4 type=0 name=\"virtio_input_multi_touch_10\", 5 type=0 name=\"virtio_input_multi_touch_9\", 6 type=0 name=\"virtio_input_multi_touch_8\", 7 type=0 name=\"virtio_input_multi_touch_7\", 8 type=0 name=\"virtio_input_multi_touch_6\", 9 type=0 name=\"virtio_input_multi_touch_5\", 10 type=0 name=\"virtio_input_multi_touch_4\", 11 type=0 name=\"virtio_input_multi_touch_3\", 12 type=0 name=\"virtio_input_multi_touch_2\"
mouse tap, mouse touch events off: event: MOUSE_BUTTON_DOWN(which=0 button=0 clicks=1 x=400.0000 y=500.0000 w=W)
mouse tap, mouse touch events off: event: MOUSE_BUTTON_UP(which=0 button=0 clicks=1 x=400.0000 y=500.0000 w=W)
mouse touch events reset: touch devices: 13 type=0 name=\"virtio_input_multi_touch_1\", -3 type=0 name=\"Virtual\", 3 type=0 name=\"virtio_input_multi_touch_11\", 4 type=0 name=\"virtio_input_multi_touch_10\", 5 type=0 name=\"virtio_input_multi_touch_9\", 6 type=0 name=\"virtio_input_multi_touch_8\", 7 type=0 name=\"virtio_input_multi_touch_7\", 8 type=0 name=\"virtio_input_multi_touch_6\", 9 type=0 name=\"virtio_input_multi_touch_5\", 10 type=0 name=\"virtio_input_multi_touch_4\", 11 type=0 name=\"virtio_input_multi_touch_3\", 12 type=0 name=\"virtio_input_multi_touch_2\", -1 type=0 name=\"mouse_input\"
mouse hover: event: MOUSE_MOTION(which=0 state=0x0 x=520.0000 y=720.0000 xrel=120.0000 yrel=220.0000 w=W)
mouse hover: event: MOUSE_MOTION(which=0 state=0x0 x=540.0000 y=740.0000 xrel=20.0000 yrel=20.0000 w=W)
mouse button 0x1: event: MOUSE_MOTION(which=0 state=0x0 x=600.0000 y=800.0000 xrel=60.0000 yrel=60.0000 w=W)
mouse button 0x1: event: FINGER_DOWN(touch=-1 finger=1 x=0.5556 y=0.3333 dx=0.0000 dy=0.0000 p=1.0000 w=W)
mouse button 0x1: event: MOUSE_BUTTON_DOWN(which=0 button=1 clicks=1 x=600.0000 y=800.0000 w=W)
mouse button 0x1: event: FINGER_MOTION(touch=-1 finger=1 x=0.5648 y=0.3375 dx=0.0093 dy=0.0042 p=1.0000 w=W)
mouse button 0x1: event: MOUSE_MOTION(which=0 state=0x1 x=610.0000 y=810.0000 xrel=10.0000 yrel=10.0000 w=W)
mouse button 0x1: event: FINGER_UP(touch=-1 finger=1 x=0.5648 y=0.3375 dx=0.0000 dy=0.0000 p=1.0000 w=W)
mouse button 0x1: event: MOUSE_BUTTON_UP(which=0 button=1 clicks=1 x=610.0000 y=810.0000 w=W)
mouse button 0x2: event: MOUSE_MOTION(which=0 state=0x0 x=600.0000 y=800.0000 xrel=-10.0000 yrel=-10.0000 w=W)
mouse button 0x2: event: MOUSE_BUTTON_DOWN(which=0 button=3 clicks=1 x=600.0000 y=800.0000 w=W)
mouse button 0x2: event: MOUSE_MOTION(which=0 state=0x4 x=610.0000 y=810.0000 xrel=10.0000 yrel=10.0000 w=W)
mouse button 0x2: event: MOUSE_BUTTON_UP(which=0 button=3 clicks=1 x=610.0000 y=810.0000 w=W)
mouse button 0x4: event: MOUSE_MOTION(which=0 state=0x0 x=600.0000 y=800.0000 xrel=-10.0000 yrel=-10.0000 w=W)
mouse button 0x4: event: MOUSE_BUTTON_DOWN(which=0 button=2 clicks=1 x=600.0000 y=800.0000 w=W)
mouse button 0x4: event: MOUSE_MOTION(which=0 state=0x2 x=610.0000 y=810.0000 xrel=10.0000 yrel=10.0000 w=W)
mouse button 0x4: event: MOUSE_BUTTON_UP(which=0 button=2 clicks=1 x=610.0000 y=810.0000 w=W)
mouse button 0x8: event: MOUSE_MOTION(which=0 state=0x0 x=600.0000 y=800.0000 xrel=-10.0000 yrel=-10.0000 w=W)
mouse button 0x8: event: MOUSE_BUTTON_DOWN(which=0 button=5 clicks=1 x=600.0000 y=800.0000 w=W)
mouse button 0x8: event: MOUSE_MOTION(which=0 state=0x10 x=610.0000 y=810.0000 xrel=10.0000 yrel=10.0000 w=W)
mouse button 0x8: event: MOUSE_BUTTON_UP(which=0 button=5 clicks=1 x=610.0000 y=810.0000 w=W)
mouse button 0x10: event: MOUSE_MOTION(which=0 state=0x0 x=600.0000 y=800.0000 xrel=-10.0000 yrel=-10.0000 w=W)
mouse button 0x10: event: MOUSE_BUTTON_DOWN(which=0 button=4 clicks=1 x=600.0000 y=800.0000 w=W)
mouse button 0x10: event: MOUSE_MOTION(which=0 state=0x8 x=610.0000 y=810.0000 xrel=10.0000 yrel=10.0000 w=W)
mouse button 0x10: event: MOUSE_BUTTON_UP(which=0 button=4 clicks=1 x=610.0000 y=810.0000 w=W)
mouse wheel: event: MOUSE_WHEEL(which=0 x=-0.5000 y=1.5000 direction=0 mouse=610.0000,810.0000 integer=0,1 w=W)
color cursor: ok
set color cursor: ok
color cursor: pointer icon: PointerIcon{type=CUSTOM, hotspotX=3.0, hotspotY=4.0}
hide cursor: ok
hidden: pointer icon: PointerIcon{type=CUSTOM, hotspotX=0.0, hotspotY=0.0}
show cursor: ok
shown: pointer icon: PointerIcon{type=CUSTOM, hotspotX=3.0, hotspotY=4.0}
default cursor: ok
default: pointer icon: PointerIcon{type=ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 0: ok PointerIcon{type=ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 1: ok PointerIcon{type=TEXT, hotspotX=0.0, hotspotY=0.0}
system cursor 2: ok PointerIcon{type=WAIT, hotspotX=0.0, hotspotY=0.0}
system cursor 3: ok PointerIcon{type=CROSSHAIR, hotspotX=0.0, hotspotY=0.0}
system cursor 4: ok PointerIcon{type=WAIT, hotspotX=0.0, hotspotY=0.0}
system cursor 5: ok PointerIcon{type=TOP_LEFT_DIAGONAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 6: ok PointerIcon{type=TOP_RIGHT_DIAGONAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 7: ok PointerIcon{type=HORIZONTAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 8: ok PointerIcon{type=VERTICAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 9: ok PointerIcon{type=GRAB, hotspotX=0.0, hotspotY=0.0}
system cursor 10: ok PointerIcon{type=NO_DROP, hotspotX=0.0, hotspotY=0.0}
system cursor 11: ok PointerIcon{type=HAND, hotspotX=0.0, hotspotY=0.0}
system cursor 12: ok PointerIcon{type=TOP_LEFT_DIAGONAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 13: ok PointerIcon{type=VERTICAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 14: ok PointerIcon{type=TOP_RIGHT_DIAGONAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 15: ok PointerIcon{type=HORIZONTAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 16: ok PointerIcon{type=TOP_LEFT_DIAGONAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 17: ok PointerIcon{type=VERTICAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 18: ok PointerIcon{type=TOP_RIGHT_DIAGONAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 19: ok PointerIcon{type=HORIZONTAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 20: ok PointerIcon{type=CONTEXT_MENU, hotspotX=0.0, hotspotY=0.0}
system cursor 21: ok PointerIcon{type=HELP, hotspotX=0.0, hotspotY=0.0}
system cursor 22: ok PointerIcon{type=CELL, hotspotX=0.0, hotspotY=0.0}
system cursor 23: ok PointerIcon{type=VERTICAL_TEXT, hotspotX=0.0, hotspotY=0.0}
system cursor 24: ok PointerIcon{type=ALIAS, hotspotX=0.0, hotspotY=0.0}
system cursor 25: ok PointerIcon{type=COPY, hotspotX=0.0, hotspotY=0.0}
system cursor 26: ok PointerIcon{type=NO_DROP, hotspotX=0.0, hotspotY=0.0}
system cursor 27: ok PointerIcon{type=GRAB, hotspotX=0.0, hotspotY=0.0}
system cursor 28: ok PointerIcon{type=GRABBING, hotspotX=0.0, hotspotY=0.0}
system cursor 29: ok PointerIcon{type=HORIZONTAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 30: ok PointerIcon{type=VERTICAL_DOUBLE_ARROW, hotspotX=0.0, hotspotY=0.0}
system cursor 31: ok PointerIcon{type=ALL_SCROLL, hotspotX=0.0, hotspotY=0.0}
system cursor 32: ok PointerIcon{type=ZOOM_IN, hotspotX=0.0, hotspotY=0.0}
system cursor 33: ok PointerIcon{type=ZOOM_OUT, hotspotX=0.0, hotspotY=0.0}
system cursors: pointer icon: PointerIcon{type=ARROW, hotspotX=0.0, hotspotY=0.0}
relative mode on: ok
relative mode on: events: (none)
relative mode on: enabled=yes pointer capture=yes
relative mode on: pointer icon: PointerIcon{type=CUSTOM, hotspotX=0.0, hotspotY=0.0}
captured pointer: event: MOUSE_MOTION(which=1 state=0x0 x=622.0000 y=803.0000 xrel=12.0000 yrel=-7.0000 w=W)
captured pointer: event: MOUSE_MOTION(which=1 state=0x0 x=619.0000 y=808.0000 xrel=-3.0000 yrel=5.0000 w=W)
mouse tap in relative mode: event: MOUSE_MOTION(which=1 state=0x0 x=1029.0000 y=1318.0000 xrel=410.0000 yrel=510.0000 w=W)
mouse tap in relative mode: event: MOUSE_BUTTON_DOWN(which=1 button=0 clicks=1 x=1029.0000 y=1318.0000 w=W)
mouse tap in relative mode: event: MOUSE_MOTION(which=1 state=0x80000000 x=1079.0000 y=1828.0000 xrel=410.0000 yrel=510.0000 w=W)
mouse tap in relative mode: event: MOUSE_BUTTON_UP(which=1 button=0 clicks=1 x=1079.0000 y=1828.0000 w=W)
relative mode off: ok
relative mode off: events: (none)
relative mode off: enabled=no pointer capture=no
relative mode off: pointer icon: PointerIcon{type=ARROW, hotspotX=0.0, hotspotY=0.0}
stylus tap: event: PEN_PROXIMITY_IN(which=p1 w=W)
stylus tap: event: PEN_MOTION(which=p1 state=0x80000000 x=600.0000 y=900.0000 w=W)
stylus tap: event: MOUSE_MOTION(which=pen state=0x0 x=600.0000 y=900.0000 xrel=-479.0000 yrel=-928.0000 w=W)
stylus tap: event: PEN_AXIS(which=p1 state=0x80000000 x=600.0000 y=900.0000 axis=0 value=1.0000 w=W)
stylus tap: event: PEN_DOWN(which=p1 state=0x80000001 x=600.0000 y=900.0000 eraser=0 w=W)
stylus tap: event: MOUSE_BUTTON_DOWN(which=pen button=1 clicks=1 x=600.0000 y=900.0000 w=W)
stylus tap: event: FINGER_DOWN(touch=-2 finger=1 x=0.5556 y=0.3750 dx=0.0000 dy=0.0000 p=1.0000 w=W)
stylus tap: event: PEN_AXIS(which=p1 state=0x80000001 x=600.0000 y=900.0000 axis=0 value=0.0000 w=W)
stylus tap: event: FINGER_MOTION(touch=-2 finger=1 x=0.5556 y=0.3750 dx=0.0000 dy=0.0000 p=0.0000 w=W)
stylus tap: event: PEN_UP(which=p1 state=0x80000000 x=600.0000 y=900.0000 eraser=0 w=W)
stylus tap: event: MOUSE_BUTTON_UP(which=pen button=1 clicks=1 x=600.0000 y=900.0000 w=W)
stylus tap: event: FINGER_UP(touch=-2 finger=1 x=0.5556 y=0.3750 dx=0.0000 dy=0.0000 p=0.0000 w=W)
stylus hover: event: PEN_MOTION(which=p1 state=0x80000000 x=300.0000 y=400.0000 w=W)
stylus hover: event: MOUSE_MOTION(which=pen state=0x0 x=300.0000 y=400.0000 xrel=-300.0000 yrel=-500.0000 w=W)
stylus hover: event: PEN_MOTION(which=p1 state=0x80000000 x=310.0000 y=410.0000 w=W)
stylus hover: event: MOUSE_MOTION(which=pen state=0x0 x=310.0000 y=410.0000 xrel=10.0000 yrel=10.0000 w=W)
stylus hover: event: PEN_PROXIMITY_OUT(which=p1 w=W)
stylus down: event: PEN_PROXIMITY_IN(which=p1 w=W)
stylus down: event: PEN_MOTION(which=p1 state=0x80000000 x=320.0000 y=420.0000 w=W)
stylus down: event: MOUSE_MOTION(which=pen state=0x0 x=320.0000 y=420.0000 xrel=10.0000 yrel=10.0000 w=W)
stylus down: event: PEN_AXIS(which=p1 state=0x80000000 x=320.0000 y=420.0000 axis=0 value=0.2500 w=W)
stylus down: event: PEN_DOWN(which=p1 state=0x80000001 x=320.0000 y=420.0000 eraser=0 w=W)
stylus down: event: MOUSE_BUTTON_DOWN(which=pen button=1 clicks=1 x=320.0000 y=420.0000 w=W)
stylus down: event: FINGER_DOWN(touch=-2 finger=1 x=0.2963 y=0.1750 dx=0.0000 dy=0.0000 p=0.2500 w=W)
stylus button: event: PEN_MOTION(which=p1 state=0x80000001 x=330.0000 y=430.0000 w=W)
stylus button: event: MOUSE_MOTION(which=pen state=0x1 x=330.0000 y=430.0000 xrel=10.0000 yrel=10.0000 w=W)
stylus button: event: FINGER_MOTION(touch=-2 finger=1 x=0.3056 y=0.1792 dx=0.0093 dy=0.0042 p=0.2500 w=W)
stylus button: event: PEN_AXIS(which=p1 state=0x80000001 x=330.0000 y=430.0000 axis=0 value=0.6000 w=W)
stylus button: event: FINGER_MOTION(touch=-2 finger=1 x=0.3056 y=0.1792 dx=0.0000 dy=0.0000 p=0.6000 w=W)
stylus button: event: PEN_BUTTON_DOWN(which=p1 state=0x80000003 x=330.0000 y=430.0000 button=1 w=W)
stylus button: event: MOUSE_BUTTON_DOWN(which=pen button=3 clicks=1 x=330.0000 y=430.0000 w=W)
stylus moved: event: PEN_MOTION(which=p1 state=0x80000003 x=340.0000 y=440.0000 w=W)
stylus moved: event: MOUSE_MOTION(which=pen state=0x5 x=340.0000 y=440.0000 xrel=10.0000 yrel=10.0000 w=W)
stylus moved: event: FINGER_MOTION(touch=-2 finger=1 x=0.3148 y=0.1833 dx=0.0093 dy=0.0042 p=0.6000 w=W)
stylus moved: event: PEN_AXIS(which=p1 state=0x80000003 x=340.0000 y=440.0000 axis=0 value=1.0000 w=W)
stylus moved: event: FINGER_MOTION(touch=-2 finger=1 x=0.3148 y=0.1833 dx=0.0000 dy=0.0000 p=1.0000 w=W)
stylus moved: event: PEN_BUTTON_UP(which=p1 state=0x80000001 x=340.0000 y=440.0000 button=1 w=W)
stylus moved: event: MOUSE_BUTTON_UP(which=pen button=3 clicks=1 x=340.0000 y=440.0000 w=W)
stylus up: event: PEN_AXIS(which=p1 state=0x80000001 x=340.0000 y=440.0000 axis=0 value=0.0000 w=W)
stylus up: event: FINGER_MOTION(touch=-2 finger=1 x=0.3148 y=0.1833 dx=0.0000 dy=0.0000 p=0.0000 w=W)
stylus up: event: PEN_UP(which=p1 state=0x80000000 x=340.0000 y=440.0000 eraser=0 w=W)
stylus up: event: MOUSE_BUTTON_UP(which=pen button=1 clicks=1 x=340.0000 y=440.0000 w=W)
stylus up: event: FINGER_UP(touch=-2 finger=1 x=0.3148 y=0.1833 dx=0.0000 dy=0.0000 p=0.0000 w=W)
eraser: event: PEN_MOTION(which=p1 state=0x80000000 x=400.0000 y=500.0000 w=W)
eraser: event: MOUSE_MOTION(which=pen state=0x0 x=400.0000 y=500.0000 xrel=60.0000 yrel=60.0000 w=W)
eraser: event: PEN_AXIS(which=p1 state=0x80000000 x=400.0000 y=500.0000 axis=0 value=0.5000 w=W)
eraser: event: PEN_DOWN(which=p1 state=0xc0000001 x=400.0000 y=500.0000 eraser=1 w=W)
eraser: event: MOUSE_BUTTON_DOWN(which=pen button=1 clicks=1 x=400.0000 y=500.0000 w=W)
eraser: event: FINGER_DOWN(touch=-2 finger=1 x=0.3704 y=0.2083 dx=0.0000 dy=0.0000 p=0.5000 w=W)
eraser: event: PEN_AXIS(which=p1 state=0xc0000001 x=400.0000 y=500.0000 axis=0 value=0.0000 w=W)
eraser: event: FINGER_MOTION(touch=-2 finger=1 x=0.3704 y=0.2083 dx=0.0000 dy=0.0000 p=0.0000 w=W)
eraser: event: PEN_UP(which=p1 state=0xc0000000 x=400.0000 y=500.0000 eraser=1 w=W)
eraser: event: MOUSE_BUTTON_UP(which=pen button=1 clicks=1 x=400.0000 y=500.0000 w=W)
eraser: event: FINGER_UP(touch=-2 finger=1 x=0.3704 y=0.2083 dx=0.0000 dy=0.0000 p=0.0000 w=W)
end: touch devices: 13 type=0 name=\"virtio_input_multi_touch_1\", -3 type=0 name=\"Virtual\", 3 type=0 name=\"virtio_input_multi_touch_11\", 4 type=0 name=\"virtio_input_multi_touch_10\", 5 type=0 name=\"virtio_input_multi_touch_9\", 6 type=0 name=\"virtio_input_multi_touch_8\", 7 type=0 name=\"virtio_input_multi_touch_7\", 8 type=0 name=\"virtio_input_multi_touch_6\", 9 type=0 name=\"virtio_input_multi_touch_5\", 10 type=0 name=\"virtio_input_multi_touch_4\", 11 type=0 name=\"virtio_input_multi_touch_3\", 12 type=0 name=\"virtio_input_multi_touch_2\", -1 type=0 name=\"mouse_input\", -2 type=0 name=\"pen_input\"
end: mice=0 has mouse=no
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

    // JNI: the activity, MotionEvent.obtain(), the focused view's dispatch

    struct Jni {
        env: JNIEnv,
        activity: Jobject,
        motion_event: Jobject,
        props: Jobject,
        coords: Jobject,
        system_clock: Jobject,
    }

    // SAFETY: the program's one thread (SDL's) uses it.
    unsafe impl Send for Jni {}

    static JNI: Mutex<Option<Jni>> = Mutex::new(None);

    fn with_jni<R>(f: impl FnOnce(&Jni) -> R) -> R {
        f(JNI.lock().unwrap().as_ref().expect("the JNI state"))
    }

    fn global_class(env: JNIEnv, name: &str) -> Jobject {
        let class = jni::find_class(env, name);
        let global = jni::new_global_ref(env, class);
        jni::delete_local_ref(env, class);
        global
    }

    fn to_string(env: JNIEnv, obj: Jobject) -> String {
        if obj.is_null() {
            return "null".into();
        }
        let s = jni::call_object(env, obj, "toString", "()Ljava/lang/String;", &[]);
        let text = jni::string(env, s);
        jni::delete_local_ref(env, s);
        text
    }

    /// The activity's focused view (the surface).
    fn focus_view(j: &Jni) -> Jobject {
        jni::call_object(
            j.env,
            j.activity,
            "getCurrentFocus",
            "()Landroid/view/View;",
            &[],
        )
    }

    /// The focused view's pointer icon, as a string.
    fn pointer_icon() -> String {
        with_jni(|j| {
            let v = focus_view(j);
            if v.is_null() {
                return "(no focused view)".into();
            }
            let icon = jni::call_object(
                j.env,
                v,
                "getPointerIcon",
                "()Landroid/view/PointerIcon;",
                &[],
            );
            let text = to_string(j.env, icon);
            if !icon.is_null() {
                jni::delete_local_ref(j.env, icon);
            }
            jni::delete_local_ref(j.env, v);
            text
        })
    }

    fn has_pointer_capture() -> bool {
        with_jni(|j| {
            let v = focus_view(j);
            if v.is_null() {
                return false;
            }
            let r = jni::call_boolean(j.env, v, "hasPointerCapture", "()Z", &[]);
            jni::delete_local_ref(j.env, v);
            r
        })
    }

    /// A stand-in gesture's times: each event 50 ms after the last, from 2
    /// s ago (InputDispatcher drops events older than 10 s; the steps are
    /// longer than InputConsumer's resampling delta, so no move is
    /// resampled).
    static GESTURE: Mutex<(i64, i64)> = Mutex::new((0, 0));

    fn gesture() {
        let now =
            with_jni(|j| jni::call_static_long(j.env, j.system_clock, "uptimeMillis", "()J", &[]));
        *GESTURE.lock().unwrap() = (now - 2000, now - 2000);
    }

    #[derive(Clone, Copy, Default)]
    struct Pointer {
        id: i32,
        tool: i32,
        x: f32,
        y: f32,
        p: f32,
        hscroll: f32,
        vscroll: f32,
        relx: f32,
        rely: f32,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Via {
        Pointer,
        Trackball,
    }

    /// `MotionEvent.obtain(...)` of the pointers, dispatched to the focused
    /// view (`dispatchCapturedPointerEvent()` `Via::Trackball`,
    /// `dispatchGenericMotionEvent()` for the hovering and the wheel, else
    /// `dispatchTouchEvent()`) and recycled.
    fn inject(action: i32, ps: &[Pointer], buttons: i32, source: i32, via: Via) {
        with_jni(|j| {
            let env = j.env;
            let n = ps.len() as i32;
            let props = jni::new_object_array(env, n, j.props);
            let coords = jni::new_object_array(env, n, j.coords);
            for (i, pt) in ps.iter().enumerate() {
                let pr = jni::new_object(env, j.props, "()V", &[]);
                jni::set_int_field(env, j.props, pr, "id", pt.id);
                jni::set_int_field(env, j.props, pr, "toolType", pt.tool);
                jni::set_object_array_element(env, props, i as i32, pr);
                jni::delete_local_ref(env, pr);
                let co = jni::new_object(env, j.coords, "()V", &[]);
                jni::set_float_field(env, j.coords, co, "x", pt.x);
                jni::set_float_field(env, j.coords, co, "y", pt.y);
                jni::set_float_field(env, j.coords, co, "pressure", pt.p);
                for (axis, value) in [
                    (10, pt.hscroll), // AXIS_HSCROLL
                    (9, pt.vscroll),  // AXIS_VSCROLL
                    (27, pt.relx),    // AXIS_RELATIVE_X
                    (28, pt.rely),    // AXIS_RELATIVE_Y
                ] {
                    if value != 0.0 {
                        jni::call_void(
                            env,
                            co,
                            "setAxisValue",
                            "(IF)V",
                            &[Jvalue { i: axis }, Jvalue { f: value }],
                        );
                    }
                }
                jni::set_object_array_element(env, coords, i as i32, co);
                jni::delete_local_ref(env, co);
            }
            let (down, time) = *GESTURE.lock().unwrap();
            let ev = jni::call_static_object(
                env,
                j.motion_event,
                "obtain",
                "(JJII[Landroid/view/MotionEvent$PointerProperties;[Landroid/view/MotionEvent$PointerCoords;IIFFIIII)Landroid/view/MotionEvent;",
                &[
                    Jvalue { j: down },
                    Jvalue { j: time },
                    Jvalue { i: action },
                    Jvalue { i: n },
                    Jvalue { l: props },
                    Jvalue { l: coords },
                    Jvalue { i: 0 }, // metaState
                    Jvalue { i: buttons },
                    Jvalue { f: 1.0 },
                    Jvalue { f: 1.0 },
                    Jvalue { i: -1 }, // deviceId
                    Jvalue { i: 0 },  // edgeFlags
                    Jvalue { i: source },
                    Jvalue { i: 0 }, // flags
                ],
            );
            GESTURE.lock().unwrap().1 += 50;
            let masked = action & 0xff;
            let dispatch = if via == Via::Trackball {
                "dispatchCapturedPointerEvent"
            } else if matches!(masked, HOVER_MOVE | SCROLL | HOVER_ENTER | HOVER_EXIT) {
                "dispatchGenericMotionEvent"
            } else {
                "dispatchTouchEvent"
            };
            let v = focus_view(j);
            let thrown = jni::call_boolean_catching(
                env,
                v,
                dispatch,
                "(Landroid/view/MotionEvent;)Z",
                &[Jvalue { l: ev }],
            );
            jni::delete_local_ref(env, v);
            if let Err(e) = thrown {
                let text = to_string(env, e);
                t(format!("inject: {text}"));
                jni::delete_local_ref(env, e);
            }
            jni::call_void(env, ev, "recycle", "()V", &[]);
            jni::delete_local_ref(env, ev);
            jni::delete_local_ref(env, props);
            jni::delete_local_ref(env, coords);
        });
    }

    // MotionEvent's constants
    const DOWN: i32 = 0;
    const UP: i32 = 1;
    const MOVE: i32 = 2;
    const CANCEL: i32 = 3;
    const fn pointer_down(i: i32) -> i32 {
        5 | (i << 8)
    }
    const fn pointer_up(i: i32) -> i32 {
        6 | (i << 8)
    }
    const HOVER_MOVE: i32 = 7;
    const SCROLL: i32 = 8;
    const HOVER_ENTER: i32 = 9;
    const HOVER_EXIT: i32 = 10;
    const FINGER: i32 = 1;
    const STYLUS: i32 = 2;
    const MOUSE: i32 = 3;
    const ERASER: i32 = 4;
    const SRC_TOUCHSCREEN: i32 = 0x1002;
    const SRC_MOUSE: i32 = 0x2002;
    const SRC_STYLUS: i32 = 0x4002;
    const SRC_MOUSE_RELATIVE: i32 = 0x20004;

    /// One pointer's event.
    #[allow(clippy::too_many_arguments)] // (the C's)
    fn inject1(action: i32, id: i32, tool: i32, x: f32, y: f32, p: f32, buttons: i32, source: i32) {
        let pt = Pointer {
            id,
            tool,
            x,
            y,
            p,
            ..Pointer::default()
        };
        inject(action, &[pt], buttons, source, Via::Pointer);
    }

    /// Two fingers' event.
    fn inject2(action: i32, x0: f32, y0: f32, x1: f32, y1: f32) {
        let pts = [
            Pointer {
                id: 0,
                tool: FINGER,
                x: x0,
                y: y0,
                p: 1.0,
                ..Pointer::default()
            },
            Pointer {
                id: 1,
                tool: FINGER,
                x: x1,
                y: y1,
                p: 1.0,
                ..Pointer::default()
            },
        ];
        inject(action, &pts, 0, SRC_TOUCHSCREEN, Via::Pointer);
    }

    // Events

    static LINES: Mutex<Vec<(EventType, String)>> = Mutex::new(Vec::new());
    static PENS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    static WINDOW: Mutex<WindowID> = Mutex::new(0);

    fn pen_label(id: u32) -> usize {
        let mut pens = PENS.lock().unwrap();
        match pens.iter().position(|&p| p == id) {
            Some(i) => i + 1,
            None => {
                pens.push(id);
                pens.len()
            }
        }
    }

    fn mouse_label(id: mouse::MouseID) -> String {
        match id {
            mouse::TOUCH_MOUSE_ID => "touch".into(),
            mouse::PEN_MOUSE_ID => "pen".into(),
            _ => format!("{id}"),
        }
    }

    fn wl(id: WindowID) -> &'static str {
        if id == *WINDOW.lock().unwrap() {
            "W"
        } else if id == 0 {
            "0"
        } else {
            "?"
        }
    }

    /// An event as the trace names it (the C program's names).
    fn describe(event: &Event) -> String {
        let n = event_name(event.event_type());
        match event {
            Event::MouseDevice(e) => format!("{n}(which={})", mouse_label(e.which)),
            Event::TouchFinger(e) => format!(
                "{n}(touch={} finger={} x={:.4} y={:.4} dx={:.4} dy={:.4} p={:.4} w={})",
                e.touch_id as i64,
                e.finger_id,
                e.x,
                e.y,
                e.dx,
                e.dy,
                e.pressure,
                wl(e.window_id)
            ),
            Event::Pinch(e) => format!(
                "{n}(scale={:.4} span={:.4},{:.4} focus={:.4},{:.4} w={})",
                e.scale,
                e.span_x,
                e.span_y,
                e.focus_x,
                e.focus_y,
                wl(e.window_id)
            ),
            Event::MouseMotion(e) => format!(
                "{n}(which={} state={:#x} x={:.4} y={:.4} xrel={:.4} yrel={:.4} w={})",
                mouse_label(e.which),
                e.state.0,
                e.x,
                e.y,
                e.xrel,
                e.yrel,
                wl(e.window_id)
            ),
            Event::MouseButton(e) => format!(
                "{n}(which={} button={} clicks={} x={:.4} y={:.4} w={})",
                mouse_label(e.which),
                e.button,
                e.clicks,
                e.x,
                e.y,
                wl(e.window_id)
            ),
            Event::MouseWheel(e) => format!(
                "{n}(which={} x={:.4} y={:.4} direction={} mouse={:.4},{:.4} integer={},{} w={})",
                mouse_label(e.which),
                e.x,
                e.y,
                e.direction as i32,
                e.mouse_x,
                e.mouse_y,
                e.integer_x,
                e.integer_y,
                wl(e.window_id)
            ),
            Event::PenProximity(e) => {
                format!("{n}(which=p{} w={})", pen_label(e.which), wl(e.window_id))
            }
            Event::PenTouch(e) => format!(
                "{n}(which=p{} state={:#x} x={:.4} y={:.4} eraser={} w={})",
                pen_label(e.which),
                e.pen_state.0,
                e.x,
                e.y,
                e.eraser as i32,
                wl(e.window_id)
            ),
            Event::PenMotion(e) => format!(
                "{n}(which=p{} state={:#x} x={:.4} y={:.4} w={})",
                pen_label(e.which),
                e.pen_state.0,
                e.x,
                e.y,
                wl(e.window_id)
            ),
            Event::PenButton(e) => format!(
                "{n}(which=p{} state={:#x} x={:.4} y={:.4} button={} w={})",
                pen_label(e.which),
                e.pen_state.0,
                e.x,
                e.y,
                e.button,
                wl(e.window_id)
            ),
            Event::PenAxis(e) => format!(
                "{n}(which=p{} state={:#x} x={:.4} y={:.4} axis={} value={:.4} w={})",
                pen_label(e.which),
                e.pen_state.0,
                e.x,
                e.y,
                e.axis as i32,
                e.value,
                wl(e.window_id)
            ),
            Event::Window(w) => format!("{}:{n}({},{})", wl(w.window_id), w.data1, w.data2),
            _ => n,
        }
    }

    /// Whether the trace keeps an event: the input's and the window's.
    fn traced(ty: EventType) -> bool {
        (0x200..0x300).contains(&ty.0)
            || (EventType::MOUSE_MOTION.0..=EventType::MOUSE_REMOVED.0).contains(&ty.0)
            || (EventType::FINGER_DOWN.0..=EventType::PINCH_END.0).contains(&ty.0)
            || (EventType::PEN_PROXIMITY_IN.0..=EventType::PEN_AXIS.0).contains(&ty.0)
    }

    fn take_queued() {
        events::pump();
        while let Some(event) = events::poll() {
            let ty = event.event_type();
            if traced(ty) {
                let line = describe(&event);
                LINES.lock().unwrap().push((ty, line));
            }
        }
    }

    fn saw_type(ty: EventType) -> bool {
        LINES.lock().unwrap().iter().any(|(t, _)| *t == ty)
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
    }

    fn clear_events() {
        LINES.lock().unwrap().clear();
    }

    /// The events since the last trace, one line each.
    fn trace_events(label: &str) {
        let lines = std::mem::take(&mut *LINES.lock().unwrap());
        if lines.is_empty() {
            t(format!("{label}: events: (none)"));
        }
        for (_, line) in lines {
            t(format!("{label}: event: {line}"));
        }
    }

    /// As [`trace_events`], with the motion events (finger, mouse) between
    /// a finger's down and up as whether there were any, and the events
    /// from there on by their names: `input swipe`'s moves, and so where it
    /// ends, depend on its timing.
    fn trace_events_moves_collapsed(label: &str) {
        let lines = std::mem::take(&mut *LINES.lock().unwrap());
        let mut in_gesture = false;
        let mut moves = 0;
        for (ty, line) in lines {
            if in_gesture && (ty == EventType::FINGER_MOTION || ty == EventType::MOUSE_MOTION) {
                moves += 1;
                continue;
            }
            if ty == EventType::FINGER_UP && in_gesture {
                t(format!("{label}: moves: {}", yes(moves > 0)));
                in_gesture = false;
                t(format!("{label}: event: {}", event_name(ty)));
                continue;
            }
            if in_gesture {
                t(format!("{label}: event: {}", event_name(ty)));
                continue;
            }
            if ty == EventType::FINGER_DOWN {
                in_gesture = true;
                moves = 0;
            }
            t(format!("{label}: event: {line}"));
        }
    }

    fn touch_devices(label: &str) {
        let ids = touch::touch_devices();
        let items: Vec<String> = ids
            .iter()
            .map(|&id| {
                format!(
                    "{} type={} name=\"{}\"",
                    id as i64,
                    touch::touch_device_type(id) as i32,
                    touch::touch_device_name(id).unwrap_or_else(|_| "(null)".into())
                )
            })
            .collect();
        t(format!(
            "{label}: touch devices: {}",
            if items.is_empty() {
                "(none)".into()
            } else {
                items.join(", ")
            }
        ));
    }

    fn mice(label: &str) {
        t(format!(
            "{label}: mice={} has mouse={}",
            mouse::mice().len(),
            yes(mouse::has_mouse())
        ));
    }

    fn icon(label: &str) {
        t(format!("{label}: pointer icon: {}", pointer_icon()));
    }

    /// A request of the runner's `input`, and its events until `ty`.
    fn real(label: &str, args: &str, ty: EventType) {
        // (longer than the double click's time since the last click)
        std::thread::sleep(Duration::from_millis(600));
        request(&format!("input {args}"));
        settle(Some(ty), 300);
        trace_events(label);
    }

    fn hint(name: &str) -> String {
        hints::get(name).unwrap_or_else(|| "(null)".into())
    }

    pub(super) fn main() {
        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        t(format!(
            "hints: touch mouse events={} mouse touch events={}",
            hint(hints::TOUCH_MOUSE_EVENTS),
            hint(hints::MOUSE_TOUCH_EVENTS)
        ));
        touch_devices("start");
        mice("start");

        let env = android::jni_env().expect("a JNI environment").as_ptr();
        let activity = android::activity().expect("the activity").as_ptr();
        *JNI.lock().unwrap() = Some(Jni {
            env,
            activity: jni::new_global_ref(env, activity),
            motion_event: global_class(env, "android/view/MotionEvent"),
            props: global_class(env, "android/view/MotionEvent$PointerProperties"),
            coords: global_class(env, "android/view/MotionEvent$PointerCoords"),
            system_clock: global_class(env, "android/os/SystemClock"),
        });
        jni::delete_local_ref(env, activity);

        let created = Window::create("SDL input", 320, 240, WindowFlags::RESIZABLE);
        t(format!("create: {}", result(&created)));
        let w = created.expect("the window");
        *WINDOW.lock().unwrap() = w.id();
        // (until the activity's surface is ready and has the focus)
        settle(Some(EventType::WINDOW_FOCUS_GAINED), 2000);
        clear_events();
        let (ww, wh) = w.size().unwrap_or_default();
        t(format!("window: size={ww}x{wh}"));
        let focused = with_jni(|j| {
            let v = focus_view(j);
            if v.is_null() {
                return "null".to_string();
            }
            let name = jni::class_name(j.env, v);
            jni::delete_local_ref(j.env, v);
            name
        });
        t(format!("focused view: {focused}"));
        touch_devices("created");
        mice("created");
        icon("created");

        // Real touches
        real("tap", "tap 300 600", EventType::FINGER_UP);
        std::thread::sleep(Duration::from_millis(600));
        request("input swipe 200 800 800 1600 400");
        settle(Some(EventType::FINGER_UP), 300);
        trace_events_moves_collapsed("swipe");
        touch_devices("touches");

        // Stand-ins: two fingers, a pinch, a cancel, a pressure above 1
        gesture();
        inject1(DOWN, 0, FINGER, 200.0, 1000.0, 1.0, 0, SRC_TOUCHSCREEN);
        settle(None, 200);
        trace_events("fingers down 1");
        inject2(pointer_down(1), 200.0, 1000.0, 800.0, 1000.0);
        settle(None, 200);
        trace_events("fingers down 2");
        inject2(MOVE, 220.0, 1010.0, 780.0, 990.0);
        settle(None, 200);
        trace_events("fingers moved");
        inject2(pointer_up(0), 220.0, 1010.0, 780.0, 990.0);
        settle(None, 200);
        trace_events("finger 1 up");
        inject1(MOVE, 1, FINGER, 760.0, 980.0, 1.0, 0, SRC_TOUCHSCREEN);
        settle(None, 200);
        trace_events("finger 2 moved");
        inject1(UP, 1, FINGER, 760.0, 980.0, 1.0, 0, SRC_TOUCHSCREEN);
        settle(None, 200);
        trace_events("finger 2 up");

        gesture();
        inject1(DOWN, 0, FINGER, 300.0, 1200.0, 1.0, 0, SRC_TOUCHSCREEN);
        inject2(pointer_down(1), 300.0, 1200.0, 780.0, 1200.0);
        settle(None, 200);
        trace_events("pinch down");
        inject2(MOVE, 250.0, 1200.0, 830.0, 1200.0);
        settle(None, 200);
        trace_events("pinch move 1");
        inject2(MOVE, 200.0, 1200.0, 880.0, 1200.0);
        settle(None, 200);
        trace_events("pinch move 2");
        inject2(MOVE, 150.0, 1210.0, 930.0, 1190.0);
        settle(None, 200);
        trace_events("pinch move 3");
        inject2(pointer_up(1), 150.0, 1210.0, 930.0, 1190.0);
        settle(None, 200);
        trace_events("pinch up 2");
        inject1(UP, 0, FINGER, 150.0, 1210.0, 1.0, 0, SRC_TOUCHSCREEN);
        settle(None, 200);
        trace_events("pinch up 1");

        gesture();
        inject1(DOWN, 0, FINGER, 500.0, 500.0, 0.5, 0, SRC_TOUCHSCREEN);
        inject1(CANCEL, 0, FINGER, 510.0, 520.0, 0.5, 0, SRC_TOUCHSCREEN);
        settle(None, 200);
        trace_events("cancel");

        // (longer than the double click's time since the last click, as
        // below)
        std::thread::sleep(Duration::from_millis(600));
        gesture();
        inject1(DOWN, 0, FINGER, 100.0, 100.0, 1.5, 0, SRC_TOUCHSCREEN);
        inject1(UP, 0, FINGER, 100.0, 100.0, 1.5, 0, SRC_TOUCHSCREEN);
        settle(None, 200);
        trace_events("pressure");

        // Touches without their mouse
        let _ = hints::set(hints::TOUCH_MOUSE_EVENTS, "0");
        real(
            "tap, touch mouse events off",
            "tap 300 600",
            EventType::FINGER_UP,
        );
        hints::reset(hints::TOUCH_MOUSE_EVENTS);
        touch_devices("touch mouse events");

        // The mouse: real
        real("mouse tap", "mouse tap 400 500", EventType::MOUSE_BUTTON_UP);
        mice("mouse tap");
        real(
            "mouse scroll",
            "mouse scroll 400 500 --axis VSCROLL,2 --axis HSCROLL,-1",
            EventType::MOUSE_WHEEL,
        );
        let _ = hints::set(hints::MOUSE_TOUCH_EVENTS, "0");
        touch_devices("mouse touch events off");
        real(
            "mouse tap, mouse touch events off",
            "mouse tap 400 500",
            EventType::MOUSE_BUTTON_UP,
        );
        hints::reset(hints::MOUSE_TOUCH_EVENTS);
        touch_devices("mouse touch events reset");

        // The mouse: stand-ins (hovering, buttons, wheel)
        gesture();
        inject1(HOVER_ENTER, 0, MOUSE, 500.0, 700.0, 0.0, 0, SRC_MOUSE);
        inject1(HOVER_MOVE, 0, MOUSE, 520.0, 720.0, 0.0, 0, SRC_MOUSE);
        inject1(HOVER_MOVE, 0, MOUSE, 540.0, 740.0, 0.0, 0, SRC_MOUSE);
        inject1(HOVER_EXIT, 0, MOUSE, 540.0, 740.0, 0.0, 0, SRC_MOUSE);
        settle(None, 300);
        trace_events("mouse hover");
        for buttons in [1, 2, 4, 8, 16] {
            gesture();
            inject1(DOWN, 0, MOUSE, 600.0, 800.0, 1.0, buttons, SRC_MOUSE);
            inject1(MOVE, 0, MOUSE, 610.0, 810.0, 1.0, buttons, SRC_MOUSE);
            inject1(UP, 0, MOUSE, 610.0, 810.0, 0.0, 0, SRC_MOUSE);
            settle(None, 300);
            trace_events(&format!("mouse button {buttons:#x}"));
        }
        {
            let pt = Pointer {
                id: 0,
                tool: MOUSE,
                x: 600.0,
                y: 800.0,
                hscroll: -0.5,
                vscroll: 1.5,
                ..Pointer::default()
            };
            gesture();
            inject(SCROLL, &[pt], 0, SRC_MOUSE, Via::Pointer);
            settle(None, 300);
            trace_events("mouse wheel");
        }

        // Cursors
        {
            let mut s = Surface::new(16, 16, PixelFormat::ARGB8888).expect("a surface");
            let _ = s.fill_rect(None, 0xff20_80c0);
            let c = mouse::create_color_cursor(&s, 3, 4);
            drop(s);
            t(format!("color cursor: {}", result(&c)));
            t(format!(
                "set color cursor: {}",
                result(&mouse::set_cursor(c.as_ref().ok()))
            ));
            icon("color cursor");
            mouse::hide_cursor();
            t("hide cursor: ok");
            icon("hidden");
            mouse::show_cursor();
            t("show cursor: ok");
            icon("shown");
            t(format!(
                "default cursor: {}",
                result(&mouse::set_cursor(mouse::default_cursor().as_ref()))
            ));
            icon("default");
            if let Ok(c) = c {
                mouse::destroy_cursor(c);
            }
            for i in 0..mouse::SystemCursor::COUNT as i32 {
                let sc = mouse::create_system_cursor(
                    mouse::SystemCursor::from_index(i).expect("a system cursor"),
                );
                let ok = mouse::set_cursor(sc.as_ref().ok());
                t(format!(
                    "system cursor {i}: {} {}",
                    result(&ok),
                    pointer_icon()
                ));
                let _ = mouse::set_cursor(mouse::default_cursor().as_ref());
                if let Ok(sc) = sc {
                    mouse::destroy_cursor(sc);
                }
            }
            icon("system cursors");
        }

        // Relative mode
        t(format!(
            "relative mode on: {}",
            result(&w.set_relative_mouse_mode(true))
        ));
        settle(None, 500);
        trace_events("relative mode on");
        t(format!(
            "relative mode on: enabled={} pointer capture={}",
            yes(w.relative_mouse_mode().unwrap_or(false)),
            yes(has_pointer_capture())
        ));
        icon("relative mode on");
        {
            let mut pt = Pointer {
                id: 0,
                tool: MOUSE,
                x: 12.0,
                y: -7.0,
                relx: 12.0,
                rely: -7.0,
                ..Pointer::default()
            };
            gesture();
            inject(HOVER_MOVE, &[pt], 0, SRC_MOUSE_RELATIVE, Via::Trackball);
            pt.x = -3.0;
            pt.y = 5.0;
            pt.relx = -3.0;
            pt.rely = 5.0;
            inject(HOVER_MOVE, &[pt], 0, SRC_MOUSE_RELATIVE, Via::Trackball);
            pt.x = 0.0;
            pt.y = 0.0;
            pt.relx = 0.0;
            pt.rely = 0.0;
            inject(DOWN, &[pt], 1, SRC_MOUSE_RELATIVE, Via::Trackball);
            inject(UP, &[pt], 0, SRC_MOUSE_RELATIVE, Via::Trackball);
            settle(None, 300);
            trace_events("captured pointer");
        }
        real(
            "mouse tap in relative mode",
            "mouse tap 410 510",
            EventType::MOUSE_BUTTON_UP,
        );
        t(format!(
            "relative mode off: {}",
            result(&w.set_relative_mouse_mode(false))
        ));
        settle(None, 500);
        trace_events("relative mode off");
        t(format!(
            "relative mode off: enabled={} pointer capture={}",
            yes(w.relative_mouse_mode().unwrap_or(false)),
            yes(has_pointer_capture())
        ));
        icon("relative mode off");

        // The stylus: real
        real("stylus tap", "stylus tap 600 900", EventType::PEN_UP);

        // The stylus: stand-ins (hovering, pressure, its button, the eraser)
        gesture();
        inject1(HOVER_ENTER, 0, STYLUS, 300.0, 400.0, 0.0, 0, SRC_STYLUS);
        inject1(HOVER_MOVE, 0, STYLUS, 310.0, 410.0, 0.0, 0, SRC_STYLUS);
        inject1(HOVER_EXIT, 0, STYLUS, 310.0, 410.0, 0.0, 0, SRC_STYLUS);
        settle(None, 300);
        trace_events("stylus hover");
        gesture();
        inject1(DOWN, 0, STYLUS, 320.0, 420.0, 0.25, 0, SRC_STYLUS);
        settle(None, 200);
        trace_events("stylus down");
        // (BUTTON_STYLUS_PRIMARY)
        inject1(MOVE, 0, STYLUS, 330.0, 430.0, 0.6, 32, SRC_STYLUS);
        settle(None, 200);
        trace_events("stylus button");
        inject1(MOVE, 0, STYLUS, 340.0, 440.0, 1.5, 0, SRC_STYLUS);
        settle(None, 200);
        trace_events("stylus moved");
        inject1(UP, 0, STYLUS, 340.0, 440.0, 0.0, 0, SRC_STYLUS);
        settle(None, 200);
        trace_events("stylus up");
        std::thread::sleep(Duration::from_millis(600));
        gesture();
        inject1(DOWN, 0, ERASER, 400.0, 500.0, 0.5, 0, SRC_STYLUS);
        inject1(UP, 0, ERASER, 400.0, 500.0, 0.0, 0, SRC_STYLUS);
        settle(None, 300);
        trace_events("eraser");
        touch_devices("end");
        mice("end");

        w.destroy();
        settle(None, 200);
        clear_events();
        init::quit();
        t("quit");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
