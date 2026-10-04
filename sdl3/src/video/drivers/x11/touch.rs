// Rust translation of src/video/x11/SDL_x11touch.c and SDL_x11touch.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Touch devices (through XInput2 multitouch).

use super::video::X11Video;

impl X11Video {
    /// Translation of `X11_InitTouch()`.
    pub(crate) fn x11_init_touch(&self) {
        self.x11_init_xinput2_multitouch();
    }

    /// Translation of `X11_QuitTouch()`.
    pub(crate) fn x11_quit_touch(&self) {
        crate::events::touch::quit_touch();
    }

    /// Translation of `X11_ResetTouch()`.
    pub(crate) fn x11_reset_touch(&self) {
        self.x11_quit_touch();
        self.x11_init_touch();
    }
}
