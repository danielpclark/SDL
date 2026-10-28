// Tests of the UIKit video driver on visionOS: its fake display, its flat
// windows, the Apple Vision Pro's file system, processes and the
// renderers' linear colorspace.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The UIKit video driver only starts in an app, so this is a program of
//! its own (`harness = false`), run as an app on the visionOS Simulator by
//! `tools/ios_run.py` (the `__DATA,__sdl3_runner` section below), as the
//! iOS programs (`uikit_*`) are on the iOS Simulator. Its main function
//! traces the platform and the drivers (no OpenGL ES), the form factor
//! (a headset) and the system theme, the power state (no battery on the
//! simulator), the file system (the pref path in Application Support, the
//! user folders, a relative path's file in the pref path), a process
//! (POSIX: the host's `/bin/sh`, its output and exit status), the sensors
//! (CoreMotion, none on the simulator) and the virtual joysticks (none:
//! upstream's CMake build has no virtual joystick driver without HIDAPI,
//! which it doesn't build on visionOS), and the status bar's orientation
//! (unsupported); everything the display answers (the fake 1280x720 one,
//! of density 2 at 90 Hz); then windows: a plain one hidden and shown,
//! given a title, made borderless, fullscreen and windowed, its
//! orientations with `SDL_HINT_ORIENTATIONS`, resized (the window scene is
//! asked for the size); a resizable high density one made hidden, given
//! animation callbacks (the display link at 90 frames a second), the
//! clipboard, then its renderers: the Metal one, whose output colorspace
//! is linear sRGB whatever was asked for (upstream's
//! `SDL_SetupRendererColorspace()` on visionOS: the RealityKit texture's),
//! cleared and read back; the software one, which can't draw to it (on the
//! window, on a surface, and so the window's surface, which is a texture
//! of a renderer); one made with visionOS settings (a curved window
//! upstream, with its Swift code; a flat one here, whose settings are
//! kept); one fullscreen from the start. Of each it traces what SDL and
//! UIKit say: sizes in points and pixels, flags (but OpenGL), the safe
//! area, the `UIWindow` (its frame is the window's size), its root view
//! controller, its orientations and container background style (UIKit's
//! automatic one, which a curved window hides), the view's gesture
//! recognizers and interactions, and the properties.
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an Apple Vision Pro with
//! visionOS 2.5), upstream built with CMake for the visionOS Simulator
//! (with SDL_UIKitBridge.m's `FIXME (upstream)` fixed, see
//! `docs/HARDWARE_TESTING.md`). Paths in the app's container start with
//! `HOME`, and the mice's device events are left out. Elsewhere this
//! program does nothing.

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
    use sdl3::render::{Renderer, RendererCreateInfo};
    use sdl3::video::{self, Colorspace, DisplayMode, PixelFormat, Point, Rect, Surface, Window};

    use super::uikit_support::objc::{self, Id};
    use super::uikit_support::{check_trace, checked, elapsed, event_name, exit};

    /// Upstream's trace (the C program, on an Apple Vision Pro simulator
    /// with visionOS 2.5).
    const EXPECTED: &str = "\
platform: visionOS
init video and audio: ok
video driver: uikit; audio driver: coreaudio
video drivers: uikit, offscreen, dummy
render drivers: metal, vulkan, gpu, software
camera drivers: dummy
gpu drivers: metal, vulkan
events: KEYBOARD_ADDED, AUDIO_DEVICE_ADDED, AUDIO_DEVICE_ADDED
application state: 0
idiom: 6
form factor: 9
system theme: 0
power: state=0 secs=-1 pct=-1
pref path: HOME/Library/Application Support/org.libsdl/visionos test/
pref path: HOME/Library/Application Support/org.libsdl/visionos test/: type=2 size=0
pref path without org: HOME/Library/Application Support/visionos test/
pref path empty: HOME/Library/Application Support//
base path: ends with .app/: yes
user folder home: HOME/
user folder documents: HOME/Documents/
user folder saved games: error: Saved games folder not supported on Cocoa
user folder desktop: HOME/Desktop/
user folder downloads: HOME/Downloads/
relative file written: ok
relative file: HOME/Library/Application Support//visionos-relative.txt: type=1 size=5
relative file removed: ok
relative file: HOME/Library/Application Support//visionos-relative.txt: error: Can't stat: No such file or directory
process: ok
process: output=visionOS exit=3
init sensor and joystick: ok
sensors: 0
virtual joystick: SDL not built with virtual-joystick support
virtual joystick 1 detach: error: SDL not built with virtual-joystick support is virtual: no
sensor and joystick quit: events: JOYSTICK_ADDED, JOYSTICK_REMOVED, JOYSTICK_REMOVED
status bar orientation: error: That operation is not supported
start: displays: 1
start: display 1 name=1 primary=yes
start: bounds=0,0 1280x720 usable=0,0 1280x720
start: content_scale=1 natural=1 current=1
start: desktop mode 1280x720 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=90 (90/1) display=1
start: current mode 1280x720 fmt=SDL_PIXELFORMAT_ABGR8888 density=2 refresh=90 (90/1) display=1
start: fullscreen modes: 0
start: closest to 1920x1080 at 0 high density yes: error: Couldn't find any matching video modes
start: closest to 1920x1080 at 0 high density no: error: Couldn't find any matching video modes
start: closest to 1080x1920 at 0 high density yes: error: Couldn't find any matching video modes
start: closest to 1280x720 at 0 high density yes: error: Couldn't find any matching video modes
start: closest to 3840x2160 at 0 high density yes: error: Couldn't find any matching video modes
start: closest to 1920x1080 at 30 high density yes: error: Couldn't find any matching video modes
start: closest to 0x0 at 0 high density yes: error: Couldn't find any matching video modes
start: properties: SDL.display.HDR_enabled=true
start: display for point 10,10: 1
start: display for point 1900,1000: 1
start: display for point -50,-50: 1
A: create 320x480: ok
A: created: events: A:WINDOW_MOUSE_ENTER(0,0), A:WINDOW_FOCUS_GAINED(0,0), A:WINDOW_SHOWN(0,0), A:WINDOW_PIXEL_SIZE_CHANGED(320,480)
A: created: pos=480,120 size=320x480 pixels=320x480 density=1 display_scale=1
A: created: flags=0x20000600 safe_area=0,0 320x480 display=1
A: created: fullscreen mode (none)
A: created: uiwindow: UIWindow frame=0,0 320x480 hidden=no key=yes scene=yes safe insets=0,0,0,0
A: created: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 320x480 orientations=6 background style=0
A: created: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
A: created: view interactions: UIPointerInteraction
A: created: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: another window: error: Only one window allowed per display.
A: another window: events: ?:WINDOW_DESTROYED(0,0)
A: hide: ok
A: hidden: events: A:WINDOW_HIDDEN(0,0)
A: hidden: pos=480,120 size=320x480 pixels=320x480 density=1 display_scale=1
A: hidden: flags=0x20000608 safe_area=0,0 320x480 display=1
A: hidden: fullscreen mode (none)
A: hidden: uiwindow: UIWindow frame=0,0 320x480 hidden=yes key=no scene=yes safe insets=0,0,0,0
A: hidden: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 320x480 orientations=6 background style=0
A: hidden: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
A: hidden: view interactions: UIPointerInteraction
A: hidden: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: show: ok
A: shown: events: A:WINDOW_SHOWN(0,0)
A: shown: pos=480,120 size=320x480 pixels=320x480 density=1 display_scale=1
A: shown: flags=0x20000600 safe_area=0,0 320x480 display=1
A: shown: fullscreen mode (none)
A: shown: uiwindow: UIWindow frame=0,0 320x480 hidden=no key=yes scene=yes safe insets=0,0,0,0
A: shown: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 320x480 orientations=6 background style=0
A: shown: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
A: shown: view interactions: UIPointerInteraction
A: shown: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: title: ok
A: title: window=Title A view controller=Title A
A: borderless: ok
A: borderless: events: (none)
A: borderless: pos=480,120 size=320x480 pixels=320x480 density=1 display_scale=1
A: borderless: flags=0x20000610 safe_area=0,0 320x480 display=1
A: borderless: fullscreen mode (none)
A: borderless: uiwindow: UIWindow frame=0,0 320x480 hidden=no key=yes scene=yes safe insets=0,0,0,0
A: borderless: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 320x480 orientations=6 background style=0
A: borderless: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
A: borderless: view interactions: UIPointerInteraction
A: borderless: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: bordered: ok
A: bordered: events: (none)
A: fullscreen: ok
A: sync: ok
A: fullscreen: events: A:WINDOW_ENTER_FULLSCREEN(0,0)
A: fullscreen: pos=0,0 size=320x480 pixels=320x480 density=1 display_scale=1
A: fullscreen: flags=0x20000601 safe_area=0,0 320x480 display=1
A: fullscreen: fullscreen mode (none)
A: fullscreen: uiwindow: UIWindow frame=0,0 320x480 hidden=no key=yes scene=yes safe insets=0,0,0,0
A: fullscreen: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 320x480 orientations=6 background style=0
A: fullscreen: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
A: fullscreen: view interactions: UIPointerInteraction
A: fullscreen: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: windowed: ok
A: sync: ok
A: windowed: events: A:WINDOW_LEAVE_FULLSCREEN(0,0)
A: windowed: pos=480,120 size=320x480 pixels=320x480 density=1 display_scale=1
A: windowed: flags=0x20000600 safe_area=0,0 320x480 display=1
A: windowed: fullscreen mode (none)
A: windowed: uiwindow: UIWindow frame=0,0 320x480 hidden=no key=yes scene=yes safe insets=0,0,0,0
A: windowed: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 320x480 orientations=6 background style=0
A: windowed: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
A: windowed: view interactions: UIPointerInteraction
A: windowed: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: orientations with the hint: 18
A: resize to 800x600: ok
A: resized: events: A:WINDOW_RESIZED(800,600), A:WINDOW_PIXEL_SIZE_CHANGED(800,600), A:WINDOW_SAFE_AREA_CHANGED(0,0)
A: resized: pos=480,120 size=800x600 pixels=800x600 density=1 display_scale=1
A: resized: flags=0x20000600 safe_area=0,0 800x600 display=1
A: resized: fullscreen mode (none)
A: resized: uiwindow: UIWindow frame=0,0 800x600 hidden=no key=yes scene=yes safe insets=0,0,0,0
A: resized: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 800x600 orientations=2 background style=0
A: resized: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
A: resized: view interactions: UIPointerInteraction
A: resized: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
A: destroyed
A: destroyed: events: A:WINDOW_DESTROYED(0,0)
B: create 640x200 resizable high-density hidden: ok
B: created: events: B:WINDOW_PIXEL_SIZE_CHANGED(1280,400), B:WINDOW_DISPLAY_SCALE_CHANGED(0,0)
B: created: pos=320,260 size=640x200 pixels=1280x400 density=2 display_scale=2
B: created: flags=0x20002028 safe_area=0,0 640x200 display=1
B: created: fullscreen mode (none)
B: created: uiwindow: UIWindow frame=0,0 640x200 hidden=yes key=no scene=yes safe insets=0,0,0,0
B: created: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 640x200 orientations=30 background style=0
B: created: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
B: created: view interactions: UIPointerInteraction
B: created: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
B: show: ok
B: shown: events: B:WINDOW_MOUSE_ENTER(0,0), B:WINDOW_FOCUS_GAINED(0,0), B:WINDOW_SHOWN(0,0)
B: shown: pos=320,260 size=640x200 pixels=1280x400 density=2 display_scale=2
B: shown: flags=0x20002620 safe_area=0,0 640x200 display=1
B: shown: fullscreen mode (none)
B: shown: uiwindow: UIWindow frame=0,0 640x200 hidden=no key=yes scene=yes safe insets=0,0,0,0
B: shown: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 640x200 orientations=30 background style=0
B: shown: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
B: shown: view interactions: UIPointerInteraction
B: shown: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
B: animation every frame: ok
B: animation every frame: 30 frames
B: animation every 3rd frame: ok
B: animation every 3rd frame: 10 frames
B: animation stopped: ok
B: animation: events: (none)
clipboard: set: ok
clipboard: has text: yes text=visionOS text é
clipboard: events: CLIPBOARD_UPDATE
B: renderer: metal
B: renderer: name=metal output=1280x400 colorspace=0x12000500 hdr=yes sdr white=1 headroom=2
B: renderer: clear 255,128,0: read 4x4 SDL_PIXELFORMAT_RGBA64_FLOAT colorspace=0x12000500: 217,108,0,255
B: renderer: clear 0.25,0.5,1.0: read 4x4 SDL_PIXELFORMAT_RGBA64_FLOAT colorspace=0x12000500: 51,104,210,255
B: renderer: present: ok
B: software renderer: Unsupported output colorspace
surface renderer: Unsupported output colorspace
B: metal renderer asked for sRGB: metal
B: metal renderer asked for sRGB: name=metal output=1280x400 colorspace=0x12000500 hdr=yes sdr white=1 headroom=2
B: metal renderer asked for sRGB: clear 255,128,0: read 4x4 SDL_PIXELFORMAT_RGBA64_FLOAT colorspace=0x12000500: 217,108,0,255
B: metal renderer asked for sRGB: clear 0.25,0.5,1.0: read 4x4 SDL_PIXELFORMAT_RGBA64_FLOAT colorspace=0x12000500: 51,104,210,255
B: metal renderer asked for sRGB: present: ok
B: renderers: events: (none)
B: window surface: SDL_PIXELFORMAT_ARGB8888
B: window surface: events: (none)
B: destroyed
B: destroyed: events: B:WINDOW_DESTROYED(0,0)
D: create 1000x500 with settings: ok
D: created: events: D:WINDOW_MOUSE_ENTER(0,0), D:WINDOW_FOCUS_GAINED(0,0), D:WINDOW_SHOWN(0,0), D:WINDOW_PIXEL_SIZE_CHANGED(1000,500)
D: created: pos=140,110 size=1000x500 pixels=1000x500 density=1 display_scale=1
D: created: flags=0x20000600 safe_area=0,0 1000x500 display=1
D: created: fullscreen mode (none)
D: created: uiwindow: UIWindow frame=0,0 1000x500 hidden=no key=yes scene=yes safe insets=0,0,0,0
D: created: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 1000x500 orientations=2 background style=0
D: created: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
D: created: view interactions: UIPointerInteraction
D: created: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), SDL.window.visionos.settings={\"curvatureRadius\":1000}, sdl2-compat.window.preferred_fullscreen_display=1
D: destroyed
D: destroyed: events: D:WINDOW_DESTROYED(0,0)
C: create 200x300 fullscreen: ok
C: created: events: C:WINDOW_MOUSE_ENTER(0,0), C:WINDOW_FOCUS_GAINED(0,0), C:WINDOW_SHOWN(0,0), C:WINDOW_ENTER_FULLSCREEN(0,0), C:WINDOW_PIXEL_SIZE_CHANGED(1280,720)
C: created: pos=0,0 size=1280x720 pixels=1280x720 density=1 display_scale=1
C: created: flags=0x20000601 safe_area=0,0 1280x720 display=1
C: created: fullscreen mode (none)
C: created: uiwindow: UIWindow frame=0,0 1280x720 hidden=no key=yes scene=yes safe insets=0,0,0,0
C: created: root: SDL_uikitviewcontroller (UIViewController) view=SDL_uikitview frame=0,0 1280x720 orientations=6 background style=0
C: created: view gesture recognizers: UIPinchGestureRecognizer, UIHoverGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, _UIPointerInteractionHoverGestureRecognizer, _UIPointerInteractionPressGestureRecognizer, UIHoverGestureRecognizer
C: created: view interactions: UIPointerInteraction
C: created: properties: SDL.window.HDR_enabled=true, SDL.window.HDR_headroom=2, SDL.window.SDR_white_level=1, SDL.window.uikit.metal_view_tag=255, SDL.window.uikit.window=(pointer), sdl2-compat.window.preferred_fullscreen_display=1
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
            let orientations: usize = objc::send0(vc, "supportedInterfaceOrientations");
            let background_style: isize = objc::send0(vc, "preferredContainerBackgroundStyle");
            t(format!(
                "{label}: root: {} ({}) view={} frame={} orientations={orientations} background style={background_style}",
                objc::class_name(vc),
                objc::superclass_name(vc),
                objc::class_name(view),
                cgrect(view_frame),
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
            let interactions: Id = if view.is_null() {
                std::ptr::null_mut()
            } else {
                objc::send0(view, "interactions")
            };
            t(format!(
                "{label}: view interactions: {}",
                list(
                    array_items(interactions)
                        .into_iter()
                        .map(objc::class_name)
                        .collect()
                )
            ));
        }
        props_state(label, w.properties().ok());
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

    /// The top left pixel of a renderer's read back, and the read back.
    fn read_pixel(r: &mut Renderer) -> String {
        match r.read_pixels(Some(&Rect::new(0, 0, 4, 4))) {
            Ok(s) => {
                let colorspace = s.colorspace();
                let c = s.convert(PixelFormat::RGBA32).unwrap();
                let p = &c.pixels().unwrap()[..4];
                format!(
                    "read {}x{} {} colorspace={:#x}: {},{},{},{}",
                    s.width(),
                    s.height(),
                    s.format().name(),
                    colorspace.0,
                    p[0],
                    p[1],
                    p[2],
                    p[3]
                )
            }
            Err(e) => format!("read: error: {e}"),
        }
    }

    /// A renderer's colorspace and HDR properties, and two clears read back.
    fn renderer_state(label: &str, r: &mut Renderer) {
        let p = r.properties();
        let (w, h) = r.output_size().unwrap_or_default();
        t(format!(
            "{label}: name={} output={w}x{h} colorspace={:#x} hdr={} sdr white={} headroom={}",
            r.name(),
            p.get_number(sdl3::render::PROP_RENDERER_OUTPUT_COLORSPACE_NUMBER)
                .unwrap_or(0),
            yes_no(
                p.get_bool(sdl3::render::PROP_RENDERER_HDR_ENABLED_BOOLEAN)
                    .unwrap_or(false)
            ),
            p.get_float(sdl3::render::PROP_RENDERER_SDR_WHITE_POINT_FLOAT)
                .unwrap_or(0.0),
            p.get_float(sdl3::render::PROP_RENDERER_HDR_HEADROOM_FLOAT)
                .unwrap_or(0.0)
        ));
        r.set_draw_color(255, 128, 0, 255);
        let _ = r.clear();
        t(format!("{label}: clear 255,128,0: {}", read_pixel(r)));
        r.set_draw_color_float(0.25, 0.5, 1.0, 1.0);
        let _ = r.clear();
        t(format!("{label}: clear 0.25,0.5,1.0: {}", read_pixel(r)));
        t(format!("{label}: present: {}", ok(&r.present())));
    }

    pub(super) fn main() {
        elapsed();
        let status = sdl3::app::run_app(|_args| {
            let status = checked(main_function);
            exit(status);
        });
        println!("uikit_visionos: run_app returned {status}, which it never does");
        exit(1);
    }

    fn main_function() {
        println!("uikit_visionos: {:.3} s: main", elapsed());
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
        t(format!(
            "render drivers: {}",
            list(
                (0..sdl3::render::num_render_drivers())
                    .filter_map(|i| sdl3::render::render_driver(i).ok())
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
        t(format!(
            "gpu drivers: {}",
            list(
                (0..sdl3::gpu::num_gpu_drivers())
                    .filter_map(|i| sdl3::gpu::gpu_driver(i).ok())
                    .map(String::from)
                    .collect()
            )
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
        // SAFETY: UIDevice's class method and property.
        let idiom: isize = unsafe {
            let device: Id = objc::send0(objc::class("UIDevice"), "currentDevice");
            objc::send0(device, "userInterfaceIdiom")
        };
        t(format!("idiom: {idiom}"));
        t(format!(
            "form factor: {}",
            init::device_form_factor() as i32
        ));
        t(format!("system theme: {}", video::system_theme() as i32));

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

        // The file system
        let pref = filesystem::get_pref_path(Some("org.libsdl"), "visionos test");
        t(format!("pref path: {}", home_path(pref.clone())));
        path_exists("pref path", pref.as_deref().unwrap_or(""));
        t(format!(
            "pref path without org: {}",
            home_path(filesystem::get_pref_path(None, "visionos test"))
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
        let written =
            sdl3::io::IoStream::from_file("visionos-relative.txt", "w").and_then(|mut io| {
                if io.write(b"xrOS\n") != 5 {
                    return Err(sdl3::Error::new("short write"));
                }
                io.close()
            });
        t(format!("relative file written: {}", ok(&written)));
        let relative = format!(
            "{}visionos-relative.txt",
            filesystem::get_pref_path(Some(""), "").unwrap_or_default()
        );
        path_exists("relative file", &relative);
        t(format!(
            "relative file removed: {}",
            ok(&filesystem::remove_path("visionos-relative.txt"))
        ));
        path_exists("relative file", &relative);

        // Processes (POSIX), sensors (CoreMotion: none on the simulator),
        // no virtual joysticks
        let process =
            sdl3::process::Process::create(&["/bin/sh", "-c", "echo visionOS; exit 3"], true);
        t(format!("process: {}", ok(&process)));
        if let Ok(mut process) = process {
            let (out, code) = process.read().unwrap_or_default();
            let out = String::from_utf8_lossy(&out);
            t(format!(
                "process: output={} exit={code}",
                out.strip_suffix('\n').unwrap_or(&out)
            ));
        }
        t(format!(
            "init sensor and joystick: {}",
            ok(&init::init_subsystem(
                InitFlags::SENSOR | InitFlags::JOYSTICK
            ))
        ));
        t(format!("sensors: {}", sdl3::sensor::sensors().len()));
        let attached =
            sdl3::joystick::attach_virtual_joystick(sdl3::joystick::VirtualJoystickDesc {
                naxes: 2,
                nbuttons: 2,
                ..Default::default()
            });
        t(format!(
            "virtual joystick: {}",
            match attached {
                Ok(_) => "attached".to_string(),
                Err(e) => e.to_string(),
            }
        ));
        t(format!(
            "virtual joystick 1 detach: {} is virtual: {}",
            ok(&sdl3::joystick::detach_virtual_joystick(1)),
            yes_no(sdl3::joystick::is_joystick_virtual(1))
        ));
        init::quit_subsystem(InitFlags::SENSOR | InitFlags::JOYSTICK);
        trace_events("sensor and joystick quit", 300);
        t(format!(
            "status bar orientation: {}",
            ok(&video::on_application_did_change_status_bar_orientation())
        ));

        // The display: visionOS's fake one
        displays_state("start");

        // A: a plain window
        let r = Window::create("A", 320, 480, WindowFlags::NONE);
        t(format!("A: create 320x480: {}", ok(&r)));
        let a = r.unwrap();
        name_window(&a, "A");
        trace_events("A: created", 1000);
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
        set_hint(hints::ORIENTATIONS, Some("LandscapeLeft Portrait"));
        // SAFETY: a UIViewController method.
        let orientations: usize =
            unsafe { objc::send0(vc_of(&a), "supportedInterfaceOrientations") };
        t(format!("A: orientations with the hint: {orientations}"));
        set_hint(hints::ORIENTATIONS, None);
        t(format!(
            "A: resize to 800x600: {}",
            ok(&a.set_size(800, 600))
        ));
        trace_events("A: resized", 2000);
        window_state("A: resized", &a);
        a.destroy();
        t("A: destroyed");
        trace_events("A: destroyed", 500);

        // B: resizable, high density, made hidden, with animation callbacks
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
        trace_events("B: created", 1000);
        window_state("B: created", &b);
        t(format!("B: show: {}", ok(&b.show())));
        trace_events("B: shown", 500);
        window_state("B: shown", &b);
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

        // The clipboard
        t(format!(
            "clipboard: set: {}",
            ok(&video::clipboard::set_clipboard_text(
                "visionOS text \u{e9}"
            ))
        ));
        t(format!(
            "clipboard: has text: {} text={}",
            yes_no(video::clipboard::has_clipboard_text().unwrap_or(false)),
            video::clipboard::clipboard_text().unwrap_or_default()
        ));
        trace_events("clipboard", 500);

        // The renderers on B: the linear colorspace
        match Renderer::for_window(&b, None) {
            Ok(mut r) => {
                t(format!("B: renderer: {}", r.name()));
                renderer_state("B: renderer", &mut r);
            }
            Err(e) => t(format!("B: renderer: {e}")),
        }
        t(format!(
            "B: software renderer: {}",
            match Renderer::for_window(&b, Some("software")) {
                Ok(r) => r.name().to_string(),
                Err(e) => e.to_string(),
            }
        ));
        t(format!(
            "surface renderer: {}",
            match Renderer::software(Surface::new(16, 16, PixelFormat::ARGB8888).unwrap()) {
                Ok(r) => r.name().to_string(),
                Err(e) => e.to_string(),
            }
        ));
        let info = RendererCreateInfo {
            name: Some("metal".into()),
            output_colorspace: Some(Colorspace::SRGB),
            ..Default::default()
        };
        match Renderer::for_window_with(&b, &info) {
            Ok(mut r) => {
                t(format!("B: metal renderer asked for sRGB: {}", r.name()));
                renderer_state("B: metal renderer asked for sRGB", &mut r);
            }
            Err(e) => t(format!("B: metal renderer asked for sRGB: {e}")),
        }
        trace_events("B: renderers", 500);
        t(format!(
            "B: window surface: {}",
            match b.surface() {
                Ok(s) => s.lock().format().name().to_string(),
                Err(e) => e.to_string(),
            }
        ));
        let _ = b.destroy_surface();
        trace_events("B: window surface", 500);

        b.destroy();
        t("B: destroyed");
        trace_events("B: destroyed", 500);

        // D: made with visionOS settings (a curved window upstream, with
        // its Swift code)
        let props = Properties::new();
        props
            .set(video::window::PROP_WINDOW_CREATE_TITLE_STRING, "D")
            .unwrap();
        props
            .set(video::window::PROP_WINDOW_CREATE_WIDTH_NUMBER, 1000i64)
            .unwrap();
        props
            .set(video::window::PROP_WINDOW_CREATE_HEIGHT_NUMBER, 500i64)
            .unwrap();
        props
            .set(
                video::window::PROP_WINDOW_CREATE_VISIONOS_SETTINGS_STRING,
                "{\"curvatureRadius\":1000}",
            )
            .unwrap();
        let r = Window::create_with_properties(&props);
        t(format!("D: create 1000x500 with settings: {}", ok(&r)));
        let d = r.unwrap();
        name_window(&d, "D");
        trace_events("D: created", 1000);
        window_state("D: created", &d);
        d.destroy();
        t("D: destroyed");
        trace_events("D: destroyed", 500);

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
