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
//! **no third-party dependencies**. See `docs/ROADMAP.md` in the repository
//! for what is translated so far and the plan for the rest of SDL and its
//! satellite libraries.

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]

pub mod atomic;
pub mod error;
pub mod guid;
pub mod hints;
pub mod log;
pub mod power;
pub mod properties;
pub mod stdlib;
mod thread;
pub mod time;
pub mod timer;
pub mod utils;
pub mod version;
pub mod video;

pub use error::{Error, ErrorKind, Result};
pub use guid::Guid;
pub use version::{revision, version, Version};

/// Shut down the translated subsystems that keep global state (timers,
/// logging, hints, the global property group, the tick counter).
///
/// This is the part of `SDL_Quit()` that applies to what has been translated
/// so far; it will grow into the full `SDL_Quit()` as `SDL.c` is translated.
/// Calling it is optional: everything re-initializes lazily on next use.
pub fn shutdown() {
    timer::quit_timers();
    log::quit_log();
    hints::quit_hints();
    properties::quit_properties();
    timer::quit_ticks();
}
