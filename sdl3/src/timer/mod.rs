// Rust translation of src/timer/SDL_timer.c and SDL_timer_c.h from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Time management: the SDL tick counter, delays and callback timers.
//!
//! Direct translation of `SDL_timer.c`. The platform pieces
//! (`SDL_GetPerformanceCounter`, `SDL_SYS_DelayNS`) are the Unix backend
//! (`clock_gettime()` on `CLOCK_MONOTONIC_RAW`, `nanosleep()`, and on
//! Emscripten `emscripten_sleep()` in a program built with Asyncify) and the
//! Windows one (`QueryPerformanceCounter()`, high resolution waitable
//! timers, `timeBeginPeriod()`); elsewhere they are written with
//! `std::time` and `std::thread::sleep`. The callback timers run on a
//! thread of their own, but on Emscripten, where upstream schedules them
//! on the browser's event loop instead (`emscripten_set_timeout()`): they
//! fire only once control is back there (see [`Timer::new`]).

#[cfg(not(target_os = "emscripten"))]
use std::collections::HashMap;
#[cfg(not(target_os = "emscripten"))]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
#[cfg(not(target_os = "emscripten"))]
use std::sync::{Arc, LazyLock};
use std::time::Duration;

#[cfg(not(target_os = "emscripten"))]
use crate::err;
use crate::error::Result;
#[cfg(not(target_os = "emscripten"))]
use crate::thread::{InitState, Semaphore};

#[cfg(target_os = "emscripten")]
mod emscripten;
#[cfg(target_os = "emscripten")]
pub(crate) use emscripten::quit_timers;
#[cfg(target_os = "emscripten")]
use emscripten::{create_timer, remove_timer, timer_is_active};
#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

/// Nanoseconds in a second, as an `i64` (SDL's `SDL_NS_PER_SECOND` is `long long`).
pub(crate) const NS_PER_SECOND: i64 = 1_000_000_000;
const MS_PER_SECOND: u64 = 1000;
const NS_PER_MS: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// Performance counter (platform layer)
// ---------------------------------------------------------------------------

/// The performance counter of platforms without a translated backend:
/// nanoseconds since the first call (plus one).
#[cfg(not(any(unix, windows)))]
mod sys {
    use std::sync::OnceLock;
    use std::time::Instant;

    static PERF_START: OnceLock<Instant> = OnceLock::new();

    pub(super) fn get_performance_counter() -> u64 {
        let start = *PERF_START.get_or_init(Instant::now);
        (Instant::now().duration_since(start).as_nanos() as u64).wrapping_add(1)
    }

    pub(super) fn get_performance_frequency() -> u64 {
        super::NS_PER_SECOND as u64
    }

    pub(super) fn sys_delay_ns(ns: u64) {
        if ns == 0 {
            std::thread::yield_now();
        } else {
            std::thread::sleep(std::time::Duration::from_nanos(ns));
        }
    }
}

/// The current value of the high resolution counter.
///
/// Counter values are only meaningful relative to each other; divide a
/// difference by [`performance_frequency`] to get seconds.
/// Translation of `SDL_GetPerformanceCounter()`.
pub fn performance_counter() -> u64 {
    sys::get_performance_counter()
}

/// Counts per second of the high resolution counter. Translation of `SDL_GetPerformanceFrequency()`.
pub fn performance_frequency() -> u64 {
    sys::get_performance_frequency()
}

/// Translation of `SDL_SYS_DelayNS()`.
fn sys_delay(d: Duration) {
    sys::sys_delay_ns(d.as_nanos().min(u64::MAX as u128) as u64);
}

// ---------------------------------------------------------------------------
// Ticks
// ---------------------------------------------------------------------------

static TICK_START: AtomicU64 = AtomicU64::new(0);
static TICK_NUMERATOR_NS: AtomicU32 = AtomicU32::new(1);
static TICK_DENOMINATOR_NS: AtomicU32 = AtomicU32::new(1);
static TICK_NUMERATOR_MS: AtomicU32 = AtomicU32::new(1);
static TICK_DENOMINATOR_MS: AtomicU32 = AtomicU32::new(1);
static TICKS_INIT_LOCK: Mutex<Option<crate::hints::Callback>> = Mutex::new(None);

