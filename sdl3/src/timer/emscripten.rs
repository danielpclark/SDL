// Rust translation of the Emscripten-specific implementation in
// src/timer/SDL_timer.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The timers on Emscripten: `SDL_timer.c`'s `#else // Emscripten-specific
//! implementation.`, which upstream uses on Emscripten instead of the timer
//! thread (with or without pthreads).
//!
//! Each timer is an `emscripten_set_timeout()`: the browser's event loop
//! (node's, under node) calls the callback once the interval has passed
//! *and* control is back in the event loop, between main loop iterations
//! or after `main` returns. While the program runs code (a test, a busy
//! wait, a `delay()`) no timer fires. A periodic timer schedules a new
//! timeout each time its callback asks for one. A pending timeout keeps
//! the runtime alive, so a program whose `main` returns ends when its last
//! timer does.

use std::collections::HashMap;
use std::ffi::{c_int, c_void};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use sdl3_emscripten::sys;

use super::{TimerCallback, NS_PER_MS};
use crate::error::Result;

/// Translation of `SDL_TimerMap`: one timer, keyed by its `timerID` in
/// [`TIMERMAP`].
struct TimerMap {
    /// The pending `emscripten_set_timeout()`'s id (the last one, once
    /// the timer finished).
    timeout_id: c_int,
    /// The interval the callback runs with next, in ns; 0 once it asked
    /// for no more.
    interval: u64,
    /// `callback_ns`/`userdata`. In an `Arc` so the map isn't locked while
    /// it runs (it may add and remove timers).
    callback: Arc<Mutex<TimerCallback>>,
}

/// Translation of `SDL_timer_data` (upstream's linked list is a map).
static TIMERMAP: LazyLock<Mutex<HashMap<u32, TimerMap>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn timermap() -> MutexGuard<'static, HashMap<u32, TimerMap>> {
    TIMERMAP.lock().unwrap_or_else(|e| e.into_inner())
}

/// `SDL_NS_TO_MS()`, as the `double` `emscripten_set_timeout()` takes.
fn ns_to_ms(ns: u64) -> f64 {
    (ns / NS_PER_MS) as f64
}

/// `emscripten_set_timeout(&SDL_Emscripten_TimerHelper, SDL_NS_TO_MS(interval), entry)`.
fn set_timeout(id: u32, interval: u64) -> c_int {
    // The timeout's userdata is the timer's id, not the entry (see
    // emscripten_timer_helper()).
    // SAFETY: the helper is an extern "C-unwind" fn taking the userdata,
    // which is only an integer here.
    unsafe {
        sys::emscripten_set_timeout(
            emscripten_timer_helper,
            ns_to_ms(interval),
            id as usize as *mut c_void,
        )
    }
}

/// Translation of `SDL_Emscripten_TimerHelper()`.
///
/// Note (upstream): upstream's userdata is the `SDL_TimerMap` itself, which
/// `SDL_RemoveTimer()` (from the callback) and `SDL_QuitTimers()` free
/// without clearing a pending timeout, and the helper then uses the freed
/// entry: after `SDL_Quit()` the timers keep firing (and a periodic one
/// keeps the program from ever ending). Here the userdata is the timer's id:
/// a timer that is gone is not called again, and its timeout ends.
unsafe extern "C-unwind" fn emscripten_timer_helper(userdata: *mut c_void) {
    let id = userdata as usize as u32;
    let (callback, interval) = match timermap().get(&id) {
        Some(entry) => (entry.callback.clone(), entry.interval),
        None => return,
    };
    // (the map isn't locked while the app's callback runs)
    let interval = {
        let mut callback = callback.lock().unwrap_or_else(|e| e.into_inner());
        callback(Duration::from_nanos(interval))
            .map_or(0, |d| d.as_nanos().min(u64::MAX as u128) as u64)
    };
    let mut map = timermap();
    let Some(entry) = map.get_mut(&id) else {
        return; // (removed by its own callback)
    };
    entry.interval = interval;
    if entry.interval > 0 {
        entry.timeout_id = set_timeout(id, entry.interval);
    }
}

