// Rust translation of src/core/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Per-platform glue the backends share (`src/core/<platform>/`).

// Parts are used only by the backends that need them.
#![allow(dead_code)]

/// Where upstream builds with `SDL_USE_LIBDBUS` (and the other
/// `core/linux/` pieces).
/// The Objective-C runtime and the system frameworks the Apple backends
/// share.
#[cfg(target_vendor = "apple")]
pub(crate) mod apple;
/// SDL.c's Emscripten parts (`Module['SDL3']`, the JavaScript-callable
/// functions, `SDL_ExitProcess()`).
#[cfg(target_os = "emscripten")]
pub(crate) mod emscripten;
#[cfg(all(
    unix,
    not(target_vendor = "apple"),
    not(target_os = "android"),
    not(target_os = "emscripten")
))]
pub(crate) mod linux;
// (CMake's Android branch doesn't build src/core/unix/)
#[cfg(all(unix, not(target_os = "android")))]
pub(crate) mod unix;
#[cfg(windows)]
pub(crate) mod windows;