/// Translation of `SDL_SetSystemTimerResolutionMS()`. Only Windows has a
/// system timer resolution to request (`timeBeginPeriod`,
/// `HAVE_TIME_BEGIN_PERIOD`); elsewhere this is a no-op.
fn set_system_timer_resolution_ms(period: i32) {
    #[cfg(windows)]
    windows::set_system_timer_resolution_ms(period);
    #[cfg(not(windows))]
    let _ = period;
}

/// Translation of `SDL_InitTicks()`.
fn init_ticks() {
    let mut guard = TICKS_INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if TICK_START.load(Ordering::Acquire) != 0 {
        return;
    }

    /* If we didn't set a precision, set it high. This affects lots of things
    on Windows besides the SDL timers, like audio callbacks, etc. */
    *guard = crate::hints::watch(crate::hints::TIMER_RESOLUTION, |change| {
        // Unless the hint says otherwise, let's have good sleep precision
        let period = match change.new_value {
            Some(h) if !h.is_empty() => crate::stdlib::atoi(h),
            _ => 1,
        };
        if period != 0 || change.old_value != change.new_value {
            set_system_timer_resolution_ms(period);
        }
    })
    .ok()
    .map(|cb| cb.detach_guard());

    let tick_freq = performance_frequency();
    debug_assert!(tick_freq > 0 && tick_freq <= u32::MAX as u64);

    let mut gcd = crate::utils::gcd(NS_PER_SECOND as u32, tick_freq as u32);
    TICK_NUMERATOR_NS.store(NS_PER_SECOND as u32 / gcd, Ordering::Relaxed);
    TICK_DENOMINATOR_NS.store((tick_freq / gcd as u64) as u32, Ordering::Relaxed);

    gcd = crate::utils::gcd(MS_PER_SECOND as u32, tick_freq as u32);
    TICK_NUMERATOR_MS.store(MS_PER_SECOND as u32 / gcd, Ordering::Relaxed);
    TICK_DENOMINATOR_MS.store((tick_freq / gcd as u64) as u32, Ordering::Relaxed);

    let mut tick_start = performance_counter();
    if tick_start == 0 {
        tick_start = tick_start.wrapping_sub(1);
    }
    TICK_START.store(tick_start, Ordering::Release);
}

/// Translation of `SDL_QuitTicks()`.
pub(crate) fn quit_ticks() {
    let mut guard = TICKS_INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    *guard = None; // removes the hint watcher
    set_system_timer_resolution_ms(0); // always release our timer resolution request.
    TICK_START.store(0, Ordering::Release);
}

/// Nanoseconds since the tick counter started (first use of this module or
/// of SDL). Translation of `SDL_GetTicksNS()`.
pub fn ticks_ns() -> u64 {
    if TICK_START.load(Ordering::Acquire) == 0 {
        init_ticks();
    }
    let starting_value = performance_counter().wrapping_sub(TICK_START.load(Ordering::Acquire));
    let num = TICK_NUMERATOR_NS.load(Ordering::Relaxed) as u64;
    let den = TICK_DENOMINATOR_NS.load(Ordering::Relaxed) as u64;
    (starting_value / den) * num + (starting_value % den) * num / den
}

/// Milliseconds since the tick counter started. Translation of `SDL_GetTicks()`.
pub fn ticks_ms() -> u64 {
    if TICK_START.load(Ordering::Acquire) == 0 {
        init_ticks();
    }
    let starting_value = performance_counter().wrapping_sub(TICK_START.load(Ordering::Acquire));
    let num = TICK_NUMERATOR_MS.load(Ordering::Relaxed) as u64;
    let den = TICK_DENOMINATOR_MS.load(Ordering::Relaxed) as u64;
    (starting_value / den) * num + (starting_value % den) * num / den
}

/// Time since the tick counter started, as a `Duration`.
pub fn ticks() -> Duration {
    Duration::from_nanos(ticks_ns())
}

