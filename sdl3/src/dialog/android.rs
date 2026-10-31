// Rust translation of src/dialog/android/SDL_androiddialog.c from Simple
// DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The file dialogs on Android: the system's documents' picker, which the
//! activity starts (`core::android::show_file_dialog()`), its answer the
//! chosen documents' `content://` URIs (none when cancelled), with no
//! filter (`None`), on the activity's main thread. Only one at a time.
//! The filters are the picker's MIME types; the title and the buttons'
//! labels aren't used.

use super::{DialogFileCallback, FileDialogOptions, FileDialogType};
use crate::error::Error;
use crate::hints;

/// Translation of `SDL_SYS_ShowFileDialogWithProperties()`.
pub(super) fn show_file_dialog(
    dialog_type: FileDialogType,
    callback: DialogFileCallback,
    options: FileDialogOptions,
) {
    // (no filters: upstream's null pointer)
    let filters = (!options.filters.is_empty()).then_some(options.filters.as_slice());
    let allow_many = options.many;
    let base_folder = options.location.as_deref();

    if hints::get(hints::FILE_DIALOG_DRIVER).is_some() {
        callback(
            Err(Error::new(
                "File dialog driver unsupported (don't set SDL_HINT_FILE_DIALOG_DRIVER)",
            )),
            None,
        );
        return;
    }

    if let Err((callback, error)) = crate::core::android::show_file_dialog(
        callback,
        filters,
        dialog_type,
        allow_many,
        base_folder,
    ) {
        // SDL_SetError is already called when it fails
        callback(Err(error), None);
    }
}
