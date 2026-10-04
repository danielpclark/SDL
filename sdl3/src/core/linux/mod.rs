// Rust translation of src/core/linux/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Linux (and other free Unix) glue shared by the backends.

pub(crate) mod dbus;
#[cfg(target_os = "linux")]
pub(crate) mod threadprio;
