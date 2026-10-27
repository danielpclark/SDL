// Tests of the UIKit video driver's displays: their bounds, modes and
// orientation.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as
//! `uikit_main` is. Its main function asks the driver for everything it
//! knows about its display: the bounds and usable bounds, the name, the
//! content scale, the natural and current orientations, the desktop and
//! current modes, the fullscreen modes (each with its format, size,
//! density and refresh rate), the closest fullscreen modes to some sizes,
//! the displays at some points, and the properties. Then it turns the app
//! in-process (`requestGeometryUpdateWithPreferences:`, iOS 16, on a
//! window of its own in the app's scene: no SDL window can be made yet) to
//! landscape right, landscape left and back to portrait, with the display
//! events each turn sends and everything above after it; posts the
//! screens' connection and disconnection notifications for the main screen
//! (a display is added for it, and the first display of it is removed);
//! and starts the video subsystem again.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5).
//! Upstream's driver also adds a keyboard (`GCKeyboard`), which isn't
//! translated yet, so the keyboards' and mice's device events are left
//! out of it. Elsewhere this program does nothing.

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
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::{self, DisplayID, Event};
    use sdl3::init::{self, InitFlags};
    use sdl3::properties::ValueKind;
    use sdl3::video::{self, DisplayMode, Point, Rect};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
