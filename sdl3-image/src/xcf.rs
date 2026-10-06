// Rust translation of IMG_isXCF() from src/IMG_xcf.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! GIMP images (`.xcf`): the detector. The decoder is not translated yet.

use sdl3::io::{IoStream, IoWhence};

use crate::util::read_ok;

/* See if an image is contained in a data source */

/// Whether `src` holds a GIMP image; the stream position is unchanged.
/// Translation of `IMG_isXCF()`.
pub fn is_xcf(src: &mut IoStream<'_>) -> bool {
    let mut is_xcf = false;
    let mut magic = [0u8; 14];

    let start = src.tell().unwrap_or(-1);
    if read_ok(src, &mut magic) && &magic[..9] == b"gimp xcf " {
        is_xcf = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_xcf
}
