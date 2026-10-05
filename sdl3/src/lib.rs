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
pub mod hidapi;
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

    /// The variables [`NoDisplay`] unsets, and the ones it sets (to what).
    const NO_DISPLAY_UNSET: [&str; 3] = ["DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET"];
    const NO_DISPLAY_SET: [(&str, &str); 1] = [("XDG_SESSION_TYPE", "tty")];

    /// While alive, `DISPLAY`, `WAYLAND_DISPLAY` and `WAYLAND_SOCKET` are
    /// unset and `XDG_SESSION_TYPE` is "tty", so that no X11 or Wayland video
    /// device is found (for tests of having no video device). Changed in
    /// SDL's environment snapshot too. Hold [`TEST_LOCK`].
    pub(crate) struct NoDisplay(Vec<(&'static str, Option<std::ffi::OsString>)>);

    impl NoDisplay {
        pub(crate) fn new() -> NoDisplay {
            let mut old = Vec::new();
            for name in NO_DISPLAY_UNSET {
                old.push((name, std::env::var_os(name)));
                crate::stdlib::unsetenv_unsafe(name).unwrap();
            }
            for (name, value) in NO_DISPLAY_SET {
                old.push((name, std::env::var_os(name)));
                crate::stdlib::setenv_unsafe(name, value, true).unwrap();
            }
            NoDisplay(old)
        }
    }

    impl Drop for NoDisplay {
        fn drop(&mut self) {
            for (name, value) in self.0.drain(..) {
                match value {
                    Some(value) => {
                        let _ = crate::stdlib::setenv_unsafe(name, &value.to_string_lossy(), true);
                    }
                    None => {
                        let _ = crate::stdlib::unsetenv_unsafe(name);
                    }
                }
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

    /// The environment pieces a test may find missing and skip over, by the
    /// name [`skip`] and `SDL3_TEST_REQUIRE` use (docs/HARDWARE_TESTING.md
    /// says what provides each one).
    pub(crate) const CAPABILITIES: &[(&str, &str)] = &[
        ("x11", "libX11 and the X extension libraries (libXtst, ...)"),
        ("xvfb", "an X server: Xvfb, or DISPLAY"),
        ("glx", "libGL with GLX contexts on the X server"),
        ("egl", "libEGL with contexts for the platform (Mesa, ANGLE)"),
        (
            "vulkan",
            "a Vulkan loader and driver with surface extensions",
        ),
        (
            "wayland",
            "libwayland-client, libxkbcommon and sway (or weston)",
        ),
        ("dbus", "libdbus and dbus-daemon"),
        ("pulseaudio", "libpulse and the pulseaudio binary (pactl)"),
        ("pipewire", "libpipewire and a running PipeWire server"),
        ("alsa", "libasound with its file/null PCM plugins"),
        ("udev", "libudev and a udev database"),
        ("uinput", "a writable /dev/uinput"),
        ("hidapi", "the HID backend: hidraw/libudev, or hid.dll"),
        ("xinput", "an XInput DLL"),
        (
            "gameinput",
            "a GameInput DLL with the v3 API (the redistributable, or a recent inbox one)",
        ),
        ("wgl", "opengl32.dll with a WGL pixel format and context"),
        ("d3d11", "d3d11.dll and dxgi.dll with a Direct3D 11 device"),
        (
            "wasapi",
            "WASAPI with default playback and recording endpoints",
        ),
        (
            "mediafoundation",
            "Media Foundation (mf, mfplat, mfreadwrite)",
        ),
        ("v4l2", "the V4L2 camera driver"),
        ("camera", "a camera (hardware tests)"),
        ("controller", "a connected game controller (hardware tests)"),
        (
            "desktop",
            "an interactive Windows desktop that can show windows",
        ),
    ];

    /// Whether `SDL3_TEST_REQUIRE` (`all`, or a comma-separated list of
    /// [`CAPABILITIES`]) makes `capability` mandatory.
    pub(crate) fn required(capability: &str) -> bool {
        let list = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
        required_by(&list, capability).unwrap_or_else(|e| panic!("{e}"))
    }

    fn required_by(list: &str, capability: &str) -> Result<bool, String> {
        let mut found = false;
        for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if name != "all" && !CAPABILITIES.iter().any(|&(c, _)| c == name) {
                return Err(format!(
                    "SDL3_TEST_REQUIRE names an unknown capability {name:?} (known: all, {})",
                    CAPABILITIES
                        .iter()
                        .map(|&(c, _)| c)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            found |= name == "all" || name == capability;
        }
        Ok(found)
    }

    /// The skips so far in this process: (test, capability, reason).
    pub(crate) static SKIPPED: std::sync::Mutex<Vec<(String, String, String)>> =
        std::sync::Mutex::new(Vec::new());

    /// Report that the calling test skips (some of) its checks because
    /// `capability` (one of [`CAPABILITIES`]) is missing here, and return so
    /// the caller can bail out. Prints a uniform note, records the skip in
    /// [`SKIPPED`] and, when `SDL3_TEST_SKIP_LOG` names a file, appends a
    /// tab-separated line to it. When `SDL3_TEST_REQUIRE` makes the
    /// capability mandatory, panics instead: the environment was meant to
    /// have it.
    pub(crate) fn skip(capability: &str, reason: impl std::fmt::Display) {
        assert!(
            CAPABILITIES.iter().any(|&(c, _)| c == capability),
            "unknown test capability {capability:?}"
        );
        let reason = reason.to_string();
        if required(capability) {
            panic!("required capability {capability} unavailable: {reason}");
        }
        let thread = std::thread::current();
        let test = thread.name().unwrap_or("?").to_string();
        eprintln!("note: skipping, {capability} unavailable: {reason}");
        if let Ok(path) = std::env::var("SDL3_TEST_SKIP_LOG") {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{capability}\t{test}\t{reason}");
            }
        }
        SKIPPED.lock().unwrap_or_else(|p| p.into_inner()).push((
            test,
            capability.to_string(),
            reason,
        ));
    }

    #[test]
    fn require_lists() {
        assert_eq!(required_by("", "x11"), Ok(false));
        assert_eq!(required_by("all", "x11"), Ok(true));
        assert_eq!(required_by("glx, x11", "x11"), Ok(true));
        assert_eq!(required_by("glx,xvfb,", "x11"), Ok(false));
        assert!(required_by("x11,nope", "x11").is_err());
        let mut names: Vec<_> = CAPABILITIES.iter().map(|&(c, _)| c).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CAPABILITIES.len());
    }
}
