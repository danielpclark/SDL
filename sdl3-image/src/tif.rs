// Rust translation of IMG_isTIF() from src/IMG_tif.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! TIFF images: the detector. The decoder (libtiff) is not translated.

use sdl3::io::{IoStream, IoWhence};

use crate::util::read_ok;

/// Whether `src` holds a TIFF image (little- or big-endian); the stream
/// position is unchanged. Translation of `IMG_isTIF()`.
pub fn is_tif(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_tif = false;
    let mut magic = [0u8; 4];
    if read_ok(src, &mut magic)
        && ((magic[0] == b'I' && magic[1] == b'I' && magic[2] == 0x2a && magic[3] == 0x00)
            || (magic[0] == b'M' && magic[1] == b'M' && magic[2] == 0x00 && magic[3] == 0x2a))
    {
        is_tif = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_tif
}
