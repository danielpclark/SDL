// Rust translation of src/video/wayland/SDL_waylandmessagebox.c and
// SDL_waylandmessagebox.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Message boxes, shown with zenity.

use crate::error::{Error, Result};
use crate::video::messagebox::MessageBoxData;

/// Translation of `Wayland_ShowMessageBox()` (the bootstrap's
/// `ShowMessageBox`): the ID of the button chosen, or -1 if none was (C
/// leaves the caller's `buttonID` alone then).
pub(crate) fn wayland_show_message_box(messageboxdata: &MessageBoxData) -> Result<i32> {
    // Are we trying to connect to or are currently in a Wayland session?
    if crate::stdlib::getenv("WAYLAND_DISPLAY").is_none() {
        let session = crate::stdlib::getenv("XDG_SESSION_TYPE");
        if session.is_some_and(|s| !s.eq_ignore_ascii_case("wayland")) {
            return Err(Error::new("Not on a wayland display"));
        }
    }

    show_zenity(messageboxdata)
}

#[cfg(not(any(target_os = "android", target_os = "haiku")))]
fn show_zenity(messageboxdata: &MessageBoxData) -> Result<i32> {
    let mut button_id = -1;
    crate::dialog::zenity_show_message_box(messageboxdata, &mut button_id)?;
    Ok(button_id)
}

#[cfg(any(target_os = "android", target_os = "haiku"))]
fn show_zenity(_messageboxdata: &MessageBoxData) -> Result<i32> {
    Err(Error::unsupported())
}
