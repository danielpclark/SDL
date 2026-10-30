// Tests of the android video driver: its display, its window, the
// activity's surface and orientation.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function traces what the android video driver gives, against upstream's
//! C SDL:
//!
//! * the display (its name, bounds, usable bounds, content scale, natural
//!   and current orientation), its desktop mode (the activity's metrics:
//!   the device's size, the surface's format, the refresh rate) and its
//!   fullscreen modes, and the system's theme;
//! * a resizable window: what its creation sends (in order: SDL's thread
//!   sends them), its size (the surface's), pixel size, density, display
//!   scale, flags and safe area (the window insets), its display, its
//!   `SDL_PROP_WINDOW_ANDROID_WINDOW_POINTER` and `..._SURFACE_POINTER`
//!   (none: it isn't an OpenGL window), a second window (refused), its
//!   title (the activity's, set on the activity's thread);
//! * fullscreen on and off (the activity's window style, immersive), the
//!   size and safe area then, and the events (a set: the activity's
//!   thread sends them as the surface changes);
//! * the display turned to landscape and back (the runner's `rotate`,
//!   with the rotation locked: the resizable window lets the activity
//!   follow the user's rotation), the display's orientation, the desktop
//!   mode and the window's size then, and the events;
//! * a trip to the background and back: the application's events in
//!   order, the window's as a set, the surface destroyed and made again
//!   (the window's `ANativeWindow`, there again);
//! * an OpenGL window, whose EGL surface is in its
//!   `SDL_PROP_WINDOW_ANDROID_SURFACE_POINTER`.
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

    use sdl3::events::window::WindowFlags;
    use sdl3::events::EventType;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::{self, DisplayOrientation, Window};
    use sdl3::{android, video::window};

    use super::android_support::events::{self as ev, is_app_event};
    use super::android_support::jni;
    use super::android_support::{check_trace, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
video driver: android
displays: 1
display: name=1 bounds=0,0 1080x2400 usable=0,0 1080x2400 content_scale=2.625 natural=3 current=3
display: desktop mode 1080x2400 format=SDL_PIXELFORMAT_RGB565 density=1.000 refresh=60.000
fullscreen modes: 0
current mode: 1080x2400 format=SDL_PIXELFORMAT_RGB565 density=1.000 refresh=60.000
system theme: 1
events after init: (none)
create: ok
created: events: WINDOW_MOUSE_ENTER(0,0), WINDOW_FOCUS_GAINED(0,0), WINDOW_SAFE_AREA_CHANGED(0,0), WINDOW_SHOWN(0,0), WINDOW_PIXEL_SIZE_CHANGED(1080,2400), WINDOW_DISPLAY_SCALE_CHANGED(0,0)
window: size=1080x2400 pixels=1080x2400 density=1.000 display_scale=2.625 flags=0x620 safe_area=78,160 924x2156
window display: yes
android window pointer: yes, android surface pointer: no
second window: error: Android only supports one window
set title: ok
activity title: SDL video
fullscreen on: ok
fullscreen on: size=1080x2400 pixels=1080x2400 density=1.000 display_scale=2.625 flags=0x421 safe_area=78,160 924x2156
fullscreen on: events: WINDOW_ENTER_FULLSCREEN, WINDOW_FOCUS_LOST
fullscreen off: ok
fullscreen off: size=1080x2400 pixels=1080x2400 density=1.000 display_scale=2.625 flags=0x620 safe_area=78,160 924x2156
fullscreen off: events: WINDOW_FOCUS_GAINED, WINDOW_LEAVE_FULLSCREEN
landscape: orientation 1: yes
landscape: name=1 bounds=0,0 2400x1080 usable=0,0 2400x1080 content_scale=2.625 natural=3 current=1
landscape: desktop mode 2400x1080 format=SDL_PIXELFORMAT_RGB565 density=1.000 refresh=60.000
landscape: size=2400x1080 pixels=2400x1080 density=1.000 display_scale=2.625 flags=0x620 safe_area=128,63 2194x933
landscape: events: DISPLAY_CURRENT_MODE_CHANGED, DISPLAY_DESKTOP_MODE_CHANGED, DISPLAY_ORIENTATION, WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST, WINDOW_PIXEL_SIZE_CHANGED, WINDOW_RESIZED, WINDOW_SAFE_AREA_CHANGED
portrait: orientation 3: yes
portrait: name=1 bounds=0,0 1080x2400 usable=0,0 1080x2400 content_scale=2.625 natural=3 current=3
portrait: desktop mode 1080x2400 format=SDL_PIXELFORMAT_RGB565 density=1.000 refresh=60.000
portrait: size=1080x2400 pixels=1080x2400 density=1.000 display_scale=2.625 flags=0x620 safe_area=78,160 924x2156
portrait: events: DISPLAY_CURRENT_MODE_CHANGED, DISPLAY_DESKTOP_MODE_CHANGED, DISPLAY_ORIENTATION, WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST, WINDOW_PIXEL_SIZE_CHANGED, WINDOW_RESIZED, WINDOW_SAFE_AREA_CHANGED
trip: app events: WILL_ENTER_BACKGROUND(0,0), DID_ENTER_BACKGROUND(0,0), WILL_ENTER_FOREGROUND(0,0), DID_ENTER_FOREGROUND(0,0)
trip: window events: WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST, WINDOW_MINIMIZED, WINDOW_RESTORED
trip: android window pointer: yes
trip: size=1080x2400 pixels=1080x2400 density=1.000 display_scale=2.625 flags=0x620 safe_area=78,160 924x2156
opengl window: ok
opengl window: android window pointer: yes, android surface pointer: yes
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

    fn rect(r: &video::Rect) -> String {
        format!("{},{} {}x{}", r.x, r.y, r.w, r.h)
    }

    fn mode(m: &video::DisplayMode) -> String {
        format!(
            "{}x{} format={} density={:.3} refresh={:.3}",
            m.w,
            m.h,
            m.format.name(),
            m.pixel_density,
            m.refresh_rate
        )
    }

    /// The display's state.
    fn display_line(label: &str, display: sdl3::events::DisplayID) {
        t(format!(
            "{label}: name={} bounds={} usable={} content_scale={:.3} natural={} current={}",
            video::display_name(display).unwrap_or_default(),
            video::display_bounds(display).map_or_else(|e| e.to_string(), |r| rect(&r)),
            video::display_usable_bounds(display).map_or_else(|e| e.to_string(), |r| rect(&r)),
            video::display_content_scale(display).unwrap_or(0.0),
            video::natural_display_orientation(display) as i32,
            video::current_display_orientation(display) as i32
        ));
        t(format!(
            "{label}: desktop mode {}",
            video::desktop_display_mode(display).map_or_else(|e| e.to_string(), |m| mode(&m))
        ));
    }

    /// The window's state.
    fn window_line(label: &str, w: &Window) {
        let (width, height) = w.size().unwrap_or_default();
        let (pw, ph) = w.size_in_pixels().unwrap_or_default();
        t(format!(
            "{label}: size={width}x{height} pixels={pw}x{ph} density={:.3} display_scale={:.3} flags=0x{:x} safe_area={}",
            w.pixel_density().unwrap_or(0.0),
            w.display_scale().unwrap_or(0.0),
            w.flags().map_or(0, |f| f.0),
            w.safe_area().map_or_else(|e| e.to_string(), |r| rect(&r))
        ));
    }

    /// Whether the window has the property (a pointer).
    fn has(w: &Window, name: &str) -> bool {
        w.properties()
            .ok()
            .and_then(|p| p.get_number(name))
            .is_some_and(|n| n != 0)
    }

    /// Whether `ty` is a window event.
    fn is_window_event(ty: EventType) -> bool {
        (0x200..0x300).contains(&ty.0)
    }

    /// Whether `ty` is a display event.
    fn is_display_event(ty: EventType) -> bool {
        (0x150..0x200).contains(&ty.0)
    }

    /// The activity's title (`getTitle().toString()`).
    fn activity_title() -> String {
        let env = android::jni_env().expect("a JNI environment").as_ptr();
        let activity = android::activity().expect("the activity").as_ptr();
        let title = jni::call_object(env, activity, "getTitle", "()Ljava/lang/CharSequence;", &[]);
        let string = jni::call_object(env, title, "toString", "()Ljava/lang/String;", &[]);
        let text = jni::string(env, string);
        jni::delete_local_ref(env, string);
        jni::delete_local_ref(env, title);
        jni::delete_local_ref(env, activity);
        text
    }

    /// The display turned to `rotation` (the runner's `rotate`), until the
    /// display's orientation is `wanted`.
    fn rotate(
        label: &str,
        display: sdl3::events::DisplayID,
        w: &Window,
        rotation: u32,
        wanted: DisplayOrientation,
    ) {
        ev::take();
        request(&format!("rotate {rotation}"));
        let came = ev::pump_until(
            || video::current_display_orientation(display) == wanted,
            10_000,
            1500,
        );
        let events = ev::take();
        t(format!(
            "{label}: orientation {}: {}",
            wanted as i32,
            yes(came)
        ));
        display_line(label, display);
        window_line(label, w);
        t(format!(
            "{label}: events: {}",
            ev::names(&events, |ty| is_window_event(ty) || is_display_event(ty))
        ));
    }

    pub(super) fn main() {
        // Video (the default driver), and the events
        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        ev::watch();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        let displays = video::displays().unwrap_or_default();
        t(format!("displays: {}", displays.len()));
        let display = displays[0];
        display_line("display", display);
        let modes = video::fullscreen_display_modes(display).unwrap_or_default();
        t(format!("fullscreen modes: {}", modes.len()));
        for (i, m) in modes.iter().enumerate() {
            t(format!("fullscreen mode {i}: {}", mode(m)));
        }
        t(format!(
            "current mode: {}",
            video::current_display_mode(display).map_or_else(|e| e.to_string(), |m| mode(&m))
        ));
        t(format!("system theme: {}", video::system_theme() as i32));
        ev::pump_for(200);
        t(format!(
            "events after init: {}",
            ev::ordered(&ev::take(), |ty| !is_app_event(ty))
        ));

        // A window
        let created = Window::create("android video", 320, 240, WindowFlags::RESIZABLE);
        t(format!("create: {}", result(&created)));
        let w = created.expect("the window");
        t(format!(
            "created: events: {}",
            ev::ordered(&ev::take(), |ty| is_window_event(ty)
                || is_display_event(ty))
        ));
        window_line("window", &w);
        t(format!(
            "window display: {}",
            yes(w.display().ok() == Some(display))
        ));
        t(format!(
            "android window pointer: {}, android surface pointer: {}",
            yes(has(&w, window::PROP_WINDOW_ANDROID_WINDOW_POINTER)),
            yes(has(&w, window::PROP_WINDOW_ANDROID_SURFACE_POINTER))
        ));
        let second = Window::create("second", 100, 100, WindowFlags::default());
        t(format!("second window: {}", result(&second)));
        if let Ok(second) = second {
            second.destroy();
        }
        ev::pump_for(500);
        ev::take();
        t(format!("set title: {}", result(&w.set_title("SDL video"))));
        ev::pump_for(500);
        t(format!("activity title: {}", activity_title()));

        // Fullscreen
        for (label, on) in [("fullscreen on", true), ("fullscreen off", false)] {
            ev::take();
            t(format!("{label}: {}", result(&w.set_fullscreen(on))));
            ev::pump_for(2000);
            let events = ev::take();
            window_line(label, &w);
            t(format!(
                "{label}: events: {}",
                ev::names(&events, |ty| is_window_event(ty) || is_display_event(ty))
            ));
        }

        // The display turned
        rotate("landscape", display, &w, 1, DisplayOrientation::Landscape);
        rotate("portrait", display, &w, 0, DisplayOrientation::Portrait);

        // A trip to the background and back: the surface destroyed, then
        // made again
        let events = ev::trip(1500);
        t(format!(
            "trip: app events: {}",
            ev::ordered(&events, |ty| is_app_event(ty)
                && ty != EventType::LOW_MEMORY)
        ));
        t(format!(
            "trip: window events: {}",
            ev::names(&events, is_window_event)
        ));
        t(format!(
            "trip: android window pointer: {}",
            yes(has(&w, window::PROP_WINDOW_ANDROID_WINDOW_POINTER))
        ));
        window_line("trip", &w);
        w.destroy();
        ev::pump_for(200);
        ev::take();

        // An OpenGL window: its EGL surface
        let created = Window::create("android gl", 320, 240, WindowFlags::OPENGL);
        t(format!("opengl window: {}", result(&created)));
        if let Ok(w) = created {
            t(format!(
                "opengl window: android window pointer: {}, android surface pointer: {}",
                yes(has(&w, window::PROP_WINDOW_ANDROID_WINDOW_POINTER)),
                yes(has(&w, window::PROP_WINDOW_ANDROID_SURFACE_POINTER))
            ));
            w.destroy();
        }
        ev::pump_for(200);
        ev::take();

        ev::unwatch();
        init::quit();
        t("quit");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
