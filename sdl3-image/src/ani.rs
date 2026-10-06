// Rust translation of IMG_isANI() from src/IMG_ani.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Windows animated cursors (`.ani`): the detector. The ANI decoder and
//! encoder are part of the animation API, not translated yet.

use sdl3::io::{IoStream, IoWhence};

/// Translation of `RIFF_FOURCC()`.
const fn riff_fourcc(c0: u8, c1: u8, c2: u8, c3: u8) -> u32 {
    c0 as u32 | (c1 as u32) << 8 | (c2 as u32) << 16 | (c3 as u32) << 24
}

/// Whether `src` holds a Windows animated cursor (a RIFF `ACON` file); the
/// stream position is unchanged. Translation of `IMG_isANI()`.
pub fn is_ani(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_ani = false;
    // RIFFHEADER: riffID, cbSize, chunkID
    if let (Ok(riff_id), Ok(_cb_size), Ok(chunk_id)) =
        (src.read_u32_le(), src.read_u32_le(), src.read_u32_le())
    {
        if riff_id == riff_fourcc(b'R', b'I', b'F', b'F')
            && chunk_id == riff_fourcc(b'A', b'C', b'O', b'N')
        {
            is_ani = true;
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_ani
}
