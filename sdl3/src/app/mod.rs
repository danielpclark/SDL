// Rust translation of src/main/SDL_main_callbacks.c, SDL_main_callbacks.h,
// SDL_runapp.c, src/main/generic/SDL_sysmain_callbacks.c and
// include/SDL3/SDL_main.h from Simple DirectMedia Layer (the Emscripten
// ones are in emscripten.rs).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Application entry points: [`run_app`] (`SDL_RunApp()`) and the main
//! callbacks ([`enter_app_main_callbacks`]), where SDL runs the main loop
//! and calls the application's [`AppCallbacks`] once per iteration and for
//! every event.
//!
//! This is upstream's `src/main/` (the module is named `app` because
//! `main` is reserved for binaries) in its generic implementation, used on
//! desktop platforms, and in Emscripten's, where the browser runs the main
//! loop: there [`enter_app_main_callbacks`] returns at once and the program
//! ends after [`AppCallbacks::quit`], and [`run_app`] reads hints from the
//! page's URL first. The other platforms that need their own main loop
//! (iOS, consoles) arrive with their platform layers.

use std::cell::RefCell;
#[cfg(not(target_os = "emscripten"))]
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(not(target_os = "emscripten"))]
use std::time::Duration;

use crate::events::queue::{self, EventWatch};
use crate::events::{Event, EventType};
#[cfg(not(target_os = "emscripten"))]
use crate::hints;
use crate::init::{self, InitFlags};
use crate::thread::ReentrantMutex;
#[cfg(not(target_os = "emscripten"))]
use crate::timer;

#[cfg(target_os = "emscripten")]
mod emscripten;
#[cfg(all(target_os = "emscripten", feature = "emscripten-persistent-path"))]
pub(crate) use emscripten::PERSISTENT_PATH_STRING;

/// Return values from the main callbacks. Translation of `SDL_AppResult`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AppResult {
    /// Value that requests that the app continue from the main callbacks.
    Continue,
    /// Value that requests termination with success from the main callbacks.
    Success,
    /// Value that requests termination with error from the main callbacks.
    Failure,
}

impl AppResult {
    fn from_i32(v: i32) -> AppResult {
        match v {
            0 => AppResult::Continue,
            1 => AppResult::Success,
            _ => AppResult::Failure,
        }
    }
}

/// The application's side of the main callbacks: the state that
/// `SDL_AppInit()` creates, with `SDL_AppIterate()`, `SDL_AppEvent()` and
/// `SDL_AppQuit()` as methods.
///
/// `event` may be called from any thread for the events that must be
/// handled immediately (`TERMINATING`, `LOW_MEMORY` and the
/// background/foreground transitions); the others arrive on the main
/// thread between iterations.
pub trait AppCallbacks: Send + 'static {
    /// Called once per frame. Translation of `SDL_AppIterate_func`.
    fn iterate(&mut self) -> AppResult;

    /// Called for each event. Translation of `SDL_AppEvent_func`.
    fn event(&mut self, event: &Event) -> AppResult;

    /// Called once before the app exits, with the result that ended it.
    /// Translation of `SDL_AppQuit_func`.
    fn quit(&mut self, result: AppResult) {
        let _ = result;
    }
}

type AppState = Arc<ReentrantMutex<RefCell<Box<dyn AppCallbacks>>>>;

/// Translation of `SDL_main_iteration_callback`/`SDL_main_event_callback`/
/// `SDL_main_quit_callback`/`SDL_main_appstate`.
static MAIN_APPSTATE: Mutex<Option<AppState>> = Mutex::new(None);
/// use an atomic, since events might land from any thread and we don't want
/// to wrap this all in a mutex. A CAS makes sure we only move from zero
/// once. Translation of `apprc`.
static APPRC: AtomicI32 = AtomicI32::new(0);
static EVENT_WATCH: Mutex<Option<EventWatch>> = Mutex::new(None);
/// Events that arrived while the app was in a callback on the same thread
/// (dispatched when it returns).
static DEFERRED: Mutex<Vec<Event>> = Mutex::new(Vec::new());

