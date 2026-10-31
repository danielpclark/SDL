// Rust translation of src/video/android/SDL_androidmessagebox.c and
// SDL_androidmessagebox.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Message boxes: the activity's `AlertDialog`, shown on its main thread
//! while the calling thread waits (`core::android::show_message_box()`).
//! The bootstrap's, so it works before video is initialized.

use crate::error::Result;
use crate::video::messagebox::MessageBoxData;

/// Translation of `Android_ShowMessageBox()`.
pub(crate) fn show_message_box(messageboxdata: &MessageBoxData) -> Result<i32> {
    crate::core::android::show_message_box(messageboxdata)
}
