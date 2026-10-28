// Tests of the main callbacks on UIKit.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! On UIKit the main callbacks run from a `CADisplayLink`, once the main
//! function that `run_app()` (`UIApplicationMain()`) calls has returned to
//! the run loop, and SDL ends the process with `exit()` when they're done,
//! so this is a program of its own (`harness = false`), run as an app on
//! the iOS Simulator by `tools/ios_run.py` (the `__DATA,__sdl3_runner`
//! section below). It traces the order of the main function, the app's
//! init, iterations and events (a queued display event, an application
//! event from a notification posted during an iteration, a user event),
//! and quit; the iterations' pace (the display's, 60 Hz on the simulated
//! iPhone); and that the process exits after quit, from an `atexit()`
//! handler, which prints the runner's exit line (upstream's exit status,
//! 0 after `AppResult::Success`, isn't visible to the runner).
//!
//! The expected trace is upstream's C SDL's, from the same program in C
//! run the same way on the same simulator (an iPhone 16 with iOS 18.5),
//! the mice's device events left out (see `tests/uikit_main.rs`). It runs
//! on the tvOS Simulator too (an Apple TV 4K (3rd generation) with tvOS
//! 18.5), against upstream's C there. Elsewhere this program does nothing.

#[cfg(not(any(target_os = "ios", target_os = "tvos")))]
fn main() {}

/// The runner's words: run as an app.
#[cfg(any(target_os = "ios", target_os = "tvos"))]
#[used]
#[link_section = "__DATA,__sdl3_runner"]
static RUNNER: [u8; 4] = *b"app\0";

#[cfg(any(target_os = "ios", target_os = "tvos"))]
mod uikit_support;

#[cfg(any(target_os = "ios", target_os = "tvos"))]
fn main() {
    ios::main()
}

#[cfg(any(target_os = "ios", target_os = "tvos"))]
mod ios {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use sdl3::app::{self, AppCallbacks, AppResult};
    use sdl3::events::{self, Event, EventType, UserEvent};
    use sdl3::init::{self, InitFlags};

    use super::uikit_support::{
        check_trace, checked, elapsed, event_name, exit, objc, print_exit_line,
    };

    /// Upstream's trace (the C program, on an iPhone 16 simulator with
    /// iOS 18.5).
    #[cfg(target_os = "ios")]
    const EXPECTED: &str = "\
main: argc=1
init: argc=1
init video: ok
main: enter_app_main_callbacks returned 0
main returns
event DISPLAY_ORIENTATION(3)
event KEYBOARD_ADDED
warmup: application state 0
iterate 1
iterate 2: posting a memory warning
event LOW_MEMORY
iterate 3: pushing a user event
event USER
iterate 4
iterate 5: success
quit: SUCCESS
iterations paced by the display: yes
exit: after quit
";

    /// Upstream's trace on tvOS (the C program, on an Apple TV 4K (3rd
    /// generation) simulator with tvOS 18.5).
    #[cfg(target_os = "tvos")]
    const EXPECTED: &str = "\
main: argc=1
init: argc=1
init video: ok
main: enter_app_main_callbacks returned 0
main returns
event KEYBOARD_ADDED
warmup: application state 0
iterate 1
iterate 2: posting a memory warning
event LOW_MEMORY
iterate 3: pushing a user event
event USER
iterate 4
iterate 5: success
quit: SUCCESS
iterations paced by the display: yes
exit: after quit
";

