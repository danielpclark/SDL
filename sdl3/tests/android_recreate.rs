// A test of an Android app whose activity is recreated: upstream's
// SDLActivity ends the process.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, with a
//! window, whose display's density changes (the runner's `density`: a
//! configuration change upstream's manifest doesn't handle, so the system
//! recreates the activity). The activity being destroyed sends SDL's
//! thread `SDL_EVENT_QUIT` and `SDL_EVENT_TERMINATING`, and waits for it
//! (up to a second); `main` returns then, and the activity made again,
//! finding SDL's main thread finished (and
//! `SDL_HINT_ANDROID_ALLOW_RECREATE_ACTIVITY` unset), ends the process
//! (`System.exit(0)`). The trace (the events from `SDL_EVENT_QUIT` on, as
//! a pause may come first, or not) is compared with upstream's C SDL's,
//! from the same program in C on the same emulator, and the runner checks
//! that the process ends (`android_support::finish_once_the_process_ends`).
//! Elsewhere this program does nothing.

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
    if status == 0 {
        android_support::finish_once_the_process_ends(status)
    } else {
        android_support::finish(status)
    }
}

#[cfg(target_os = "android")]
mod app {
    use std::sync::Mutex;

    use sdl3::events::window::WindowFlags;
    use sdl3::events::EventType;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::{self, Window};

    use super::android_support::events::{self as ev, is_app_event};
    use super::android_support::{check_trace, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
init: ok
video driver: android
window: ok
density changed: QUIT(0,0), TERMINATING(0,0)
terminating: yes
main returns
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    fn result<T>(r: &sdl3::Result<T>) -> String {
        match r {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {e}"),
        }
    }

    /// The events from `QUIT` on.
    fn from_quit(events: &str) -> &str {
        events.find("QUIT").map_or(events, |i| &events[i..])
    }

    pub(super) fn main() {
        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        ev::watch();
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        let created = Window::create("android recreate", 320, 240, WindowFlags::default());
        t(format!("window: {}", result(&created)));
        ev::pump_for(500);
        ev::take();

        // The display's density changed: the activity recreated
        request("density 320");
        let terminated = ev::pump_until(|| ev::saw(EventType::TERMINATING), 15_000, 0);
        let events = ev::take();
        t(format!(
            "density changed: {}",
            from_quit(&ev::ordered(&events, |ty| is_app_event(ty)
                && ty != EventType::LOW_MEMORY))
        ));
        t(format!(
            "terminating: {}",
            if terminated { "yes" } else { "no" }
        ));

        if let Ok(w) = created {
            w.destroy();
        }
        ev::unwatch();
        init::quit();
        t("main returns");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
