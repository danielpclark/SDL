// Tests of the Emscripten main callbacks, timers and SDL_RunApp().
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! On Emscripten the browser's event loop runs the main callbacks
//! (`emscripten_set_main_loop()`) and the timers (`emscripten_set_timeout()`),
//! after `main` has returned to it, and the main loop ends the program with
//! `exit()`. The test harness can't see any of that, so this is a program
//! of its own (`harness = false`), run under node, whose `main` is an
//! application's: `run_app()` and `enter_app_main_callbacks()`, then back to
//! the event loop.
//!
//! The scenarios that end with another exit status (a failure, an init
//! failure, a JavaScript exception) run in child programs: this same
//! program, which the first one starts again with node's `child_process`
//! and `SDL3_EM_SCENARIO` set, and whose status and output it checks. The
//! expected traces (iterations, timer order, rates, exit statuses) are
//! upstream's C SDL's, built with emcc and run under node the same way.
//! Under node there's no `requestAnimationFrame()`: emscripten's stand-in
//! is a `setTimeout()` at 60 Hz.

#[cfg(not(target_os = "emscripten"))]
fn main() {}

#[cfg(target_os = "emscripten")]
fn main() -> std::process::ExitCode {
    emscripten::main()
}

#[cfg(target_os = "emscripten")]
mod emscripten {
    use std::process::ExitCode;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use sdl3::app::{self, AppCallbacks, AppResult};
    use sdl3::events::{self, Event, EventType, UserEvent};
    use sdl3::hints;
    use sdl3::timer::{self, Timer};
    use sdl3_emscripten::{em_asm, em_asm_int};

    /// What happened, with when (SDL ticks).
    type Trace = Arc<Mutex<Vec<(String, Duration)>>>;

    fn record(trace: &Trace, what: impl Into<String>) {
        let what = what.into();
        let now = timer::ticks();
        println!("{what} at {:.1} ms", now.as_secs_f64() * 1000.0);
        trace.lock().unwrap().push((what, now));
    }

    fn when(trace: &[(String, Duration)], what: &str) -> Duration {
        match trace.iter().find(|(w, _)| w == what) {
            Some((_, t)) => *t,
            None => panic!("no {what:?} in {trace:#?}"),
        }
    }

    fn push_user_event() {
        events::push(Event::User(UserEvent {
            event_type: EventType::USER,
            timestamp: Duration::ZERO,
            window_id: 0,
            code: 0,
            data1: None,
            data2: None,
        }))
        .unwrap();
    }

    fn scenario() -> String {
        std::env::var("SDL3_EM_SCENARIO").unwrap_or_default()
    }

    /// The app of upstream's reference program: eight iterations, with
    /// timers set up in `init`.
    struct App {
        scenario: String,
        iterations: u32,
        trace: Trace,
        start: Duration,
        finished: Option<Timer>,
        _kept: Vec<Timer>,
    }

    impl App {
        fn init(scenario: String) -> (App, AppResult) {
            let trace: Trace = Arc::new(Mutex::new(Vec::new()));
            if scenario == "initfail" {
                record(&trace, "init fails");
                let app = App {
                    scenario,
                    iterations: 0,
                    trace,
                    start: timer::ticks(),
                    finished: None,
                    _kept: Vec::new(),
                };
                return (app, AppResult::Failure);
            }
            match scenario.as_str() {
                "rate" => {
                    hints::set(hints::MAIN_CALLBACK_RATE, "20").unwrap();
                }
                "waitevent" => {
                    hints::set(hints::MAIN_CALLBACK_RATE, "waitevent").unwrap();
                }
                _ => {}
            }
            let waitevent = scenario == "waitevent";
            let start = timer::ticks();

            let one_shot = |name: &'static str, trace: &Trace| {
                let trace = trace.clone();
                move |interval: Duration| {
                    record(
                        &trace,
                        format!("timer {name} ({} ms)", interval.as_millis()),
                    );
                    if waitevent {
                        push_user_event();
                    }
                    None
                }
            };
            let b30 = Timer::new(Duration::from_millis(30), one_shot("b30", &trace)).unwrap();
            let a10 = Timer::new(Duration::from_millis(10), one_shot("a10", &trace)).unwrap();
            let t = trace.clone();
            let mut count = 0;
            let periodic = Timer::new(Duration::from_millis(20), move |interval| {
                count += 1;
                record(
                    &t,
                    format!("periodic #{count} ({} ms)", interval.as_millis()),
                );
                if waitevent {
                    push_user_event();
                }
                (count < 3).then(|| interval + Duration::from_millis(10))
            })
            .unwrap();

