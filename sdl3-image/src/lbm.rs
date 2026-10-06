// Rust translation of IMG_isLBM() from src/IMG_lbm.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! ILBM (Amiga IFF) images: the detector. The decoder is not translated
//! yet.

use sdl3::io::{IoStream, IoWhence};

use crate::util::read_ok;

/// Whether `src` holds an IFF `PBM ` or `ILBM` picture; the stream
/// position is unchanged. Translation of `IMG_isLBM()`.
pub fn is_lbm(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_lbm = false;
    let mut magic = [0u8; 4 + 4 + 4];
    if read_ok(src, &mut magic)
        && &magic[..4] == b"FORM"
        && (&magic[8..12] == b"PBM " || &magic[8..12] == b"ILBM")
    {
        is_lbm = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_lbm
}