/// Wait at least `duration` before returning (possibly longer due to OS
/// scheduling). Translation of `SDL_Delay()`/`SDL_DelayNS()`.
pub fn delay(duration: Duration) {
    sys_delay(duration);
}

/// Wait `duration` as precisely as possible, sleeping in short slices and
/// busy-waiting the remainder. Translation of `SDL_DelayPrecise()`.
pub fn delay_precise(duration: Duration) {
    let ns = duration.as_nanos().min(u64::MAX as u128) as u64;
    let mut current_value = ticks_ns();
    let target_value = current_value.wrapping_add(ns);

    // Sleep for a short number of cycles when real sleeps are desired.
    // We'll use 1 ms, it's the minimum guaranteed to produce real sleeps across
    // all platforms.
    const SHORT_SLEEP_NS: u64 = NS_PER_MS;

    // Try to sleep short of target_value. If for some crazy reason
    // a particular platform sleeps for less than 1 ms when 1 ms was requested,
    // that's fine, the code below can cope with that, but in practice no
    // platforms behave that way.
    let mut max_sleep_ns = SHORT_SLEEP_NS;
    while current_value.wrapping_add(max_sleep_ns) < target_value {
        // Sleep for a short time
        sys_delay(Duration::from_nanos(SHORT_SLEEP_NS));

        let now = ticks_ns();
        // Upstream's unsigned subtraction wraps if the tick counter
        // restarted meanwhile (SDL_Quit() on another thread), which makes
        // the wait last until the new counter passes the old target; here
        // a restart ends the wait.
        let Some(next_sleep_ns) = now.checked_sub(current_value) else {
            return;
        };
        if next_sleep_ns > max_sleep_ns {
            max_sleep_ns = next_sleep_ns;
        }
        current_value = now;
    }

    // Do a shorter sleep of the remaining time here, less the max overshoot in
    // the first loop. (See the upstream source for the full rationale.)
    if current_value < target_value
        && (target_value - current_value) > (max_sleep_ns - SHORT_SLEEP_NS)
    {
        let delay_ns = (target_value - current_value) - (max_sleep_ns - SHORT_SLEEP_NS);
        sys_delay(Duration::from_nanos(delay_ns));
        let now = ticks_ns();
        if now < current_value {
            return; // (the tick counter restarted)
        }
        current_value = now;
    }

    // We've likely undershot target_value at this point by a pretty small
    // amount, but maybe not; handle a large undershoot with more short sleeps.
    while current_value.wrapping_add(SHORT_SLEEP_NS) < target_value {
        sys_delay(Duration::from_nanos(SHORT_SLEEP_NS));
        let now = ticks_ns();
        if now < current_value {
            return; // (the tick counter restarted)
        }
        current_value = now;
    }

    // Spin for any remaining time
    while current_value < target_value {
        std::hint::spin_loop();
        let now = ticks_ns();
        if now < current_value {
            return; // (the tick counter restarted)
        }
        current_value = now;
    }
}

// ---------------------------------------------------------------------------
// Timers
// ---------------------------------------------------------------------------

/// A timer callback: receives the interval it was scheduled with and returns
/// the next interval, or `None` (or a zero duration) to stop.
/// Translation of `SDL_NSTimerCallback`.
pub type TimerCallback = Box<dyn FnMut(Duration) -> Option<Duration> + Send + 'static>;

#[cfg(not(target_os = "emscripten"))]
/// Shared, cross-thread part of a timer (translation of the shared fields of `SDL_Timer`).
struct TimerShared {
    callback: Mutex<TimerCallback>,
    canceled: AtomicBool,
}

#[cfg(not(target_os = "emscripten"))]
/// A scheduled timer; only touched by the timer thread once queued.
struct Scheduled {
    id: u32,
    shared: Arc<TimerShared>,
    interval: u64,
    scheduled: u64,
}