            // Cancelled ones never fire: by cancel(), by drop, and from
            // another timer's callback.
            let t = trace.clone();
            let cancelled = Timer::new(Duration::from_millis(15), move |_| {
                record(&t, "cancelled timer fired");
                None
            })
            .unwrap();
            assert!(cancelled.is_active());
            assert!(cancelled.cancel());
            let t = trace.clone();
            drop(Timer::new(Duration::from_millis(5), move |_| {
                record(&t, "dropped timer fired");
                None
            }));
            let t = trace.clone();
            let late = Arc::new(Mutex::new(Some(
                Timer::new(Duration::from_millis(60), move |_| {
                    record(&t, "late timer fired");
                    None
                })
                .unwrap(),
            )));
            let t = trace.clone();
            Timer::new(Duration::from_millis(25), move |_| {
                let late = late.lock().unwrap().take().unwrap();
                record(&t, format!("cancel late: {}", late.cancel()));
                None
            })
            .unwrap()
            .detach();

            if scenario == "pending" {
                // Still pending when the app quits: SDL_Quit() drops it, and
                // the program ends (upstream's keeps calling it, forever).
                let t = trace.clone();
                Timer::new(Duration::from_millis(300), move |interval| {
                    record(&t, "forever");
                    Some(interval)
                })
                .unwrap()
                .detach();
            }
            if scenario == "exit-in-timer" {
                Timer::new(Duration::from_millis(40), |_| {
                    println!("exit(7) from a timer");
                    std::process::exit(7)
                })
                .unwrap()
                .detach();
            }

            record(&trace, "init");
            let app = App {
                scenario,
                iterations: 0,
                trace,
                start,
                finished: Some(a10),
                _kept: vec![b30, periodic],
            };
            (app, AppResult::Continue)
        }

        fn last_iteration(&self) -> u32 {
            if self.scenario == "waitevent" {
                3
            } else {
                8
            }
        }