events: DISPLAY_ORIENTATION(1,3)
start: displays: 1
start: display 1 name=1 primary=yes
start: bounds=0,0 393x852 usable=0,0 393x852
start: content_scale=1 natural=3 current=3
start: desktop mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
start: current mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
start: fullscreen modes: 2
start: fullscreen mode 0: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
start: fullscreen mode 1: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
start: closest to 393x852 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
start: closest to 852x393 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
start: closest to 393x852 at 0 high density no: error: Couldn't find any matching video modes
start: closest to 200x200 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
start: closest to 393x852 at 30 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
start: closest to 1000x1000 at 0 high density yes: error: Couldn't find any matching video modes
start: closest to 0x0 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
start: properties: SDL.display.HDR_enabled=false
start: display for point 10,10: 1
start: display for point 500,100: 1
start: display for point -50,-50: 1
start: display for point 1000,1000: 1
start: display for rect 300,0 200x200: 1
application state: 0
landscape right: events: DISPLAY_DESKTOP_MODE_CHANGED(1,852), DISPLAY_CURRENT_MODE_CHANGED(1,852), DISPLAY_ORIENTATION(1,1)
landscape right: displays: 1
landscape right: display 1 name=1 primary=yes
landscape right: bounds=0,0 852x393 usable=0,0 852x393
landscape right: content_scale=1 natural=3 current=1
landscape right: desktop mode 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape right: current mode 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape right: fullscreen modes: 2
landscape right: fullscreen mode 0: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape right: fullscreen mode 1: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape right: closest to 393x852 at 0 high density yes: error: Couldn't find any matching video modes
landscape right: closest to 852x393 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape right: closest to 393x852 at 0 high density no: error: Couldn't find any matching video modes
landscape right: closest to 200x200 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape right: closest to 393x852 at 30 high density yes: error: Couldn't find any matching video modes
landscape right: closest to 1000x1000 at 0 high density yes: error: Couldn't find any matching video modes
landscape right: closest to 0x0 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape right: properties: SDL.display.HDR_enabled=false
landscape right: display for point 10,10: 1
landscape right: display for point 500,100: 1
landscape right: display for point -50,-50: 1
landscape right: display for point 1000,1000: 1
landscape right: display for rect 300,0 200x200: 1
landscape left: events: DISPLAY_ORIENTATION(1,2)
landscape left: displays: 1
landscape left: display 1 name=1 primary=yes
landscape left: bounds=0,0 852x393 usable=0,0 852x393
landscape left: content_scale=1 natural=3 current=2
landscape left: desktop mode 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape left: current mode 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape left: fullscreen modes: 2
landscape left: fullscreen mode 0: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape left: fullscreen mode 1: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape left: closest to 393x852 at 0 high density yes: error: Couldn't find any matching video modes
landscape left: closest to 852x393 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape left: closest to 393x852 at 0 high density no: error: Couldn't find any matching video modes
landscape left: closest to 200x200 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape left: closest to 393x852 at 30 high density yes: error: Couldn't find any matching video modes
landscape left: closest to 1000x1000 at 0 high density yes: error: Couldn't find any matching video modes
landscape left: closest to 0x0 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
landscape left: properties: SDL.display.HDR_enabled=false
landscape left: display for point 10,10: 1
landscape left: display for point 500,100: 1
landscape left: display for point -50,-50: 1
landscape left: display for point 1000,1000: 1
landscape left: display for rect 300,0 200x200: 1
portrait: events: DISPLAY_DESKTOP_MODE_CHANGED(1,393), DISPLAY_CURRENT_MODE_CHANGED(1,393), DISPLAY_ORIENTATION(1,3)
portrait: displays: 1
portrait: display 1 name=1 primary=yes
portrait: bounds=0,0 393x852 usable=0,0 393x852
portrait: content_scale=1 natural=3 current=3
portrait: desktop mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
portrait: current mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
portrait: fullscreen modes: 2
portrait: fullscreen mode 0: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
portrait: fullscreen mode 1: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
portrait: closest to 393x852 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
portrait: closest to 852x393 at 0 high density yes: error: Couldn't find any matching video modes
portrait: closest to 393x852 at 0 high density no: error: Couldn't find any matching video modes
portrait: closest to 200x200 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
portrait: closest to 393x852 at 30 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
portrait: closest to 1000x1000 at 0 high density yes: error: Couldn't find any matching video modes
portrait: closest to 0x0 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
portrait: properties: SDL.display.HDR_enabled=false
portrait: display for point 10,10: 1
portrait: display for point 500,100: 1
portrait: display for point -50,-50: 1
portrait: display for point 1000,1000: 1
portrait: display for rect 300,0 200x200: 1
connected: events: DISPLAY_ADDED(2,0)
connected: displays: 1, 2
connected: display 1 name=1 primary=yes
connected: bounds=0,0 393x852 usable=0,0 393x852
connected: content_scale=1 natural=3 current=3
connected: desktop mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
connected: current mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
connected: fullscreen modes: 2
connected: fullscreen mode 0: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
connected: fullscreen mode 1: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
connected: closest to 393x852 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
connected: closest to 852x393 at 0 high density yes: error: Couldn't find any matching video modes
connected: closest to 393x852 at 0 high density no: error: Couldn't find any matching video modes
connected: closest to 200x200 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
connected: closest to 393x852 at 30 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
connected: closest to 1000x1000 at 0 high density yes: error: Couldn't find any matching video modes
connected: closest to 0x0 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=1
connected: properties: SDL.display.HDR_enabled=false
connected: display 2 name=2 primary=no
connected: bounds=393,0 393x852 usable=393,0 393x852
connected: content_scale=1 natural=3 current=1
connected: desktop mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
connected: current mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
connected: fullscreen modes: 2
connected: fullscreen mode 0: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
connected: fullscreen mode 1: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
connected: closest to 393x852 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
connected: closest to 852x393 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
connected: closest to 393x852 at 0 high density no: error: Couldn't find any matching video modes
connected: closest to 200x200 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
connected: closest to 393x852 at 30 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
connected: closest to 1000x1000 at 0 high density yes: error: Couldn't find any matching video modes
connected: closest to 0x0 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
connected: properties: SDL.display.HDR_enabled=false
connected: display for point 10,10: 1
connected: display for point 500,100: 2
connected: display for point -50,-50: 1
connected: display for point 1000,1000: 2
connected: display for rect 300,0 200x200: 2
disconnected: events: DISPLAY_REMOVED(1,0)
disconnected: displays: 2
disconnected: display 2 name=2 primary=yes
disconnected: bounds=0,0 393x852 usable=0,0 393x852
disconnected: content_scale=1 natural=3 current=1
disconnected: desktop mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
disconnected: current mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
disconnected: fullscreen modes: 2
disconnected: fullscreen mode 0: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
disconnected: fullscreen mode 1: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
disconnected: closest to 393x852 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
disconnected: closest to 852x393 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
disconnected: closest to 393x852 at 0 high density no: error: Couldn't find any matching video modes
disconnected: closest to 200x200 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
disconnected: closest to 393x852 at 30 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
disconnected: closest to 1000x1000 at 0 high density yes: error: Couldn't find any matching video modes
disconnected: closest to 0x0 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=2
disconnected: properties: SDL.display.HDR_enabled=false
disconnected: display for point 10,10: 2
disconnected: display for point 500,100: 2
disconnected: display for point -50,-50: 2
disconnected: display for point 1000,1000: 2
disconnected: display for rect 300,0 200x200: 2
disconnected again: events: DISPLAY_REMOVED(2,0)
disconnected again: displays: (none)
disconnected again: display for point 10,10: error: Couldn't find any displays
disconnected again: display for point 500,100: error: Couldn't find any displays
disconnected again: display for point -50,-50: error: Couldn't find any displays
disconnected again: display for point 1000,1000: error: Couldn't find any displays
disconnected again: display for rect 300,0 200x200: error: Couldn't find any displays
connected again: events: DISPLAY_ADDED(3,0)
connected again: displays: 3
connected again: display 3 name=3 primary=yes
connected again: bounds=0,0 393x852 usable=0,0 393x852
connected again: content_scale=1 natural=3 current=1
connected again: desktop mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=3
connected again: current mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=3
connected again: fullscreen modes: 2
connected again: fullscreen mode 0: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=3
connected again: fullscreen mode 1: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=3
connected again: closest to 393x852 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=3
connected again: closest to 852x393 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=3
connected again: closest to 393x852 at 0 high density no: error: Couldn't find any matching video modes
connected again: closest to 200x200 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=3
connected again: closest to 393x852 at 30 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=3
connected again: closest to 1000x1000 at 0 high density yes: error: Couldn't find any matching video modes
connected again: closest to 0x0 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=3
connected again: properties: SDL.display.HDR_enabled=false
connected again: display for point 10,10: 3
connected again: display for point 500,100: 3
connected again: display for point -50,-50: 3
connected again: display for point 1000,1000: 3
connected again: display for rect 300,0 200x200: 3
quit video: displays: error: Video subsystem has not been initialized
init video again: ok
events: DISPLAY_ORIENTATION(4,3)
again: displays: 4
again: display 4 name=4 primary=yes
again: bounds=0,0 393x852 usable=0,0 393x852
again: content_scale=1 natural=3 current=3
again: desktop mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=4
again: current mode 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=4
again: fullscreen modes: 2
again: fullscreen mode 0: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=4
again: fullscreen mode 1: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=4
again: closest to 393x852 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=4
again: closest to 852x393 at 0 high density yes: 852x393 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=4
again: closest to 393x852 at 0 high density no: error: Couldn't find any matching video modes
again: closest to 200x200 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=4
again: closest to 393x852 at 30 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=4
again: closest to 1000x1000 at 0 high density yes: error: Couldn't find any matching video modes
again: closest to 0x0 at 0 high density yes: 393x852 fmt=SDL_PIXELFORMAT_ABGR8888 density=3 refresh=60 (60/1) display=4
again: properties: SDL.display.HDR_enabled=false
again: display for point 10,10: 4
again: display for point 500,100: 4
again: display for point -50,-50: 4
again: display for point 1000,1000: 4
again: display for rect 300,0 200x200: 4
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

    /// An event as the trace names it (the C program's names), `None` for
    /// the keyboards' and mice's device events (see the module's
    /// description).
    fn describe(event: &Event) -> Option<String> {
        let name = event_name(event.event_type());
        Some(match event {
            Event::KeyboardDevice(_) | Event::MouseDevice(_) => return None,
            Event::Display(d) => format!("{name}({},{})", d.display_id, d.data1),
            _ => name,
        })
    }

    /// Pump, and the queued events.
    fn take_queued() -> Vec<String> {
        events::pump();
        let mut out = Vec::new();
        while let Some(event) = events::poll() {
            out.extend(describe(&event));
        }
        out
    }

    /// Pump events (in a loop with short sleeps) until one named `until`
    /// has come (then a little longer, for the rest of what came with it),
    /// or for at most `limit`; the queued events.
    fn pump_until(until: &str, limit: Duration) -> Vec<String> {
        let end = Instant::now() + limit;
        let mut seen = Vec::new();
        while Instant::now() < end {
            seen.extend(take_queued());
            if seen.iter().any(|e| e.starts_with(until)) {
                for _ in 0..20 {
                    std::thread::sleep(Duration::from_millis(10));
                    seen.extend(take_queued());
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        seen
    }

    fn rect(r: &Rect) -> String {
        format!("{},{} {}x{}", r.x, r.y, r.w, r.h)
    }

    fn mode(m: &DisplayMode) -> String {
        format!(
            "{}x{} fmt={} density={} refresh={} ({}/{}) display={}",
            m.w,
            m.h,
            m.format.name(),
            m.pixel_density,
            m.refresh_rate,
            m.refresh_rate_numerator,
            m.refresh_rate_denominator,
            m.display_id
        )
    }

    fn result<T>(r: sdl3::Result<T>, f: impl FnOnce(T) -> String) -> String {
        match r {
            Ok(v) => f(v),
            Err(e) => format!("error: {e}"),
        }
    }

    /// Everything a display's queries answer.
    fn display_state(label: &str, display: DisplayID) {
        t(format!(
            "{label}: display {display} name={} primary={}",
            result(video::display_name(display), |n| n),
            yes_no(video::primary_display().ok() == Some(display))
        ));
        t(format!(
            "{label}: bounds={} usable={}",
            result(video::display_bounds(display), |r| rect(&r)),
            result(video::display_usable_bounds(display), |r| rect(&r))
        ));
        t(format!(
            "{label}: content_scale={} natural={} current={}",
            result(video::display_content_scale(display), |s| s.to_string()),
            video::natural_display_orientation(display) as i32,
            video::current_display_orientation(display) as i32
        ));
        t(format!(
            "{label}: desktop mode {}",
            result(video::desktop_display_mode(display), |m| mode(&m))
        ));
        t(format!(
            "{label}: current mode {}",
            result(video::current_display_mode(display), |m| mode(&m))
        ));
        let modes = video::fullscreen_display_modes(display).unwrap_or_default();
        t(format!("{label}: fullscreen modes: {}", modes.len()));
        for (i, m) in modes.iter().enumerate() {
            t(format!("{label}: fullscreen mode {i}: {}", mode(m)));
        }
        for (w, h, refresh_rate, high_density) in [
            (393, 852, 0.0, true),
            (852, 393, 0.0, true),
            (393, 852, 0.0, false),
            (200, 200, 0.0, true),
            (393, 852, 30.0, true),
            (1000, 1000, 0.0, true),
            (0, 0, 0.0, true),
        ] {
            t(format!(
                "{label}: closest to {w}x{h} at {refresh_rate} high density {}: {}",
                yes_no(high_density),
                result(
                    video::closest_fullscreen_display_mode(
                        display,
                        w,
                        h,
                        refresh_rate,
                        high_density
                    ),
                    |m| mode(&m)
                )
            ));
        }
        let mut props = Vec::new();
        if let Ok(p) = video::display_properties(display) {
            let mut names = p.names();
            names.sort();
            for name in names {
                let value = match p.kind(&name) {
                    Some(ValueKind::Bool) => p.get_bool(&name).unwrap_or_default().to_string(),
                    Some(ValueKind::Number) => p.get_number(&name).unwrap_or_default().to_string(),
                    Some(ValueKind::Float) => p.get_float(&name).unwrap_or_default().to_string(),
                    Some(ValueKind::String) => p.get_string(&name).unwrap_or_default(),
                    Some(ValueKind::Any) => "(pointer)".into(),
                    None => "(none)".into(),
                };
                props.push(format!("{name}={value}"));
            }
        }
        t(format!("{label}: properties: {}", list(props)));
    }

    /// The displays, and each one's state.
    fn displays_state(label: &str) {
        let displays = video::displays().unwrap_or_default();
        t(format!(
            "{label}: displays: {}",
            list(displays.iter().map(|d| d.to_string()).collect())
        ));
        for display in displays {
            display_state(label, display);
        }
        for (x, y) in [(10, 10), (500, 100), (-50, -50), (1000, 1000)] {
            t(format!(
                "{label}: display for point {x},{y}: {}",
                result(video::display_for_point(Point::new(x, y)), |d| d
                    .to_string())
            ));
        }
        let r = Rect::new(300, 0, 200, 200);
        t(format!(
            "{label}: display for rect {}: {}",
            rect(&r),
            result(video::display_for_rect(&r), |d| d.to_string())
        ));
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    /// The app's (one) scene.
    fn scene() -> Id {
        // SAFETY: UIApplication's connected scenes (an NSSet).
        unsafe {
            let scenes: Id = objc::send0(objc::shared_application(), "connectedScenes");
            let all: Id = objc::send0(scenes, "allObjects");
            objc::send0(all, "firstObject")
        }
    }

    /// A window of the program's own in the scene, with a plain view
    /// controller (which turns to any orientation but upside down on an
    /// iPhone), key and visible; the view controller.
    fn make_window() -> Id {
        // SAFETY: UIWindow's and UIViewController's initializers (the
        // objects are kept for the rest of the process) and methods.
        unsafe {
            let window: Id = objc::send0(objc::class("UIWindow"), "alloc");
            let window: Id = objc::send1(window, "initWithWindowScene:", scene());
            let vc: Id = objc::send0(objc::class("UIViewController"), "alloc");
            let vc: Id = objc::send0(vc, "init");
            let () = objc::send1(window, "setRootViewController:", vc);
            let () = objc::send0(window, "makeKeyAndVisible");
            vc
        }
    }

    /// Ask the scene to turn to an orientation (a `UIInterfaceOrientationMask`).
    fn request_orientation(vc: Id, mask: usize) {
        // SAFETY: UIWindowSceneGeometryPreferencesIOS's initializer, the
        // view controller's and the scene's methods (iOS 16).
        unsafe {
            let prefs: Id =
                objc::send0(objc::class("UIWindowSceneGeometryPreferencesIOS"), "alloc");
            let prefs: Id = objc::send1(prefs, "initWithInterfaceOrientations:", mask);
            let () = objc::send0(vc, "setNeedsUpdateOfSupportedInterfaceOrientations");
            let () = objc::send2(
                scene(),
                "requestGeometryUpdateWithPreferences:errorHandler:",
                prefs,
                std::ptr::null_mut::<std::ffi::c_void>(),
            );
        }
    }

    fn main_screen() -> Id {
        // SAFETY: UIScreen's class method.
        unsafe { objc::send0(objc::class("UIScreen"), "mainScreen") }
    }

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_displays: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_displays: {:.3} s: main", elapsed());
        let r = init::init(InitFlags::VIDEO);
        t(format!(
            "init video: {}",
            r.as_ref().map_or_else(|e| e.to_string(), |_| "ok".into())
        ));
        r.unwrap();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap()
        ));
        t(format!("events: {}", list(take_queued())));
        displays_state("start");

        // The app becomes active around when the main function starts
        // (the events of its activation are left out: they may come before
        // SDL is up)
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            take_queued();
            std::thread::sleep(Duration::from_millis(10));
        }
        for _ in 0..10 {
            take_queued();
            std::thread::sleep(Duration::from_millis(10));
        }
        t(format!("application state: {}", application_state()));

        // A window of our own, to turn the app with
        let vc = make_window();
        for _ in 0..20 {
            take_queued();
            std::thread::sleep(Duration::from_millis(10));
        }

        // UIInterfaceOrientationMaskLandscapeRight, ...LandscapeLeft and
        // ...Portrait
        for (label, mask) in [
            ("landscape right", 1 << 3),
            ("landscape left", 1 << 4),
            ("portrait", 1 << 1),
        ] {
            println!("uikit_displays: {:.3} s: turning to {label}", elapsed());
            request_orientation(vc, mask);
            let seen = pump_until("DISPLAY_ORIENTATION", Duration::from_secs(10));
            println!("uikit_displays: {:.3} s: turned", elapsed());
            t(format!("{label}: events: {}", list(seen)));
            displays_state(label);
        }

        // The screens' notifications, for the main screen
        // SAFETY: UIKit's notification names.
        let (connect, disconnect) = unsafe {
            (
                objc::UIScreenDidConnectNotification,
                objc::UIScreenDidDisconnectNotification,
            )
        };
        for (label, name) in [
            ("connected", connect),
            ("disconnected", disconnect),
            ("disconnected again", disconnect),
            ("connected again", connect),
        ] {
            objc::post_notification_for(name, main_screen());
            t(format!("{label}: events: {}", list(take_queued())));
            displays_state(label);
        }

        // The video subsystem again
        init::quit_subsystem(InitFlags::VIDEO);
        t(format!(
            "quit video: displays: {}",
            result(video::displays(), |d| d.len().to_string())
        ));
        let r = init::init_subsystem(InitFlags::VIDEO);
        t(format!(
            "init video again: {}",
            r.as_ref().map_or_else(|e| e.to_string(), |_| "ok".into())
        ));
        r.unwrap();
        t(format!("events: {}", list(take_queued())));
        displays_state("again");

        init::quit();
        t("quit");

        check_trace(&TRACE.lock().unwrap(), EXPECTED);
    }
}
