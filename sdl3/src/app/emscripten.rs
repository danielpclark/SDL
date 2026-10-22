// Rust translation of src/main/emscripten/SDL_sysmain_callbacks.c and
// src/main/emscripten/SDL_sysmain_runapp.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The main callbacks and `SDL_RunApp()` on Emscripten.
//!
//! A browser page can't block, so the main callbacks don't loop here:
//! [`enter_app_main_callbacks`] sets up `emscripten_set_main_loop()` and
//! returns, and the browser's event loop then calls each iteration (at the
//! display's refresh rate through `requestAnimationFrame()`, or at the rate
//! `SDL_HINT_MAIN_CALLBACK_RATE` asks for through `setTimeout()`; under
//! node, which has neither a display nor `requestAnimationFrame()`,
//! emscripten's stand-in is a `setTimeout()` at 60 Hz). Once a callback
//! returns something other than [`AppResult::Continue`], SDL calls
//! [`AppCallbacks::quit`](super::AppCallbacks::quit), stops the loop and
//! calls `exit()` with 0 (success) or 1 (failure), which ends the program
//! (or, while timers or other asynchronous work keep emscripten's runtime
//! alive, once they are done).

use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use sdl3_emscripten::sys;

use super::{AppCallbacks, AppResult};
use crate::events::queue;
use crate::events::Event;
use crate::hints;

// For Emscripten, we let you use SDL_HINT_MAIN_CALLBACK_RATE, because it might be useful to drop it super-low for
//  things like loopwave that don't really do much but wait on the audio device, but be warned that browser timers
//  are super-unreliable in modern times, so you likely won't hit your desired callback rate with good precision.
// Almost all apps should leave this alone, so we can use requestAnimationFrame, which is intended to run reliably
//  at the refresh rate of the user's display.

/// Translation of `callback_rate_increment` (milliseconds).
static CALLBACK_RATE_INCREMENT: AtomicU32 = AtomicU32::new(0);
/// Translation of `iterate_after_waitevent`.
static ITERATE_AFTER_WAITEVENT: AtomicBool = AtomicBool::new(false);
/// Translation of `callback_rate_changed`.
static CALLBACK_RATE_CHANGED: AtomicBool = AtomicBool::new(false);

/// Translation of `MainCallbackRateHintChanged()` (Emscripten's).
fn main_callback_rate_hint_changed(new_value: Option<&str>) {
    CALLBACK_RATE_CHANGED.store(true, Ordering::Relaxed);
    let iterate_after_waitevent = new_value == Some("waitevent");
    ITERATE_AFTER_WAITEVENT.store(iterate_after_waitevent, Ordering::Relaxed);
    if iterate_after_waitevent {
        CALLBACK_RATE_INCREMENT.store(0, Ordering::Relaxed);
    } else {
        let callback_rate = new_value.map_or(0.0, |v| crate::stdlib::string::strtod(v).0);
        if callback_rate > 0.0 {
            // (Uint32) SDL_NS_TO_MS((double) SDL_NS_PER_SECOND / callback_rate)
            let ms = (crate::timer::NS_PER_SECOND as f64 / callback_rate) / 1_000_000.0;
            CALLBACK_RATE_INCREMENT.store(ms as u32, Ordering::Relaxed);
        } else {
            CALLBACK_RATE_INCREMENT.store(0, Ordering::Relaxed);
        }
    }
}

/// just tell us when any new event is pushed on the queue, so we can check a flag for "waitevent" mode.
/// Translation of `saw_new_event`.
static SAW_NEW_EVENT: AtomicBool = AtomicBool::new(false);

/// Translation of `EmscriptenMainCallbackEventWatcher()`.
fn emscripten_main_callback_event_watcher(_event: &Event) {
    SAW_NEW_EVENT.store(true, Ordering::Relaxed);
}

/// Translation of `EmscriptenInternalMainloop()`. `"C-unwind"`: its
/// `exit()` leaves it with a JavaScript exception (see `sys::em_callback_func`).
unsafe extern "C-unwind" fn emscripten_internal_mainloop() {
    let force_iterate = CALLBACK_RATE_CHANGED.load(Ordering::Relaxed)
        && ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed);

    // callback rate changed? Update emscripten's mainloop iteration speed.
    if CALLBACK_RATE_CHANGED.load(Ordering::Relaxed) {
        CALLBACK_RATE_CHANGED.store(false, Ordering::Relaxed);
        let increment = CALLBACK_RATE_INCREMENT.load(Ordering::Relaxed);
        // SAFETY: plain calls; there is a main loop (this is it).
        unsafe {
            if increment == 0 {
                sys::emscripten_set_main_loop_timing(sys::EM_TIMING_RAF, 1);
            } else {
                sys::emscripten_set_main_loop_timing(sys::EM_TIMING_SETTIMEOUT, increment as c_int);
            }
        }
    }

    let iterate_after_waitevent = ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed);
    if iterate_after_waitevent {
        queue::pump();
        if !SAW_NEW_EVENT.load(Ordering::Relaxed) && !force_iterate {
            // do nothing yet. Note that we're still going to iterate here because we can't block,
            // but we can stop the app's iteration from progressing until there's an event.
            return;
        }
        SAW_NEW_EVENT.store(false, Ordering::Relaxed);
    }

    let rc = super::iterate_main_callbacks(!iterate_after_waitevent);
    if rc != AppResult::Continue {
        super::quit_main_callbacks(rc);
        // SAFETY: a plain call.
        unsafe { sys::emscripten_cancel_main_loop() }; // kill" the mainloop, so it stops calling back into it.
        std::process::exit(if rc == AppResult::Failure { 1 } else { 0 }); // hopefully this takes down everything else, too.
    }
}

