// Rust translation of Simple DirectMedia Layer (SDL) 3.x.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3 — Simple DirectMedia Layer, translated to Rust
//!
//! A pure-Rust translation of the C sources of
//! [SDL 3](https://github.com/libsdl-org/SDL): the *implementation* is
//! carried over line by line (same algorithms, constants, tables, error
//! messages and quirks), while the *API* is designed as a Rust library:
//!
//! * fallible operations return [`Result`] with an [`Error`] carrying SDL's
//!   message, instead of `false` plus a thread-local `SDL_GetError()`;
//! * values are typed ([`properties::Value`], [`time::Time`],
//!   [`video::pixels::PixelFormat`], ...) rather than `void *`/`Uint32`;
//! * callbacks are closures; `void *userdata` does not exist;
//! * resources are owned: timers, hint watchers and property groups are
//!   values with `Drop`, not integer IDs you must remember to free;
//! * operations live on the types they belong to (`rect.intersection(&other)`,
//!   `format.details()`, `palette.find_color(c)`).
//!
//! Every item's documentation names the C function or type it translates, so
//! upstream knowledge and documentation carry over. Module layout mirrors
//! upstream `src/`.
//!
//! This crate contains **no C code** and the platform-independent core has
//! **no third-party dependencies**; the platform layer uses only the
//! pure-Rust OS declarations of `libc` and `windows-sys`. See `docs/ROADMAP.md` in the repository
//! for what is translated so far and the plan for the rest of SDL and its
//! satellite libraries.

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

pub mod app;
pub mod assert;
pub mod atomic;
pub mod audio;
pub mod camera;
mod core;
pub mod cpuinfo;
pub mod dialog;
pub mod error;
pub mod events;
pub mod filesystem;
pub mod guid;
pub mod haptic;
pub mod hints;
pub mod init;
pub mod io;
pub mod joystick;
pub mod loadso;
pub mod locale;
pub mod log;
pub mod misc;
pub mod notification;
pub mod power;
pub mod process;
pub mod properties;
pub mod render;
pub mod sensor;
pub mod stdlib;
pub mod storage;
pub mod thread;
pub mod time;
pub mod timer;
pub mod tray;
pub mod utils;
pub mod version;
pub mod video;

pub use error::{Error, ErrorKind, Result};
pub use guid::Guid;
pub use joystick::gamepad;
pub use version::{revision, version, Version};

/// Shut down every subsystem and free the library's global state.
/// Equivalent to [`init::quit`] (translation of `SDL_Quit()`), kept under
/// this name from before `SDL.c` was translated. Calling it is optional:
/// everything re-initializes lazily on next use.
pub fn shutdown() {
    init::quit();
}

/// Shared by every test that touches process-global state (hints, log output,
/// subsystem init, the event queue), so those tests don't race each other.
#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Take [`TEST_LOCK`], with SDL shut down: subsystems a previous holder
    /// left initialized would still belong to its (now finished) thread, so
    /// the events thread checks would fail. If the last holder panicked (a
    /// failed assertion), its hints are reset too, so one failing test
    /// doesn't fail or hang the ones after it.
    pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        let guard = match TEST_LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                TEST_LOCK.clear_poison();
                crate::hints::reset_all();
                poisoned.into_inner()
            }
        };
        if !crate::init::was_init(crate::init::InitFlags::ALL).is_empty() {
            crate::init::quit();
        }
        guard
    }

    /// While alive, `DISPLAY` is unset, so that no X11 video device is
    /// found (for tests of having no video device). Hold [`TEST_LOCK`].
    pub(crate) struct NoDisplay(Option<std::ffi::OsString>);

    impl NoDisplay {
        pub(crate) fn new() -> NoDisplay {
            let old = std::env::var_os("DISPLAY");
            std::env::remove_var("DISPLAY");
            NoDisplay(old)
        }
    }

    impl Drop for NoDisplay {
        fn drop(&mut self) {
            if let Some(display) = self.0.take() {
                std::env::set_var("DISPLAY", display);
            }
        }
    }

    /// A fresh directory under the system temp dir, removed on drop.
    pub(crate) struct TempDir(pub String);

    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!(
                "sdl3-rs-{tag}-{}-{}",
                std::process::id(),
                crate::timer::ticks_ns()
            ));
            let dir = dir.to_string_lossy().into_owned();
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        pub fn path(&self, rel: &str) -> String {
            format!("{}/{rel}", self.0)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
