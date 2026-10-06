// Rust translation of webp_getinfo() and IMG_isWEBP() from src/IMG_webp.c
// from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebP images: the detector. The decoder and encoder (libwebp) are not
//! translated.

use sdl3::io::{IoStream, IoWhence};

use crate::util::read_ok;

/// Whether `src` holds a WebP image, and if so (with `datasize`) the size
/// of the data from the stream position to its end. Translation of
/// `webp_getinfo()`.
fn webp_getinfo(src: &mut IoStream<'_>, datasize: Option<&mut usize>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_webp = false;
    let mut magic = [0u8; 20];
    if read_ok(src, &mut magic)
        && &magic[0..4] == b"RIFF"
        && &magic[8..12] == b"WEBP"
        && &magic[12..15] == b"VP8"
        && (magic[15] == b' ' || magic[15] == b'X' || magic[15] == b'L')
    {
        is_webp = true;
        if let Some(datasize) = datasize {
            let size = src.size().unwrap_or(-1);
            if size > 0 {
                *datasize = (size - start) as usize;
            } else {
                *datasize = 0;
            }
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_webp
}

/* See if an image is contained in a data source */

/// Whether `src` holds a WebP image; the stream position is unchanged.
/// Translation of `IMG_isWEBP()`.
pub fn is_webp(src: &mut IoStream<'_>) -> bool {
    webp_getinfo(src, None)
}
