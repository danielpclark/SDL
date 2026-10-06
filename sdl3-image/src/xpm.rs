// Rust translation of IMG_isXPM() from src/IMG_xpm.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! XPM images: the detector. The decoder is not translated yet.

use sdl3::io::{IoStream, IoWhence};

use crate::util::read_ok;

/* See if an image is contained in a data source */

/// Whether `src` holds an XPM image (starting with `/* XPM */`); the
/// stream position is unchanged. Translation of `IMG_isXPM()`.
pub fn is_xpm(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_xpm = false;
    let mut magic = [0u8; 9];
    if read_ok(src, &mut magic) && &magic == b"/* XPM */" {
        is_xpm = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_xpm
}
