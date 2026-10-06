// Rust translation of include/SDL3/SDL_test.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-test — SDL's test framework, translated to Rust
//!
//! The pure-Rust translation of `SDL_test`, the library SDL's own test
//! programs are built on (`src/test/SDL_test_*.c` and
//! `include/SDL3/SDL_test*.h`). This code is a part of the SDL test library,
//! not the main SDL library: it is a separate crate over [`sdl3`].
//!
//! * [`assert`]: assertions that log and count instead of aborting.
//! * [`compare`]: surface and memory comparisons with a report of the
//!   differences.
//! * [`crc32`], [`md5`]: the checksums the harness and memory tracker use.
//! * [`font`]: text drawing with the debug font and multi-line text windows.
//! * [`fuzzer`]: reproducible random test data from an execution key.
//! * [`harness`]: test suites and cases, run with seeds, filters, iterations
//!   and a summary.
//! * [`log`]: logging in the `TEST` category with timestamps.
//! * [`memory`]: an allocation tracker for finding leaks.
//!
//! As in the [`sdl3`] crate, the implementation is a line-by-line translation
//! and the API is designed for Rust; every item names the C symbol it
//! translates.

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

pub mod assert;
pub mod compare;
pub mod crc32;
pub mod font;
pub mod fuzzer;
pub mod harness;
mod internal;
pub mod log;
pub mod md5;
pub mod memory;

/* Global definitions */

/// The longest log message, in bytes, including the terminating NUL of the
/// C buffer (so messages are cut at one byte less). Translation of
/// `SDLTEST_MAX_LOGMESSAGE_LENGTH`.
///
/// Note: Maximum size of SDLTest log message is less than SDL's limit
/// to ensure we can fit additional information such as the timestamp.
pub const MAX_LOGMESSAGE_LENGTH: usize = 3584;

/// Shared by this crate's tests that touch its process-global state (the
/// assert counters, the fuzzer, log output), so those tests don't race.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Arc, Mutex, MutexGuard};

    use sdl3::log::{Category, Priority};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Take the lock, recovering it if a previous holder panicked.
    pub(crate) fn test_lock() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| {
            TEST_LOCK.clear_poison();
            e.into_inner()
        })
    }

    /// The messages logged while it is alive (`TEST` and `APPLICATION`
    /// categories, every priority), with timestamps and colors off.
    pub(crate) struct Capture {
        lines: Arc<Mutex<Vec<(Category, Priority, String)>>>,
    }

    impl Capture {
        pub(crate) fn new() -> Capture {
            let lines: Arc<Mutex<Vec<(Category, Priority, String)>>> = Arc::default();
            let sink = lines.clone();
            sdl3::log::set_priority(Category::Test, Priority::Trace);
            sdl3::log::set_priority(Category::Application, Priority::Trace);
            sdl3::log::set_output(move |record| {
                sink.lock().unwrap().push((
                    record.category,
                    record.priority,
                    record.message.to_owned(),
                ));
            });
            crate::log::set_timestamps(false);
            crate::internal::set_color(false);
            Capture { lines }
        }

        /// The messages so far.
        pub(crate) fn lines(&self) -> Vec<(Category, Priority, String)> {
            self.lines.lock().unwrap().clone()
        }

        /// Just the text of the messages so far.
        pub(crate) fn messages(&self) -> Vec<String> {
            self.lines().into_iter().map(|(_, _, m)| m).collect()
        }

        /// Forget the messages so far.
        pub(crate) fn clear(&self) {
            self.lines.lock().unwrap().clear();
        }
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            sdl3::log::reset_output();
            sdl3::log::reset_priorities();
            crate::log::set_timestamps(true);
            crate::internal::set_color(true);
        }
    }
}