        /// The checks of the trace, once the app quits.
        fn check(&self, result: AppResult) {
            let trace = self.trace.lock().unwrap().clone();
            let expected = if self.scenario == "failure" {
                AppResult::Failure
            } else {
                AppResult::Success
            };
            assert_eq!(result, expected);
            assert_eq!(self.iterations, self.last_iteration());
            let names: Vec<&str> = trace.iter().map(|(w, _)| w.as_str()).collect();
            for never in [
                "cancelled timer fired",
                "dropped timer fired",
                "late timer fired",
                "forever",
            ] {
                assert!(!names.contains(&never), "{never}: {names:?}");
            }
            assert!(names.contains(&"cancel late: true"), "{names:?}");

            // The timers, in order, each after its interval.
            let since = |what: &str| when(&trace, what) - self.start;
            let ms = Duration::from_millis;
            let a10 = since("timer a10 (10 ms)");
            let p1 = since("periodic #1 (20 ms)");
            let b30 = since("timer b30 (30 ms)");
            // (node's timers count from its loop's millisecond clock, which
            // can be up to a millisecond behind SDL's ticks)
            let slack = ms(1);
            assert!(
                a10 + slack >= ms(10) && p1 + slack >= ms(20) && b30 + slack >= ms(30),
                "{trace:#?}"
            );
            assert!(a10 < p1 && p1 < b30, "{trace:#?}");
            if self.scenario != "waitevent" {
                let p2 = since("periodic #2 (30 ms)");
                let p3 = since("periodic #3 (40 ms)");
                assert!(
                    p2 + slack >= p1 + ms(30) && p3 + slack >= p2 + ms(40),
                    "{trace:#?}"
                );
                assert!(b30 < p2, "{trace:#?}");
            }

            // The iterations: at 60 Hz (the stand-in for
            // requestAnimationFrame()), or every 50 ms with the rate hint
            // at 20 Hz (from the second on: the first follows the 60 Hz
            // one that emscripten_set_main_loop() starts with).
            let iterations: Vec<Duration> = (1..=self.iterations)
                .map(|n| when(&trace, &format!("iterate {n}")))
                .collect();
            let gaps: Vec<Duration> = iterations.windows(2).map(|w| w[1] - w[0]).collect();
            match self.scenario.as_str() {
                "rate" => {
                    assert!(gaps[1..].iter().all(|&g| g >= ms(48)), "{gaps:?}");
                }
                "waitevent" => {
                    // One at once, then one only after new events: each
                    // event a timer pushes (dispatched in the iteration).
                    let user = format!("event {:?}", EventType::USER);
                    let positions = |what: &str| {
                        trace
                            .iter()
                            .position(|(w, _)| w == what)
                            .unwrap_or_else(|| panic!("{what}: {trace:#?}"))
                    };
                    for n in 2..=self.iterations {
                        let (from, to) = (
                            positions(&format!("iterate {}", n - 1)),
                            positions(&format!("iterate {n}")),
                        );
                        assert!(
                            trace[from..to].iter().any(|(w, _)| *w == user),
                            "no event before iterate {n}: {trace:#?}"
                        );
                    }
                }
                _ => {
                    // (emscripten's 60 Hz stand-in keeps to its schedule:
                    // after a late frame the next one comes sooner)
                    let total = iterations[iterations.len() - 1] - iterations[0];
                    assert!(total >= ms(90), "{gaps:?}");
                }
            }
            assert!(gaps.iter().all(|&g| g < ms(2000)), "{gaps:?}");
        }
    }

    impl AppCallbacks for App {
        fn iterate(&mut self) -> AppResult {
            self.iterations += 1;
            record(&self.trace, format!("iterate {}", self.iterations));
            if self.iterations == 3 {
                // A timer whose callback asked for no more is still found
                // on Emscripten (as upstream's there), so cancel() is true.
                let a10 = self.finished.take().unwrap();
                assert!(!a10.is_active());
                assert!(a10.cancel());
            }
            if self.scenario == "waitevent" && self.iterations == 2 {
                push_user_event();
            }
            if self.iterations == self.last_iteration() {
                if self.scenario == "failure" {
                    return AppResult::Failure;
                }
                return AppResult::Success;
            }
            AppResult::Continue
        }

        fn event(&mut self, event: &Event) -> AppResult {
            record(&self.trace, format!("event {:?}", event.event_type()));
            AppResult::Continue
        }

        fn quit(&mut self, result: AppResult) {
            println!("quit {result:?} after {} iterations", self.iterations);
            if self.scenario != "initfail" {
                self.check(result);
            }
            println!("{}: ok", self.scenario);
        }
    }

    /// The program's main function, for `run_app()`.
    fn sdl_main(args: &[String]) -> i32 {
        let scenario = scenario();
        if scenario.is_empty() {
            parent(args);
        }
        if scenario == "assert" {
            // A failed assertion, without SDL_HINT_ASSERT, under node as it
            // is (the URL's `window` gone).
            // SAFETY: plain JavaScript.
            unsafe { em_asm!("delete globalThis.window;") };
            println!("before");
            let x = 1;
            sdl3::sdl_assert_always!(x == 2);
            println!("after");
            return 0;
        }
        let name = if scenario.is_empty() {
            "success".to_string()
        } else {
            scenario
        };
        app::enter_app_main_callbacks(args, move |_| App::init(name))
    }

    pub(super) fn main() -> ExitCode {
        let scenario = scenario();
        if scenario != "nowindow" {
            // The page's URL, as a browser gives it.
            // SAFETY: plain JavaScript.
            unsafe {
                em_asm!(
                    r#"globalThis.window = { location: { search: '?SDL_EM_TEST_HINT=from%20the%20url&NOT_SDL=1&SDL_EM_TEST_EMPTY=' } };"#
                )
            };
        }
        // As C's `return SDL_RunApp(...)` from `main`: 0 goes back to the
        // event loop, which runs the main loop; 1 (an init failure) ends
        // the program.
        ExitCode::from(app::run_app(sdl_main) as u8)
    }

    /// A child program's exit status (-1 if it didn't end by itself) and
    /// output.
    fn run_child(scenario: &str) -> (i32, String) {
        let scenario = format!("{scenario}\0");
        // SAFETY: plain JavaScript; the string is NUL-terminated.
        let status = unsafe {
            em_asm_int!(
                r#"
                var r = require('child_process').spawnSync(process.execPath, [process.argv[1]], {
                    env: Object.assign({}, process.env, { SDL3_EM_SCENARIO: UTF8ToString($0) }),
                    encoding: 'utf8',
                    timeout: 60000,
                });
                Module['sdl3_child_output'] = String(r.stdout) + '\n' + String(r.stderr);
                return r.status === null ? -1 : r.status;
            "#,
                scenario.as_ptr()
            )
        };
        // SAFETY: as above; the buffer is as long as the JavaScript is told.
        let output = unsafe {
            let len = em_asm_int!("return lengthBytesUTF8(Module['sdl3_child_output']);");
            let mut buf = vec![0u8; len as usize + 1];
            em_asm!(
                "stringToUTF8(Module['sdl3_child_output'], $0, $1);",
                buf.as_mut_ptr(),
                buf.len() as i32
            );
            buf.truncate(len as usize);
            String::from_utf8(buf).unwrap()
        };
        (status, output)
    }

    /// The first program: the URL parameters, then each child scenario.
    fn parent(args: &[String]) {
        // SDL_RunApp() moved the URL's SDL_* parameters to the
        // environment, where hints see them.
        assert_eq!(args.len(), 1);
        assert_eq!(
            std::env::var("SDL_EM_TEST_HINT").as_deref(),
            Ok("from the url")
        );
        assert_eq!(
            hints::get("SDL_EM_TEST_HINT").as_deref(),
            Some("from the url")
        );
        assert_eq!(std::env::var("SDL_EM_TEST_EMPTY").as_deref(), Ok(""));
        assert!(std::env::var("NOT_SDL").is_err());
        println!("url parameters: ok");

        for (scenario, status, needles) in [
            (
                "failure",
                1,
                &["quit Failure after 8 iterations", "failure: ok"][..],
            ),
            (
                "initfail",
                1,
                &["init fails", "quit Failure after 0 iterations"][..],
            ),
            (
                "rate",
                0,
                &["quit Success after 8 iterations", "rate: ok"][..],
            ),
            (
                "waitevent",
                0,
                &["quit Success after 3 iterations", "waitevent: ok"][..],
            ),
            (
                "pending",
                0,
                &["quit Success after 8 iterations", "pending: ok"][..],
            ),
            // exit() while the main loop runs only stops that callback
            // (emscripten's runtime stays alive, as for C), and the main
            // loop's own exit(0) ends the program.
            (
                "exit-in-timer",
                0,
                &["exit(7) from a timer", "quit Success after 8 iterations"][..],
            ),
            // Upstream's JavaScript, under node: no `window`.
            (
                "nowindow",
                1,
                &["ReferenceError: window is not defined"][..],
            ),
            // The assertion's prompt: no `window` either (upstream's ends
            // one step earlier, at its message box's `alert()`, which isn't
            // there either).
            (
                "assert",
                1,
                &[
                    "before",
                    "Assertion failure at",
                    "ReferenceError: window is not defined",
                ][..],
            ),
        ] {
            let (got, output) = run_child(scenario);
            println!("child {scenario}: exit status {got}");
            assert_eq!(got, status, "{scenario}:\n{output}");
            for needle in needles {
                assert!(
                    output.contains(needle),
                    "{scenario}: no {needle:?} in\n{output}"
                );
            }
            assert!(!output.contains("after\n"), "{scenario}:\n{output}");
            if scenario == "pending" {
                assert!(!output.contains("forever"), "{output}");
            }
        }
    }
}
