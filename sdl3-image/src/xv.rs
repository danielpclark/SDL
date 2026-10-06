// Rust translation of src/IMG_xv.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a XV thumbnail image file loading framework

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::stdlib::string::strtol;
use sdl3::video::{PixelFormat, Surface};

use crate::util::{isspace, read_byte, read_ok};

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

/// Load an XV thumbnail: 3-3-2 RGB pixels, as an RGB332 surface.
/// Translation of `IMG_LoadXV_IO()`.
pub fn load_xv_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);
    load_xv(src).map_err(|error| {
        let _ = src.seek(start, IoWhence::Set);
        Error::new(error)
    })
}

fn load_xv(src: &mut IoStream<'_>) -> std::result::Result<Surface<'static>, &'static str> {
    /* Read the header */
    let Some((w, h)) = get_header(src) else {
        return Err("Unsupported image format");
    };

    /* Create the 3-3-2 indexed palette surface */
    let Ok(mut surface) = Surface::new(w, h, PixelFormat::RGB332) else {
        return Err("Out of memory");
    };

    /* Load the image data */
    let pitch = surface.pitch() as usize;
    let pixels: &mut [u8] = surface.pixels_mut().unwrap_or(&mut []);
    // (the rows of a surface of zero width read nothing, and always succeed)
    let rows = if w > 0 { h as usize } else { 0 };
    for y in 0..rows {
        let row = &mut pixels[y * pitch..y * pitch + w as usize];
        if !read_ok(src, row) {
            return Err("Couldn't read image data");
        }
    }
    Ok(surface)
}
