// Helpers shared by the translated files of SDL_image (no upstream
// counterpart: SDL's C library functions and macros they rely on).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use sdl3::error::{Error, Result};
use sdl3::io::IoStream;

/// Whether `buf.len()` bytes were read: `SDL_ReadIO(src, buf, len) == len`.
pub(crate) fn read_ok(src: &mut IoStream<'_>, buf: &mut [u8]) -> bool {
    src.read(buf) == buf.len()
}

/// Read one byte, `None` at the end of the stream or on an error.
pub(crate) fn read_byte(src: &mut IoStream<'_>) -> Option<u8> {
    let mut b = [0u8];
    read_ok(src, &mut b).then_some(b[0])
}

/// The error of a short read: what the stream reported, or (where upstream
/// leaves `SDL_GetError()` as it was, since a read at the end of the stream
/// sets no error) "End of stream".
pub(crate) fn read_error(src: &IoStream<'_>) -> Error {
    src.last_error()
        .cloned()
        .unwrap_or_else(|| Error::new("End of stream"))
}

/// The error of a short write: what the stream reported, or a generic one.
pub(crate) fn write_error(dst: &IoStream<'_>) -> Error {
    dst.last_error()
        .cloned()
        .unwrap_or_else(|| Error::new("Error writing to datastream"))
}

/// `SDL_SetError("%s", error)` and a failure.
pub(crate) fn error<T>(message: &'static str) -> Result<T> {
    Err(Error::new(message))
}

/// Translation of `SDL_isspace()`: the C locale's white space.
pub(crate) fn isspace(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Translation of `SDL_isdigit()`.
pub(crate) fn isdigit(c: u8) -> bool {
    c.is_ascii_digit()
}
