// Tests of the UIKit video driver's input: touches, the Apple Pencil, the
// pointer, the pinch gesture and the game controllers' mice.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as
//! `uikit_main` is. Its main function makes a window and traces the touch
//! devices, the mice, what its view is (its gesture recognizers and
//! interactions, what it answers the pointer interaction) and the events
//! that come from:
//!
//! * touches given to the view's handlers (`touchesBegan:withEvent:` and
//!   the others), made in the program (the simulator has no other way in):
//!   stand-ins for `UITouch` and `UIEvent` answering what the handlers ask
//!   (location, type, force, timestamp, the Pencil's angles, the pointer's
//!   buttons). Fingers (down, moved, two at once, up, cancelled, with the
//!   mouse the first one emulates), a touch of the indirect kind, the
//!   Pencil (down, moved, up, hovering through its gesture's handler,
//!   cancelled) and the pointer (its buttons pressed, moved, released,
//!   cancelled, and hovering);
//! * the pinch gesture's handler, given a stand-in recognizer (began,
//!   changed with two touches and with one, ended);
//! * the relative mode (which locks the pointer: the view controller's
//!   `prefersPointerLocked`) without a mouse and with one;
//! * a game controller's mouse (the simulator has none): a stand-in
//!   `GCMouse` (with its input, buttons and scroll wheel) posted with
//!   `GCMouseDidConnectNotification`, whose handlers, set by the driver,
//!   the program calls (buttons, motion outside and in the relative mode,
//!   the scroll wheel), and then `GCMouseDidDisconnectNotification`;
//! * real touches, through idb (the runner's `tap` and `swipe` requests,
//!   when it drives idb: else those checks are skipped, `note: skipping,
//!   idb unavailable`): a tap, and a swipe, whose moves are counted (as
//!   many as the frames it takes), not traced. The view is made opaque
//!   first: the simulator passes touches through a window with nothing
//!   drawn in it, to what is behind.
//!
//! Then the window is destroyed. Timestamps aren't traced; the IDs that
//! are addresses (the fingers', the game controller mouse's) and the pen's
//! are named by their first appearance (`f1`, `m1`, `p1`), a finger's from
//! the start of its touch (another may have had its address before).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5).
//! The keyboards' device events (upstream's `GCKeyboard`, which isn't
//! translated yet) are left out. Elsewhere this program does nothing.

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
    use std::ffi::c_void;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, mouse, touch, Event, WindowID};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::{self, Window};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit, idb, request};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
