// Rust translation of src/video/x11/SDL_x11xtest.c and SDL_x11xtest.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Warping the mouse with the XTEST extension (disabled upstream, since it
//! doesn't appear to work on XWayland).

use std::sync::atomic::{AtomicBool, Ordering};

use super::modes::{display_driver_data, display_driver_data_for_window};
use super::sys::*;
use super::video::X11Video;
use crate::events::WindowID;
use crate::stdlib::math::roundf;
use crate::video::core::with_window;
use crate::video::display::primary_display;

static XTEST_INITIALIZED: AtomicBool = AtomicBool::new(false);

impl X11Video {
    /// Translation of `X11_InitXTest()`.
    pub(crate) fn x11_init_xtest(&self) {
        // This is currently disabled since it doesn't appear to work on XWayland
        // (#if 0: the XTEST query is not compiled upstream)
    }

    /// Translation of `X11_WarpMouseXTest()`.
    pub(crate) fn x11_warp_mouse_xtest(&self, window: Option<WindowID>, x: f32, y: f32) -> bool {
        if !x11_xtest_is_initialized() {
            return false;
        }
        let Some(xtest) = &self.x.xtest else {
            return false;
        };

        let display = self.display;
        let displaydata = match window {
            Some(window) => display_driver_data_for_window(window),
            Option::None => primary_display().ok().and_then(display_driver_data),
        };
        let Some(displaydata) = displaydata else {
            return false;
        };

        let mut motion_x = roundf(x) as i32;
        let mut motion_y = roundf(y) as i32;
        if let Some(window) = window {
            if let Ok((wx, wy)) = with_window(window, |w| (w.core.x, w.core.y)) {
                motion_x += wx;
                motion_y += wy;
            }
        }

        // SAFETY: the display is open and the screen one of its screens.
        unsafe {
            if (xtest.XTestFakeMotionEvent)(
                display,
                displaydata.screen,
                motion_x,
                motion_y,
                CurrentTime,
            ) == 0
            {
                return false;
            }
            (self.x.XSync)(display, False);
        }

        true
    }
}

/// Translation of `X11_XTestIsInitialized()`.
pub(crate) fn x11_xtest_is_initialized() -> bool {
    XTEST_INITIALIZED.load(Ordering::Relaxed)
}