/// Translation of `SDL_QuitTimers()` (Emscripten): the timers are dropped;
/// their pending timeouts are left to fire (and do nothing), as upstream's
/// are.
pub(crate) fn quit_timers() {
    timermap().clear();
}

/// Translation of `SDL_CreateTimer()` (Emscripten). (Upstream's
/// `CHECK_PARAM(!callback)` can't fail: the callback is a closure.)
pub(super) fn create_timer(interval: u64, callback: TimerCallback) -> Result<u32> {
    let id = crate::utils::next_object_id();
    let timeout_id = set_timeout(id, interval);
    timermap().insert(
        id,
        TimerMap {
            timeout_id,
            interval,
            callback: Arc::new(Mutex::new(callback)),
        },
    );
    Ok(id)
}

/// Translation of `SDL_RemoveTimer()` (Emscripten).
///
/// As upstream's, it finds a timer whose callback asked for no more (the
/// entry stays in the map until it's removed or SDL quits), and succeeds:
/// clearing its timeout, which already fired, does nothing.
pub(super) fn remove_timer(id: u32) -> bool {
    // Find the timer
    let entry = timermap().remove(&id);
    match entry {
        Some(entry) => {
            // SAFETY: a plain call (a timeout that already fired or was
            // cleared is ignored).
            unsafe { sys::emscripten_clear_timeout(entry.timeout_id) };
            true
        }
        None => false, // "Timer not found"
    }
}

/// Whether the timer has a timeout pending: it's in the map and its
/// callback didn't ask for no more.
pub(super) fn timer_is_active(id: u32) -> bool {
    timermap().get(&id).is_some_and(|entry| entry.interval > 0)
}

#[cfg(test)]
mod tests {
    use super::super::{delay, Timer};
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// What a test can check without the event loop: a test runs from
    /// start to end without returning to it, so no timer fires meanwhile
    /// (tests/emscripten_main.rs checks them firing).
    #[test]
    fn timers_wait_for_the_event_loop() {
        let _l = crate::test_support::test_lock();
        let fired = Arc::new(AtomicUsize::new(0));
        let f = fired.clone();
        let timer = Timer::new(Duration::from_millis(1), move |interval| {
            f.fetch_add(1, Ordering::SeqCst);
            Some(interval)
        })
        .unwrap();
        assert!(timer.is_active());
        delay(Duration::from_millis(20)); // (a busy wait, without Asyncify)
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        assert!(timer.is_active());
        let entry_timeout = timermap()[&timer.id].timeout_id;
        assert!(entry_timeout > 0);

        // Dropping a handle removes the timer, and clears its timeout.
        let dropped = Timer::new(Duration::ZERO, |_| None).unwrap();
        let dropped_id = dropped.id;
        drop(dropped);
        assert!(!timer_is_active(dropped_id));
        assert!(!remove_timer(dropped_id)); // "Timer not found"

        assert!(timer.cancel());

        // SDL_Quit() drops the timers that are left; their timeouts stay
        // pending (cleared here, so that the program doesn't wait an hour
        // for this one after the tests).
        let detached = Timer::new(Duration::from_secs(3600), |_| None).unwrap();
        let id = detached.id;
        detached.detach();
        assert!(timer_is_active(id));
        let timeout = timermap()[&id].timeout_id;
        crate::init::quit();
        assert!(!timer_is_active(id));
        assert!(!remove_timer(id));
        // SAFETY: a plain call.
        unsafe { sys::emscripten_clear_timeout(timeout) };
        assert_eq!(fired.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn milliseconds() {
        assert_eq!(ns_to_ms(0), 0.0);
        assert_eq!(ns_to_ms(999_999), 0.0);
        assert_eq!(ns_to_ms(1_999_999), 1.0);
        assert_eq!(ns_to_ms(30 * NS_PER_MS), 30.0);
    }
}