start: touch devices: -1 type=0 name=\"mouse_input\"
start: mice=0 has mouse=no
create: ok
created: event: W:WINDOW_SAFE_AREA_CHANGED(0,0)
created: event: W:WINDOW_MOUSE_ENTER(0,0)
created: event: W:WINDOW_FOCUS_GAINED(0,0)
created: event: W:WINDOW_SHOWN(0,0)
created: event: W:WINDOW_PIXEL_SIZE_CHANGED(393,852)
created: touch devices: -1 type=0 name=\"mouse_input\", 1 type=0 name=\"\"
created: mice=0 has mouse=no
view: SDL_uikitview multiple touch=yes
view: gesture UIPinchGestureRecognizer touch types=0,1,2,3 cancels=no delays began=no ended=no
view: gesture UIHoverGestureRecognizer touch types=2 cancels=yes delays began=no ended=yes
view: gesture UIHoverGestureRecognizer touch types=3 cancels=yes delays began=no ended=yes
view: interaction UIPointerInteraction delegate=view
view: pointer delegate=yes
view: prefers pointer locked=no
pointer: region UIPointerRegion rect=0,0 393x852 identifier=nil
pointer: style with the cursor shown: nil
pointer: style with the cursor hidden: UIPointerStyle
pointer: cursor: events: (none)
finger 1 down: event: MOUSE_MOTION(which=touch state=0x0 x=100.0000 y=200.0000 xrel=0.0000 yrel=0.0000 w=W)
finger 1 down: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=100.0000 y=200.0000 w=W)
finger 1 down: event: FINGER_DOWN(touch=1 finger=f1 x=0.2545 y=0.2347 dx=0.0000 dy=0.0000 p=0.5000 w=W)
finger 1 moved: event: MOUSE_MOTION(which=touch state=0x1 x=150.0000 y=250.0000 xrel=50.0000 yrel=50.0000 w=W)
finger 1 moved: event: FINGER_MOTION(touch=1 finger=f1 x=0.3817 y=0.2934 dx=0.1272 dy=0.0587 p=0.5000 w=W)
finger 2 down: event: FINGER_DOWN(touch=1 finger=f2 x=0.7634 y=0.7042 dx=0.0000 dy=0.0000 p=0.2500 w=W)
fingers 1 and 2 moved: event: MOUSE_MOTION(which=touch state=0x1 x=160.0000 y=260.0000 xrel=10.0000 yrel=10.0000 w=W)
fingers 1 and 2 moved: event: FINGER_MOTION(touch=1 finger=f1 x=0.4071 y=0.3052 dx=0.0254 dy=0.0117 p=0.5000 w=W)
fingers 1 and 2 moved: event: FINGER_MOTION(touch=1 finger=f2 x=0.7888 y=0.7160 dx=0.0254 dy=0.0117 p=0.7500 w=W)
finger 2 up: event: FINGER_UP(touch=1 finger=f2 x=0.7888 y=0.7160 dx=0.0000 dy=0.0000 p=0.7500 w=W)
finger 1 up: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=160.0000 y=260.0000 w=W)
finger 1 up: event: FINGER_UP(touch=1 finger=f1 x=0.4071 y=0.3052 dx=0.0000 dy=0.0000 p=0.0000 w=W)
finger 3 down: event: MOUSE_MOTION(which=touch state=0x0 x=50.0000 y=60.0000 xrel=0.0000 yrel=0.0000 w=W)
finger 3 down: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=50.0000 y=60.0000 w=W)
finger 3 down: event: FINGER_DOWN(touch=1 finger=f3 x=0.1272 y=0.0704 dx=0.0000 dy=0.0000 p=0.0000 w=W)
finger 3 cancelled: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=50.0000 y=60.0000 w=W)
finger 3 cancelled: event: FINGER_CANCELED(touch=1 finger=f3 x=0.1272 y=0.0704 dx=0.0000 dy=0.0000 p=0.0000 w=W)
fingers: touch devices: -1 type=0 name=\"mouse_input\", 1 type=0 name=\"\"
indirect down: event: MOUSE_MOTION(which=touch state=0x0 x=200.0000 y=400.0000 xrel=0.0000 yrel=0.0000 w=W)
indirect down: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=200.0000 y=400.0000 w=W)
indirect down: event: FINGER_DOWN(touch=2 finger=f4 x=0.5089 y=0.4695 dx=0.0000 dy=0.0000 p=0.0000 w=W)
indirect moved: event: MOUSE_MOTION(which=touch state=0x1 x=210.0000 y=420.0000 xrel=10.0000 yrel=20.0000 w=W)
indirect moved: event: FINGER_MOTION(touch=2 finger=f4 x=0.5344 y=0.4930 dx=0.0254 dy=0.0235 p=0.0000 w=W)
indirect up: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=210.0000 y=420.0000 w=W)
indirect up: event: FINGER_UP(touch=2 finger=f4 x=0.5344 y=0.4930 dx=0.0000 dy=0.0000 p=0.0000 w=W)
indirect: touch devices: -1 type=0 name=\"mouse_input\", 1 type=0 name=\"\", 2 type=2 name=\"\"
pencil down: event: PEN_PROXIMITY_IN(which=p1 w=W)
pencil down: event: PEN_MOTION(which=p1 state=0x80000000 x=120.0000 y=340.0000 w=W)
pencil down: event: MOUSE_MOTION(which=pen state=0x0 x=120.0000 y=340.0000 xrel=-90.0000 yrel=-80.0000 w=W)
pencil down: event: PEN_AXIS(which=p1 state=0x80000000 x=120.0000 y=340.0000 axis=0 value=0.5000 w=W)
pencil down: event: PEN_AXIS(which=p1 state=0x80000000 x=120.0000 y=340.0000 axis=1 value=61.3521 w=W)
pencil down: event: PEN_AXIS(which=p1 state=0x80000000 x=120.0000 y=340.0000 axis=2 value=32.7042 w=W)
pencil down: event: PEN_AXIS(which=p1 state=0x80000000 x=120.0000 y=340.0000 axis=4 value=14.3239 w=W)
pencil down: event: PEN_DOWN(which=p1 state=0x80000001 x=120.0000 y=340.0000 eraser=0 w=W)
pencil down: event: MOUSE_BUTTON_DOWN(which=pen button=1 clicks=1 x=120.0000 y=340.0000 w=W)
pencil down: event: FINGER_DOWN(touch=-2 finger=f5 x=0.3053 y=0.3991 dx=0.0000 dy=0.0000 p=0.5000 w=W)
pencil: touch devices: -1 type=0 name=\"mouse_input\", 1 type=0 name=\"\", 2 type=2 name=\"\", -2 type=0 name=\"pen_input\"
pencil moved: event: PEN_MOTION(which=p1 state=0x80000001 x=130.0000 y=350.0000 w=W)
pencil moved: event: MOUSE_MOTION(which=pen state=0x1 x=130.0000 y=350.0000 xrel=10.0000 yrel=10.0000 w=W)
pencil moved: event: FINGER_MOTION(touch=-2 finger=f5 x=0.3308 y=0.4108 dx=0.0254 dy=0.0117 p=0.5000 w=W)
pencil moved: event: PEN_AXIS(which=p1 state=0x80000001 x=130.0000 y=350.0000 axis=0 value=0.7500 w=W)
pencil moved: event: FINGER_MOTION(touch=-2 finger=f5 x=0.3308 y=0.4108 dx=0.0000 dy=0.0000 p=0.7500 w=W)
pencil moved: event: PEN_AXIS(which=p1 state=0x80000001 x=130.0000 y=350.0000 axis=1 value=-53.2394 w=W)
pencil moved: event: PEN_AXIS(which=p1 state=0x80000001 x=130.0000 y=350.0000 axis=2 value=-61.3521 w=W)
pencil up: event: PEN_UP(which=p1 state=0x80000000 x=130.0000 y=350.0000 eraser=0 w=W)
pencil up: event: MOUSE_BUTTON_UP(which=pen button=1 clicks=1 x=130.0000 y=350.0000 w=W)
pencil up: event: FINGER_UP(touch=-2 finger=f5 x=0.3308 y=0.4108 dx=0.0000 dy=0.0000 p=0.7500 w=W)
pencil up: event: PEN_AXIS(which=p1 state=0x80000000 x=130.0000 y=350.0000 axis=0 value=0.0000 w=W)
pencil hover began: event: PEN_MOTION(which=p1 state=0x80000000 x=140.0000 y=360.0000 w=W)
pencil hover began: event: MOUSE_MOTION(which=pen state=0x0 x=140.0000 y=360.0000 xrel=10.0000 yrel=10.0000 w=W)
pencil hover began: event: PEN_AXIS(which=p1 state=0x80000000 x=140.0000 y=360.0000 axis=1 value=61.3521 w=W)
pencil hover began: event: PEN_AXIS(which=p1 state=0x80000000 x=140.0000 y=360.0000 axis=2 value=-47.0282 w=W)
pencil hover began: event: PEN_AXIS(which=p1 state=0x80000000 x=140.0000 y=360.0000 axis=4 value=0.0000 w=W)
pencil hover began: event: PEN_AXIS(which=p1 state=0x80000000 x=140.0000 y=360.0000 axis=3 value=0.5000 w=W)
pencil hover changed: event: PEN_MOTION(which=p1 state=0x80000000 x=150.0000 y=370.0000 w=W)
pencil hover changed: event: MOUSE_MOTION(which=pen state=0x0 x=150.0000 y=370.0000 xrel=10.0000 yrel=10.0000 w=W)
pencil hover ended: events: (none)
pencil down again: event: PEN_MOTION(which=p1 state=0x80000000 x=160.0000 y=380.0000 w=W)
pencil down again: event: MOUSE_MOTION(which=pen state=0x0 x=160.0000 y=380.0000 xrel=10.0000 yrel=10.0000 w=W)
pencil down again: event: PEN_AXIS(which=p1 state=0x80000000 x=160.0000 y=380.0000 axis=0 value=0.2500 w=W)
pencil down again: event: PEN_AXIS(which=p1 state=0x80000000 x=160.0000 y=380.0000 axis=1 value=-81.8873 w=W)
pencil down again: event: PEN_AXIS(which=p1 state=0x80000000 x=160.0000 y=380.0000 axis=2 value=61.3521 w=W)
pencil down again: event: PEN_AXIS(which=p1 state=0x80000000 x=160.0000 y=380.0000 axis=4 value=14.3239 w=W)
pencil down again: event: PEN_AXIS(which=p1 state=0x80000000 x=160.0000 y=380.0000 axis=3 value=0.0000 w=W)
pencil down again: event: PEN_DOWN(which=p1 state=0x80000001 x=160.0000 y=380.0000 eraser=0 w=W)
pencil down again: event: MOUSE_BUTTON_DOWN(which=pen button=1 clicks=1 x=160.0000 y=380.0000 w=W)
pencil down again: event: FINGER_DOWN(touch=-2 finger=f6 x=0.4071 y=0.4460 dx=0.0000 dy=0.0000 p=0.2500 w=W)
pencil cancelled: event: PEN_UP(which=p1 state=0x80000000 x=160.0000 y=380.0000 eraser=0 w=W)
pencil cancelled: event: MOUSE_BUTTON_UP(which=pen button=1 clicks=1 x=160.0000 y=380.0000 w=W)
pencil cancelled: event: FINGER_UP(touch=-2 finger=f6 x=0.4071 y=0.4460 dx=0.0000 dy=0.0000 p=0.2500 w=W)
pointer pressed: event: FINGER_DOWN(touch=-1 finger=f7 x=0.4071 y=0.4460 dx=0.0000 dy=0.0000 p=1.0000 w=W)
pointer pressed: event: MOUSE_BUTTON_DOWN(which=global button=1 clicks=1 x=160.0000 y=380.0000 w=W)
pointer moved: event: FINGER_MOTION(touch=-1 finger=f7 x=0.5344 y=0.3638 dx=0.1272 dy=-0.0822 p=1.0000 w=W)
pointer moved: event: MOUSE_MOTION(which=global state=0x1 x=210.0000 y=310.0000 xrel=50.0000 yrel=-70.0000 w=W)
pointer released: event: FINGER_UP(touch=-1 finger=f7 x=0.5344 y=0.3638 dx=0.0000 dy=0.0000 p=1.0000 w=W)
pointer released: event: MOUSE_BUTTON_UP(which=global button=1 clicks=1 x=210.0000 y=310.0000 w=W)
pointer pressed (4 buttons): event: FINGER_DOWN(touch=-1 finger=f8 x=0.5344 y=0.3638 dx=0.0000 dy=0.0000 p=1.0000 w=W)
pointer pressed (4 buttons): event: MOUSE_BUTTON_DOWN(which=global button=1 clicks=1 x=210.0000 y=310.0000 w=W)
pointer pressed (4 buttons): event: MOUSE_BUTTON_DOWN(which=global button=3 clicks=1 x=210.0000 y=310.0000 w=W)
pointer pressed (4 buttons): event: MOUSE_BUTTON_DOWN(which=global button=2 clicks=1 x=210.0000 y=310.0000 w=W)
pointer pressed (4 buttons): event: MOUSE_BUTTON_DOWN(which=global button=4 clicks=1 x=210.0000 y=310.0000 w=W)
pointer cancelled: event: FINGER_UP(touch=-1 finger=f8 x=0.5344 y=0.3638 dx=0.0000 dy=0.0000 p=1.0000 w=W)
pointer cancelled: event: MOUSE_BUTTON_UP(which=global button=1 clicks=1 x=210.0000 y=310.0000 w=W)
pointer cancelled: event: MOUSE_BUTTON_UP(which=global button=2 clicks=1 x=210.0000 y=310.0000 w=W)
pointer cancelled: event: MOUSE_BUTTON_UP(which=global button=3 clicks=1 x=210.0000 y=310.0000 w=W)
pointer cancelled: event: MOUSE_BUTTON_UP(which=global button=4 clicks=1 x=210.0000 y=310.0000 w=W)
pointer hover changed: event: MOUSE_MOTION(which=global state=0x0 x=220.0000 y=320.0000 xrel=10.0000 yrel=10.0000 w=W)
pointer hover ended: events: (none)
pinch began: event: PINCH_BEGIN(scale=0.0000 span=200.0000,100.0000 focus=200.0000,450.0000 w=W)
pinch changed: event: PINCH_UPDATE(scale=1.5000 span=220.0000,120.0000 focus=200.0000,450.0000 w=W)
pinch changed (one touch): events: (none)
pinch changed again: event: PINCH_UPDATE(scale=1.2500 span=220.0000,120.0000 focus=200.0000,450.0000 w=W)
pinch ended: event: PINCH_END(scale=0.0000 span=-1.0000,-1.0000 focus=-1.0000,-1.0000 w=W)
relative mode on: ok
relative mode on: prefers pointer locked=yes
relative mode on: events: (none)
relative mode off: ok
relative mode off: prefers pointer locked=no
relative mode off: events: (none)
gcmouse: mice: 0
gcmouse connected: event: MOUSE_ADDED(which=m1)
gcmouse connected: mice=1 has mouse=yes
gcmouse: handlers: left=yes middle=yes right=yes aux=yes,yes moved=yes scroll=yes queue=yes
gcmouse left pressed: event: FINGER_DOWN(touch=-1 finger=f9 x=0.5598 y=0.3756 dx=0.0000 dy=0.0000 p=1.0000 w=W)
gcmouse left pressed: event: MOUSE_BUTTON_DOWN(which=global button=1 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse left released: event: FINGER_UP(touch=-1 finger=f9 x=0.5598 y=0.3756 dx=0.0000 dy=0.0000 p=1.0000 w=W)
gcmouse left released: event: MOUSE_BUTTON_UP(which=global button=1 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse other buttons pressed: event: MOUSE_BUTTON_DOWN(which=global button=3 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse other buttons pressed: event: MOUSE_BUTTON_DOWN(which=global button=2 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse other buttons pressed: event: MOUSE_BUTTON_DOWN(which=global button=4 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse other buttons pressed: event: MOUSE_BUTTON_DOWN(which=global button=5 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse other buttons released: event: MOUSE_BUTTON_UP(which=global button=3 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse other buttons released: event: MOUSE_BUTTON_UP(which=global button=2 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse other buttons released: event: MOUSE_BUTTON_UP(which=global button=4 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse other buttons released: event: MOUSE_BUTTON_UP(which=global button=5 clicks=1 x=220.0000 y=320.0000 w=W)
gcmouse moved: events: (none)
relative mode on: ok
gcmouse relative mode on: prefers pointer locked=yes
gcmouse relative mode on: events: (none)
gcmouse moved in relative mode: event: MOUSE_MOTION(which=m1 state=0x0 x=225.0000 y=317.0000 xrel=5.0000 yrel=-3.0000 w=W)
gcmouse moved back in relative mode: event: MOUSE_MOTION(which=m1 state=0x0 x=222.5000 y=318.5000 xrel=-2.5000 yrel=1.5000 w=W)
relative mode off: ok
gcmouse relative mode off: prefers pointer locked=no
gcmouse relative mode off: events: (none)
gcmouse scrolled 1,0: event: MOUSE_WHEEL(which=global x=-0.0000 y=1.0000 direction=1 mouse=222.5000,318.5000 integer=0,1 w=W)
gcmouse scrolled -0.5,2: event: MOUSE_WHEEL(which=global x=-2.0000 y=-0.5000 direction=1 mouse=222.5000,318.5000 integer=-2,0 w=W)
pointer pressed and released with a mouse: events: (none)
gcmouse disconnected: event: MOUSE_REMOVED(which=m1)
gcmouse disconnected: mice=0 has mouse=no
gcmouse: handlers: left=no middle=no right=no aux=no,no moved=no scroll=yes
idb: tap: event: MOUSE_MOTION(which=touch state=0x0 x=100.0000 y=300.0000 xrel=0.0000 yrel=0.0000 w=W)
idb: tap: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=100.0000 y=300.0000 w=W)
idb: tap: event: FINGER_DOWN(touch=1 finger=f10 x=0.2545 y=0.3521 dx=0.0000 dy=0.0000 p=0.0000 w=W)
idb: tap: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=100.0000 y=300.0000 w=W)
idb: tap: event: FINGER_UP(touch=1 finger=f10 x=0.2545 y=0.3521 dx=0.0000 dy=0.0000 p=0.0000 w=W)
idb: swipe: event: MOUSE_BUTTON_DOWN(which=touch button=1 clicks=1 x=100.0000 y=600.0000 w=W)
idb: swipe: event: FINGER_DOWN(touch=1 finger=f11 x=0.2545 y=0.7042 dx=0.0000 dy=0.0000 p=0.0000 w=W)
idb: swipe: event: MOUSE_BUTTON_UP(which=touch button=1 clicks=1 x=300.0000 y=650.0000 w=W)
idb: swipe: event: FINGER_UP(touch=1 finger=f11 x=0.7634 y=0.7629 dx=0.0000 dy=0.0000 p=0.0000 w=W)
idb: swipe: moved: yes
destroyed
destroyed: event: W:WINDOW_DESTROYED(0,0)
destroyed: touch devices: -1 type=0 name=\"mouse_input\", 1 type=0 name=\"\", 2 type=2 name=\"\", -2 type=0 name=\"pen_input\"
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

    // ---- the objects the translated handlers are given

    /// `CGPoint`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
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

    /// What a `UITouch`, `UIEvent`, `UIPinchGestureRecognizer` and
    /// `UIHoverGestureRecognizer` tell the view (an `SDLTestFake`).
    #[derive(Clone, Copy, Default)]
    struct Fake {
        location: CGPoint,
        location2: CGPoint,
        kind: isize,
        force: f64,
        max_force: f64,
        stamp: f64,
        azimuth: f64,
        altitude: f64,
        roll: f64,
        z_offset: f64,
        state: isize,
        scale: f64,
        touch_count: usize,
        buttons: isize,
    }

    /// A `GCMouse`, its `GCMouseInput`, buttons and scroll wheel (an
    /// `SDLTestGC`), keeping the handlers the driver sets.
    struct Gc {
        input: Id,
        left: Id,
        middle: Id,
        right: Id,
        aux: [Id; 2],
        scroll: Id,
        handler: Id,
        queue_set: bool,
    }

    impl Default for Gc {
        fn default() -> Gc {
            let nil = std::ptr::null_mut();
            Gc {
                input: nil,
                left: nil,
                middle: nil,
                right: nil,
                aux: [nil; 2],
                scroll: nil,
                handler: nil,
                queue_set: false,
            }
        }
    }

    thread_local! {
        static FAKES: RefCell<HashMap<usize, Fake>> = RefCell::new(HashMap::new());
        static GCS: RefCell<HashMap<usize, Gc>> = RefCell::new(HashMap::new());
    }

    fn fake(this: Id) -> Fake {
        FAKES.with(|f| {
            f.borrow()
                .get(&(this as usize))
                .copied()
                .unwrap_or_default()
        })
    }

    fn update(this: Id, change: impl FnOnce(&mut Fake)) {
        FAKES.with(|f| change(f.borrow_mut().entry(this as usize).or_default()));
    }

    fn gc<R>(this: Id, read: impl FnOnce(&mut Gc) -> R) -> R {
        GCS.with(|g| read(g.borrow_mut().entry(this as usize).or_default()))
    }

    extern "C" fn location_in_view(this: Id, _: objc::Sel, _view: Id) -> CGPoint {
        fake(this).location
    }
    extern "C" fn touch_type(this: Id, _: objc::Sel) -> isize {
        fake(this).kind
    }
    extern "C" fn force(this: Id, _: objc::Sel) -> f64 {
        fake(this).force
    }
    extern "C" fn maximum_possible_force(this: Id, _: objc::Sel) -> f64 {
        fake(this).max_force
    }
    extern "C" fn timestamp(this: Id, _: objc::Sel) -> f64 {
        fake(this).stamp
    }
    extern "C" fn azimuth_angle_in_view(this: Id, _: objc::Sel, _view: Id) -> f64 {
        fake(this).azimuth
    }
    extern "C" fn altitude_angle(this: Id, _: objc::Sel) -> f64 {
        fake(this).altitude
    }
    extern "C" fn roll_angle(this: Id, _: objc::Sel) -> f64 {
        fake(this).roll
    }
    extern "C" fn z_offset(this: Id, _: objc::Sel) -> f64 {
        fake(this).z_offset
    }
    extern "C" fn state(this: Id, _: objc::Sel) -> isize {
        fake(this).state
    }
    extern "C" fn scale(this: Id, _: objc::Sel) -> f64 {
        fake(this).scale
    }
    extern "C" fn number_of_touches(this: Id, _: objc::Sel) -> usize {
        fake(this).touch_count
    }
    extern "C" fn location_of_touch(this: Id, _: objc::Sel, index: usize, _view: Id) -> CGPoint {
        let f = fake(this);
        if index == 0 {
            f.location
        } else {
            f.location2
        }
    }
    extern "C" fn button_mask(this: Id, _: objc::Sel) -> isize {
        fake(this).buttons
    }
    extern "C" fn tap_count(_this: Id, _: objc::Sel) -> usize {
        1
    }

    extern "C" fn mouse_input(this: Id, _: objc::Sel) -> Id {
        gc(this, |g| g.input)
    }
    extern "C" fn left_button(this: Id, _: objc::Sel) -> Id {
        gc(this, |g| g.left)
    }
    extern "C" fn middle_button(this: Id, _: objc::Sel) -> Id {
        gc(this, |g| g.middle)
    }
    extern "C" fn right_button(this: Id, _: objc::Sel) -> Id {
        gc(this, |g| g.right)
    }
    extern "C" fn auxiliary_buttons(this: Id, _: objc::Sel) -> Id {
        let aux = gc(this, |g| g.aux);
        // SAFETY: NSArray's class method (autoreleased).
        unsafe {
            objc::send2(
                objc::class("NSArray"),
                "arrayWithObjects:count:",
                aux.as_ptr(),
                aux.len(),
            )
        }
    }
    extern "C" fn scroll(this: Id, _: objc::Sel) -> Id {
        gc(this, |g| g.scroll)
    }
    extern "C" fn set_handler(this: Id, _: objc::Sel, handler: Id) {
        let copy = objc::block_copy(handler);
        let old = gc(this, |g| std::mem::replace(&mut g.handler, copy));
        objc::block_release(old);
    }
    extern "C" fn handler(this: Id, _: objc::Sel) -> Id {
        gc(this, |g| g.handler)
    }
    extern "C" fn set_handler_queue(this: Id, _: objc::Sel, queue: Id) {
        gc(this, |g| g.queue_set = !queue.is_null());
    }

    /// Declare `SDLTestFake` and `SDLTestGC`.
    fn declare_classes() {
        let point = "{CGPoint=dd}";
        // SAFETY: the implementations have the encodings' signatures.
        unsafe {
            objc::declare_class(
                "SDLTestFake",
                "NSObject",
                &[
                    (
                        "locationInView:",
                        location_in_view as *const c_void,
                        &format!("{point}@:@"),
                    ),
                    ("type", touch_type as *const c_void, "q@:"),
                    ("force", force as *const c_void, "d@:"),
                    (
                        "maximumPossibleForce",
                        maximum_possible_force as *const c_void,
                        "d@:",
                    ),
                    ("timestamp", timestamp as *const c_void, "d@:"),
                    (
                        "azimuthAngleInView:",
                        azimuth_angle_in_view as *const c_void,
                        "d@:@",
                    ),
                    ("altitudeAngle", altitude_angle as *const c_void, "d@:"),
                    ("rollAngle", roll_angle as *const c_void, "d@:"),
                    ("zOffset", z_offset as *const c_void, "d@:"),
                    ("state", state as *const c_void, "q@:"),
                    ("scale", scale as *const c_void, "d@:"),
                    ("numberOfTouches", number_of_touches as *const c_void, "Q@:"),
                    (
                        "locationOfTouch:inView:",
                        location_of_touch as *const c_void,
                        &format!("{point}@:Q@"),
                    ),
                    ("buttonMask", button_mask as *const c_void, "q@:"),
                    ("tapCount", tap_count as *const c_void, "Q@:"),
                ],
            );
            objc::declare_class(
                "SDLTestGC",
                "NSObject",
                &[
                    ("mouseInput", mouse_input as *const c_void, "@@:"),
                    ("leftButton", left_button as *const c_void, "@@:"),
                    ("middleButton", middle_button as *const c_void, "@@:"),
                    ("rightButton", right_button as *const c_void, "@@:"),
                    (
                        "auxiliaryButtons",
                        auxiliary_buttons as *const c_void,
                        "@@:",
                    ),
                    ("scroll", scroll as *const c_void, "@@:"),
                    (
                        "setPressedChangedHandler:",
                        set_handler as *const c_void,
                        "v@:@?",
                    ),
                    ("pressedChangedHandler", handler as *const c_void, "@?@:"),
                    (
                        "setMouseMovedHandler:",
                        set_handler as *const c_void,
                        "v@:@?",
                    ),
                    ("mouseMovedHandler", handler as *const c_void, "@?@:"),
                    (
                        "setValueChangedHandler:",
                        set_handler as *const c_void,
                        "v@:@?",
                    ),
                    ("valueChangedHandler", handler as *const c_void, "@?@:"),
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

    /// A touch of a kind (`UITouchType`) at a place.
    fn touch_at(kind: isize, x: f64, y: f64, force: f64) -> Id {
        let touch = new_object("SDLTestFake");
        update(touch, |f| {
            f.kind = kind;
            f.location = CGPoint { x, y };
            f.force = force;
            f.max_force = 4.0;
            f.stamp = 1.0;
        });
        touch
    }

    /// An event, with these pointer buttons pressed.
    fn event_with(buttons: isize) -> Id {
        let event = new_object("SDLTestFake");
        update(event, |f| {
            f.stamp = 1.0;
            f.buttons = buttons;
        });
        event
    }

    fn move_to(touch: Id, x: f64, y: f64) {
        update(touch, |f| f.location = CGPoint { x, y });
    }

    /// `[NSSet setWithObject:touch]`.
    fn set_of(touch: Id) -> Id {
        // SAFETY: NSSet's class method (autoreleased).
        unsafe { objc::send1(objc::class("NSSet"), "setWithObject:", touch) }
    }

    /// `@[a, b]` (enumerated by the handlers as a set is).
    fn array_of(touches: &[Id]) -> Id {
        // SAFETY: NSArray's class method (autoreleased).
        unsafe {
            objc::send2(
                objc::class("NSArray"),
                "arrayWithObjects:count:",
                touches.as_ptr(),
                touches.len(),
            )
        }
    }

    /// `[view selector touches withEvent:event]`.
    fn touches(view: Id, selector: &str, touches: Id, event: Id) {
        // SAFETY: the view's touch handlers.
        unsafe { objc::send2::<Id, Id, ()>(view, selector, touches, event) }
    }

    /// `[view selector recognizer]`.
    fn gesture(view: Id, selector: &str, recognizer: Id) {
        // SAFETY: the view's gesture handlers.
        unsafe { objc::send1::<Id, ()>(view, selector, recognizer) }
    }

    // ---- labels: the IDs that are addresses, by first appearance (a
    // finger's from its touch's start: the address of one that ended may
    // be the next one's)

    static FINGERS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    static MICE: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    static PENS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

    fn label_of(labels: &Mutex<Vec<u64>>, id: u64) -> usize {
        let mut l = labels.lock().unwrap();
        match l.iter().position(|&i| i == id) {
            Some(i) => i + 1,
            None => {
                l.push(id);
                l.len()
            }
        }
    }

    /// A finger's label: a new one when it touches the screen.
    fn finger_label(id: u64, down: bool) -> usize {
        if down {
            let mut l = FINGERS.lock().unwrap();
            if let Some(old) = l.iter_mut().find(|i| **i == id) {
                // (the label stays taken)
                *old = u64::MAX;
            }
            l.push(id);
            return l.len();
        }
        label_of(&FINGERS, id)
    }

    fn mouse_label(id: mouse::MouseID) -> String {
        match id {
            0 => "global".into(),
            mouse::TOUCH_MOUSE_ID => "touch".into(),
            mouse::PEN_MOUSE_ID => "pen".into(),
            _ => format!("m{}", label_of(&MICE, id as u64)),
        }
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

    /// An event as the trace names it (the C program's names), `None` for
    /// the keyboards' device events (see the module's description).
    fn describe(event: &Event) -> Option<String> {
        let n = event_name(event.event_type());
        Some(match event {
            Event::KeyboardDevice(_) => return None,
            Event::MouseDevice(e) => format!("{n}(which={})", mouse_label(e.which)),
            Event::TouchFinger(e) => format!(
                "{n}(touch={} finger=f{} x={:.4} y={:.4} dx={:.4} dy={:.4} p={:.4} w={})",
                e.touch_id as i64,
                finger_label(e.finger_id, e.event_type == events::EventType::FINGER_DOWN),
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
            Event::PenProximity(e) => format!(
                "{n}(which=p{} w={})",
                label_of(&PENS, e.which as u64),
                wl(e.window_id)
            ),
            Event::PenTouch(e) => format!(
                "{n}(which=p{} state={:#x} x={:.4} y={:.4} eraser={} w={})",
                label_of(&PENS, e.which as u64),
                e.pen_state.0,
                e.x,
                e.y,
                e.eraser as i32,
                wl(e.window_id)
            ),
            Event::PenMotion(e) => format!(
                "{n}(which=p{} state={:#x} x={:.4} y={:.4} w={})",
                label_of(&PENS, e.which as u64),
                e.pen_state.0,
                e.x,
                e.y,
                wl(e.window_id)
            ),
            Event::PenAxis(e) => format!(
                "{n}(which=p{} state={:#x} x={:.4} y={:.4} axis={} value={:.4} w={})",
                label_of(&PENS, e.which as u64),
                e.pen_state.0,
                e.x,
                e.y,
                e.axis as i32,
                e.value,
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

    fn mice_state(label: &str) {
        t(format!(
            "{label}: mice={} has mouse={}",
            mouse::mice().len(),
            yes_no(mouse::has_mouse())
        ));
    }

    /// The window's `UIWindow` (its property, a number here).
    fn uiwindow_of(w: &Window) -> Id {
        w.properties()
            .ok()
            .and_then(|p| p.get_number(video::window::PROP_WINDOW_UIKIT_WINDOW_POINTER))
            .map_or(std::ptr::null_mut(), |p| p as usize as Id)
    }

    fn vc_of(w: &Window) -> Id {
        // SAFETY: a UIWindow property.
        unsafe { objc::send0(uiwindow_of(w), "rootViewController") }
    }

    fn view_of(w: &Window) -> Id {
        // SAFETY: a UIViewController property.
        unsafe { objc::send0(vc_of(w), "view") }
    }

    fn pointer_lock(label: &str, w: &Window) {
        // SAFETY: a UIViewController property (iOS 14).
        let locked: bool = unsafe { objc::send0(vc_of(w), "prefersPointerLocked") };
        t(format!(
            "{label}: prefers pointer locked={}",
            yes_no(locked)
        ));
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

    #[link(name = "GameController", kind = "framework")]
    unsafe extern "C" {
        static GCMouseDidConnectNotification: Id;
        static GCMouseDidDisconnectNotification: Id;
    }

    /// `UITouchType`s.
    const DIRECT: isize = 0;
    const INDIRECT: isize = 1;
    const PENCIL: isize = 2;
    const INDIRECT_POINTER: isize = 3;
    /// `UIGestureRecognizerState`s.
    const BEGAN: isize = 1;
    const CHANGED: isize = 2;
    const ENDED: isize = 3;

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_input: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_input: {:.3} s: main", elapsed());
        declare_classes();
        // (clicks counted one by one, whatever the time between them)
        hints::set(hints::MOUSE_DOUBLE_CLICK_TIME, "0").unwrap();
        let r = init::init(InitFlags::VIDEO);
        t(format!("init video: {}", ok(&r)));
        r.unwrap();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap()
        ));
        touch_devices("start");
        mice_state("start");

        // The app becomes active around when the main function starts
        let mut ignored = Vec::new();
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            ignored.clear();
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
        settle(&mut ignored, 100);

        let r = Window::create("input", 320, 480, WindowFlags::NONE);
        t(format!("create: {}", ok(&r)));
        let w = r.unwrap();
        *WINDOW.lock().unwrap() = w.id();
        trace_events("created", 500);
        touch_devices("created");
        mice_state("created");

        let view = view_of(&w);
        // SAFETY: UIView's, UIGestureRecognizer's and UIPointerInteraction's
        // properties.
        unsafe {
            let multiple: bool = objc::send0(view, "isMultipleTouchEnabled");
            t(format!(
                "view: {} multiple touch={}",
                objc::class_name(view),
                yes_no(multiple)
            ));
            for g in items(objc::send0(view, "gestureRecognizers")) {
                let types: Vec<String> = items(objc::send0(g, "allowedTouchTypes"))
                    .into_iter()
                    .map(|n| objc::send0::<isize>(n, "integerValue").to_string())
                    .collect();
                let cancels: bool = objc::send0(g, "cancelsTouchesInView");
                let began: bool = objc::send0(g, "delaysTouchesBegan");
                let ended: bool = objc::send0(g, "delaysTouchesEnded");
                t(format!(
                    "view: gesture {} touch types={} cancels={} delays began={} ended={}",
                    objc::class_name(g),
                    types.join(","),
                    yes_no(cancels),
                    yes_no(began),
                    yes_no(ended)
                ));
            }
            for i in items(objc::send0(view, "interactions")) {
                let delegate = if is_kind_of(i, "UIPointerInteraction") {
                    let d: Id = objc::send0(i, "delegate");
                    if d == view {
                        "view"
                    } else {
                        "other"
                    }
                } else {
                    "-"
                };
                t(format!(
                    "view: interaction {} delegate={delegate}",
                    objc::class_name(i)
                ));
            }
        }
        t(format!(
            "view: pointer delegate={}",
            yes_no(objc::conforms_to(view, "UIPointerInteractionDelegate"))
        ));
        pointer_lock("view", &w);

        // The pointer interaction's delegate
        // SAFETY: UIView's interactions, the view's delegate methods and
        // UIPointerRegion's properties.
        unsafe {
            let pi = items(objc::send0(view, "interactions"))
                .into_iter()
                .rfind(|&i| is_kind_of(i, "UIPointerInteraction"))
                .unwrap_or(std::ptr::null_mut());
            let nil: Id = std::ptr::null_mut();
            let region: Id = objc::send3(
                view,
                "pointerInteraction:regionForRequest:defaultRegion:",
                pi,
                nil,
                nil,
            );
            let rect: CGRect = objc::send0(region, "rect");
            let identifier: Id = objc::send0(region, "identifier");
            t(format!(
                "pointer: region {} rect={},{} {}x{} identifier={}",
                objc::class_name(region),
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                if identifier.is_null() { "nil" } else { "yes" }
            ));
            let style_name = |style: Id| {
                if style.is_null() {
                    "nil".to_string()
                } else {
                    objc::class_name(style)
                }
            };
            let style: Id = objc::send2(view, "pointerInteraction:styleForRegion:", pi, region);
            t(format!(
                "pointer: style with the cursor shown: {}",
                style_name(style)
            ));
            mouse::hide_cursor();
            let style: Id = objc::send2(view, "pointerInteraction:styleForRegion:", pi, region);
            t(format!(
                "pointer: style with the cursor hidden: {}",
                style_name(style)
            ));
            mouse::show_cursor();
        }
        now_events("pointer: cursor");

        // Fingers
        let t1 = touch_at(DIRECT, 100.0, 200.0, 0.5);
        let t2 = touch_at(DIRECT, 300.0, 600.0, 0.25);
        let t3 = touch_at(DIRECT, 50.0, 60.0, 0.0);
        let ev = event_with(0);
        touches(view, "touchesBegan:withEvent:", set_of(t1), ev);
        now_events("finger 1 down");
        move_to(t1, 150.0, 250.0);
        touches(view, "touchesMoved:withEvent:", set_of(t1), ev);
        now_events("finger 1 moved");
        touches(view, "touchesBegan:withEvent:", set_of(t2), ev);
        now_events("finger 2 down");
        move_to(t1, 160.0, 260.0);
        move_to(t2, 310.0, 610.0);
        update(t2, |f| f.force = 0.75);
        touches(view, "touchesMoved:withEvent:", array_of(&[t1, t2]), ev);
        now_events("fingers 1 and 2 moved");
        touches(view, "touchesEnded:withEvent:", set_of(t2), ev);
        now_events("finger 2 up");
        update(t1, |f| f.force = 0.0);
        touches(view, "touchesEnded:withEvent:", set_of(t1), ev);
        now_events("finger 1 up");
        touches(view, "touchesBegan:withEvent:", set_of(t3), ev);
        now_events("finger 3 down");
        touches(view, "touchesCancelled:withEvent:", set_of(t3), ev);
        now_events("finger 3 cancelled");
        touch_devices("fingers");

        // An indirect touch (the Siri Remote's surface)
        let t4 = touch_at(INDIRECT, 200.0, 400.0, 0.0);
        touches(view, "touchesBegan:withEvent:", set_of(t4), ev);
        now_events("indirect down");
        move_to(t4, 210.0, 420.0);
        touches(view, "touchesMoved:withEvent:", set_of(t4), ev);
        now_events("indirect moved");
        touches(view, "touchesEnded:withEvent:", set_of(t4), ev);
        now_events("indirect up");
        touch_devices("indirect");

        // The Pencil
        let t5 = touch_at(PENCIL, 120.0, 340.0, 2.0);
        update(t5, |f| {
            f.azimuth = 0.5;
            f.altitude = 1.0;
            f.roll = 0.25;
        });
        touches(view, "touchesBegan:withEvent:", set_of(t5), ev);
        now_events("pencil down");
        touch_devices("pencil");
        update(t5, |f| {
            f.location = CGPoint { x: 130.0, y: 350.0 };
            f.force = 3.0;
            f.azimuth = -2.5;
            f.altitude = 0.5;
        });
        touches(view, "touchesMoved:withEvent:", set_of(t5), ev);
        now_events("pencil moved");
        update(t5, |f| f.force = 0.0);
        touches(view, "touchesEnded:withEvent:", set_of(t5), ev);
        now_events("pencil up");
        let hover = new_object("SDLTestFake");
        update(hover, |f| {
            f.location = CGPoint { x: 140.0, y: 360.0 };
            f.z_offset = 0.5;
            f.azimuth = -0.5;
            f.altitude = 0.75;
            f.state = BEGAN;
        });
        gesture(view, "pencilHovering:", hover);
        now_events("pencil hover began");
        update(hover, |f| {
            f.location = CGPoint { x: 150.0, y: 370.0 };
            f.state = CHANGED;
        });
        gesture(view, "pencilHovering:", hover);
        now_events("pencil hover changed");
        update(hover, |f| f.state = ENDED);
        gesture(view, "pencilHovering:", hover);
        now_events("pencil hover ended");
        update(t5, |f| {
            f.location = CGPoint { x: 160.0, y: 380.0 };
            f.force = 1.0;
            f.azimuth = 3.0;
        });
        touches(view, "touchesBegan:withEvent:", set_of(t5), ev);
        now_events("pencil down again");
        touches(view, "touchesCancelled:withEvent:", set_of(t5), ev);
        now_events("pencil cancelled");

        // The pointer (a trackpad's or a mouse's)
        let t6 = touch_at(INDIRECT_POINTER, 200.0, 300.0, 0.0);
        touches(view, "touchesBegan:withEvent:", set_of(t6), event_with(1));
        now_events("pointer pressed");
        move_to(t6, 210.0, 310.0);
        touches(view, "touchesMoved:withEvent:", set_of(t6), event_with(1));
        now_events("pointer moved");
        touches(view, "touchesEnded:withEvent:", set_of(t6), event_with(0));
        now_events("pointer released");
        touches(
            view,
            "touchesBegan:withEvent:",
            set_of(t6),
            event_with(1 | 2 | 4 | 8),
        );
        now_events("pointer pressed (4 buttons)");
        touches(
            view,
            "touchesCancelled:withEvent:",
            set_of(t6),
            event_with(0),
        );
        now_events("pointer cancelled");
        let phover = new_object("SDLTestFake");
        update(phover, |f| {
            f.location = CGPoint { x: 220.0, y: 320.0 };
            f.state = CHANGED;
        });
        gesture(view, "indirectPointerHovering:", phover);
        now_events("pointer hover changed");
        update(phover, |f| f.state = ENDED);
        gesture(view, "indirectPointerHovering:", phover);
        now_events("pointer hover ended");

        // The pinch
        let pinch = new_object("SDLTestFake");
        update(pinch, |f| {
            f.touch_count = 2;
            f.location = CGPoint { x: 100.0, y: 400.0 };
            f.location2 = CGPoint { x: 300.0, y: 500.0 };
            f.scale = 1.0;
            f.state = BEGAN;
        });
        gesture(view, "sdlPinchGesture:", pinch);
        now_events("pinch began");
        update(pinch, |f| {
            f.location = CGPoint { x: 90.0, y: 390.0 };
            f.location2 = CGPoint { x: 310.0, y: 510.0 };
            f.scale = 1.5;
            f.state = CHANGED;
        });
        gesture(view, "sdlPinchGesture:", pinch);
        now_events("pinch changed");
        update(pinch, |f| {
            f.touch_count = 1;
            f.scale = 2.0;
        });
        gesture(view, "sdlPinchGesture:", pinch);
        now_events("pinch changed (one touch)");
        update(pinch, |f| {
            f.touch_count = 2;
            f.scale = 2.5;
        });
        gesture(view, "sdlPinchGesture:", pinch);
        now_events("pinch changed again");
        update(pinch, |f| f.state = ENDED);
        gesture(view, "sdlPinchGesture:", pinch);
        now_events("pinch ended");

        // The relative mode, with no mouse
        t(format!(
            "relative mode on: {}",
            ok(&w.set_relative_mouse_mode(true))
        ));
        pointer_lock("relative mode on", &w);
        now_events("relative mode on");
        t(format!(
            "relative mode off: {}",
            ok(&w.set_relative_mouse_mode(false))
        ));
        pointer_lock("relative mode off", &w);
        now_events("relative mode off");

        // A game controller's mouse (GCMouse), connected through its
        // notification
        let gcmouse = new_object("SDLTestGC");
        let input = new_object("SDLTestGC");
        let left = new_object("SDLTestGC");
        let middle = new_object("SDLTestGC");
        let right = new_object("SDLTestGC");
        let aux = [new_object("SDLTestGC"), new_object("SDLTestGC")];
        let scroll_pad = new_object("SDLTestGC");
        gc(gcmouse, |g| g.input = input);
        gc(input, |g| {
            g.left = left;
            g.middle = middle;
            g.right = right;
            g.aux = aux;
            g.scroll = scroll_pad;
        });
        // SAFETY: GCMouse's class method.
        let count: usize = unsafe {
            let mice: Id = objc::send0(objc::class("GCMouse"), "mice");
            objc::send0(mice, "count")
        };
        t(format!("gcmouse: mice: {count}"));
        // SAFETY: GameController's notification names.
        let (connected, disconnected) = unsafe {
            (
                GCMouseDidConnectNotification,
                GCMouseDidDisconnectNotification,
            )
        };
        objc::post_notification_for(connected, gcmouse);
        now_events("gcmouse connected");
        mice_state("gcmouse connected");
        let has = |o: Id| yes_no(!gc(o, |g| g.handler).is_null());
        t(format!(
            "gcmouse: handlers: left={} middle={} right={} aux={},{} moved={} scroll={} queue={}",
            has(left),
            has(middle),
            has(right),
            has(aux[0]),
            has(aux[1]),
            has(input),
            has(scroll_pad),
            yes_no(gc(gcmouse, |g| g.queue_set))
        ));
        // `^(GCControllerButtonInput *button, float value, BOOL pressed)`
        let press = |button: Id, pressed: bool| {
            let block = gc(button, |g| g.handler);
            // SAFETY: the driver's handler, of that signature.
            unsafe { objc::call_block3(block, button, if pressed { 1.0f32 } else { 0.0 }, pressed) }
        };
        // `^(GCMouseInput *mouse, float deltaX, float deltaY)` and
        // `^(GCControllerDirectionPad *dpad, float xValue, float yValue)`
        let xy = |obj: Id, x: f32, y: f32| {
            let block = gc(obj, |g| g.handler);
            // SAFETY: the driver's handler, of that signature.
            unsafe { objc::call_block3(block, obj, x, y) }
        };
        press(left, true);
        now_events("gcmouse left pressed");
        press(left, false);
        now_events("gcmouse left released");
        press(right, true);
        press(middle, true);
        press(aux[0], true);
        press(aux[1], true);
        now_events("gcmouse other buttons pressed");
        press(right, false);
        press(middle, false);
        press(aux[0], false);
        press(aux[1], false);
        now_events("gcmouse other buttons released");
        xy(input, 5.0, 3.0);
        now_events("gcmouse moved");
        t(format!(
            "relative mode on: {}",
            ok(&w.set_relative_mouse_mode(true))
        ));
        pointer_lock("gcmouse relative mode on", &w);
        now_events("gcmouse relative mode on");
        xy(input, 5.0, 3.0);
        now_events("gcmouse moved in relative mode");
        xy(input, -2.5, -1.5);
        now_events("gcmouse moved back in relative mode");
        t(format!(
            "relative mode off: {}",
            ok(&w.set_relative_mouse_mode(false))
        ));
        pointer_lock("gcmouse relative mode off", &w);
        now_events("gcmouse relative mode off");
        xy(scroll_pad, 1.0, 0.0);
        now_events("gcmouse scrolled 1,0");
        xy(scroll_pad, -0.5, 2.0);
        now_events("gcmouse scrolled -0.5,2");
        touches(view, "touchesBegan:withEvent:", set_of(t6), event_with(1));
        touches(view, "touchesEnded:withEvent:", set_of(t6), event_with(0));
        now_events("pointer pressed and released with a mouse");
        objc::post_notification_for(disconnected, gcmouse);
        now_events("gcmouse disconnected");
        mice_state("gcmouse disconnected");
        t(format!(
            "gcmouse: handlers: left={} middle={} right={} aux={},{} moved={} scroll={}",
            has(left),
            has(middle),
            has(right),
            has(aux[0]),
            has(aux[1]),
            has(input),
            has(scroll_pad)
        ));

        // Real touches, through idb
        let idb = idb();
        match &idb {
            Ok(()) => {
                // (the simulator passes touches through a window with
                // nothing drawn in it)
                // SAFETY: UIColor's class method and a UIView property.
                unsafe {
                    let black: Id = objc::send0(objc::class("UIColor"), "blackColor");
                    let () = objc::send1(view, "setBackgroundColor:", black);
                }
                let mut seen = Vec::new();
                println!("uikit_input: {:.3} s: asking for a tap", elapsed());
                request("tap 100 300");
                wait_for_finger_up(&mut seen);
                println!("uikit_input: {:.3} s: tapped", elapsed());
                trace_seen("idb: tap", &seen);

                let mut seen = Vec::new();
                request("swipe 100 600 300 650 1");
                wait_for_finger_up(&mut seen);
                println!("uikit_input: {:.3} s: swiped", elapsed());
                // (the moves, as many as the frames the swipe took, are
                // counted, not traced)
                let (moves, kept): (Vec<String>, Vec<String>) = seen.into_iter().partition(|e| {
                    e.starts_with("FINGER_MOTION") || e.starts_with("MOUSE_MOTION(which=touch")
                });
                trace_seen("idb: swipe", &kept);
                t(format!("idb: swipe: moved: {}", yes_no(!moves.is_empty())));
            }
            Err(why) => println!("note: skipping, idb unavailable: {why}"),
        }

        w.destroy();
        t("destroyed");
        trace_events("destroyed", 300);
        touch_devices("destroyed");

        init::quit();
        t("quit");

        let expected: String = if idb.is_ok() {
            EXPECTED.to_string()
        } else {
            EXPECTED
                .lines()
                .filter(|l| !l.starts_with("idb: "))
                .map(|l| format!("{l}\n"))
                .collect()
        };
        check_trace(&TRACE.lock().unwrap(), &expected);
    }

    /// Pump until a finger is up (for at most a minute), then 300 ms more.
    fn wait_for_finger_up(seen: &mut Vec<String>) {
        let end = Instant::now() + Duration::from_secs(60);
        while !seen.iter().any(|e| e.contains("FINGER_UP")) && Instant::now() < end {
            take_queued(seen);
            std::thread::sleep(Duration::from_millis(10));
        }
        settle(seen, 300);
    }
}
