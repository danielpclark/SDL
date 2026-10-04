// Rust translation of src/core/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Per-platform glue the backends share (`src/core/<platform>/`).

// Parts are used only by the backends that need them.
#![allow(dead_code)]

/// Where upstream builds with `SDL_USE_LIBDBUS` (and the other
/// `core/linux/` pieces).
#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
pub(crate) mod linux;
#[cfg(unix)]
pub(crate) mod unix;
#[cfg(windows)]
pub(crate) mod windows;
