// Rust translation of src/video/android/SDL_androidclipboard.c and
// SDL_androidclipboard.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The clipboard: the system's primary clip, its text set, read and
//! tested through the activity's clipboard handler
//! (`core::android::java::clipboard_handler`). Upstream offers text only
//! (the clipboard front end keeps the app's other data to itself); the
//! clip's changes made other than through SDL come as clipboard updates the
//! app doesn't own, with no MIME types (`onNativeClipboardChanged()`).

use crate::error::{Error, Result};

/// Translation of `Android_SetClipboardText()`.
pub(crate) fn set_clipboard_text(text: &str) -> Result<()> {
    if crate::core::android::set_clipboard_text(text) {
        Ok(())
    } else {
        Err(Error::new("Couldn't set the clipboard's text"))
    }
}

/// Translation of `Android_GetClipboardText()`.
pub(crate) fn get_clipboard_text() -> String {
    crate::core::android::get_clipboard_text()
}

/// Translation of `Android_HasClipboardText()`.
pub(crate) fn has_clipboard_text() -> bool {
    crate::core::android::has_clipboard_text()
}
