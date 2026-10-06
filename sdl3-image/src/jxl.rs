// Rust translation of IMG_isJXL() from src/IMG_jxl.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! JPEG XL images: the detector. The decoder (libjxl) is not translated.

use sdl3::io::{IoStream, IoWhence};

use crate::util::read_ok;

/* See if an image is contained in a data source */

/// Whether `src` holds a JPEG XL codestream or container; the stream
/// position is unchanged. Translation of `IMG_isJXL()`.
pub fn is_jxl(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_jxl = false;
    let mut magic = [0u8; 12];
    if read_ok(src, &mut magic[..2]) {
        if magic[0] == 0xFF && magic[1] == 0x0A {
            /* This is a JXL codestream */
            is_jxl = true;
        } else if read_ok(src, &mut magic[2..])
            && magic
                == [
                    0x00, 0x00, 0x00, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A,
                ]
        {
            /* This is a JXL container */
            is_jxl = true;
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_jxl
}
