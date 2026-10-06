// Rust translation of IMG_isSVG() from src/IMG_svg.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SVG images: the detector. The decoder (nanosvg) is not translated yet.

use sdl3::io::{IoStream, IoWhence};

/* See if an image is contained in a data source */

/// Whether `src` looks like an SVG image: `<svg` within its first 4095
/// bytes (before any NUL byte, as upstream searches a C string); the stream
/// position is unchanged. Translation of `IMG_isSVG()`.
pub fn is_svg(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_svg = false;
    let mut magic = [0u8; 4096];
    let magic_len = src.read(&mut magic[..4096 - 1]);
    if magic_len > 0 {
        let text = &magic[..magic_len];
        let text = &text[..text.iter().position(|&b| b == 0).unwrap_or(text.len())];
        if text.windows(4).any(|w| w == b"<svg") {
            is_svg = true;
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_svg
}