#[cfg(not(target_os = "emscripten"))]
/// Translation of `SDL_TimerData`.
struct TimerData {
    init: InitState,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    timermap: LazyLock<Mutex<HashMap<u32, Arc<TimerShared>>>>,
    // upstream hands `pending` over under a spinlock; a mutex is equivalent
    pending: Mutex<Vec<Scheduled>>,
    sem: Semaphore,
    active: AtomicBool,
}

#[cfg(not(target_os = "emscripten"))]
static TIMER_DATA: TimerData = TimerData {
    init: InitState::new(),
    thread: Mutex::new(None),
    timermap: LazyLock::new(|| Mutex::new(HashMap::new())),
    pending: Mutex::new(Vec::new()),
    sem: Semaphore::new(0),
    active: AtomicBool::new(false),
};

#[cfg(not(target_os = "emscripten"))]
/* The idea here is that any thread might add a timer, but a single
 * thread manages the active timer queue, sorted by scheduling time.
 *
 * Timers are removed by simply setting a canceled flag
 */
#[cfg(not(target_os = "emscripten"))]
fn add_timer_internal(timers: &mut Vec<Scheduled>, timer: Scheduled) {
    // Keep the list sorted by `scheduled`, inserting after equal entries.
    let pos = timers
        .iter()
        .position(|curr| curr.scheduled > timer.scheduled)
        .unwrap_or(timers.len());
    timers.insert(pos, timer);
}

#[cfg(not(target_os = "emscripten"))]
fn timer_thread(data: &'static TimerData) {
    // List of timers - this is only touched by the timer thread
    let mut timers: Vec<Scheduled> = Vec::new();

    /* Threaded timer loop:
     *  1. Queue timers added by other threads
     *  2. Handle any timers that should dispatch this cycle
     *  3. Wait until next dispatch time or new timer arrives
     */
    loop {
        // Get any timers ready to be queued
        let pending = std::mem::take(&mut *data.pending.lock().unwrap_or_else(|e| e.into_inner()));

        // Sort the pending timers into our list
        for current in pending {
            add_timer_internal(&mut timers, current);
        }

        // Check to see if we're still running, after maintenance
        if !data.active.load(Ordering::Acquire) {
            break;
        }

        // Initial delay if there are no timers
        let mut delay: Option<u64> = None;
        let tick = ticks_ns();

        // Process all the pending timers for this tick
        while let Some(front) = timers.first() {
            if tick < front.scheduled {
                // Scheduled for the future, wait a bit
                delay = Some(front.scheduled - tick);
                break;
            }

            // We're going to do something with this timer
            let mut current = timers.remove(0);

            let interval = if current.shared.canceled.load(Ordering::Acquire) {
                0
            } else {
                let mut cb = current
                    .shared
                    .callback
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                cb(Duration::from_nanos(current.interval))
                    .map_or(0, |d| d.as_nanos().min(u64::MAX as u128) as u64)
            };

            if interval > 0 {
                // Reschedule this timer
                current.interval = interval;
                current.scheduled = tick + interval;
                add_timer_internal(&mut timers, current);
            } else {
                // Upstream parks the struct on a freelist with canceled=1 and
                // leaves the map entry until the ID is recycled; dropping the
                // entry now is observably identical.
                current.shared.canceled.store(true, Ordering::Release);
                data.timermap
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&current.id);
            }
        }

        // Adjust the delay based on processing time
        let now = ticks_ns();
        // (Uint64 arithmetic, as upstream: the tick counter restarts when
        // SDL quits and inits again)
        let elapsed = now.wrapping_sub(tick);
        let delay = delay.map(|d| d.saturating_sub(elapsed));

        /* Note that each time a timer is added, this will return
          immediately, but we process the timers added all at once.
          That's okay, it just means we run through the loop a few
          extra times.
        */
        data.sem.wait_timeout(delay.map(Duration::from_nanos));
    }
}

