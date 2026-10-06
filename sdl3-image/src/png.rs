// Rust translation of src/IMG_png.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a PNG image file loading framework
//!
//! As upstream built without libpng (or WIC, or ImageIO), loading and saving
//! go through SDL's own PNG support: `SDL_LoadPNG_IO()` (stb_image) and
//! `SDL_SavePNG_IO()` (miniz), translated in the `sdl3` crate.

use std::path::Path;

use sdl3::error::Result;
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::Surface;

use crate::img::verify_can_save_surface;
use crate::util::read_ok;

/* See if an image is contained in a data source */

/// Whether `src` holds a PNG image; the stream position is unchanged.
/// Translation of `IMG_isPNG()`.
pub fn is_png(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_png = false;
    let mut magic = [0u8; 4];
    if read_ok(src, &mut magic) && magic[0] == 0x89 && &magic[1..4] == b"PNG" {
        is_png = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_png
}

/// Load a PNG image. Translation of `IMG_LoadPNG_IO()`.
pub fn load_png_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    Surface::load_png_io(src)
}

/// Save a surface in PNG format: indexed surfaces as paletted images,
/// others as RGBA. Translation of `IMG_SavePNG_IO()`.
pub fn save_png_io(surface: &mut Surface<'_>, dst: &mut IoStream<'_>) -> Result<()> {
    verify_can_save_surface(surface)?;
    surface.save_png_io(dst)
}

/// Save a surface to a PNG file. Translation of `IMG_SavePNG()`.
pub fn save_png(surface: &mut Surface<'_>, file: impl AsRef<Path>) -> Result<()> {
    verify_can_save_surface(surface)?;
    let mut dst = IoStream::from_file(file, "wb")?;
    let result = save_png_io(surface, &mut dst);
    let closed = dst.close();
    result.and(closed)
}
