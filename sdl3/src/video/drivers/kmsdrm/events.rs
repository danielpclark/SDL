// Rust translation of src/video/kmsdrm/SDL_kmsdrmevents.c and
// SDL_kmsdrmevents.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The event pump: the console input reader's (`SDL_INPUT_LINUXEV`; the
//! wscons reader of the BSDs isn't translated), whose VT switches call the
//! driver back (see [`super::video`]).

/// Translation of `KMSDRM_PumpEvents()`.
pub(crate) fn kmsdrm_pump_events() {
    crate::core::linux::evdev::poll();
}
