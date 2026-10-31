// Tests of the android video driver's message boxes: the activity's
// AlertDialog, answered by real taps and keys.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! An app (`harness = false`), run by `tools/android_run.py`, whose main
//! function shows message boxes, each answered from outside by the runner
//! while SDL's thread waits in `show_message_box()` (the requests are made
//! on a thread of their own): the box's views as uiautomator dumps them
//! (the title, the message, the buttons and their places on the screen:
//! the runner's `ui-dump`), the colors of the message and of a button
//! where a color scheme is given (`ui-colors`: the two most frequent
//! colors of the view's place in a screen capture), then a real tap on a
//! button found in that dump (`ui-tap`), or a key (Enter, Escape) the box
//! maps to its buttons (`input keyevent`). Each box's result (the
//! button's ID) and the window's events while it was up are traced. The
//! boxes:
//!
//! * a simple one before `init()` (the bootstrap's: no video yet);
//! * a simple one, of the window;
//! * three buttons, left to right and right to left (reversed);
//! * Enter and Escape, the return and escape keys' defaults;
//! * a color scheme (the background, the text and the buttons'
//!   background are upstream's; the border and selected colors aren't
//!   used), and the same box without one;
//! * empty title and message;
//! * one shown from another thread of SDL's;
//! * `sdl_assert_always!`'s (the default assertion handler's prompt, with
//!   `SDL_HINT_ASSERT` unset: Retry, Break, Abort, Ignore, Always
//!   Ignore), answered Ignore, then Always Ignore (no box the third time).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! (built with upstream's android-project, Gradle and CMake, run by the
//! runner's `--run-apk`) on the same emulator (API 35, Google APIs,
//! x86-64). Elsewhere this program does nothing.

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
    use std::thread::JoinHandle;

    use sdl3::events::window::WindowFlags;
    use sdl3::events::{self, Event, EventWatch};
    use sdl3::hints;
    use sdl3::init::{self, InitFlags};
    use sdl3::video::messagebox::{
        show_message_box, show_simple_message_box, MessageBoxButtonData, MessageBoxButtonFlags,
        MessageBoxColor, MessageBoxColorScheme, MessageBoxData, MessageBoxFlags,
    };
    use sdl3::video::{self, Window};

    use super::android_support::events::pump_for;
    use super::android_support::{check_trace, event_name, request};

    /// Upstream's trace (the C program, on the emulator: API 35, Google
    /// APIs, x86-64).
    const EXPECTED: &str = "\
logcat: ui: TextView \"Before init\" [397,1079][687,1173]
logcat: ui: TextView \"No video yet\" [371,1213][713,1264]
logcat: ui: Button \"OK\" [488,1264][595,1390]
logcat: ui: tapped OK
before init: simple: ok
init: ok
video driver: android
window: ok
start: window events: WINDOW_DISPLAY_SCALE_CHANGED, WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST, WINDOW_MOUSE_ENTER, WINDOW_PIXEL_SIZE_CHANGED, WINDOW_RESIZED, WINDOW_SAFE_AREA_CHANGED, WINDOW_SHOWN
logcat: ui: TextView \"Simple title\" [1109,386][1423,480]
logcat: ui: TextView \"Simple message\" [1083,520][1449,571]
logcat: ui: Button \"OK\" [1212,571][1319,697]
logcat: ui: tapped OK
simple: ok
simple: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: TextView \"Three buttons\" [1073,386][1459,480]
logcat: ui: TextView \"Left to right\" [1047,520][1485,571]
logcat: ui: Button \"Yes\" [1069,571][1187,697]
logcat: ui: Button \"No\" [1187,571][1292,697]
logcat: ui: Button \"Cancel\" [1292,571][1462,697]
logcat: ui: tapped Cancel
left to right: ok, button 3
left to right: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: TextView \"Three buttons\" [1073,386][1459,480]
logcat: ui: TextView \"Right to left\" [1047,520][1485,571]
logcat: ui: Button \"Cancel\" [1069,571][1239,697]
logcat: ui: Button \"No\" [1239,571][1344,697]
logcat: ui: Button \"Yes\" [1344,571][1462,697]
logcat: ui: tapped Yes
right to left: ok, button 1
right to left: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: TextView \"Three buttons\" [1073,386][1459,480]
logcat: ui: TextView \"Enter\" [1047,520][1485,571]
logcat: ui: Button \"Yes\" [1069,571][1187,697]
logcat: ui: Button \"No\" [1187,571][1292,697]
logcat: ui: Button \"Cancel\" [1292,571][1462,697]
enter: ok, button 1
enter: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: TextView \"Three buttons\" [1073,386][1459,480]
logcat: ui: TextView \"Escape\" [1047,520][1485,571]
logcat: ui: Button \"Yes\" [1069,571][1187,697]
logcat: ui: Button \"No\" [1187,571][1292,697]
logcat: ui: Button \"Cancel\" [1292,571][1462,697]
escape: ok, button 2
escape: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: TextView \"Colors\" [1141,386][1391,480]
logcat: ui: TextView \"Colored message\" [1115,520][1417,571]
logcat: ui: Button \"Colored button\" [1115,571][1417,697]
logcat: colors Colored message: #204060 #ffee00
logcat: colors Colored button: #b51501 #b41602
logcat: ui: tapped Colored button
colors: ok, button 42
colors: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: colors Colored message: #000000 #bebebe
logcat: colors Colored button: #000000 #d5d5d5
logcat: ui: tapped Colored button
no colors: ok, button 42
no colors: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: Button \"Fine\" [1203,504][1330,630]
logcat: ui: tapped Fine
empty: ok, button 7
empty: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: Button \"Fine\" [1203,504][1330,630]
logcat: ui: tapped Fine
null: ok, button 7
null: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: TextView \"Another thread\" [1050,386][1482,480]
logcat: ui: TextView \"Shown from a thread of SDL's\" [1024,520][1508,571]
logcat: ui: Button \"Fine\" [1202,571][1329,697]
logcat: ui: tapped Fine
thread: button 7
thread: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: Button \"Retry\"
logcat: ui: Button \"Break\"
logcat: ui: Button \"Abort\"
logcat: ui: Button \"Ignore\"
logcat: ui: Button \"Always Ignore\"
logcat: ui: tapped Ignore
assert 0: returned
assert: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
logcat: ui: tapped Always Ignore
assert 1: returned
assert: window events: WINDOW_FOCUS_LOST, WINDOW_FOCUS_GAINED
assert 2: returned
assert: window events: (none)
assert report: triggered 3 times, always ignore: yes
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

    fn yes(b: bool) -> &'static str {
        if b {
            "yes"
        } else {
            "no"
        }
    }

    /// The window's events since the last [`trace_events`].
    static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn watch() -> EventWatch {
        events::add_watch(|event| {
            if let Event::Window(e) = event {
                EVENTS.lock().unwrap().push(event_name(e.event_type));
            }
        })
    }

    fn trace_events(step: &str) {
        trace_window_events(step, false);
    }

    /// The window's events since the last step, in their order, or (`set`)
    /// sorted and each once: the order of a new window's own (its surface's
    /// size, safe area and the rest) is the order the activity's callbacks
    /// happen to arrive in, which differs from run to run.
    fn trace_window_events(step: &str, set: bool) {
        pump_for(800);
        let mut events = std::mem::take(&mut *EVENTS.lock().unwrap());
        if set {
            events.sort();
            events.dedup();
        }
        t(format!(
            "{step}: window events: {}",
            if events.is_empty() {
                "(none)".to_string()
            } else {
                events.join(", ")
            }
        ));
    }

    /// The runner's requests, made on a thread of their own while SDL's
    /// waits in the box (one after the other): their answers.
    fn ask(whats: &[&str]) -> JoinHandle<Vec<String>> {
        let whats: Vec<String> = whats.iter().map(|w| w.to_string()).collect();
        std::thread::Builder::new()
            .name("asks".into())
            .spawn(move || whats.iter().flat_map(|what| request(what)).collect())
            .unwrap()
    }

    /// The answers of [`ask`]'s requests, traced as the C program's are
    /// (the runner's lines, `logcat: LINE`).
    fn answers(asker: JoinHandle<Vec<String>>) {
        for line in asker.join().expect("the requests' thread") {
            t(format!("logcat: {line}"));
        }
    }

    fn show(step: &str, data: &MessageBoxData, whats: &[&str]) {
        let asker = ask(whats);
        let shown = show_message_box(data);
        answers(asker);
        t(format!(
            "{step}: {}, button {}",
            result(&shown),
            shown.as_ref().copied().unwrap_or(-2)
        ));
        trace_events(step);
    }

    fn button(flags: MessageBoxButtonFlags, button_id: i32, text: &str) -> MessageBoxButtonData {
        MessageBoxButtonData {
            flags,
            button_id,
            text: text.to_string(),
        }
    }

    pub(super) fn main() {
        // Before init(): the bootstrap's message box
        {
            let asker = ask(&["ui-dump app", "ui-tap OK"]);
            let shown = show_simple_message_box(
                MessageBoxFlags::INFORMATION,
                "Before init",
                "No video yet",
                None,
            );
            answers(asker);
            t(format!("before init: simple: {}", result(&shown)));
        }

        t(format!("init: {}", result(&init::init(InitFlags::VIDEO))));
        t(format!(
            "video driver: {}",
            video::current_video_driver().unwrap_or("(none)")
        ));
        let watch = watch();
        let win = Window::create("SDL message boxes", 320, 240, WindowFlags::default());
        t(format!("window: {}", result(&win)));
        let window = win.as_ref().ok().copied();
        pump_for(1500);
        trace_window_events("start", true);

        // A simple box, of the window
        {
            let asker = ask(&["ui-dump app", "ui-tap OK"]);
            let shown = show_simple_message_box(
                MessageBoxFlags::ERROR,
                "Simple title",
                "Simple message",
                window.as_ref(),
            );
            answers(asker);
            t(format!("simple: {}", result(&shown)));
            trace_events("simple");
        }

        // Three buttons
        let three = vec![
            button(MessageBoxButtonFlags::RETURNKEY_DEFAULT, 1, "Yes"),
            button(MessageBoxButtonFlags::ESCAPEKEY_DEFAULT, 2, "No"),
            button(MessageBoxButtonFlags::NONE, 3, "Cancel"),
        ];
        let mut data = MessageBoxData {
            flags: MessageBoxFlags::WARNING | MessageBoxFlags::BUTTONS_LEFT_TO_RIGHT,
            window,
            title: "Three buttons".to_string(),
            message: "Left to right".to_string(),
            buttons: three,
            color_scheme: None,
        };
        show("left to right", &data, &["ui-dump app", "ui-tap Cancel"]);
        data.flags = MessageBoxFlags::WARNING | MessageBoxFlags::BUTTONS_RIGHT_TO_LEFT;
        data.message = "Right to left".to_string();
        show("right to left", &data, &["ui-dump app", "ui-tap Yes"]);

        // The keys the box maps to its buttons (Enter: the return key's
        // default; Escape: the escape key's)
        data.flags = MessageBoxFlags::INFORMATION;
        data.message = "Enter".to_string();
        show(
            "enter",
            &data,
            &["ui-dump app", "input keyevent KEYCODE_ENTER"],
        );
        data.message = "Escape".to_string();
        show(
            "escape",
            &data,
            &["ui-dump app", "input keyevent KEYCODE_ESCAPE"],
        );

        // A color scheme
        let color = |r, g, b| MessageBoxColor { r, g, b };
        let scheme = MessageBoxColorScheme {
            colors: [
                color(0x20, 0x40, 0x60), // background
                color(0xff, 0xee, 0x00), // text
                color(0x00, 0xff, 0x00), // button border
                color(0xc0, 0x30, 0x30), // button background
                color(0x00, 0x00, 0xff), // button selected
            ],
        };
        data.flags = MessageBoxFlags::INFORMATION;
        data.title = "Colors".to_string();
        data.message = "Colored message".to_string();
        data.buttons = vec![button(
            MessageBoxButtonFlags::RETURNKEY_DEFAULT | MessageBoxButtonFlags::ESCAPEKEY_DEFAULT,
            42,
            "Colored button",
        )];
        data.color_scheme = Some(scheme);
        show(
            "colors",
            &data,
            &[
                "ui-dump app",
                "ui-colors Colored message",
                "ui-colors Colored button",
                "ui-tap Colored button",
            ],
        );
        // (the same box without the scheme, for its colors)
        data.color_scheme = None;
        data.title = "No colors".to_string();
        show(
            "no colors",
            &data,
            &[
                "ui-colors Colored message",
                "ui-colors Colored button",
                "ui-tap Colored button",
            ],
        );

        // Empty title and message
        data.title = String::new();
        data.message = String::new();
        data.buttons = vec![button(MessageBoxButtonFlags::NONE, 7, "Fine")];
        show("empty", &data, &["ui-dump app", "ui-tap Fine"]);
        // (upstream's null title and message are empty ones)
        show("null", &data, &["ui-dump app", "ui-tap Fine"]);

        // From another thread
        data.title = "Another thread".to_string();
        data.message = "Shown from a thread of SDL's".to_string();
        {
            let asker = ask(&["ui-dump app", "ui-tap Fine"]);
            let thread_data = data.clone();
            let thread = sdl3::thread::Thread::spawn("box", move || {
                show_message_box(&thread_data).unwrap_or(-100)
            })
            .expect("a thread");
            let status = thread.wait();
            answers(asker);
            t(format!("thread: button {status}"));
            trace_events("thread");
        }

        // The assertion's box (the default handler's prompt: no SDL_ASSERT)
        let _ = hints::set_with_priority(hints::ASSERT, None, hints::Priority::Override);
        for i in 0..3 {
            let asker = match i {
                0 => Some(ask(&[
                    "ui-dump app --class Button --no-bounds",
                    "ui-tap Ignore",
                ])),
                1 => Some(ask(&["ui-tap Always Ignore"])),
                _ => None,
            };
            sdl3::sdl_assert_always!(i < 0);
            if let Some(asker) = asker {
                answers(asker);
            }
            t(format!("assert {i}: returned"));
            trace_events("assert");
        }
        let report = sdl3::assert::assertion_report();
        t(format!(
            "assert report: triggered {} times, always ignore: {}",
            report.first().map_or(0, |a| a.trigger_count()),
            yes(report.first().is_some_and(|a| a.always_ignore()))
        ));
        sdl3::assert::reset_assertion_report();

        drop(watch);
        if let Ok(w) = win {
            w.destroy();
        }
        init::quit();
        t("main returns");
        let trace = TRACE.lock().unwrap().clone();
        check_trace(&trace, EXPECTED);
    }
}
