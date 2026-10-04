// Rust translation of part of src/core/linux/SDL_evdev.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The evdev helpers the joystick driver shares with the console input
//! code: event timestamps. (The rest of `SDL_evdev.c`, the keyboard, mouse
//! and touch reader for the KMS/DRM video driver, comes with that driver,
//! along with `SDL_evdev_kbd.c`.)

use std::sync::atomic::{AtomicU64, Ordering};

use super::input::input_event;

/// Translation of the `static Uint64 timestamp_offset` of
/// `SDL_EVDEV_GetEventTimestamp()`.
static TIMESTAMP_OFFSET: AtomicU64 = AtomicU64::new(0);

/// The SDL timestamp (nanoseconds on the `ticks_ns()` clock) of an evdev
/// event. Translation of `SDL_EVDEV_GetEventTimestamp()`.
pub(crate) fn get_event_timestamp(event: &input_event) -> u64 {
    let now = crate::timer::ticks_ns();

    /* The kernel internally has nanosecond timestamps, but converts it
    to microseconds when delivering the events */
    let mut timestamp = event.time.tv_sec as u64;
    timestamp = timestamp.wrapping_mul(1_000_000_000);
    timestamp = timestamp.wrapping_add((event.time.tv_usec as u64).wrapping_mul(1000));

    let mut timestamp_offset = TIMESTAMP_OFFSET.load(Ordering::Relaxed);
    if timestamp_offset == 0 {
        timestamp_offset = now.wrapping_sub(timestamp);
    }
    timestamp = timestamp.wrapping_add(timestamp_offset);

    if timestamp > now {
        timestamp_offset = timestamp_offset.wrapping_sub(timestamp - now);
        timestamp = now;
    }
    TIMESTAMP_OFFSET.store(timestamp_offset, Ordering::Relaxed);
    timestamp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::linux::input::input_event_time;

    #[test]
    fn timestamps_follow_the_event_clock_and_never_run_ahead() {
        let event = |sec: i64, usec: i64| input_event {
            time: input_event_time {
                tv_sec: sec as _,
                tv_usec: usec as _,
            },
            ..input_event::default()
        };
        let a = get_event_timestamp(&event(100, 0));
        let b = get_event_timestamp(&event(100, 500));
        assert!(b >= a);
        assert!(b <= crate::timer::ticks_ns());
        // An event from the future is clamped to now, and pulls the offset back
        let c = get_event_timestamp(&event(1_000_000, 0));
        assert!(c <= crate::timer::ticks_ns());
        let d = get_event_timestamp(&event(1_000_000, 0));
        assert!(d >= c.saturating_sub(1_000_000_000));
    }
}
