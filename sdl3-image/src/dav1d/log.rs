// Rust translation of src/log.c and src/log.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Logging through the context's logger callback.

use std::sync::Arc;

use super::internal::{Dav1dContext, Dav1dLogger};

/// `dav1d_log_default_callback()`: print to stderr.
pub(crate) fn log_default_callback() -> Dav1dLogger {
    Some(Arc::new(|msg: &str| eprint!("{msg}")))
}

/// `dav1d_log()`
pub(crate) fn dav1d_log(c: &Dav1dContext, msg: &str) {
    if let Some(cb) = &c.logger {
        cb(msg);
    }
}
