// Rust translation of libtiff/tif_error.c and libtiff/tif_warning.c from
// libtiff (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's
// external/libtiff pins it), with the default handlers of tif_unix.c and
// tif_win32.c.
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Error and warning reporting. SDL_image installs no handler, so the
//! messages go to libtiff's default ones (`unixErrorHandler()` and
//! `unixWarningHandler()`, the same as Windows' `Win32ErrorHandler()` and
//! `Win32WarningHandler()`), which print them to the standard error. The
//! last error's text is also kept for the thread (see [`take_last_error`]),
//! to name the failure in the loader's `Err` where upstream leaves
//! `SDL_GetError()` as it was.

use std::cell::RefCell;
use std::fmt;
use std::io::Write;

thread_local! {
    static LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Write to the standard error, ignoring failures (and in tests through
/// `eprint!`, which the test harness captures).
fn emit(s: &str) {
    if cfg!(test) {
        eprint!("{s}");
    } else {
        let _ = std::io::stderr().write_all(s.as_bytes());
    }
}

/// Translation of `unixErrorHandler()` (as `TIFFErrorExtR()` calls it).
pub(crate) fn tiff_error_ext_r_impl(module: &str, args: fmt::Arguments<'_>) {
    let message = format!("{module}: {args}");
    emit(&format!("{message}.\n"));
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(message));
}

/// Translation of `unixWarningHandler()` (as `TIFFWarningExtR()` calls it).
pub(crate) fn tiff_warning_ext_r_impl(module: &str, args: fmt::Arguments<'_>) {
    emit(&format!("{module}: Warning, {args}.\n"));
}

/// The text of the last error reported on this thread, cleared.
pub(crate) fn take_last_error() -> Option<String> {
    LAST_ERROR.with(|e| e.borrow_mut().take())
}

/// Translation of `TIFFErrorExtR(tif, module, fmt, ...)` (the handle,
/// which has no error handler of its own, is left out).
macro_rules! tiff_error_ext_r {
    ($module:expr, $($arg:tt)*) => {
        $crate::tif::tif_error::tiff_error_ext_r_impl(&$module, format_args!($($arg)*))
    };
}

/// Translation of `TIFFWarningExtR(tif, module, fmt, ...)` (the handle,
/// which has no warning handler of its own, is left out).
macro_rules! tiff_warning_ext_r {
    ($module:expr, $($arg:tt)*) => {
        $crate::tif::tif_error::tiff_warning_ext_r_impl(&$module, format_args!($($arg)*))
    };
}

pub(crate) use tiff_error_ext_r;
pub(crate) use tiff_warning_ext_r;
