// Rust translation of src/dialog/unix/SDL_unixdialog.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Picks the Unix file dialog backend: the XDG desktop portal, else zenity
//! (or the one `SDL_HINT_FILE_DIALOG_DRIVER` names).

use std::sync::Mutex;

use super::{DialogFileCallback, FileDialogOptions, FileDialogType};
use crate::error::{Error, Result};
use crate::hints;

mod portaldialog;
mod zenitydialog;
mod zenitymessagebox;

pub(crate) use zenitymessagebox::zenity_show_message_box;

type ShowFn = fn(FileDialogType, DialogFileCallback, FileDialogOptions);

/// Translation of `detected_function`.
static DETECTED_FUNCTION: Mutex<Option<ShowFn>> = Mutex::new(None);
static HINT_WATCH: Mutex<Option<hints::Callback>> = Mutex::new(None);

/// Translation of `set_callback()`.
fn set_callback() {
    let mut watch = HINT_WATCH.lock().unwrap_or_else(|e| e.into_inner());
    if watch.is_none() {
        // (translation of `hint_callback()`; the first call happens now,
        // with the current value)
        *watch = hints::watch(hints::FILE_DIALOG_DRIVER, |change| {
            let _ = detect_available_methods(change.new_value);
        })
        .ok();
    }
}

/// Translation of `detect_available_methods()`.
fn detect_available_methods(value: Option<&str>) -> Result<()> {
    let driver = match value {
        Some(v) => Some(v.to_string()),
        None => hints::get(hints::FILE_DIALOG_DRIVER),
    };

    // (not from inside the hint callback that set_callback() installs)
    if HINT_WATCH.try_lock().is_ok() {
        set_callback();
    }

    if (driver.is_none() || driver.as_deref() == Some("portal")) && portaldialog::portal_detect() {
        *DETECTED_FUNCTION.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(portaldialog::portal_show_file_dialog);
        return Ok(());
    }

    if (driver.is_none() || driver.as_deref() == Some("zenity")) && zenitydialog::zenity_detect() {
        *DETECTED_FUNCTION.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(zenitydialog::zenity_show_file_dialog);
        return Ok(());
    }

    Err(Error::new(
        "File dialog driver unsupported (supported values for SDL_HINT_FILE_DIALOG_DRIVER are 'zenity' and 'portal')",
    ))
}

/// Translation of `SDL_SYS_ShowFileDialogWithProperties()` (Unix).
pub(super) fn show_file_dialog(
    dialog_type: FileDialogType,
    callback: DialogFileCallback,
    options: FileDialogOptions,
) {
    // Call detect_available_methods() again each time in case the situation changed
    let detected = *DETECTED_FUNCTION.lock().unwrap_or_else(|e| e.into_inner());
    let function = match detected {
        Some(function) => function,
        None => match detect_available_methods(None) {
            Ok(()) => match *DETECTED_FUNCTION.lock().unwrap_or_else(|e| e.into_inner()) {
                Some(function) => function,
                None => return callback(Err(Error::new("File dialog driver unsupported")), None),
            },
            // SetError() done by detect_available_methods()
            Err(e) => return callback(Err(e), None),
        },
    };

    function(dialog_type, callback, options);
}