#[cfg(not(target_os = "emscripten"))]
/// Translation of `SDL_InitTimers()`.
fn init_timers() -> Result<()> {
    let data = &TIMER_DATA;
    if !data.init.should_init() {
        return Ok(());
    }

    data.active.store(true, Ordering::Release);

    // Timer threads use a callback into the app, so we can't set a limited stack size here.
    match std::thread::Builder::new()
        .name("SDLTimer".into())
        .spawn(move || timer_thread(&TIMER_DATA))
    {
        Ok(handle) => {
            *data.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
            data.init.set_initialized(true);
            Ok(())
        }
        Err(e) => {
            data.init.set_initialized(true);
            quit_timers();
            Err(err!("Couldn't create timer thread: {e}"))
        }
    }
}

#[cfg(not(target_os = "emscripten"))]
/// Stop the timer thread and drop all timers. Translation of `SDL_QuitTimers()`.
pub(crate) fn quit_timers() {
    let data = &TIMER_DATA;
    if !data.init.should_quit() {
        return;
    }

    data.active.store(false, Ordering::Release);

    // Shutdown the timer thread
    let thread = data.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(handle) = thread {
        data.sem.signal();
        let _ = handle.join();
    }

    // Clean up the timer entries
    data.pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    data.timermap
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();

    data.init.set_initialized(false);
}

/// Translation of `SDL_CreateTimer()`: queue the timer for the timer
/// thread, starting it first if need be.
#[cfg(not(target_os = "emscripten"))]
fn create_timer(interval_ns: u64, callback: TimerCallback) -> Result<u32> {
    let data = &TIMER_DATA;
    init_timers()?;

    let id = crate::utils::next_object_id();
    let shared = Arc::new(TimerShared {
        callback: Mutex::new(callback),
        canceled: AtomicBool::new(false),
    });
    let scheduled = Scheduled {
        id,
        shared: shared.clone(),
        interval: interval_ns,
        scheduled: ticks_ns() + interval_ns,
    };

    data.timermap
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, shared);

    // Add the timer to the pending list for the timer thread
    data.pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(scheduled);

    // Wake up the timer thread if necessary
    data.sem.signal();

    Ok(id)
}

/// Whether the timer is still in the map and not canceled.
#[cfg(not(target_os = "emscripten"))]
fn timer_is_active(id: u32) -> bool {
    TIMER_DATA
        .timermap
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .is_some_and(|t| !t.canceled.load(Ordering::Acquire))
}

/// A handle to a running timer. Translation of `SDL_TimerID`.
///
/// Dropping the handle cancels the timer (RAII). Call [`Timer::detach`] to
/// let it run for the life of the process instead, which is C SDL's default.
#[must_use = "dropping a Timer cancels it; call .detach() to keep it running"]
#[derive(Debug)]
pub struct Timer {
    id: u32,
}

impl Timer {
    /// Schedule `callback` to run on SDL's timer thread after `interval`.
    ///
    /// The callback receives the interval it was scheduled with and returns
    /// the next interval (`Some(interval)` keeps a periodic alarm going) or
    /// `None` to stop. For short intervals it can run before this function
    /// returns. Translation of `SDL_AddTimerNS()`/`SDL_AddTimer()`.
    ///
    /// On Emscripten there is no timer thread: the browser's event loop
    /// runs the callback (`emscripten_set_timeout()`, in whole
    /// milliseconds) once `interval` has passed and the program has
    /// returned to it, from a main loop iteration (see
    /// [`enter_app_main_callbacks`](crate::app::enter_app_main_callbacks))
    /// or from `main`.
    pub fn new(
        interval: Duration,
        callback: impl FnMut(Duration) -> Option<Duration> + Send + 'static,
    ) -> Result<Timer> {
        let interval_ns = interval.as_nanos().min(u64::MAX as u128) as u64;
        let id = create_timer(interval_ns, Box::new(callback))?;
        Ok(Timer { id })
    }

    /// Cancel the timer. Returns `true` if it was still active, `false` if it
    /// had already finished or been cancelled. Translation of `SDL_RemoveTimer()`.
    ///
    /// On Emscripten, as upstream's there, a timer whose callback asked for
    /// no more is still found, and cancelling it returns `true`.
    pub fn cancel(self) -> bool {
        let id = self.id;
        std::mem::forget(self);
        remove_timer(id)
    }

