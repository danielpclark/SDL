// Rust translation of src/timer/SDL_timer.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Time management: the SDL tick counter, delays and callback timers.
//!
//! Direct translation of `SDL_timer.c`. The platform pieces
//! (`SDL_GetPerformanceCounter`, `SDL_SYS_DelayNS`) are implemented with
//! `std::time` and `std::thread::sleep`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::err;
use crate::error::Result;
use crate::thread::{InitState, Semaphore};

/// Nanoseconds in a second, as an `i64` (SDL's `SDL_NS_PER_SECOND` is `long long`).
pub(crate) const NS_PER_SECOND: i64 = 1_000_000_000;
const MS_PER_SECOND: u64 = 1000;
const NS_PER_MS: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// Performance counter (platform layer)
// ---------------------------------------------------------------------------

static PERF_START: OnceLock<Instant> = OnceLock::new();

/// The current value of the high resolution counter.
///
/// Counter values are only meaningful relative to each other; divide a
/// difference by [`performance_frequency`] to get seconds.
/// Translation of `SDL_GetPerformanceCounter()`. This implementation counts
/// nanoseconds from the first call and never returns 0.
pub fn performance_counter() -> u64 {
    let start = *PERF_START.get_or_init(Instant::now);
    (Instant::now().duration_since(start).as_nanos() as u64).wrapping_add(1)
}

/// Counts per second of the high resolution counter. Translation of `SDL_GetPerformanceFrequency()`.
pub fn performance_frequency() -> u64 {
    NS_PER_SECOND as u64
}

/// Translation of `SDL_SYS_DelayNS()` (Unix: `nanosleep`).
fn sys_delay(d: Duration) {
    if d.is_zero() {
        std::thread::yield_now();
    } else {
        std::thread::sleep(d);
    }
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
/// system timer resolution to request (`timeBeginPeriod`); elsewhere, as
/// here, this is a no-op.
fn set_system_timer_resolution_ms(_period: i32) {}

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

/// Shared, cross-thread part of a timer (translation of the shared fields of `SDL_Timer`).
struct TimerShared {
    callback: Mutex<TimerCallback>,
    canceled: AtomicBool,
}

/// A scheduled timer; only touched by the timer thread once queued.
struct Scheduled {
    id: u32,
    shared: Arc<TimerShared>,
    interval: u64,
    scheduled: u64,
}

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

static TIMER_DATA: TimerData = TimerData {
    init: InitState::new(),
    thread: Mutex::new(None),
    timermap: LazyLock::new(|| Mutex::new(HashMap::new())),
    pending: Mutex::new(Vec::new()),
    sem: Semaphore::new(0),
    active: AtomicBool::new(false),
};

/* The idea here is that any thread might add a timer, but a single
 * thread manages the active timer queue, sorted by scheduling time.
 *
 * Timers are removed by simply setting a canceled flag
 */
fn add_timer_internal(timers: &mut Vec<Scheduled>, timer: Scheduled) {
    // Keep the list sorted by `scheduled`, inserting after equal entries.
    let pos = timers
        .iter()
        .position(|curr| curr.scheduled > timer.scheduled)
        .unwrap_or(timers.len());
    timers.insert(pos, timer);
}

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
    pub fn new(
        interval: Duration,
        callback: impl FnMut(Duration) -> Option<Duration> + Send + 'static,
    ) -> Result<Timer> {
        let data = &TIMER_DATA;
        init_timers()?;

        let interval_ns = interval.as_nanos().min(u64::MAX as u128) as u64;
        let id = crate::utils::next_object_id();
        let shared = Arc::new(TimerShared {
            callback: Mutex::new(Box::new(callback)),
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

        Ok(Timer { id })
    }

    /// Cancel the timer. Returns `true` if it was still active, `false` if it
    /// had already finished or been cancelled. Translation of `SDL_RemoveTimer()`.
    pub fn cancel(self) -> bool {
        let id = self.id;
        std::mem::forget(self);
        remove_timer(id)
    }

    /// Whether the timer is still scheduled (has neither finished nor been cancelled).
    pub fn is_active(&self) -> bool {
        TIMER_DATA
            .timermap
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&self.id)
            .is_some_and(|t| !t.canceled.load(Ordering::Acquire))
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
        assert_eq!(performance_frequency(), 1_000_000_000);
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
    fn drop_cancels_detach_keeps() {
        // (init::quit() in other tests shuts the timer thread down)
        let _l = crate::test_support::test_lock();
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let t = Timer::new(Duration::from_millis(2), move |i| {
            c.fetch_add(1, Ordering::SeqCst);
            Some(i)
        })
        .unwrap();
        drop(t);
        delay(Duration::from_millis(20));
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
