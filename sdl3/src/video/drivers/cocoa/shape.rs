// Rust translation of src/video/cocoa/SDL_cocoashape.m and
// SDL_cocoashape.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Window shapes: Cocoa windows are transparent where they're drawn
//! transparent, so the shape only decides whether the window takes the
//! mouse where the pointer is.

use super::window::window_data;
use crate::core::apple::objc::{msg_send, Bool, NO, YES};
use crate::error::Result;
use crate::events::WindowID;
use crate::video::Surface;

/// Translation of `Cocoa_UpdateWindowShape()`.
pub(crate) fn update_window_shape(
    window: WindowID,
    shape: Option<&Surface<'static>>,
) -> Result<()> {
    let Some(data) = window_data(window) else {
        return Ok(());
    };
    let mut ignores_mouse_events: Bool = NO;

    if let Some(shape) = shape {
        let (gx, gy, _) = crate::events::mouse::global_mouse_state();
        let (wx, wy, ww, wh) =
            crate::video::core::with_window(window, |w| (w.core.x, w.core.y, w.core.w, w.core.h))?;
        let px = gx - wx as f32;
        let py = gy - wy as f32;
        if px >= 0.0 && px < ww as f32 && py >= 0.0 && py < wh as f32 {
            let x = ((px / (ww - 1) as f32) * (shape.width() - 1) as f32).round() as i32;
            let y = ((py / (wh - 1) as f32) * (shape.height() - 1) as f32).round() as i32;

            match shape.read_pixel(x, y) {
                Ok(c) if c.a != 0 => {}
                _ => ignores_mouse_events = YES,
            }
        }
    }
    // SAFETY: an NSWindow.
    let _: () = unsafe { msg_send![data.nswindow(), setIgnoresMouseEvents: ignores_mouse_events] };
    Ok(())
}
