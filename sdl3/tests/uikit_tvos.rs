// Tests of the UIKit video driver on tvOS: the display, the windows, the
// Apple TV's file system and the clipboard.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the tvOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as the
//! iOS programs (`uikit_*`) are on the iOS Simulator. Its main function
//! traces the platform and the drivers, the power state (no battery), the
//! file system (the pref path is in Caches, which the system may empty:
//! there's no other place an Apple TV app may keep files; there are no
//! user folders; a relative path is in the empty pref path), everything
//! the display answers (its one mode, no orientation), the screens'
//! connection and disconnection notifications posted in-process, then
//! windows: a plain one hidden and shown, given a title, made borderless,
//! fullscreen and exclusive fullscreen in the display's mode, resized
//! (with `SDL_HINT_ORIENTATIONS` set, which tvOS has no use for); a
//! resizable high density one made hidden with
//! `SDL_HINT_APPLE_TV_CONTROLLER_UI_EVENTS`, and given animation
//! callbacks; one fullscreen from the start. Of each it traces what SDL and
//! UIKit say: sizes in points and pixels, flags (but OpenGL, which a UIKit
//! window that asks for no graphics API gets), the safe area (the TV's
//! overscan margins), the `UIWindow`, its root view controller (a
//! `GCEventViewController` on tvOS) and whether the game controllers'
//! events go to UIKit (`controllerUserInteractionEnabled`, which the hint
//! sets as it changes), the view's gesture recognizers (a swipe for each
//! direction of the Siri Remote's touch surface) and the properties.
//! Last, the clipboard, which tvOS doesn't have (no `UIPasteboard`):
//! setting its text fails, and there's none.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an Apple TV 4K (3rd
//! generation) with tvOS 18.5). Paths in the app's container start with
//! `HOME`, and the mice's device events are left out. Elsewhere this
//! program does nothing.

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
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, DisplayID, Event, WindowID};
    use sdl3::filesystem::{self, Folder, PathType};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::properties::{Properties, ValueKind};
    use sdl3::video::{self, DisplayMode, Point, Rect, Window};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an Apple TV 4K (3rd generation)
    /// simulator with tvOS 18.5).
    const EXPECTED: &str = "\
platform: iOS
init video and audio: ok
video driver: uikit; audio driver: coreaudio
video drivers: uikit, offscreen, dummy
render drivers: metal, opengles2, vulkan, gpu, software
camera drivers: dummy
events: KEYBOARD_ADDED, AUDIO_DEVICE_ADDED, AUDIO_DEVICE_ADDED
application state: 0
idiom: 2
power: state=2 secs=-1 pct=-1
pref path: HOME/Library/Caches/org.libsdl/tvos test/
pref path: HOME/Library/Caches/org.libsdl/tvos test/: type=2 size=0
pref path without org: HOME/Library/Caches/tvos test/
pref path empty: HOME/Library/Caches//
base path: ends with .app/: yes
user folder home: error: tvOS does not have persistent storage
user folder documents: error: tvOS does not have persistent storage
user folder saved games: error: tvOS does not have persistent storage
user folder desktop: error: tvOS does not have persistent storage
user folder downloads: error: tvOS does not have persistent storage
relative file written: ok
relative file: HOME/Library/Caches//tvos-relative.txt: type=1 size=5
relative file removed: ok
relative file: HOME/Library/Caches//tvos-relative.txt: error: Can't stat: No such file or directory
start: displays: 1
start: display 1 name=1 primary=yes
start: bounds=0,0 1920x1080 usable=0,0 1920x1080
start: content_scale=1 natural=1 current=1
start: desktop mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
start: current mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
start: fullscreen modes: 1
start: fullscreen mode 0: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
start: closest to 1920x1080 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
start: closest to 1920x1080 at 0 high density no: error: Couldn't find any matching video modes
start: closest to 1080x1920 at 0 high density yes: error: Couldn't find any matching video modes
start: closest to 1280x720 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
start: closest to 3840x2160 at 0 high density yes: error: Couldn't find any matching video modes
start: closest to 1920x1080 at 30 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
start: closest to 0x0 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
start: properties: SDL.display.HDR_enabled=false
start: display for point 10,10: 1
start: display for point 1900,1000: 1
start: display for point -50,-50: 1
connected: events: DISPLAY_ADDED(2,0)
connected: displays: 1, 2
connected: display 1 name=1 primary=yes
connected: bounds=0,0 1920x1080 usable=0,0 1920x1080
connected: content_scale=1 natural=1 current=1
connected: desktop mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
connected: current mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
connected: fullscreen modes: 1
connected: fullscreen mode 0: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
connected: closest to 1920x1080 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
connected: closest to 1920x1080 at 0 high density no: error: Couldn't find any matching video modes
connected: closest to 1080x1920 at 0 high density yes: error: Couldn't find any matching video modes
connected: closest to 1280x720 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
connected: closest to 3840x2160 at 0 high density yes: error: Couldn't find any matching video modes
connected: closest to 1920x1080 at 30 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
connected: closest to 0x0 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=1
connected: properties: SDL.display.HDR_enabled=false
connected: display 2 name=2 primary=no
connected: bounds=1920,0 1920x1080 usable=1920,0 1920x1080
connected: content_scale=1 natural=1 current=1
connected: desktop mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected: current mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected: fullscreen modes: 1
connected: fullscreen mode 0: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected: closest to 1920x1080 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected: closest to 1920x1080 at 0 high density no: error: Couldn't find any matching video modes
connected: closest to 1080x1920 at 0 high density yes: error: Couldn't find any matching video modes
connected: closest to 1280x720 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected: closest to 3840x2160 at 0 high density yes: error: Couldn't find any matching video modes
connected: closest to 1920x1080 at 30 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected: closest to 0x0 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected: properties: SDL.display.HDR_enabled=false
connected: display for point 10,10: 1
connected: display for point 1900,1000: 1
connected: display for point -50,-50: 1
disconnected: events: DISPLAY_REMOVED(1,0)
disconnected: displays: 2
disconnected: display 2 name=2 primary=yes
disconnected: bounds=0,0 1920x1080 usable=0,0 1920x1080
disconnected: content_scale=1 natural=1 current=1
disconnected: desktop mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
disconnected: current mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
disconnected: fullscreen modes: 1
disconnected: fullscreen mode 0: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
disconnected: closest to 1920x1080 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
disconnected: closest to 1920x1080 at 0 high density no: error: Couldn't find any matching video modes
disconnected: closest to 1080x1920 at 0 high density yes: error: Couldn't find any matching video modes
disconnected: closest to 1280x720 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
disconnected: closest to 3840x2160 at 0 high density yes: error: Couldn't find any matching video modes
disconnected: closest to 1920x1080 at 30 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
disconnected: closest to 0x0 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
disconnected: properties: SDL.display.HDR_enabled=false
disconnected: display for point 10,10: 2
disconnected: display for point 1900,1000: 2
disconnected: display for point -50,-50: 2
connected again: events: DISPLAY_ADDED(3,0)
connected again: displays: 2, 3
connected again: display 2 name=2 primary=yes
connected again: bounds=0,0 1920x1080 usable=0,0 1920x1080
connected again: content_scale=1 natural=1 current=1
connected again: desktop mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected again: current mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected again: fullscreen modes: 1
connected again: fullscreen mode 0: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected again: closest to 1920x1080 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected again: closest to 1920x1080 at 0 high density no: error: Couldn't find any matching video modes
connected again: closest to 1080x1920 at 0 high density yes: error: Couldn't find any matching video modes
connected again: closest to 1280x720 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected again: closest to 3840x2160 at 0 high density yes: error: Couldn't find any matching video modes
connected again: closest to 1920x1080 at 30 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected again: closest to 0x0 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=2
connected again: properties: SDL.display.HDR_enabled=false
connected again: display 3 name=3 primary=no
connected again: bounds=1920,0 1920x1080 usable=1920,0 1920x1080
connected again: content_scale=1 natural=1 current=1
connected again: desktop mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=3
connected again: current mode 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=3
connected again: fullscreen modes: 1
connected again: fullscreen mode 0: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=3
connected again: closest to 1920x1080 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=3
connected again: closest to 1920x1080 at 0 high density no: error: Couldn't find any matching video modes
connected again: closest to 1080x1920 at 0 high density yes: error: Couldn't find any matching video modes
connected again: closest to 1280x720 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=3
connected again: closest to 3840x2160 at 0 high density yes: error: Couldn't find any matching video modes
connected again: closest to 1920x1080 at 30 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=3
connected again: closest to 0x0 at 0 high density yes: 1920x1080 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=60 (60/1) display=3
connected again: properties: SDL.display.HDR_enabled=false
connected again: display for point 10,10: 2
connected again: display for point 1900,1000: 2
connected again: display for point -50,-50: 2
A: create 320x480: ok
A: created: events: A:WINDOW_SAFE_AREA_CHANGED(0,0), A:WINDOW_MOUSE_ENTER(0,0), A:WINDOW_FOCUS_GAINED(0,0), A:WINDOW_SHOWN(0,0), A:WINDOW_PIXEL_SIZE_CHANGED(1920,1080)
A: created: pos=800,300 size=1920x1080 pixels=1920x1080 density=1 display_scale=1
A: created: flags=0x600 safe_area=80,60 1760x960 display=2
A: created: fullscreen mode (none)
A: created: uiwindow: UIWindow frame=0,0 1920x1080 hidden=no key=yes scene=yes safe insets=60,80,60,80
A: created: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=no
A: created: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
A: created: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
A: another window: error: Only one window allowed per display.
A: another window: events: ?:WINDOW_DESTROYED(0,0)
A: hide: ok
A: hidden: events: A:WINDOW_HIDDEN(0,0)
A: hidden: pos=800,300 size=1920x1080 pixels=1920x1080 density=1 display_scale=1
A: hidden: flags=0x608 safe_area=80,60 1760x960 display=2
A: hidden: fullscreen mode (none)
A: hidden: uiwindow: UIWindow frame=0,0 1920x1080 hidden=yes key=no scene=yes safe insets=60,80,60,80
A: hidden: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=no
A: hidden: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
A: hidden: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
A: show: ok
A: shown: events: A:WINDOW_SHOWN(0,0)
A: shown: pos=800,300 size=1920x1080 pixels=1920x1080 density=1 display_scale=1
A: shown: flags=0x600 safe_area=80,60 1760x960 display=2
A: shown: fullscreen mode (none)
A: shown: uiwindow: UIWindow frame=0,0 1920x1080 hidden=no key=yes scene=yes safe insets=60,80,60,80
A: shown: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=no
A: shown: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
A: shown: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
A: title: ok
A: title: window=Title A view controller=Title A
A: borderless: ok
A: borderless: events: (none)
A: borderless: pos=800,300 size=1920x1080 pixels=1920x1080 density=1 display_scale=1
A: borderless: flags=0x610 safe_area=80,60 1760x960 display=2
A: borderless: fullscreen mode (none)
A: borderless: uiwindow: UIWindow frame=0,0 1920x1080 hidden=no key=yes scene=yes safe insets=60,80,60,80
A: borderless: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=no
A: borderless: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
A: borderless: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
A: bordered: ok
A: bordered: events: (none)
A: fullscreen: ok
A: sync: ok
A: fullscreen: events: A:WINDOW_ENTER_FULLSCREEN(0,0)
A: fullscreen: pos=0,0 size=1920x1080 pixels=1920x1080 density=1 display_scale=1
A: fullscreen: flags=0x601 safe_area=80,60 1760x960 display=2
A: fullscreen: fullscreen mode (none)
A: fullscreen: uiwindow: UIWindow frame=0,0 1920x1080 hidden=no key=yes scene=yes safe insets=60,80,60,80
A: fullscreen: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=no
A: fullscreen: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
A: fullscreen: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
A: windowed: ok
A: sync: ok
A: windowed: events: A:WINDOW_LEAVE_FULLSCREEN(0,0)
A: windowed: pos=800,300 size=1920x1080 pixels=1920x1080 density=1 display_scale=1
A: windowed: flags=0x600 safe_area=80,60 1760x960 display=2
A: windowed: fullscreen mode (none)
A: windowed: uiwindow: UIWindow frame=0,0 1920x1080 hidden=no key=yes scene=yes safe insets=60,80,60,80
A: windowed: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=no
A: windowed: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
A: windowed: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
A: exclusive 1920x1080: set mode: ok
A: exclusive 1920x1080: fullscreen: ok
A: exclusive 1920x1080: sync: ok
A: exclusive 1920x1080: events: A:WINDOW_ENTER_FULLSCREEN(0,0)
A: exclusive 1920x1080: pos=0,0 size=1920x1080 pixels=1920x1080 density=1 display_scale=1
A: exclusive 1920x1080: flags=0x601 safe_area=80,60 1760x960 display=2
A: exclusive 1920x1080: fullscreen mode 1920x1080 density=2
A: exclusive 1920x1080: uiwindow: UIWindow frame=0,0 1920x1080 hidden=no key=yes scene=yes safe insets=60,80,60,80
A: exclusive 1920x1080: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=no
A: exclusive 1920x1080: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
A: exclusive 1920x1080: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
A: exclusive 1920x1080: windowed: ok
A: exclusive 1920x1080: sync: ok
A: exclusive 1920x1080: unset mode: ok
A: exclusive 1920x1080 left: events: A:WINDOW_LEAVE_FULLSCREEN(0,0)
A: controller UI events hint 1: controller user interaction=yes
A: controller UI events hint 0: controller user interaction=no
A: controller UI events hint 1: controller user interaction=yes
A: controller UI events hint (unset): controller user interaction=no
A: resize to 100x100: ok
A: resized: events: (none)
A: resized: pos=800,300 size=1920x1080 pixels=1920x1080 density=1 display_scale=1
A: resized: flags=0x600 safe_area=80,60 1760x960 display=2
A: resized: fullscreen mode (none)
A: resized: uiwindow: UIWindow frame=0,0 1920x1080 hidden=no key=yes scene=yes safe insets=60,80,60,80
A: resized: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=no
A: resized: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
A: resized: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
A: destroyed
A: destroyed: events: A:WINDOW_DESTROYED(0,0)
B: create 640x200 resizable high-density hidden: ok
B: created: events: B:WINDOW_PIXEL_SIZE_CHANGED(3840,2160), B:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
B: created: pos=640,440 size=1920x1080 pixels=3840x2160 density=2 display_scale=2
B: created: flags=0x2028 safe_area=0,0 1920x1080 display=2
B: created: fullscreen mode (none)
B: created: uiwindow: UIWindow frame=0,0 1920x1080 hidden=yes key=no scene=yes safe insets=60,80,60,80
B: created: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=yes
B: created: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
B: created: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
B: show: ok
B: shown: events: B:WINDOW_SAFE_AREA_CHANGED(0,0), B:WINDOW_MOUSE_ENTER(0,0), B:WINDOW_FOCUS_GAINED(0,0), B:WINDOW_SHOWN(0,0)
B: shown: pos=640,440 size=1920x1080 pixels=3840x2160 density=2 display_scale=2
B: shown: flags=0x2620 safe_area=80,60 1760x960 display=2
B: shown: fullscreen mode (none)
B: shown: uiwindow: UIWindow frame=0,0 1920x1080 hidden=no key=yes scene=yes safe insets=60,80,60,80
B: shown: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=yes
B: shown: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
B: shown: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
B: animation every frame: ok
B: animation every frame: 30 frames
B: animation every 3rd frame: ok
B: animation every 3rd frame: 10 frames
B: animation stopped: ok
B: animation: events: (none)
clipboard: has text: no
clipboard: set: error: The clipboard is not available on tvOS
clipboard: has text: no text=
clipboard: set empty: error: The clipboard is not available on tvOS
clipboard: has text: no text=
clipboard: events: (none)
B: destroyed
B: destroyed: events: B:WINDOW_DESTROYED(0,0)
C: create 200x300 fullscreen: ok
C: created: events: C:WINDOW_SAFE_AREA_CHANGED(0,0), C:WINDOW_MOUSE_ENTER(0,0), C:WINDOW_FOCUS_GAINED(0,0), C:WINDOW_SHOWN(0,0), C:WINDOW_ENTER_FULLSCREEN(0,0), C:WINDOW_PIXEL_SIZE_CHANGED(1920,1080)
C: created: pos=0,0 size=1920x1080 pixels=1920x1080 density=1 display_scale=1
C: created: flags=0x601 safe_area=80,60 1760x960 display=2
C: created: fullscreen mode (none)
C: created: uiwindow: UIWindow frame=0,0 1920x1080 hidden=no key=yes scene=yes safe insets=60,80,60,80
C: created: root: SDL_uikitviewcontroller (GCEventViewController) view=SDL_uikitview frame=0,0 1920x1080 controller user interaction=no
C: created: view gesture recognizers: UISwipeGestureRecognizer(4), UISwipeGestureRecognizer(8), UISwipeGestureRecognizer(2), UISwipeGestureRecognizer(1)
C: created: properties: SDL.window.HDR_enabled=false, SDL.window.HDR_headroom=1, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=2
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

    fn result<T>(r: sdl3::Result<T>, f: impl FnOnce(T) -> String) -> String {
        match r {
            Ok(v) => f(v),
            Err(e) => format!("error: {e}"),
        }
    }

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        fn NSHomeDirectory() -> Id;
    }

    /// A path with the app's home directory (its container) as `HOME`.
    fn home_path(path: sdl3::Result<String>) -> String {
        // SAFETY: Foundation's function (an autoreleased NSString).
        let home = objc::string(unsafe { NSHomeDirectory() }).unwrap_or_default();
        match path {
            Ok(p) => match p.strip_prefix(home.as_str()) {
                Some(rest) if !home.is_empty() => format!("HOME{rest}"),
                _ => p,
            },
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

    fn trace_events(label: &str, ms: u64) {
        let mut seen = Vec::new();
        settle(&mut seen, ms);
        t(format!("{label}: events: {}", list(seen)));
    }

    fn props_state(label: &str, p: Option<Properties>) {
        let mut props = Vec::new();
        if let Some(p) = p {
            let mut names = p.names();
            names.sort();
            for name in names {
                let value = match p.kind(&name) {
                    // (the other drivers' pointer properties are numbers
                    // here too)
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

    fn rect(r: &Rect) -> String {
        format!("{},{} {}x{}", r.x, r.y, r.w, r.h)
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

    fn cgrect(r: CGRect) -> String {
        format!("{},{} {}x{}", r.x, r.y, r.w, r.h)
    }

    /// `UIEdgeInsets`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct UIEdgeInsets {
        top: f64,
        left: f64,
        bottom: f64,
        right: f64,
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

    /// Everything a display's queries answer.
    fn display_state(label: &str, display: DisplayID) {
        t(format!(
            "{label}: display {display} name={} primary={}",
            video::display_name(display).unwrap_or_default(),
            yes_no(video::primary_display().ok() == Some(display))
        ));
        t(format!(
            "{label}: bounds={} usable={}",
            result(video::display_bounds(display), |r| rect(&r)),
            result(video::display_usable_bounds(display), |r| rect(&r))
        ));
        t(format!(
            "{label}: content_scale={} natural={} current={}",
            video::display_content_scale(display).unwrap_or(0.0),
            video::natural_display_orientation(display) as i32,
            video::current_display_orientation(display) as i32
        ));
        t(match video::desktop_display_mode(display) {
            Ok(m) => format!("{label}: desktop mode {}", mode(&m)),
            Err(e) => format!("{label}: desktop mode error: {e}"),
        });
        t(match video::current_display_mode(display) {
            Ok(m) => format!("{label}: current mode {}", mode(&m)),
            Err(e) => format!("{label}: current mode error: {e}"),
        });
        let modes = video::fullscreen_display_modes(display).unwrap_or_default();
        t(format!("{label}: fullscreen modes: {}", modes.len()));
        for (i, m) in modes.iter().enumerate() {
            t(format!("{label}: fullscreen mode {i}: {}", mode(m)));
        }
        for (w, h, refresh_rate, high_density) in [
            (1920, 1080, 0.0, true),
            (1920, 1080, 0.0, false),
            (1080, 1920, 0.0, true),
            (1280, 720, 0.0, true),
            (3840, 2160, 0.0, true),
            (1920, 1080, 30.0, true),
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
        props_state(label, video::display_properties(display).ok());
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
        for (x, y) in [(10, 10), (1900, 1000), (-50, -50)] {
            t(format!(
                "{label}: display for point {x},{y}: {}",
                result(video::display_for_point(Point::new(x, y)), |d| d
                    .to_string())
            ));
        }
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

    fn is_kind_of(obj: Id, class: &str) -> bool {
        let class = objc::class(class);
        // SAFETY: NSObject's method.
        !obj.is_null()
            && !class.is_null()
            && unsafe { objc::send1::<_, bool>(obj, "isKindOfClass:", class) }
    }

    /// Whether a view controller is a `GCEventViewController` whose game
    /// controllers' events go to UIKit.
    fn controller_user_interaction(vc: Id) -> bool {
        // SAFETY: GCEventViewController's property.
        is_kind_of(vc, "GCEventViewController")
            && unsafe { objc::send0::<bool>(vc, "controllerUserInteractionEnabled") }
    }

    /// The objects of an `NSArray`.
    fn array_items(array: Id) -> Vec<Id> {
        if array.is_null() {
            return Vec::new();
        }
        // SAFETY: NSArray's methods.
        unsafe {
            let count: usize = objc::send0(array, "count");
            (0..count)
                .map(|i| objc::send1(array, "objectAtIndex:", i))
                .collect()
        }
    }

    /// What SDL and UIKit say about a window.
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
        let flags = w.flags().unwrap_or_default().0 & !WindowFlags::OPENGL.0;
        t(format!(
            "{label}: flags={flags:#x} safe_area={} display={}",
            rect(&safe),
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
        // SAFETY: UIWindow's, UIViewController's, UIView's and the gesture
        // recognizers' properties.
        unsafe {
            let vc: Id = objc::send0(uw, "rootViewController");
            let frame: CGRect = objc::send0(uw, "frame");
            let hidden: bool = objc::send0(uw, "isHidden");
            let key: bool = objc::send0(uw, "isKeyWindow");
            let scene: Id = objc::send0(uw, "windowScene");
            let insets: UIEdgeInsets = objc::send0(uw, "safeAreaInsets");
            t(format!(
                "{label}: uiwindow: {} frame={} hidden={} key={} scene={} safe insets={},{},{},{}",
                objc::class_name(uw),
                cgrect(frame),
                yes_no(hidden),
                yes_no(key),
                yes_no(!scene.is_null()),
                insets.top,
                insets.left,
                insets.bottom,
                insets.right
            ));
            let view: Id = objc::send0(vc, "view");
            let view_frame: CGRect = if view.is_null() {
                CGRect::default()
            } else {
                objc::send0(view, "frame")
            };
            t(format!(
                "{label}: root: {} ({}) view={} frame={} controller user interaction={}",
                objc::class_name(vc),
                objc::superclass_name(vc),
                objc::class_name(view),
                cgrect(view_frame),
                yes_no(controller_user_interaction(vc))
            ));
            let mut recognizers = Vec::new();
            let all: Id = if view.is_null() {
                std::ptr::null_mut()
            } else {
                objc::send0(view, "gestureRecognizers")
            };
            for g in array_items(all) {
                if is_kind_of(g, "UISwipeGestureRecognizer") {
                    let direction: usize = objc::send0(g, "direction");
                    recognizers.push(format!("{}({direction})", objc::class_name(g)));
                } else {
                    recognizers.push(objc::class_name(g));
                }
            }
            t(format!(
                "{label}: view gesture recognizers: {}",
                list(recognizers)
            ));
        }
        props_state(label, w.properties().ok());
    }

    /// Set (`Some`) or unset `SDL_HINT_APPLE_TV_CONTROLLER_UI_EVENTS`, and
    /// whether the view controller passes the controllers' events on.
    fn controller_ui_events(label: &str, w: &Window, hint: Option<&str>) {
        set_hint(hints::APPLE_TV_CONTROLLER_UI_EVENTS, hint);
        t(format!(
            "{label}: controller UI events hint {}: controller user interaction={}",
            hint.unwrap_or("(unset)"),
            yes_no(controller_user_interaction(vc_of(w)))
        ));
    }

    fn set_hint(name: &str, value: Option<&str>) {
        hints::set_with_priority(name, value, hints::Priority::Normal).unwrap();
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

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    fn path_exists(label: &str, path: &str) {
        t(match filesystem::get_path_info(path) {
            Ok(info) => format!(
                "{label}: {}: type={} size={}",
                home_path(Ok(path.to_string())),
                match info.kind {
                    PathType::None => 0,
                    PathType::File => 1,
                    PathType::Directory => 2,
                    _ => 3,
                },
                info.size
            ),
            Err(e) => format!("{label}: {}: error: {e}", home_path(Ok(path.to_string()))),
        });
    }

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_tvos: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_tvos: {:.3} s: main", elapsed());
        t(format!("platform: {}", init::platform()));
        let r = init::init(InitFlags::VIDEO | InitFlags::AUDIO);
        t(format!("init video and audio: {}", ok(&r)));
        r.unwrap();
        t(format!(
            "video driver: {}; audio driver: {}",
            video::current_video_driver().unwrap(),
            sdl3::audio::current_audio_driver().unwrap_or("(null)")
        ));
        t(format!(
            "video drivers: {}",
            list(
                (0..video::num_video_drivers())
                    .filter_map(|i| video::video_driver(i).ok())
                    .map(String::from)
                    .collect()
            )
        ));
        // (without upstream's OpenGL ES 1 renderer, which isn't translated
        // yet: its C program leaves it out)
        t(format!(
            "render drivers: {}",
            list(
                (0..sdl3::render::num_render_drivers())
                    .filter_map(|i| sdl3::render::render_driver(i).ok())
                    .filter(|&name| name != "opengles")
                    .map(String::from)
                    .collect()
            )
        ));
        t(format!(
            "camera drivers: {}",
            list(
                (0..sdl3::camera::num_camera_drivers())
                    .filter_map(|i| sdl3::camera::camera_driver(i).ok())
                    .map(String::from)
                    .collect()
            )
        ));
        // (the simulator's keyboard is GameController's coalesced keyboard:
        // added when video starts, before the audio devices, if
        // GameController has found it by then, else when its connect
        // notification comes, a moment later; it's waited for, and traced
        // where video's start adds it)
        let mut seen = Vec::new();
        let end = Instant::now() + Duration::from_secs(5);
        take_queued(&mut seen);
        while !seen.iter().any(|e| e == "KEYBOARD_ADDED") && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
            take_queued(&mut seen);
        }
        if let Some(i) = seen.iter().position(|e| e == "KEYBOARD_ADDED") {
            let keyboard = seen.remove(i);
            seen.insert(0, keyboard);
        }
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
        // SAFETY: UIDevice's class method and property.
        let idiom: isize = unsafe {
            let device: Id = objc::send0(objc::class("UIDevice"), "currentDevice");
            objc::send0(device, "userInterfaceIdiom")
        };
        t(format!("idiom: {idiom}"));

        let power = sdl3::power::power_info();
        let state = match power.state {
            sdl3::power::PowerState::Error => -1,
            sdl3::power::PowerState::Unknown => 0,
            sdl3::power::PowerState::OnBattery => 1,
            sdl3::power::PowerState::NoBattery => 2,
            sdl3::power::PowerState::Charging => 3,
            sdl3::power::PowerState::Charged => 4,
        };
        t(format!(
            "power: state={state} secs={} pct={}",
            power.time_left.map_or(-1, |d| d.as_secs() as i64),
            power.percent.map_or(-1, i32::from)
        ));

        // The file system: the pref path is in Caches, and there are no
        // user folders
        let pref = filesystem::get_pref_path(Some("org.libsdl"), "tvos test");
        t(format!("pref path: {}", home_path(pref.clone())));
        path_exists("pref path", pref.as_deref().unwrap_or(""));
        t(format!(
            "pref path without org: {}",
            home_path(filesystem::get_pref_path(None, "tvos test"))
        ));
        t(format!(
            "pref path empty: {}",
            home_path(filesystem::get_pref_path(Some(""), ""))
        ));
        let base = filesystem::get_base_path().unwrap_or_default();
        t(format!(
            "base path: ends with .app/: {}",
            yes_no(base.len() > 5 && base.ends_with(".app/"))
        ));
        for (folder, name) in [
            (Folder::Home, "home"),
            (Folder::Documents, "documents"),
            (Folder::SavedGames, "saved games"),
            (Folder::Desktop, "desktop"),
            (Folder::Downloads, "downloads"),
        ] {
            t(format!(
                "user folder {name}: {}",
                home_path(filesystem::get_user_folder(folder))
            ));
        }
        let written = sdl3::io::IoStream::from_file("tvos-relative.txt", "w").and_then(|mut io| {
            if io.write(b"tvOS\n") != 5 {
                return Err(sdl3::Error::new("short write"));
            }
            io.close()
        });
        t(format!("relative file written: {}", ok(&written)));
        let relative = format!(
            "{}tvos-relative.txt",
            filesystem::get_pref_path(Some(""), "").unwrap_or_default()
        );
        path_exists("relative file", &relative);
        t(format!(
            "relative file removed: {}",
            ok(&filesystem::remove_path("tvos-relative.txt"))
        ));
        path_exists("relative file", &relative);

        // Displays
        displays_state("start");
        for (label, name) in [
            ("connected", "UIScreenDidConnectNotification"),
            ("disconnected", "UIScreenDidDisconnectNotification"),
            ("connected again", "UIScreenDidConnectNotification"),
        ] {
            // SAFETY: UIScreen's class method.
            let screen: Id = unsafe { objc::send0(objc::class("UIScreen"), "mainScreen") };
            objc::post_notification_for(objc::ns_string(name), screen);
            let mut seen = Vec::new();
            take_queued(&mut seen);
            t(format!("{label}: events: {}", list(seen)));
            displays_state(label);
        }

        // A: a plain window
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
            t(format!(
                "{label}: windowed: {}",
                ok(&a.set_fullscreen(false))
            ));
            t(format!("{label}: sync: {}", ok(&a.sync())));
            t(format!(
                "{label}: unset mode: {}",
                ok(&a.set_fullscreen_mode(None))
            ));
            trace_events(&format!("A: exclusive {}x{} left", mode.w, mode.h), 1000);
        }
        controller_ui_events("A", &a, Some("1"));
        controller_ui_events("A", &a, Some("0"));
        controller_ui_events("A", &a, Some("1"));
        controller_ui_events("A", &a, None);
        set_hint(hints::ORIENTATIONS, Some("LandscapeLeft Portrait"));
        t(format!(
            "A: resize to 100x100: {}",
            ok(&a.set_size(100, 100))
        ));
        trace_events("A: resized", 500);
        window_state("A: resized", &a);
        set_hint(hints::ORIENTATIONS, None);
        a.destroy();
        t("A: destroyed");
        trace_events("A: destroyed", 500);

        // B: resizable, high density, made hidden, with the hint for the
        // controllers' UI events
        set_hint(hints::APPLE_TV_CONTROLLER_UI_EVENTS, Some("1"));
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
        set_hint(hints::APPLE_TV_CONTROLLER_UI_EVENTS, None);
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
        t(format!(
            "B: animation stopped: {}",
            ok(&b.set_ios_animation_callback(1, None))
        ));
        trace_events("B: animation", 200);

        // The clipboard (no pasteboard on tvOS)
        t(format!(
            "clipboard: has text: {}",
            yes_no(video::clipboard::has_clipboard_text().unwrap_or(false))
        ));
        t(format!(
            "clipboard: set: {}",
            ok(&video::clipboard::set_clipboard_text("tvOS text \u{e9}"))
        ));
        t(format!(
            "clipboard: has text: {} text={}",
            yes_no(video::clipboard::has_clipboard_text().unwrap_or(false)),
            video::clipboard::clipboard_text().unwrap_or_default()
        ));
        t(format!(
            "clipboard: set empty: {}",
            ok(&video::clipboard::set_clipboard_text(""))
        ));
        t(format!(
            "clipboard: has text: {} text={}",
            yes_no(video::clipboard::has_clipboard_text().unwrap_or(false)),
            video::clipboard::clipboard_text().unwrap_or_default()
        ));
        trace_events("clipboard", 200);

        b.destroy();
        t("B: destroyed");
        trace_events("B: destroyed", 500);

        // C: fullscreen from the start
        let r = Window::create("C", 200, 300, WindowFlags::FULLSCREEN);
        t(format!("C: create 200x300 fullscreen: {}", ok(&r)));
        let c = r.unwrap();
        name_window(&c, "C");
        trace_events("C: created", 1000);
        window_state("C: created", &c);
        c.destroy();
        t("C: destroyed");
        trace_events("C: destroyed", 500);

        init::quit();
        t("quit");

        check_trace(&TRACE.lock().unwrap(), EXPECTED);
    }
}
