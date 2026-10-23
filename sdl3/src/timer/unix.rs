// Rust translation of src/timer/unix/SDL_systimer.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Unix performance counter (`clock_gettime()` on a monotonic clock,
//! or `gettimeofday()`) and delay (`nanosleep()`, or on Emscripten
//! `emscripten_sleep()` when the program is built with Asyncify).

use std::sync::OnceLock;

use super::NS_PER_SECOND;

const US_PER_SECOND: u64 = 1_000_000;

/* The clock_gettime provides monotonous time, so we should use it if
   it's available. The clock_gettime function is behind ifdef
   for __USE_POSIX199309
   Tommi Kyntola (tommi.kyntola@ray.fi) 27/09/2005
*/
/* Reworked monotonic clock to not assume the current system has one
  as not all linux kernels provide a monotonic clock (yeah recent ones
  probably do)
  Also added macOS Monotonic clock support
  Based on work in https://github.com/ThomasHabets/monotonic_clock
*/

// Use CLOCK_MONOTONIC_RAW, if available, which is not subject to adjustment by NTP
// Older Android phones have a buggy CLOCK_MONOTONIC_RAW, use CLOCK_MONOTONIC
// See fix: https://github.com/torvalds/linux/commit/dbb236c1ceb697a559e0694ac4c9e7b9131d0b16
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
const SDL_MONOTONIC_CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC_RAW;
#[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
const SDL_MONOTONIC_CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;

/// `has_monotonic_time`, set by the first `CheckMonotonicTime()`.
static HAS_MONOTONIC_TIME: OnceLock<bool> = OnceLock::new();

/// Translation of `CheckMonotonicTime()` (once).
fn has_monotonic_time() -> bool {
    *HAS_MONOTONIC_TIME.get_or_init(|| {
        let mut value = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: value is a valid out-parameter.
        unsafe { libc::clock_gettime(SDL_MONOTONIC_CLOCK, &mut value) == 0 }
    })
}

/// Translation of `SDL_GetPerformanceCounter()` (Unix).
pub(super) fn get_performance_counter() -> u64 {
    let mut ticks: u64;

    if has_monotonic_time() {
        let mut now = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };

        // SAFETY: now is a valid out-parameter.
        unsafe { libc::clock_gettime(SDL_MONOTONIC_CLOCK, &mut now) };
        ticks = now.tv_sec as u64;
        ticks = ticks.wrapping_mul(NS_PER_SECOND as u64);
        ticks = ticks.wrapping_add(now.tv_nsec as u64);
    } else {
        let mut now = libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        };

        // SAFETY: now is a valid out-parameter; no time zone is asked for.
        unsafe { libc::gettimeofday(&mut now, std::ptr::null_mut()) };
        ticks = now.tv_sec as u64;
        ticks = ticks.wrapping_mul(US_PER_SECOND);
        ticks = ticks.wrapping_add(now.tv_usec as u64);
    }
    ticks
}

/// Translation of `SDL_GetPerformanceFrequency()` (Unix).
pub(super) fn get_performance_frequency() -> u64 {
    if has_monotonic_time() {
        return NS_PER_SECOND as u64;
    }

    US_PER_SECOND
}

/// Translation of `SDL_SYS_DelayNS()` (Unix, with `nanosleep()`).
///
/// On Emscripten, in a program linked with `-sASYNCIFY` (or `-sJSPI`) and
/// unless `SDL_HINT_EMSCRIPTEN_ASYNCIFY` is false, the delay is an
/// `emscripten_sleep()`, in whole milliseconds, which returns to the
/// browser's event loop meanwhile (so timers fire and the page stays
/// responsive). Otherwise emscripten's `nanosleep()` waits busily.
///
/// Note (upstream): with JSPI only what JavaScript called through
/// `WebAssembly.promising` can suspend (`main`, which emscripten wraps), so
/// a delay in the main callbacks' iterations (emscripten's main loop calls
/// them), in a timer's callback or in an event handler throws a
/// `SuspendError`, as upstream's does; so do the window framebuffer's
/// updates, buffer swaps and message boxes, which sleep the same way. The
/// hint false keeps them from sleeping. With Asyncify they can sleep, but
/// Rust's WebAssembly exceptions, which Asyncify doesn't fully support,
/// break a sleep while a panic unwinds (in a `Drop`).
pub(super) fn sys_delay_ns(ns: u64) {
    #[cfg(target_os = "emscripten")]
    {
        use sdl3_emscripten::sys;
        // SAFETY: takes nothing; returns 0 without Asyncify or JSPI.
        let has_asyncify = unsafe { sys::emscripten_has_asyncify() } != 0;
        if has_asyncify && crate::hints::get_bool(crate::hints::EMSCRIPTEN_ASYNCIFY, true) {
            // pseudo-synchronous pause, used directly or through e.g. SDL_WaitEvent
            // SAFETY: the program is built to unwind and rewind here.
            unsafe { sys::emscripten_sleep((ns / super::NS_PER_MS) as std::ffi::c_uint) };
            return;
        }
    }

    // Set the timeout interval
    let mut remaining = libc::timespec {
        tv_sec: (ns / NS_PER_SECOND as u64) as libc::time_t,
        tv_nsec: (ns % NS_PER_SECOND as u64) as _,
    };
    loop {
        let tv = remaining;
        // SAFETY: both pointers are valid.
        let was_error = unsafe { libc::nanosleep(&tv, &mut remaining) } != 0;
        let errno = std::io::Error::last_os_error().raw_os_error();
        if !(was_error && errno == Some(libc::EINTR)) {
            break;
        }
    }
}
