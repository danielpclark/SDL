// Rust translation of src/diag.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2021 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Diagnostics: the detailed message of the first error of an API call.
//! SDL_image only reports `avifResultToString()`, but the messages are
//! kept as upstream writes them.

use std::cell::RefCell;
use std::fmt;

use super::avif::AVIF_DIAGNOSTICS_ERROR_BUFFER_SIZE;

/// Translation of `avifDiagnostics` (the error buffer is a string behind
/// a `RefCell`, so that the parsers can share it as upstream shares the
/// pointer).
#[derive(Debug, Default)]
pub(crate) struct AvifDiagnostics {
    /// Upon receiving an error from any non-const libavif API call, if the toplevel structure used
    /// in the API call (avifDecoder, avifEncoder) contains a diag member, this buffer may be
    /// populated with a NULL-terminated, freeform error string explaining the first encountered error in
    /// more detail. It will be cleared at the beginning of every non-const API call.
    ///
    /// Note: If an error string contains the "[Strict]" prefix, it means that you encountered an
    /// error that only occurs during strict decoding. If you disable strict mode, you will no
    /// longer encounter this error.
    pub(crate) error: RefCell<String>,
}

/// Translation of `avifDiagnosticsClearError()`.
pub(crate) fn clear_error(diag: Option<&AvifDiagnostics>) {
    if let Some(diag) = diag {
        diag.error.borrow_mut().clear();
    }
}

/// Translation of `avifDiagnosticsPrintf()`.
pub(crate) fn printf(diag: Option<&AvifDiagnostics>, args: fmt::Arguments<'_>) {
    let Some(diag) = diag else {
        // It is possible this is NULL (e.g. calls to avifPeekCompatibleFileType())
        return;
    };
    let mut error = diag.error.borrow_mut();
    if !error.is_empty() {
        // There is already a detailed error set.
        return;
    }

    *error = args.to_string();
    // (vsnprintf() into the buffer, NUL-terminated)
    if error.len() > AVIF_DIAGNOSTICS_ERROR_BUFFER_SIZE - 1 {
        let mut end = AVIF_DIAGNOSTICS_ERROR_BUFFER_SIZE - 1;
        while !error.is_char_boundary(end) {
            end -= 1;
        }
        error.truncate(end);
    }
}

/// The text of a four-character code as `%.4s` prints it (up to a NUL).
pub(crate) fn fourcc(t: &[u8]) -> String {
    let t = &t[..t.len().min(4)];
    let end = t.iter().position(|&b| b == 0).unwrap_or(t.len());
    String::from_utf8_lossy(&t[..end]).into_owned()
}