/// Translation of `SDL_EnterAppMainCallbacks()` (Emscripten).
pub(super) fn enter_app_main_callbacks<A: AppCallbacks>(
    args: &[String],
    appinit: impl FnOnce(&[String]) -> (A, AppResult),
) -> i32 {
    let rc = super::init_main_callbacks(args, appinit);
    if rc == AppResult::Continue {
        // (SDL_AddEventWatch() can't fail. Upstream never removes the
        // watcher, nor the hint callback.)
        std::mem::forget(queue::add_watch(emscripten_main_callback_event_watcher));
        if let Ok(watch) = hints::watch(hints::MAIN_CALLBACK_RATE, |change| {
            main_callback_rate_hint_changed(change.new_value)
        }) {
            watch.detach();
        }
        if !ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed) {
            // if we're doing waitevent, we need callback_rate_changed to stay true for the next iteration.
            CALLBACK_RATE_CHANGED.store(false, Ordering::Relaxed);
        }
        // SAFETY: the callback is an extern "C-unwind" fn taking nothing;
        // emscripten_set_main_loop_timing() follows the loop's creation.
        unsafe {
            sys::emscripten_set_main_loop(Some(emscripten_internal_mainloop), 0, false); // don't throw an exception since we do an orderly return.
            let increment = CALLBACK_RATE_INCREMENT.load(Ordering::Relaxed);
            if increment > 0 {
                sys::emscripten_set_main_loop_timing(sys::EM_TIMING_SETTIMEOUT, increment as c_int);
            }
        }
    } else {
        super::quit_main_callbacks(rc);
    }
    if rc == AppResult::Failure {
        1
    } else {
        0
    }
}

/// `SDL_setenv_unsafe()` as the URL parameters' JavaScript calls it
/// (`dynCall('iiii', …)`): 0, or -1 on failure.
extern "C" fn setenv_unsafe(name: *const c_char, value: *const c_char, overwrite: c_int) -> c_int {
    // SAFETY: the JavaScript passes two NUL-terminated strings (non-null:
    // it checks).
    let (name, value) = unsafe { (CStr::from_ptr(name), CStr::from_ptr(value)) };
    match (name.to_str(), value.to_str()) {
        (Ok(name), Ok(value)) => match crate::stdlib::setenv_unsafe(name, value, overwrite != 0) {
            Ok(()) => 0,
            Err(_) => -1,
        },
        _ => -1, // (stringToNewUTF8() makes UTF-8)
    }
}

/// Translation of `SDL_RunApp()` (Emscripten)'s start: the page's `SDL_*`
/// URL parameters become environment variables (so hints can be set from
/// the URL).
fn run_app_url_params() {
    // (the JavaScript below frees its strings with _Emscripten_force_free)
    crate::core::emscripten::register_callbacks();

    // Move any URL params that start with "SDL_" over to environment
    //  variables, so the hint system can pick them up, etc, much like a user
    //  can set them from a shell prompt on a desktop machine. Ignore all
    //  other params, in case the app wants to use them for something.
    // SAFETY: the function pointer has the parameters the JavaScript passes.
    unsafe { sdl3_emscripten::runapp::sdl3_runapp_url_params(setenv_unsafe as *const c_void) };
}

/// Translation of `SDL_RunApp()` (Emscripten): the URL parameters, then the
/// main function.
#[cfg(not(feature = "emscripten-persistent-path"))]
pub(super) fn run_app(main_function: impl FnOnce(&[String]) -> i32) -> i32 {
    run_app_url_params();
    call_sdl_emscripten_main_function(main_function)
}