fn appstate() -> Option<AppState> {
    MAIN_APPSTATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// `SDL_CompareAndSwapAtomicInt(&apprc, SDL_APP_CONTINUE, rc)`.
fn cas_continue(rc: AppResult) -> bool {
    APPRC
        .compare_exchange(
            AppResult::Continue as i32,
            rc as i32,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

fn apprc() -> AppResult {
    AppResult::from_i32(APPRC.load(Ordering::Acquire))
}

/// Return true if this event needs to be processed before returning from
/// the event watcher. Translation of `ShouldDispatchImmediately()`.
fn should_dispatch_immediately(event: &Event) -> bool {
    matches!(
        event.event_type(),
        EventType::TERMINATING
            | EventType::LOW_MEMORY
            | EventType::WILL_ENTER_BACKGROUND
            | EventType::DID_ENTER_BACKGROUND
            | EventType::WILL_ENTER_FOREGROUND
            | EventType::DID_ENTER_FOREGROUND
    )
}

/// Run `f` on the app state; `None` if the app is in a callback on this
/// thread already.
fn with_app<R>(f: impl FnOnce(&mut dyn AppCallbacks) -> R) -> Option<R> {
    let state = appstate()?;
    let guard = state.lock();
    let mut app = guard.try_borrow_mut().ok()?;
    Some(f(app.as_mut()))
}

/// Translation of `SDL_DispatchMainCallbackEvent()`.
fn dispatch_main_callback_event(event: &Event) {
    if apprc() == AppResult::Continue {
        // if already quitting, don't send the event to the app.
        match with_app(|app| app.event(event)) {
            Some(rc) => {
                cas_continue(rc);
            }
            None => DEFERRED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(event.clone()),
        }
    }
}

fn dispatch_deferred() {
    let deferred = std::mem::take(&mut *DEFERRED.lock().unwrap_or_else(|e| e.into_inner()));
    for event in &deferred {
        dispatch_main_callback_event(event);
    }
}

/// Translation of `SDL_DispatchMainCallbackEvents()`.
fn dispatch_main_callback_events() {
    loop {
        let events = queue::get_events(EventType::FIRST, EventType::LAST, 16).unwrap_or_default();
        if events.is_empty() {
            break;
        }
        for event in &events {
            if !should_dispatch_immediately(event) {
                dispatch_main_callback_event(event);
            }
        }
    }
}

/// Translation of `SDL_MainCallbackEventWatcher()`.
fn main_callback_event_watcher(event: &Event) {
    if should_dispatch_immediately(event) {
        // Make sure any currently queued events are processed then dispatch this before continuing
        dispatch_main_callback_events();
        dispatch_main_callback_event(event);

        // Make sure that we quit if we get a terminating event
        if event.event_type() == EventType::TERMINATING {
            cas_continue(AppResult::Success);
        }
    } else {
        // We'll process this event later from the main event queue
    }
}

/// Whether the main callbacks are running. Translation of `SDL_HasMainCallbacks()`.
pub fn has_main_callbacks() -> bool {
    appstate().is_some()
}

/// Translation of `SDL_InitMainCallbacks()`.
fn init_main_callbacks<A: AppCallbacks>(
    args: &[String],
    appinit: impl FnOnce(&[String]) -> (A, AppResult),
) -> AppResult {
    APPRC.store(AppResult::Continue as i32, Ordering::Release);

    let (app, rc) = appinit(args);
    let state: AppState = Arc::new(ReentrantMutex::new(RefCell::new(Box::new(app))));
    *MAIN_APPSTATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);

    if cas_continue(rc) && rc == AppResult::Continue {
        // bounce if SDL_AppInit already said abort, otherwise...
        // make sure we definitely have events initialized, even if the app didn't do it.
        if init::init_subsystem(InitFlags::EVENTS).is_err() {
            APPRC.store(AppResult::Failure as i32, Ordering::Release);
            return AppResult::Failure;
        }
        let watch = queue::add_watch(main_callback_event_watcher);
        *EVENT_WATCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(watch);
    }

    apprc()
}

/// Translation of `SDL_IterateMainCallbacks()`.
pub(crate) fn iterate_main_callbacks(pump_events: bool) -> AppResult {
    if pump_events {
        queue::pump();
    }
    dispatch_main_callback_events();

    let mut rc = apprc();
    if rc == AppResult::Continue {
        rc = with_app(|app| app.iterate()).unwrap_or(AppResult::Continue);
        dispatch_deferred();
        if !cas_continue(rc) {
            rc = apprc(); // something else already set a quit result, keep that.
        }
    }
    rc
}

/// Translation of `SDL_QuitMainCallbacks()`.
fn quit_main_callbacks(result: AppResult) {
    let watch = EVENT_WATCH.lock().unwrap_or_else(|e| e.into_inner()).take();
    drop(watch);
    let _ = with_app(|app| app.quit(result));
    // just in case.
    let state = MAIN_APPSTATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    drop(state);
    DEFERRED.lock().unwrap_or_else(|e| e.into_inner()).clear();

    // for symmetry, you should explicitly Quit what you Init, but we might come through here uninitialized and SDL_Quit() will clear everything anyhow.
    //SDL_QuitSubSystem(SDL_INIT_EVENTS);

    init::quit();
}

#[cfg(not(target_os = "emscripten"))]
/// Translation of `callback_rate_increment` (ns).
static CALLBACK_RATE_INCREMENT: AtomicU64 = AtomicU64::new(0);
#[cfg(not(target_os = "emscripten"))]
/// Translation of `iterate_after_waitevent`.
static ITERATE_AFTER_WAITEVENT: AtomicBool = AtomicBool::new(false);
#[cfg(not(target_os = "emscripten"))]
/// Translation of `callback_rate_changed`.
static CALLBACK_RATE_CHANGED: AtomicBool = AtomicBool::new(false);

#[cfg(not(target_os = "emscripten"))]
/// Translation of `MainCallbackRateHintChanged()`.
fn main_callback_rate_hint_changed(new_value: Option<&str>) {
    CALLBACK_RATE_CHANGED.store(true, Ordering::Relaxed);
    let iterate_after_waitevent = new_value == Some("waitevent");
    ITERATE_AFTER_WAITEVENT.store(iterate_after_waitevent, Ordering::Relaxed);
    if iterate_after_waitevent {
        CALLBACK_RATE_INCREMENT.store(0, Ordering::Relaxed);
    } else {
        let callback_rate = new_value.map_or(0.0, |v| crate::stdlib::string::strtod(v).0);
        if callback_rate > 0.0 {
            CALLBACK_RATE_INCREMENT.store(
                (crate::timer::NS_PER_SECOND as f64 / callback_rate) as u64,
                Ordering::Relaxed,
            );
        } else {
            CALLBACK_RATE_INCREMENT.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(not(target_os = "emscripten"))]
/// Translation of `GenericIterateMainCallbacks()`.
fn generic_iterate_main_callbacks() -> AppResult {
    let mut should_wait = ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed);

    if CALLBACK_RATE_CHANGED.swap(false, Ordering::Relaxed)
        && ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed)
    {
        // just go immediately for one iteration (it will do a PumpEvents),
        //  and do a full blocking wait for more events next time.
        should_wait = false;
    }

    if should_wait {
        // (SDL_WaitEvent(NULL): wait without taking the event)
        if let Ok(event) = queue::wait() {
            let _ = queue::add_events(std::iter::once(event));
        }
    }

    iterate_main_callbacks(!should_wait)
}

/// Run the main callbacks: `appinit` creates the app state, which SDL then
/// iterates (and sends events to) until a callback returns something
/// other than [`AppResult::Continue`]; then SDL calls
/// [`AppCallbacks::quit`] and shuts down. Returns the process exit code
/// (1 for [`AppResult::Failure`], else 0). `SDL_HINT_MAIN_CALLBACK_RATE`
/// limits the iteration rate. Translation of `SDL_EnterAppMainCallbacks()`.
///
/// On Emscripten the browser runs the loop: this returns as soon as it's
/// set up (0, or 1 if `appinit` failed, after [`AppCallbacks::quit`]), the
/// program's `main` must then return (or `exit()`) to the browser's event
/// loop, which calls the iterations, and when they end SDL quits and calls
/// `exit()` with 0 or 1 (1 for [`AppResult::Failure`]). The rate hint is in
/// whole milliseconds per iteration there; without it, or with `"waitevent"`,
/// the iterations follow `requestAnimationFrame()`. Translation of
/// `SDL_EnterAppMainCallbacks()` in `SDL_sysmain_callbacks.c` (Emscripten).
pub fn enter_app_main_callbacks<A: AppCallbacks>(
    args: &[String],
    appinit: impl FnOnce(&[String]) -> (A, AppResult),
) -> i32 {
    #[cfg(target_os = "emscripten")]
    {
        emscripten::enter_app_main_callbacks(args, appinit)
    }
    #[cfg(not(target_os = "emscripten"))]
    {
        generic_enter_app_main_callbacks(args, appinit)
    }
}

/// Translation of the generic `SDL_EnterAppMainCallbacks()`.
#[cfg(not(target_os = "emscripten"))]
fn generic_enter_app_main_callbacks<A: AppCallbacks>(
    args: &[String],
    appinit: impl FnOnce(&[String]) -> (A, AppResult),
) -> i32 {
    let mut rc = init_main_callbacks(args, appinit);
    if rc == AppResult::Continue {
        let watch = hints::watch(hints::MAIN_CALLBACK_RATE, |change| {
            main_callback_rate_hint_changed(change.new_value)
        });

        let increment = CALLBACK_RATE_INCREMENT.load(Ordering::Relaxed);
        let mut next_iteration = if increment != 0 {
            timer::ticks_ns() + increment
        } else {
            0
        };

        loop {
            rc = generic_iterate_main_callbacks();
            if rc != AppResult::Continue {
                break;
            }
            // !!! FIXME: this can be made more complicated if we decide to
            // !!! FIXME: optionally hand off callback responsibility to the
            // !!! FIXME: video subsystem (for example, if Wayland has a
            // !!! FIXME: protocol to drive an animation loop, maybe we hand
            // !!! FIXME: off to them here if/when the video subsystem becomes
            // !!! FIXME: initialized).

            // Try to run at whatever rate the hint requested. This makes this
            //  not eat all the CPU in simple things like loopwave. By
            //  default, we run as fast as possible, which means we'll clamp to
            //  vsync in common cases, and won't be restrained to vsync if the
            //  app is doing a benchmark or doesn't want to be, based on how
            // they've set up that window.
            let increment = CALLBACK_RATE_INCREMENT.load(Ordering::Relaxed);
            if increment == 0 {
                next_iteration = 0; // just clear the timer and run at the pace the video subsystem allows.
            } else {
                let now = timer::ticks_ns();
                if next_iteration > now {
                    // Running faster than the limit, sleep a little.
                    timer::delay_precise(Duration::from_nanos(next_iteration - now));
                } else {
                    next_iteration = now; // if running behind, reset the timer. If right on time, `next_iteration` already equals `now`.
                }
                next_iteration += increment;
            }
        }

        drop(watch);
    }
    quit_main_callbacks(rc);

    if rc == AppResult::Failure {
        1
    } else {
        0
    }
}

/// The program's arguments, or `["SDL_app"]` without any.
/// Translation of `SDL_CheckDefaultArgcArgv()`.
fn check_default_args(args: Vec<String>) -> Vec<String> {
    if args.is_empty() {
        return vec!["SDL_app".to_string()];
    }
    args
}

/// Translation of `SDL_CallMainFunction()`.
fn call_main_function(main_function: impl FnOnce(&[String]) -> i32) -> i32 {
    let args = check_default_args(std::env::args().collect());
    init::set_main_ready();
    main_function(&args)
}

/// Initialize SDL's main thread state and call the application's main
/// function with the program's arguments. Translation of `SDL_RunApp()`
/// (via `SDL_CallMainFunction()`).
///
/// On Emscripten, upstream's `SDL_RunApp()` first moves the page's URL
/// parameters whose names start with `SDL_` into the environment (so
/// `?SDL_RENDER_DRIVER=software` sets a hint), with JavaScript that reads
/// `window.location`: under node, which has no `window`, that throws (a
/// `ReferenceError` that ends the program), as upstream's does. With the
/// `emscripten-persistent-path` feature (see `sdl3/Cargo.toml`), the main
/// function needs a `'static` bound: it runs later, from the event loop.
#[cfg(not(all(target_os = "emscripten", feature = "emscripten-persistent-path")))]
pub fn run_app(main_function: impl FnOnce(&[String]) -> i32) -> i32 {
    #[cfg(target_os = "emscripten")]
    {
        emscripten::run_app(main_function)
    }
    #[cfg(not(target_os = "emscripten"))]
    {
        call_main_function(main_function)
    }
}

/// Initialize SDL's main thread state and call the application's main
/// function with the program's arguments. Translation of `SDL_RunApp()`
/// on Emscripten with `SDL_EMSCRIPTEN_PERSISTENT_PATH` (the
/// `emscripten-persistent-path` feature): the page's `SDL_*` URL parameters
/// become environment variables, then IndexedDB storage is mounted at the
/// path the `SDL_EMSCRIPTEN_PERSISTENT_PATH` environment variable named when
/// this crate was built, and once it's loaded (asynchronously, in a browser)
/// the main function runs, from the event loop. This never returns: it
/// leaves `main` to the event loop with the runtime kept alive
/// (`emscripten_exit_with_live_runtime()`), and the main function's result
/// is not the program's exit status, as upstream's isn't.
#[cfg(all(target_os = "emscripten", feature = "emscripten-persistent-path"))]
pub fn run_app(main_function: impl FnOnce(&[String]) -> i32 + 'static) -> i32 {
    emscripten::run_app(main_function)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "emscripten")]
    use std::time::Duration;

    struct Counter {
        iterations: u32,
        events: Arc<Mutex<Vec<EventType>>>,
        quit_with: Arc<Mutex<Option<AppResult>>>,
    }

    impl AppCallbacks for Counter {
        fn iterate(&mut self) -> AppResult {
            self.iterations += 1;
            match self.iterations {
                1 => {
                    let _ = queue::push(Event::simple(EventType::USER, Duration::ZERO));
                    AppResult::Continue
                }
                2 => {
                    // dispatched right away (deferred until this callback returns)
                    let _ = queue::push(Event::simple(EventType::LOW_MEMORY, Duration::ZERO));
                    AppResult::Continue
                }
                3 => AppResult::Continue,
                _ => AppResult::Success,
            }
        }

        fn event(&mut self, event: &Event) -> AppResult {
            self.events.lock().unwrap().push(event.event_type());
            AppResult::Continue
        }

        fn quit(&mut self, result: AppResult) {
            *self.quit_with.lock().unwrap() = Some(result);
        }
    }

    #[test]
    #[cfg_attr(
        target_os = "emscripten",
        ignore = "the main loop runs from the event loop (tests/emscripten_main.rs)"
    )]
    fn main_callbacks_loop() {
        let _l = crate::test_support::test_lock();
        init::set_main_ready();
        let events = Arc::new(Mutex::new(Vec::new()));
        let quit_with = Arc::new(Mutex::new(None));
        let (e, q) = (events.clone(), quit_with.clone());
        let code = enter_app_main_callbacks(&["test".to_string()], move |args| {
            assert_eq!(args, ["test"]);
            (
                Counter {
                    iterations: 0,
                    events: e,
                    quit_with: q,
                },
                AppResult::Continue,
            )
        });
        assert_eq!(code, 0);
        assert_eq!(*quit_with.lock().unwrap(), Some(AppResult::Success));
        let events = events.lock().unwrap();
        assert!(events.contains(&EventType::USER));
        assert_eq!(
            events
                .iter()
                .filter(|&&t| t == EventType::LOW_MEMORY)
                .count(),
            1
        );
        assert!(!has_main_callbacks());
    }

    #[test]
    fn init_failure_goes_straight_to_quit() {
        let _l = crate::test_support::test_lock();
        init::set_main_ready();
        let quit_with = Arc::new(Mutex::new(None));
        let q = quit_with.clone();
        let code = enter_app_main_callbacks(&[], move |_| {
            (
                Counter {
                    iterations: 0,
                    events: Arc::new(Mutex::new(Vec::new())),
                    quit_with: q,
                },
                AppResult::Failure,
            )
        });
        assert_eq!(code, 1);
        assert_eq!(*quit_with.lock().unwrap(), Some(AppResult::Failure));
    }

    #[test]
    #[cfg(not(target_os = "emscripten"))]
    fn rate_hint() {
        main_callback_rate_hint_changed(Some("60"));
        assert_eq!(CALLBACK_RATE_INCREMENT.load(Ordering::Relaxed), 16_666_666);
        main_callback_rate_hint_changed(Some("waitevent"));
        assert!(ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed));
        main_callback_rate_hint_changed(None);
        assert_eq!(CALLBACK_RATE_INCREMENT.load(Ordering::Relaxed), 0);
        assert!(!ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed));
        CALLBACK_RATE_CHANGED.store(false, Ordering::Relaxed);
        assert_eq!(check_default_args(Vec::new()), ["SDL_app"]);
    }
}
