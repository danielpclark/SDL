// Rust translation of pngerror.c from libpng 1.6.59.
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2017 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngerror.c - stub functions for i/o and memory allocation
//!
//! This file provides a location for all error handling.  Users who
//! need special error handling are expected to write replacement functions
//! and use png_set_error_fn() to use those functions.  See the instructions
//! at each function.
//!
//! (SDL_image installs no error or warning functions, so libpng's default
//! ones run: they print the message to the standard error; the error's
//! `longjmp()` is returning [`PngError`].)

use super::png::{PngError, PngResult};
use super::pngpriv::*;
use super::pngstruct::PngStruct;

/// This function is called whenever there is a fatal error.  This function
/// should not be changed.  If there is a need to handle errors differently,
/// you should supply a replacement error function and use png_set_error_fn()
/// to replace the error function at run-time. (`png_error`: the error to
/// return)
pub(crate) fn png_error(png_ptr: &PngStruct<'_, '_>, error_message: &str) -> PngError {
    /* If the custom handler doesn't exist, or if it returns,
    use the default handler, which will not return. */
    png_default_error(png_ptr, error_message)
}

/// This function is called whenever there is a non-fatal error.  This function
/// should not be changed.  If there is a need to handle warnings differently,
/// you should supply a replacement warning function and use
/// png_set_error_fn() to replace the warning function at run-time.
/// (`png_warning`)
pub(crate) fn png_warning(png_ptr: &PngStruct<'_, '_>, warning_message: &str) {
    png_default_warning(png_ptr, warning_message);
}

/// `png_benign_error`
pub(crate) fn png_benign_error(png_ptr: &PngStruct<'_, '_>, error_message: &str) -> PngResult<()> {
    if (png_ptr.flags & PNG_FLAG_BENIGN_ERRORS_WARN) != 0 {
        if (png_ptr.mode & PNG_IS_READ_STRUCT) != 0 && png_ptr.chunk_name != 0 {
            png_chunk_warning(png_ptr, error_message);
        } else {
            png_warning(png_ptr, error_message);
        }
    } else if (png_ptr.mode & PNG_IS_READ_STRUCT) != 0 && png_ptr.chunk_name != 0 {
        return Err(png_chunk_error(png_ptr, error_message));
    } else {
        return Err(png_error(png_ptr, error_message));
    }
    Ok(())
}

/// `png_app_warning`
pub(crate) fn png_app_warning(png_ptr: &PngStruct<'_, '_>, error_message: &str) -> PngResult<()> {
    if (png_ptr.flags & PNG_FLAG_APP_WARNINGS_WARN) != 0 {
        png_warning(png_ptr, error_message);
        Ok(())
    } else {
        Err(png_error(png_ptr, error_message))
    }
}

/// `png_app_error`
pub(crate) fn png_app_error(png_ptr: &PngStruct<'_, '_>, error_message: &str) -> PngResult<()> {
    if (png_ptr.flags & PNG_FLAG_APP_ERRORS_WARN) != 0 {
        png_warning(png_ptr, error_message);
        Ok(())
    } else {
        Err(png_error(png_ptr, error_message))
    }
}

const PNG_MAX_ERROR_TEXT: usize = 196; /* Currently limited by profile_error in png.c */

/* These utilities are used internally to build an error message that relates
 * to the current chunk.  The chunk name comes from png_ptr->chunk_name,
 * which is used to prefix the message.  The message is limited in length
 * to 63 bytes. The name characters are output as hex digits wrapped in []
 * if the character is invalid.
 */
fn isnonalpha(c: u32) -> bool {
    !(65..=122).contains(&c) || (c > 90 && c < 97)
}

static PNG_DIGIT: [u8; 16] = *b"0123456789ABCDEF";

/// `png_format_buffer`
fn png_format_buffer(png_ptr: &PngStruct<'_, '_>, error_message: Option<&str>) -> String {
    let chunk_name = png_ptr.chunk_name;
    let mut buffer = Vec::new();
    let mut ishift: i32 = 24;

    while ishift >= 0 {
        let c = (chunk_name >> ishift) & 0xff;

        ishift -= 8;
        if isnonalpha(c) {
            buffer.push(b'[');
            buffer.push(PNG_DIGIT[((c & 0xf0) >> 4) as usize]);
            buffer.push(PNG_DIGIT[(c & 0x0f) as usize]);
            buffer.push(b']');
        } else {
            buffer.push(c as u8);
        }
    }

    if let Some(error_message) = error_message {
        buffer.push(b':');
        buffer.push(b' ');

        for &b in error_message.as_bytes().iter().take(PNG_MAX_ERROR_TEXT - 1) {
            buffer.push(b);
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

/// `png_chunk_error`: the error to return.
pub(crate) fn png_chunk_error(png_ptr: &PngStruct<'_, '_>, error_message: &str) -> PngError {
    let msg = png_format_buffer(png_ptr, Some(error_message));
    png_error(png_ptr, &msg)
}

/// `png_chunk_warning`
pub(crate) fn png_chunk_warning(png_ptr: &PngStruct<'_, '_>, warning_message: &str) {
    let msg = png_format_buffer(png_ptr, Some(warning_message));
    png_warning(png_ptr, &msg);
}

/// `png_chunk_benign_error`
pub(crate) fn png_chunk_benign_error(
    png_ptr: &PngStruct<'_, '_>,
    error_message: &str,
) -> PngResult<()> {
    if (png_ptr.flags & PNG_FLAG_BENIGN_ERRORS_WARN) != 0 {
        png_chunk_warning(png_ptr, error_message);
        Ok(())
    } else {
        Err(png_chunk_error(png_ptr, error_message))
    }
}

/// This is the default error handling function.  Note that replacements for
/// this function MUST NOT RETURN, or the program will likely crash.  This
/// function is used by default, or if the program supplies NULL for the
/// error function pointer in png_set_error_fn(). (`png_default_error`,
/// with `png_longjmp()`: the error to return)
fn png_default_error(_png_ptr: &PngStruct<'_, '_>, error_message: &str) -> PngError {
    eprintln!("libpng error: {error_message}");
    PngError
}

/// This function is called when there is a warning, but the library thinks
/// it can continue anyway.  Replacement functions don't have to do anything
/// here if you don't want them to.  In the default configuration, png_ptr is
/// not used, but it is passed in case it may be useful.
/// (`png_default_warning`)
fn png_default_warning(_png_ptr: &PngStruct<'_, '_>, warning_message: &str) {
    eprintln!("libpng warning: {warning_message}");
}