    static TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());
    /// When each iteration started.
    static ITERATIONS: Mutex<Vec<Instant>> = Mutex::new(Vec::new());
    static QUIT: Mutex<Option<AppResult>> = Mutex::new(None);

    fn t(line: impl Into<String>) {
        let line = line.into();
        println!("trace: {line}");
        TRACE.lock().unwrap().push(line);
    }

    /// The app's state: before the scenario, the iterations wait for the
    /// app to become active (a moment after the main function returns),
    /// the application events of that left out of the trace (when they
    /// come varies).
    struct App {
        warmup: bool,
        active_iterations: u32,
        warmup_start: Instant,
    }

    fn application_state() -> isize {
        // SAFETY: a UIApplication property.
        unsafe { objc::send0(objc::shared_application(), "applicationState") }
    }

    impl AppCallbacks for App {
        fn iterate(&mut self) -> AppResult {
            if self.warmup {
                if application_state() == 0 {
                    self.active_iterations += 1;
                }
                // (a few more iterations for the rest of the transition)
                if self.active_iterations >= 10
                    || self.warmup_start.elapsed() > Duration::from_secs(30)
                {
                    self.warmup = false;
                    println!(
                        "uikit_callbacks: {:.3} s: application state {}",
                        elapsed(),
                        application_state()
                    );
                    t(format!("warmup: application state {}", application_state()));
                }
                return AppResult::Continue;
            }
            let n = {
                let mut iterations = ITERATIONS.lock().unwrap();
                iterations.push(Instant::now());
                iterations.len()
            };
            match n {
                2 => {
                    t("iterate 2: posting a memory warning");
                    // SAFETY: UIKit's notification name.
                    objc::post_notification(unsafe {
                        objc::UIApplicationDidReceiveMemoryWarningNotification
                    });
                    AppResult::Continue
                }
                3 => {
                    t("iterate 3: pushing a user event");
                    events::push(Event::User(UserEvent {
                        event_type: EventType::USER,
                        timestamp: Duration::ZERO,
                        window_id: 0,
                        code: 0,
                        data1: None,
                        data2: None,
                    }))
                    .unwrap();
                    AppResult::Continue
                }
                5 => {
                    t("iterate 5: success");
                    AppResult::Success
                }
                _ => {
                    t(format!("iterate {n}"));
                    AppResult::Continue
                }
            }
        }

        fn event(&mut self, event: &Event) -> AppResult {
            let ty = event.event_type();
            let name = event_name(ty);
            let lifecycle =
                (EventType::TERMINATING.0..=EventType::DID_ENTER_FOREGROUND.0).contains(&ty.0);
            if lifecycle {
                println!("uikit_callbacks: {:.3} s: {name}", elapsed());
            }
            match event {
                // (see the module's description)
                Event::MouseDevice(_) => {}
                _ if lifecycle && self.warmup => {}
                Event::Display(d) => t(format!("event {name}({})", d.data1)),
                _ => t(format!("event {name}")),
            }
            AppResult::Continue
        }

        fn quit(&mut self, result: AppResult) {
            t(format!("quit: {}", format!("{result:?}").to_uppercase()));
            *QUIT.lock().unwrap() = Some(result);
        }
    }

    unsafe extern "C" {
        fn atexit(f: extern "C" fn()) -> std::ffi::c_int;
    }

    /// The process is exiting: the last checks, and the runner's exit line.
    extern "C" fn at_exit() {
        let status = checked(|| {
            let iterations = ITERATIONS.lock().unwrap().clone();
            // (60 Hz: 16.7 ms apart, give or take what the simulator does)
            let paced = iterations.len() == 5
                && iterations.windows(2).all(|w| {
                    let d = w[1] - w[0];
                    d >= Duration::from_millis(4) && d <= Duration::from_millis(100)
                });
            t(format!(
                "iterations paced by the display: {}",
                if paced { "yes" } else { "no" }
            ));
            if !paced {
                println!(
                    "iteration intervals: {:?}",
                    iterations
                        .windows(2)
                        .map(|w| w[1] - w[0])
                        .collect::<Vec<_>>()
                );
            }
            t(if QUIT.lock().unwrap().is_some() {
                "exit: after quit"
            } else {
                "exit: without quit"
            });
            check_trace(&TRACE.lock().unwrap(), EXPECTED);
        });
        print_exit_line(status);
    }

    pub(super) fn main() {
        // SAFETY: a function to run at exit().
        unsafe { atexit(at_exit) };
        elapsed();
        let status = app::run_app(|args| {
            t(format!("main: argc={}", args.len()));
            let rc = app::enter_app_main_callbacks(args, |args| {
                t(format!("init: argc={}", args.len()));
                let r = init::init(InitFlags::VIDEO);
                t(format!(
                    "init video: {}",
                    r.as_ref().map_or_else(|e| e.to_string(), |_| "ok".into())
                ));
                let app = App {
                    warmup: true,
                    active_iterations: 0,
                    warmup_start: Instant::now(),
                };
                (app, AppResult::Continue)
            });
            t(format!("main: enter_app_main_callbacks returned {rc}"));
            t("main returns");
            rc
        });
        println!("uikit_callbacks: run_app returned {status}, which it never does");
        exit(1);
    }
}