/// Translation of `SDL_RunApp()` (Emscripten, with
/// `SDL_EMSCRIPTEN_PERSISTENT_PATH`): the URL parameters, then the
/// persistent storage is mounted and loaded, and the main function runs
/// once it is, from the event loop; this never returns.
#[cfg(feature = "emscripten-persistent-path")]
pub(super) fn run_app(main_function: impl FnOnce(&[String]) -> i32 + 'static) -> i32 {
    run_app_url_params();
    persistent_path::run(main_function)
}

/// Translation of `CallSDLEmscriptenMainFunction()`: `SDL_CallMainFunction()`.
fn call_sdl_emscripten_main_function(main_function: impl FnOnce(&[String]) -> i32) -> i32 {
    super::call_main_function(main_function)
}

/// Upstream's `SDL_EMSCRIPTEN_PERSISTENT_PATH_STRING` part of `SDL_RunApp()`.
#[cfg(feature = "emscripten-persistent-path")]
mod persistent_path {
    use super::*;
    use sdl3_emscripten::runapp::{sdl3_runapp_persistent_path, CALL_SDL_EMSCRIPTEN_MAIN_FUNCTION};

    /// `SDL_EMSCRIPTEN_PERSISTENT_PATH`, the directory the storage is mounted
    /// at: set when this crate is built, as CMake's option is.
    const PERSISTENT_PATH: &CStr = {
        const PATH: &str = concat!(
            env!(
                "SDL_EMSCRIPTEN_PERSISTENT_PATH",
                "the emscripten-persistent-path feature needs the path: build with SDL_EMSCRIPTEN_PERSISTENT_PATH=/the/path in the environment"
            ),
            "\0"
        );
        match CStr::from_bytes_with_nul(PATH.as_bytes()) {
            Ok(path) => path,
            Err(_) => panic!("SDL_EMSCRIPTEN_PERSISTENT_PATH can't contain a NUL"),
        }
    };

    type MainFunction = Box<dyn FnOnce(&[String]) -> i32>;

    /// `_CallSDLEmscriptenMainFunction`, which the JavaScript calls when the
    /// storage is loaded; `main_function` is the boxed main function. (The
    /// arguments are the program's again, so `argc`/`argv` aren't used.)
    /// `"C-unwind"`: the main function may `exit()`.
    extern "C-unwind" fn call_from_javascript(
        _argc: c_int,
        _argv: *mut *mut c_char,
        main_function: *mut c_void,
    ) -> c_int {
        // SAFETY: run() made the pointer with Box::into_raw(), and the
        // JavaScript calls this once.
        let main_function = unsafe { Box::from_raw(main_function.cast::<MainFunction>()) };
        call_sdl_emscripten_main_function(*main_function)
    }

    pub(super) fn run(main_function: impl FnOnce(&[String]) -> i32 + 'static) -> ! {
        // SAFETY: an extern "C-unwind" fn with the parameters the
        // JavaScript passes, for the life of the program.
        unsafe { CALL_SDL_EMSCRIPTEN_MAIN_FUNCTION.register(call_from_javascript as *const ()) };
        let main_function: MainFunction = Box::new(main_function);
        let main_function = Box::into_raw(Box::new(main_function));
        // SAFETY: the path is NUL-terminated; the JavaScript hands the
        // pointer back to call_from_javascript().
        unsafe {
            sdl3_runapp_persistent_path(
                PERSISTENT_PATH.as_ptr(),
                0,
                std::ptr::null_mut(),
                main_function.cast(),
            );
            // we need to stop running code until FS.syncfs() finishes, but we need the runtime to not clean up.
            // The actual SDL_main/SDL_AppInit() will be called when the sync is done and things will pick back up where they were.
            sys::emscripten_exit_with_live_runtime()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rate hint in whole milliseconds per iteration (truncated, as
    /// upstream's `(Uint32)` cast does: past 1000 Hz it's 0, the display's
    /// rate), and "waitevent".
    #[test]
    fn rate_hint() {
        for (value, ms) in [
            (Some("60"), 16),
            (Some("20"), 50),
            (Some("1000"), 1),
            (Some("3000"), 0),
            (Some("0"), 0),
            (Some("-5"), 0),
            (Some("junk"), 0),
            (None, 0),
        ] {
            main_callback_rate_hint_changed(value);
            assert_eq!(
                CALLBACK_RATE_INCREMENT.load(Ordering::Relaxed),
                ms,
                "{value:?}"
            );
            assert!(!ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed));
            assert!(CALLBACK_RATE_CHANGED.swap(false, Ordering::Relaxed));
        }
        main_callback_rate_hint_changed(Some("waitevent"));
        assert!(ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed));
        assert_eq!(CALLBACK_RATE_INCREMENT.load(Ordering::Relaxed), 0);
        main_callback_rate_hint_changed(None);
        assert!(!ITERATE_AFTER_WAITEVENT.load(Ordering::Relaxed));
        CALLBACK_RATE_CHANGED.store(false, Ordering::Relaxed);
    }
}
