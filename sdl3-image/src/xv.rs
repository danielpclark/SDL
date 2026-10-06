// Rust translation of get_line(), get_header() and IMG_isXV() from
// src/IMG_xv.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a XV thumbnail image file loading framework
//!
//! The detector; the decoder is not translated yet.

use sdl3::io::{IoStream, IoWhence};
use sdl3::stdlib::string::strtol;

use crate::util::{isspace, read_byte};

/// Read a line of at most `size` bytes (carriage returns dropped) into
/// `line`, without the newline. Translation of `get_line()` (`None` for
/// -1: a read error, or no newline within `size` bytes).
fn get_line(src: &mut IoStream<'_>, line: &mut Vec<u8>, mut size: usize) -> Option<()> {
    line.clear();
    while size > 0 {
        let c = read_byte(src)?;
        if c == b'\r' {
            continue;
        }
        if c == b'\n' {
            return Some(());
        }
        line.push(c);
        size -= 1;
    }
    /* Out of space for the line */
    None
}

/// `SDL_sscanf(line, "%d %d", w, h)`: as many of the two numbers as parse.
fn scan_two_ints(line: &[u8], w: &mut i32, h: &mut i32) {
    // (the line is a C string: it ends at a NUL)
    let line = &line[..line.iter().position(|&b| b == 0).unwrap_or(line.len())];
    let (value, advance) = strtol(line, 10);
    if advance == 0 {
        return;
    }
    *w = value as i32;
    let mut rest = &line[advance..];
    while let Some((&c, tail)) = rest.split_first() {
        if !isspace(c) {
            break;
        }
        rest = tail;
    }
    let (value, advance) = strtol(rest, 10);
    if advance != 0 {
        *h = value as i32;
    }
}

/// Read the thumbnail header: the image's `(width, height)`. Translation
/// of `get_header()` (`None` for -1).
fn get_header(src: &mut IoStream<'_>) -> Option<(i32, i32)> {
    let mut line = Vec::with_capacity(1024);
    let mut w = 0;
    let mut h = 0;

    /* Check the header magic */
    get_line(src, &mut line, 1024)?;
    if line.len() < 6 || &line[..6] != b"P7 332" {
        return None;
    }

    /* Read the header */
    while get_line(src, &mut line, 1024).is_some() {
        if line.starts_with(b"#BUILTIN:") {
            /* Builtin image, no data */
            break;
        }
        if line.starts_with(b"#END_OF_COMMENTS") {
            if get_line(src, &mut line, 1024).is_some() {
                scan_two_ints(&line, &mut w, &mut h);
                if w >= 0 && h >= 0 {
                    return Some((w, h));
                }
            }
            break;
        }
    }
    /* No image data */
    None
}

/* See if an image is contained in a data source */

/// Whether `src` holds an XV thumbnail (`P7 332`); the stream position is
/// unchanged. Translation of `IMG_isXV()`.
pub fn is_xv(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_xv = false;
    if get_header(src).is_some() {
        is_xv = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_xv
}
