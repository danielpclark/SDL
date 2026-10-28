// Tests of the UIKit video driver's windows: their flags, sizes,
// fullscreen and borders, orientations and animation callbacks.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the iOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as
//! `uikit_main` is. Its main function makes three windows in turn and
//! traces, after each step, the events that came and everything SDL and
//! UIKit say about the window: position, size, size in pixels, pixel
//! density, display scale, flags, safe area, display, fullscreen mode; the
//! `UIWindow` (frame, hidden, key, scene), its root view controller and
//! view (their classes, the view's frame), what the view controller
//! answers UIKit (status bar hidden, home indicator auto-hidden, the
//! screen edges deferring system gestures, the supported orientations);
//! and the window's properties.
//!
//! Window A (320x480, no flags: portrait only) is made, a second window on
//! the same display refused, hidden, shown, raised, titled, made
//! borderless and bordered again, fullscreen and windowed, exclusive
//! fullscreen in each fullscreen mode (the landscape one is refused in
//! portrait), and asked about the home indicator under each value of
//! `SDL_HINT_IOS_HIDE_HOME_INDICATOR` and about its orientations under
//! some values of `SDL_HINT_ORIENTATIONS`; the app is asked to turn to
//! landscape (which A doesn't support: nothing happens) and A is resized
//! (which UIKit's windows ignore). Window B (640x200, resizable, high
//! pixel density, hidden) is shown, turned in-process
//! (`requestGeometryUpdateWithPreferences:`, iOS 16) to landscape right
//! (and made fullscreen and windowed there), landscape left and portrait,
//! and given animation callbacks, whose frames are counted (each stops
//! itself after a number of frames; one is replaced before its first
//! frame). Window C is made fullscreen and made windowed.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5).
//! Upstream gives every window that asks for no graphics API the OpenGL
//! flag on UIKit; OpenGL ES isn't translated yet, so here such a window
//! gets the Metal flag, upstream's next default
//! (`SDL_DefaultGraphicsBackends()`), and the flags are traced without
//! either; upstream's window pointer property is a pointer, here a number
//! (as the other drivers' are), traced as `(pointer)` both ways; the
//! mice's device events are left out. Elsewhere this program does
//! nothing.

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
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, WindowID};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::properties::ValueKind;
    use sdl3::video::{self, Window};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    const EXPECTED: &str = "\
init video: ok
video driver: uikit
events: DISPLAY_ORIENTATION(1,3), KEYBOARD_ADDED
application state: 0
A: create 320x480: ok
A: created: events: A:WINDOW_SAFE_AREA_CHANGED(0,0), A:WINDOW_MOUSE_ENTER(0,0), A:WINDOW_FOCUS_GAINED(0,0), A:WINDOW_SHOWN(0,0), A:WINDOW_PIXEL_SIZE_CHANGED(393,852)
A: created: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: created: flags=0x600 safe_area=0,59 393x759 display=1
A: created: fullscreen mode (none)
A: created: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: created: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: created: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: created: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: another window: error: Only one window allowed per display.
A: another window: events: ?:WINDOW_DESTROYED(0,0)
A: hide: ok
A: hidden: events: A:WINDOW_HIDDEN(0,0)
A: hidden: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: hidden: flags=0x608 safe_area=0,59 393x759 display=1
A: hidden: fullscreen mode (none)
A: hidden: uiwindow: UIWindow frame=0,0 393x852 hidden=yes key=no scene=yes
A: hidden: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: hidden: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: hidden: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: show: ok
A: shown: events: A:WINDOW_SHOWN(0,0)
A: shown: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: shown: flags=0x600 safe_area=0,59 393x759 display=1
A: shown: fullscreen mode (none)
A: shown: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: shown: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: shown: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: shown: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: raise: ok
A: raised: events: (none)
A: title: ok
A: title: window=Title A view controller=Title A
A: borderless: ok
A: borderless: events: (none)
A: borderless: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: borderless: flags=0x610 safe_area=0,59 393x759 display=1
A: borderless: fullscreen mode (none)
A: borderless: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: borderless: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: borderless: status bar hidden=yes home indicator auto hidden=no edges=15 orientations=2
A: borderless: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: bordered: ok
A: bordered: events: (none)
A: bordered: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: bordered: flags=0x600 safe_area=0,59 393x759 display=1
A: bordered: fullscreen mode (none)
A: bordered: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: bordered: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: bordered: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: bordered: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: fullscreen: ok
A: sync: ok
A: fullscreen: events: A:WINDOW_ENTER_FULLSCREEN(0,0)
A: fullscreen: pos=0,0 size=393x852 pixels=393x852 density=1 display_scale=1
A: fullscreen: flags=0x601 safe_area=0,59 393x759 display=1
A: fullscreen: fullscreen mode (none)
A: fullscreen: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: fullscreen: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: fullscreen: status bar hidden=yes home indicator auto hidden=no edges=15 orientations=2
A: fullscreen: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: windowed: ok
A: sync: ok
A: windowed: events: A:WINDOW_LEAVE_FULLSCREEN(0,0)
A: windowed: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: windowed: flags=0x600 safe_area=0,59 393x759 display=1
A: windowed: fullscreen mode (none)
A: windowed: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: windowed: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: windowed: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: windowed: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: exclusive 852x393: set mode: ok
A: exclusive 852x393: fullscreen: error: Screen orientation does not match display mode size
A: exclusive 852x393: sync: ok
A: exclusive 852x393: events: (none)
A: exclusive 852x393: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: exclusive 852x393: flags=0x600 safe_area=0,59 393x759 display=1
A: exclusive 852x393: fullscreen mode 852x393 density=3
A: exclusive 852x393: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: exclusive 852x393: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: exclusive 852x393: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: exclusive 852x393: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: exclusive 852x393: current mode 393x852
A: exclusive 852x393: windowed: ok
A: exclusive 852x393: sync: ok
A: exclusive 852x393: unset mode: ok
A: exclusive 852x393 left: events: (none)
A: exclusive 852x393 left: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: exclusive 852x393 left: flags=0x600 safe_area=0,59 393x759 display=1
A: exclusive 852x393 left: fullscreen mode (none)
A: exclusive 852x393 left: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: exclusive 852x393 left: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: exclusive 852x393 left: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: exclusive 852x393 left: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: exclusive 852x393 left: current mode 393x852
A: exclusive 393x852: set mode: ok
A: exclusive 393x852: fullscreen: ok
A: exclusive 393x852: sync: ok
A: exclusive 393x852: events: A:WINDOW_ENTER_FULLSCREEN(0,0)
A: exclusive 393x852: pos=0,0 size=393x852 pixels=393x852 density=1 display_scale=1
A: exclusive 393x852: flags=0x601 safe_area=0,59 393x759 display=1
A: exclusive 393x852: fullscreen mode 393x852 density=3
A: exclusive 393x852: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: exclusive 393x852: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: exclusive 393x852: status bar hidden=yes home indicator auto hidden=no edges=15 orientations=2
A: exclusive 393x852: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: exclusive 393x852: current mode 393x852
A: exclusive 393x852: windowed: ok
A: exclusive 393x852: sync: ok
A: exclusive 393x852: unset mode: ok
A: exclusive 393x852 left: events: A:WINDOW_LEAVE_FULLSCREEN(0,0)
A: exclusive 393x852 left: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: exclusive 393x852 left: flags=0x600 safe_area=0,59 393x759 display=1
A: exclusive 393x852 left: fullscreen mode (none)
A: exclusive 393x852 left: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: exclusive 393x852 left: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: exclusive 393x852 left: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: exclusive 393x852 left: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: exclusive 393x852 left: current mode 393x852
A: home indicator hint 1: auto hidden=yes edges=0
A: home indicator hint 2: auto hidden=no edges=15
A: home indicator hint 0: auto hidden=no edges=0
A: home indicator hint (unset): auto hidden=no edges=0
A: borderless: home indicator hint (unset): auto hidden=no edges=15
A: borderless: home indicator hint 0: auto hidden=no edges=0
A: borderless: home indicator hint (unset): auto hidden=no edges=15
A: home indicator: events: (none)
A: orientations hint (unset): supported=2
A: orientations hint LandscapeLeft: supported=16
A: orientations hint LandscapeRight Portrait: supported=10
A: orientations hint PortraitUpsideDown: supported=30
A: orientations hint PortraitUpsideDown LandscapeLeft: supported=16
A: orientations hint Sideways: supported=2
A: orientations hint (unset): supported=2
A: landscape right (unsupported): events: (none)
A: landscape right (unsupported): pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: landscape right (unsupported): flags=0x600 safe_area=0,59 393x759 display=1
A: landscape right (unsupported): fullscreen mode (none)
A: landscape right (unsupported): uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: landscape right (unsupported): root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: landscape right (unsupported): status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: landscape right (unsupported): properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: resize to 100x100: ok
A: resized: events: (none)
A: resized: pos=36,186 size=393x852 pixels=393x852 density=1 display_scale=1
A: resized: flags=0x600 safe_area=0,59 393x759 display=1
A: resized: fullscreen mode (none)
A: resized: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
A: resized: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
A: resized: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
A: resized: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: resized: orientations hint (unset): supported=2
A: destroyed
A: destroyed: events: A:WINDOW_DESTROYED(0,0)
B: create 640x200 resizable high-density hidden: ok
B: created: events: B:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), B:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
B: created: pos=-123,326 size=393x852 pixels=1179x2556 density=3 display_scale=3
B: created: flags=0x2028 safe_area=0,0 393x852 display=1
B: created: fullscreen mode (none)
B: created: uiwindow: UIWindow frame=0,0 393x852 hidden=yes key=no scene=yes
B: created: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
B: created: status bar hidden=no home indicator auto hidden=no edges=0 orientations=26
B: created: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
B: show: ok
B: shown: events: B:WINDOW_SAFE_AREA_CHANGED(0,0), B:WINDOW_MOUSE_ENTER(0,0), B:WINDOW_FOCUS_GAINED(0,0), B:WINDOW_SHOWN(0,0)
B: shown: pos=-123,326 size=393x852 pixels=1179x2556 density=3 display_scale=3
B: shown: flags=0x2620 safe_area=0,59 393x759 display=1
B: shown: fullscreen mode (none)
B: shown: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
B: shown: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
B: shown: status bar hidden=no home indicator auto hidden=no edges=0 orientations=26
B: shown: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
B: landscape right: events: DISPLAY_DESKTOP_MODE_CHANGED(1,852), DISPLAY_CURRENT_MODE_CHANGED(1,852), DISPLAY_ORIENTATION(1,1), B:WINDOW_RESIZED(852,393), B:WINDOW_PIXEL_SIZE_CHANGED(2556,1179), B:WINDOW_SAFE_AREA_CHANGED(0,0)
B: landscape right: pos=-123,326 size=852x393 pixels=2556x1179 density=3 display_scale=3
B: landscape right: flags=0x2620 safe_area=59,0 734x372 display=1
B: landscape right: fullscreen mode (none)
B: landscape right: uiwindow: UIWindow frame=0,0 852x393 hidden=no key=yes scene=yes
B: landscape right: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 852x393
B: landscape right: status bar hidden=no home indicator auto hidden=no edges=0 orientations=26
B: landscape right: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
B: fullscreen: ok
B: sync: ok
B: fullscreen in landscape: events: B:WINDOW_ENTER_FULLSCREEN(0,0)
B: fullscreen in landscape: pos=0,0 size=852x393 pixels=2556x1179 density=3 display_scale=3
B: fullscreen in landscape: flags=0x2621 safe_area=59,0 734x372 display=1
B: fullscreen in landscape: fullscreen mode (none)
B: fullscreen in landscape: uiwindow: UIWindow frame=0,0 852x393 hidden=no key=yes scene=yes
B: fullscreen in landscape: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 852x393
B: fullscreen in landscape: status bar hidden=yes home indicator auto hidden=no edges=15 orientations=26
B: fullscreen in landscape: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
B: windowed: ok
B: sync: ok
B: windowed in landscape: events: B:WINDOW_LEAVE_FULLSCREEN(0,0)
B: landscape left: events: DISPLAY_ORIENTATION(1,2)
B: landscape left: pos=-123,326 size=852x393 pixels=2556x1179 density=3 display_scale=3
B: landscape left: flags=0x2620 safe_area=59,0 734x372 display=1
B: landscape left: fullscreen mode (none)
B: landscape left: uiwindow: UIWindow frame=0,0 852x393 hidden=no key=yes scene=yes
B: landscape left: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 852x393
B: landscape left: status bar hidden=no home indicator auto hidden=no edges=0 orientations=26
B: landscape left: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
B: portrait: events: DISPLAY_DESKTOP_MODE_CHANGED(1,393), DISPLAY_CURRENT_MODE_CHANGED(1,393), DISPLAY_ORIENTATION(1,3), B:WINDOW_RESIZED(393,852), B:WINDOW_PIXEL_SIZE_CHANGED(1179,2556), B:WINDOW_SAFE_AREA_CHANGED(0,0)
B: portrait: pos=-123,326 size=393x852 pixels=1179x2556 density=3 display_scale=3
B: portrait: flags=0x2620 safe_area=0,59 393x759 display=1
B: portrait: fullscreen mode (none)
B: portrait: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
B: portrait: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
B: portrait: status bar hidden=no home indicator auto hidden=no edges=0 orientations=26
B: portrait: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
B: animation every frame: ok
B: animation every frame: 30 frames
B: animation every 3rd frame: ok
B: animation every 3rd frame: 10 frames
B: animation replaced: ok
B: animation replaced: ok
B: animation replaced: 0 and 5 frames
B: animation stopped: ok
B: animation: events: (none)
B: destroyed
B: destroyed: events: B:WINDOW_DESTROYED(0,0)
animation without a window: error: Invalid window
C: create 200x300 fullscreen: ok
C: created: events: C:WINDOW_SAFE_AREA_CHANGED(0,0), C:WINDOW_MOUSE_ENTER(0,0), C:WINDOW_FOCUS_GAINED(0,0), C:WINDOW_SHOWN(0,0), C:WINDOW_ENTER_FULLSCREEN(0,0), C:WINDOW_PIXEL_SIZE_CHANGED(393,852)
C: created: pos=0,0 size=393x852 pixels=393x852 density=1 display_scale=1
C: created: flags=0x601 safe_area=0,59 393x759 display=1
C: created: fullscreen mode (none)
C: created: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
C: created: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
C: created: status bar hidden=yes home indicator auto hidden=no edges=15 orientations=2
C: created: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
C: windowed: ok
C: sync: ok
C: windowed: events: C:WINDOW_LEAVE_FULLSCREEN(0,0)
C: windowed: pos=0,0 size=393x852 pixels=393x852 density=1 display_scale=1
C: windowed: flags=0x600 safe_area=0,59 393x759 display=1
C: windowed: fullscreen mode (none)
C: windowed: uiwindow: UIWindow frame=0,0 393x852 hidden=no key=yes scene=yes
C: windowed: root: SDL_uikitviewcontroller view=SDL_uikitview frame=0,0 393x852
C: windowed: status bar hidden=no home indicator auto hidden=no edges=0 orientations=2
C: windowed: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
C: destroyed
C: destroyed: events: C:WINDOW_DESTROYED(0,0)
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

    /// The windows the program made, by ID, for the events' labels.
    static LABELS: Mutex<Vec<(WindowID, &'static str)>> = Mutex::new(Vec::new());

    fn name_window(w: &Window, label: &'static str) {
        LABELS.lock().unwrap().push((w.id(), label));
    }

    fn window_label(id: WindowID) -> &'static str {
        LABELS
            .lock()
            .unwrap()
            .iter()
            .find(|(i, _)| *i == id)
            .map_or("?", |(_, l)| l)
    }

    /// An event as the trace names it (the C program's names), `None` for
    /// the mice's device events (see the module's description).
    fn describe(event: &Event) -> Option<String> {
        let name = event_name(event.event_type());
        Some(match event {
            Event::MouseDevice(_) => return None,
            Event::Display(d) => format!("{name}({},{})", d.display_id, d.data1),
            Event::Window(w) => format!(
                "{}:{name}({},{})",
                window_label(w.window_id),
                w.data1,
                w.data2
            ),
            _ => name,
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

    /// Pump until an event containing `until` has come (then for
    /// `after_ms` more), or for at most `limit`.
    fn pump_until(out: &mut Vec<String>, until: &str, limit: Duration, after_ms: u64) {
        let end = Instant::now() + limit;
        while Instant::now() < end {
            take_queued(out);
            if out.iter().any(|e| e.contains(until)) {
                settle(out, after_ms);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn trace_events(label: &str, ms: u64) {
        let mut seen = Vec::new();
        settle(&mut seen, ms);
        t(format!("{label}: events: {}", list(seen)));
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

    fn rect(r: CGRect) -> String {
        format!("{},{} {}x{}", r.x, r.y, r.w, r.h)
    }

    /// The window's `UIWindow` (its property, a number here).
    fn uiwindow_of(w: &Window) -> Id {
        w.properties()
            .ok()
            .and_then(|p| p.get_number(video::window::PROP_WINDOW_UIKIT_WINDOW_POINTER))
            .map_or(std::ptr::null_mut(), |p| p as usize as Id)
    }

    /// Its root view controller.
    fn vc_of(w: &Window) -> Id {
        // SAFETY: a UIWindow property.
        unsafe { objc::send0(uiwindow_of(w), "rootViewController") }
    }

    fn props_state(label: &str, w: &Window) {
        let mut props = Vec::new();
        if let Ok(p) = w.properties() {
            let mut names = p.names();
            names.sort();
            for name in names {
                let value = match p.kind(&name) {
                    // (see the module's description)
                    Some(ValueKind::Number)
                        if name == video::window::PROP_WINDOW_UIKIT_WINDOW_POINTER =>
                    {
                        "(pointer)".into()
                    }
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

    /// What SDL and UIKit say about a window (without the OpenGL flag: see
    /// the module's description).
    fn window_state(label: &str, w: &Window) {
        let (x, y) = w.position().unwrap_or_default();
        let (ww, wh) = w.size().unwrap_or_default();
        let (pw, ph) = w.size_in_pixels().unwrap_or_default();
        let safe = w.safe_area().unwrap_or_default();
        t(format!(
            "{label}: pos={x},{y} size={ww}x{wh} pixels={pw}x{ph} density={} display_scale={}",
            w.pixel_density().unwrap_or_default(),
            w.display_scale().unwrap_or_default()
        ));
        let flags =
            w.flags().unwrap_or_default().0 & !(WindowFlags::OPENGL.0 | WindowFlags::METAL.0);
        t(format!(
            "{label}: flags={flags:#x} safe_area={},{} {}x{} display={}",
            safe.x,
            safe.y,
            safe.w,
            safe.h,
            w.display().unwrap_or(0)
        ));
        match w.fullscreen_mode().ok().flatten() {
            Some(m) => t(format!(
                "{label}: fullscreen mode {}x{} density={}",
                m.w, m.h, m.pixel_density
            )),
            None => t(format!("{label}: fullscreen mode (none)")),
        }
        let uw = uiwindow_of(w);
        // SAFETY: UIWindow's, UIViewController's and UIView's properties.
        unsafe {
            let vc: Id = objc::send0(uw, "rootViewController");
            let frame: CGRect = objc::send0(uw, "frame");
            let hidden: bool = objc::send0(uw, "isHidden");
            let key: bool = objc::send0(uw, "isKeyWindow");
            let scene: Id = objc::send0(uw, "windowScene");
            t(format!(
                "{label}: uiwindow: {} frame={} hidden={} key={} scene={}",
                objc::class_name(uw),
                rect(frame),
                yes_no(hidden),
                yes_no(key),
                yes_no(!scene.is_null())
            ));
            let view: Id = objc::send0(vc, "view");
            let view_frame: CGRect = if view.is_null() {
                CGRect::default()
            } else {
                objc::send0(view, "frame")
            };
            t(format!(
                "{label}: root: {} view={} frame={}",
                objc::class_name(vc),
                objc::class_name(view),
                rect(view_frame)
            ));
            let status_bar_hidden: bool = objc::send0(vc, "prefersStatusBarHidden");
            let home_hidden: bool = objc::send0(vc, "prefersHomeIndicatorAutoHidden");
            let edges: usize = objc::send0(vc, "preferredScreenEdgesDeferringSystemGestures");
            let orientations: usize = objc::send0(vc, "supportedInterfaceOrientations");
            t(format!(
                "{label}: status bar hidden={} home indicator auto hidden={} edges={edges} orientations={orientations}",
                yes_no(status_bar_hidden),
                yes_no(home_hidden)
            ));
        }
        props_state(label, w);
    }

    /// Ask the window's scene to turn to an orientation (a
    /// `UIInterfaceOrientationMask`), and trace the events that come.
    fn turn(w: &Window, label: &str, mask: usize) {
        let mut seen = Vec::new();
        println!("uikit_windows: {:.3} s: turning to {label}", elapsed());
        // SAFETY: UIWindowSceneGeometryPreferencesIOS's initializer, the
        // view controller's and the scene's methods (iOS 16).
        unsafe {
            let prefs: Id =
                objc::send0(objc::class("UIWindowSceneGeometryPreferencesIOS"), "alloc");
            let prefs: Id = objc::send1(prefs, "initWithInterfaceOrientations:", mask);
            let () = objc::send0(vc_of(w), "setNeedsUpdateOfSupportedInterfaceOrientations");
            let scene: Id = objc::send0(uiwindow_of(w), "windowScene");
            let () = objc::send2(
                scene,
                "requestGeometryUpdateWithPreferences:errorHandler:",
                prefs,
                std::ptr::null_mut::<std::ffi::c_void>(),
            );
        }
        pump_until(
            &mut seen,
            "DISPLAY_ORIENTATION",
            Duration::from_secs(5),
            1500,
        );
        println!("uikit_windows: {:.3} s: turned", elapsed());
        t(format!("{label}: events: {}", list(seen)));
    }

    /// Pump until `count` has reached `n` (or for 10 s), then a little
    /// longer (the count stays).
    fn wait_frames(count: &Cell<i32>, n: i32) {
        let end = Instant::now() + Duration::from_secs(10);
        let mut ignored = Vec::new();
        while count.get() < n && Instant::now() < end {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(5));
        }
        for _ in 0..20 {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// An animation callback that counts its frames, and stops itself
    /// after `stop_at` of them.
    fn count_and_stop(w: Window, count: Rc<Cell<i32>>, stop_at: i32) -> Box<dyn FnMut()> {
        Box::new(move || {
            count.set(count.get() + 1);
            if count.get() == stop_at {
                w.set_ios_animation_callback(1, None).unwrap();
            }
        })
    }

    /// Set (`Some`) or unset `SDL_HINT_IOS_HIDE_HOME_INDICATOR`, and what
    /// the view controller says.
    fn home_indicator(label: &str, w: &Window, hint: Option<&str>) {
        hints::set_with_priority(
            hints::IOS_HIDE_HOME_INDICATOR,
            hint,
            hints::Priority::Normal,
        )
        .unwrap();
        let vc = vc_of(w);
        // SAFETY: the view controller's properties.
        let (hidden, edges): (bool, usize) = unsafe {
            (
                objc::send0(vc, "prefersHomeIndicatorAutoHidden"),
                objc::send0(vc, "preferredScreenEdgesDeferringSystemGestures"),
            )
        };
        t(format!(
            "{label}: home indicator hint {}: auto hidden={} edges={edges}",
            hint.unwrap_or("(unset)"),
            yes_no(hidden)
        ));
    }

    /// Set (`Some`) or unset `SDL_HINT_ORIENTATIONS`, and the orientations
    /// the view controller supports.
    fn orientations(label: &str, w: &Window, hint: Option<&str>) {
        hints::set_with_priority(hints::ORIENTATIONS, hint, hints::Priority::Normal).unwrap();
        // SAFETY: a view controller property.
        let supported: usize = unsafe { objc::send0(vc_of(w), "supportedInterfaceOrientations") };
        t(format!(
            "{label}: orientations hint {}: supported={supported}",
            hint.unwrap_or("(unset)")
        ));
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
        println!("uikit_windows: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_windows: {:.3} s: main", elapsed());
        let r = init::init(InitFlags::VIDEO);
        t(format!("init video: {}", ok(&r)));
        r.unwrap();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap()
        ));
        let mut seen = Vec::new();
        take_queued(&mut seen);
        t(format!("events: {}", list(seen)));

        // The app becomes active around when the main function starts
        // (the events of its activation are left out: they may come before
        // SDL is up)
        let mut ignored = Vec::new();
        let end = Instant::now() + Duration::from_secs(30);
        while application_state() != 0 && Instant::now() < end {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
        for _ in 0..10 {
            take_queued(&mut ignored);
            std::thread::sleep(Duration::from_millis(10));
        }
        t(format!("application state: {}", application_state()));

        // A: a plain window (portrait only, as it's taller than wide)
        let r = Window::create("A", 320, 480, WindowFlags::NONE);
        t(format!("A: create 320x480: {}", ok(&r)));
        let a = r.unwrap();
        name_window(&a, "A");
        trace_events("A: created", 500);
        window_state("A: created", &a);

        let other = Window::create("other", 100, 100, WindowFlags::NONE);
        t(format!("A: another window: {}", ok(&other)));
        trace_events("A: another window", 200);

        t(format!("A: hide: {}", ok(&a.hide())));
        trace_events("A: hidden", 500);
        window_state("A: hidden", &a);
        t(format!("A: show: {}", ok(&a.show())));
        trace_events("A: shown", 500);
        window_state("A: shown", &a);
        t(format!("A: raise: {}", ok(&a.raise())));
        trace_events("A: raised", 200);

        t(format!("A: title: {}", ok(&a.set_title("Title A"))));
        // SAFETY: a view controller property, an NSString.
        let vc_title = objc::string(unsafe { objc::send0(vc_of(&a), "title") });
        t(format!(
            "A: title: window={} view controller={}",
            a.title().unwrap_or_default(),
            vc_title.unwrap_or_default()
        ));

        t(format!("A: borderless: {}", ok(&a.set_bordered(false))));
        trace_events("A: borderless", 1000);
        window_state("A: borderless", &a);
        t(format!("A: bordered: {}", ok(&a.set_bordered(true))));
        trace_events("A: bordered", 1000);
        window_state("A: bordered", &a);

        t(format!("A: fullscreen: {}", ok(&a.set_fullscreen(true))));
        t(format!("A: sync: {}", ok(&a.sync())));
        trace_events("A: fullscreen", 1000);
        window_state("A: fullscreen", &a);
        t(format!("A: windowed: {}", ok(&a.set_fullscreen(false))));
        t(format!("A: sync: {}", ok(&a.sync())));
        trace_events("A: windowed", 1000);
        window_state("A: windowed", &a);

        let primary = video::primary_display().unwrap();
        for mode in video::fullscreen_display_modes(primary).unwrap_or_default() {
            let label = format!("A: exclusive {}x{}", mode.w, mode.h);
            t(format!(
                "{label}: set mode: {}",
                ok(&a.set_fullscreen_mode(Some(&mode)))
            ));
            t(format!(
                "{label}: fullscreen: {}",
                ok(&a.set_fullscreen(true))
            ));
            t(format!("{label}: sync: {}", ok(&a.sync())));
            trace_events(&label, 1000);
            window_state(&label, &a);
            let cm = video::current_display_mode(primary).ok();
            t(format!(
                "{label}: current mode {}x{}",
                cm.map_or(0, |m| m.w),
                cm.map_or(0, |m| m.h)
            ));
            t(format!(
                "{label}: windowed: {}",
                ok(&a.set_fullscreen(false))
            ));
            t(format!("{label}: sync: {}", ok(&a.sync())));
            t(format!(
                "{label}: unset mode: {}",
                ok(&a.set_fullscreen_mode(None))
            ));
            let label = format!("A: exclusive {}x{} left", mode.w, mode.h);
            trace_events(&label, 1000);
            window_state(&label, &a);
            let cm = video::current_display_mode(primary).ok();
            t(format!(
                "{label}: current mode {}x{}",
                cm.map_or(0, |m| m.w),
                cm.map_or(0, |m| m.h)
            ));
        }

        home_indicator("A", &a, Some("1"));
        home_indicator("A", &a, Some("2"));
        home_indicator("A", &a, Some("0"));
        home_indicator("A", &a, None);
        let _ = a.set_bordered(false);
        home_indicator("A: borderless", &a, None);
        home_indicator("A: borderless", &a, Some("0"));
        home_indicator("A: borderless", &a, None);
        let _ = a.set_bordered(true);
        trace_events("A: home indicator", 500);

        orientations("A", &a, None);
        orientations("A", &a, Some("LandscapeLeft"));
        orientations("A", &a, Some("LandscapeRight Portrait"));
        orientations("A", &a, Some("PortraitUpsideDown"));
        orientations("A", &a, Some("PortraitUpsideDown LandscapeLeft"));
        orientations("A", &a, Some("Sideways"));
        orientations("A", &a, None);

        // UIInterfaceOrientationMaskLandscapeRight, ...LandscapeLeft and
        // ...Portrait
        const LANDSCAPE_RIGHT: usize = 1 << 3;
        const LANDSCAPE_LEFT: usize = 1 << 4;
        const PORTRAIT: usize = 1 << 1;

        turn(&a, "A: landscape right (unsupported)", LANDSCAPE_RIGHT);
        window_state("A: landscape right (unsupported)", &a);

        t(format!(
            "A: resize to 100x100: {}",
            ok(&a.set_size(100, 100))
        ));
        trace_events("A: resized", 500);
        window_state("A: resized", &a);
        orientations("A: resized", &a, None);

        a.destroy();
        t("A: destroyed");
        trace_events("A: destroyed", 500);

        // B: resizable, high density, made hidden
        let r = Window::create(
            "B",
            640,
            200,
            WindowFlags::RESIZABLE | WindowFlags::HIGH_PIXEL_DENSITY | WindowFlags::HIDDEN,
        );
        t(format!(
            "B: create 640x200 resizable high-density hidden: {}",
            ok(&r)
        ));
        let b = r.unwrap();
        name_window(&b, "B");
        trace_events("B: created", 500);
        window_state("B: created", &b);
        t(format!("B: show: {}", ok(&b.show())));
        trace_events("B: shown", 500);
        window_state("B: shown", &b);

        turn(&b, "B: landscape right", LANDSCAPE_RIGHT);
        window_state("B: landscape right", &b);
        t(format!("B: fullscreen: {}", ok(&b.set_fullscreen(true))));
        t(format!("B: sync: {}", ok(&b.sync())));
        trace_events("B: fullscreen in landscape", 1000);
        window_state("B: fullscreen in landscape", &b);
        t(format!("B: windowed: {}", ok(&b.set_fullscreen(false))));
        t(format!("B: sync: {}", ok(&b.sync())));
        trace_events("B: windowed in landscape", 1000);
        turn(&b, "B: landscape left", LANDSCAPE_LEFT);
        window_state("B: landscape left", &b);
        turn(&b, "B: portrait", PORTRAIT);
        window_state("B: portrait", &b);

        // The animation callback: frames counted, each run stopped from the callback
        let every = Rc::new(Cell::new(0));
        t(format!(
            "B: animation every frame: {}",
            ok(&b.set_ios_animation_callback(1, Some(count_and_stop(b, every.clone(), 30))))
        ));
        wait_frames(&every, 30);
        t(format!("B: animation every frame: {} frames", every.get()));
        let third = Rc::new(Cell::new(0));
        t(format!(
            "B: animation every 3rd frame: {}",
            ok(&b.set_ios_animation_callback(3, Some(count_and_stop(b, third.clone(), 10))))
        ));
        wait_frames(&third, 10);
        t(format!(
            "B: animation every 3rd frame: {} frames",
            third.get()
        ));
        let replaced = Rc::new(Cell::new(0));
        let replacing = Rc::new(Cell::new(0));
        t(format!(
            "B: animation replaced: {}",
            ok(&b.set_ios_animation_callback(0, Some(count_and_stop(b, replaced.clone(), 0))))
        ));
        t(format!(
            "B: animation replaced: {}",
            ok(&b.set_ios_animation_callback(-1, Some(count_and_stop(b, replacing.clone(), 5))))
        ));
        wait_frames(&replacing, 5);
        t(format!(
            "B: animation replaced: {} and {} frames",
            replaced.get(),
            replacing.get()
        ));
        t(format!(
            "B: animation stopped: {}",
            ok(&b.set_ios_animation_callback(1, None))
        ));
        trace_events("B: animation", 200);

        b.destroy();
        t("B: destroyed");
        trace_events("B: destroyed", 500);
        // (upstream's is called without a window; here with B's, gone)
        t(format!(
            "animation without a window: {}",
            ok(&b.set_ios_animation_callback(1, Some(count_and_stop(b, replaced.clone(), 0))))
        ));

        // C: fullscreen from the start
        let r = Window::create("C", 200, 300, WindowFlags::FULLSCREEN);
        t(format!("C: create 200x300 fullscreen: {}", ok(&r)));
        let c = r.unwrap();
        name_window(&c, "C");
        trace_events("C: created", 1000);
        window_state("C: created", &c);
        t(format!("C: windowed: {}", ok(&c.set_fullscreen(false))));
        t(format!("C: sync: {}", ok(&c.sync())));
        trace_events("C: windowed", 1000);
        window_state("C: windowed", &c);
        c.destroy();
        t("C: destroyed");
        trace_events("C: destroyed", 500);

        init::quit();
        t("quit");

        check_trace(&TRACE.lock().unwrap(), EXPECTED);
    }
}
