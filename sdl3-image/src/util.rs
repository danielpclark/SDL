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

/// `SDL_malloc(len)` and `SDL_ReadIO(src, buf, len)`: the bytes read, up
/// to `len` (fewer at the end of the stream), or `None` when `len` bytes
/// can't be allocated (out of memory). The buffer grows only as data
/// arrives, so a corrupt length costs no more memory than the stream has.
pub(crate) fn read_up_to(src: &mut IoStream<'_>, len: usize) -> Option<Vec<u8>> {
    const CHUNK: usize = 64 * 1024;
    let mut buf = Vec::new();
    buf.try_reserve_exact(len).ok()?;
    while buf.len() < len {
        let old = buf.len();
        let chunk = CHUNK.min(len - old);
        buf.resize(old + chunk, 0);
        let n = src.read(&mut buf[old..]);
        buf.truncate(old + n);
        if n < chunk {
            break;
        }
    }
    Some(buf)
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

/// `SDL_sscanf(text, "%d %d ...", ...)` (glibc's, which SDL calls on the
/// platforms upstream's test runs on) for `out.len()` numbers: the count
/// converted, each `long` truncated to `int`. `text` is a C string: it
/// ends at a NUL.
pub(crate) fn scan_ints(text: &[u8], out: &mut [i32]) -> usize {
    let mut rest = &text[..text.iter().position(|&b| b == 0).unwrap_or(text.len())];
    for (count, value) in out.iter_mut().enumerate() {
        while let Some((&c, tail)) = rest.split_first() {
            if !isspace(c) {
                break;
            }
            rest = tail;
        }
        let (v, advance) = sdl3::stdlib::string::strtol(rest, 10);
        if advance == 0 {
            return count;
        }
        *value = v as i32;
        rest = &rest[advance..];
    }
    out.len()
}

/// C's `(int)f`: truncation, and for a value out of range (undefined in C)
/// `INT_MIN`, as x86's conversion instruction gives.
pub(crate) fn c_f32_to_i32(f: f32) -> i32 {
    if f.is_nan() || f >= 2147483648.0 || f < -2147483648.0 {
        i32::MIN
    } else {
        f as i32
    }
}

/// C's `(unsigned int)f` as x86-64 compilers make it: a conversion to a
/// 64-bit integer (`INT64_MIN` out of its range), truncated to 32 bits.
pub(crate) fn c_f32_to_u32(f: f32) -> u32 {
    if f.is_nan() || f >= 9223372036854775808.0 || f < -9223372036854775808.0 {
        0
    } else {
        f as i64 as u32
    }
}

/// Translation of `SDL_isdigit()`.
pub(crate) fn isdigit(c: u8) -> bool {
    c.is_ascii_digit()
}