    /// Whether the timer is still scheduled (has neither finished nor been cancelled).
    pub fn is_active(&self) -> bool {
        timer_is_active(self.id)
    }

    /// Let the timer keep running after the handle is dropped (SDL's default
    /// behaviour, where a timer runs until its callback returns 0 or
    /// `SDL_RemoveTimer` is called).
    pub fn detach(self) {
        std::mem::forget(self);
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        remove_timer(self.id);
    }
}

#[cfg(not(target_os = "emscripten"))]
/// Translation of `SDL_RemoveTimer()`.
fn remove_timer(id: u32) -> bool {
    let data = &TIMER_DATA;
    // Find the timer
    let entry = data
        .timermap
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    match entry {
        Some(timer) if !timer.canceled.swap(true, Ordering::AcqRel) => true,
        _ => false, // "Timer not found"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::time::Instant;

    /// The counter runs at its frequency and delays wait at least as long
    /// as asked, measured with both the counter and `std`'s clock.
    #[test]
    fn performance_counter_and_delays() {
        let freq = performance_frequency();
        assert!(freq > 0 && freq <= u32::MAX as u64);
        #[cfg(target_os = "linux")]
        assert_eq!(freq, NS_PER_SECOND as u64); // (CLOCK_MONOTONIC_RAW)

        for ms in [0u64, 1, 3, 16] {
            let start = Instant::now();
            let c0 = performance_counter();
            delay(Duration::from_millis(ms));
            let c1 = performance_counter();
            let wall = start.elapsed();
            assert!(c1 >= c0);
            let counted = Duration::from_secs_f64((c1 - c0) as f64 / freq as f64);
            // (Windows timers may expire a little before the counter says
            // the time is up, so allow it half a millisecond)
            let slack = Duration::from_micros(if cfg!(windows) { 500 } else { 0 });
            assert!(
                wall + slack >= Duration::from_millis(ms),
                "{ms} ms: {wall:?}"
            );
            // The counter and std's clock agree (a wrong frequency would be
            // off by orders of magnitude), whatever the scheduler adds.
            let diff = counted.abs_diff(wall);
            assert!(diff < Duration::from_millis(10), "{counted:?} vs {wall:?}");
            assert!(counted + slack + Duration::from_micros(100) >= Duration::from_millis(ms));
        }

        // Same clock as clock_gettime(CLOCK_MONOTONIC_RAW).
        #[cfg(target_os = "linux")]
        {
            let mut ts = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            // SAFETY: ts is a valid out-parameter.
            unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut ts) };
            let raw = ts.tv_sec as u64 * NS_PER_SECOND as u64 + ts.tv_nsec as u64;
            let ours = performance_counter();
            assert!(ours >= raw && ours - raw < 50_000_000, "{ours} {raw}");
        }
    }

    /// Ticks request a 1 ms system timer resolution unless
    /// `SDL_HINT_TIMER_RESOLUTION` says otherwise, and release it on quit.
    #[cfg(windows)]
    #[test]
    fn windows_timer_resolution() {
        let _l = crate::test_support::test_lock();
        quit_ticks();
        assert_eq!(windows::timer_period(), 0);
        ticks_ns();
        assert_eq!(windows::timer_period(), 1);
        crate::hints::set(crate::hints::TIMER_RESOLUTION, "5").unwrap();
        assert_eq!(windows::timer_period(), 5);
        crate::hints::set(crate::hints::TIMER_RESOLUTION, "0").unwrap();
        assert_eq!(windows::timer_period(), 0);
        crate::hints::reset(crate::hints::TIMER_RESOLUTION);
        assert_eq!(windows::timer_period(), 1);
        quit_ticks();
        assert_eq!(windows::timer_period(), 0);
        ticks_ns();

        // Delays work on threads that make their own timer or event, and
        // close them when they exit.
        let t = std::thread::spawn(|| {
            let start = Instant::now();
            delay(Duration::from_millis(2));
            (start.elapsed(), windows::has_waitable_timer())
        });
        let (elapsed, _) = t.join().unwrap();
        assert!(elapsed >= Duration::from_millis(2));
    }

    #[test]
    fn ticks_increase() {
        // (init::quit() in another test restarts the tick counter)
        let _l = crate::test_support::test_lock();
        let a = ticks_ns();
        delay(Duration::from_millis(2));
        let b = ticks_ns();
        assert!(b >= a + 1_000_000, "{a} {b}");
        assert!(ticks_ms() <= ticks_ns() / 1_000_000 + 1);
        assert!(ticks() >= Duration::from_millis(1));
        assert!(performance_counter() > 0);
    }

    #[test]
    fn precise_delay() {
        // (init::quit() in another test restarts the tick counter)
        let _l = crate::test_support::test_lock();
        let start = ticks_ns();
        delay_precise(Duration::from_millis(3));
        let elapsed = ticks_ns() - start;
        assert!(elapsed >= 3_000_000, "{elapsed}");
        assert!(elapsed < 200_000_000, "{elapsed}");
    }

    #[test]
    #[cfg_attr(target_os = "emscripten", ignore = "threads (SDL_THREADS_DISABLED)")]
    fn precise_delay_ends_when_ticks_restart() {
        let _l = crate::test_support::test_lock();
        let _ = ticks_ns();
        let restarter = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            quit_ticks();
        });
        let start = std::time::Instant::now();
        // (upstream would wait until the restarted counter reaches the old target)
        delay_precise(Duration::from_secs(5));
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "{:?}",
            start.elapsed()
        );
        restarter.join().unwrap();
    }

    fn wait_until(deadline_ms: u64, mut cond: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_millis(deadline_ms);
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            delay(Duration::from_millis(1));
        }
        cond()
    }

    #[test]
    #[cfg_attr(
        target_os = "emscripten",
        ignore = "the timers fire from the event loop (tests/emscripten_main.rs)"
    )]
    fn one_shot_and_periodic() {
        // (init::quit() in other tests shuts the timer thread down)
        let _l = crate::test_support::test_lock();
        let fired = Arc::new(AtomicUsize::new(0));
        let f = fired.clone();
        let timer = Timer::new(Duration::from_millis(5), move |_| {
            f.fetch_add(1, Ordering::SeqCst);
            None
        })
        .unwrap();
        assert!(wait_until(2000, || fired.load(Ordering::SeqCst) == 1));
        assert!(wait_until(2000, || !timer.is_active()));
        assert!(!timer.cancel()); // already finished

        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let timer = Timer::new(Duration::from_millis(2), move |interval| {
            c.fetch_add(1, Ordering::SeqCst);
            Some(interval)
        })
        .unwrap();
        assert!(wait_until(2000, || count.load(Ordering::SeqCst) >= 3));
        assert!(timer.is_active());
        assert!(timer.cancel());
        delay(Duration::from_millis(10));
        let after = count.load(Ordering::SeqCst);
        delay(Duration::from_millis(10));
        assert_eq!(count.load(Ordering::SeqCst), after);
    }

    #[test]
    #[cfg_attr(
        target_os = "emscripten",
        ignore = "the timers fire from the event loop (tests/emscripten_main.rs)"
    )]
    fn drop_cancels_detach_keeps() {
        // (init::quit() in other tests shuts the timer thread down)
        let _l = crate::test_support::test_lock();
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        // (long enough not to fire before the drop on a loaded machine, or
        // under an emulator's ARM translation)
        let t = Timer::new(Duration::from_millis(200), move |i| {
            c.fetch_add(1, Ordering::SeqCst);
            Some(i)
        })
        .unwrap();
        drop(t);
        delay(Duration::from_millis(300));
        assert_eq!(count.load(Ordering::SeqCst), 0);

        let c = count.clone();
        Timer::new(Duration::from_millis(1), move |_| {
            c.fetch_add(1, Ordering::SeqCst);
            None
        })
        .unwrap()
        .detach();
        assert!(wait_until(2000, || count.load(Ordering::SeqCst) == 1));
    }
}
