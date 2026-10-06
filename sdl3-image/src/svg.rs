// Rust translation of src/IMG_svg.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is an SVG image file loading framework, based on Nano SVG:
//! <https://github.com/memononen/nanosvg>
//!
//! The parser and rasterizer are the translations in `nanosvg.rs` and
//! `nanosvgrast.rs`.

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::stdlib::math::ceilf;
use sdl3::video::{PixelFormat, Surface};

use crate::nanosvg::nsvg_parse;
use crate::nanosvgrast::{nsvg_create_rasterizer, nsvg_rasterize};
use crate::util::c_f32_to_i32;

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

/* Load a SVG type image from an SDL datasource */

/// Load an SVG image (the rest of `src`) rasterized at a size: scaled to
/// fit `width` by `height` keeping its aspect ratio, or to `width` or
/// `height` alone when the other is 0, or at its own size when both are
/// 0. The surface is RGBA32. Translation of `IMG_LoadSizedSVG_IO()`.
pub fn load_sized_svg_io(
    src: &mut IoStream<'_>,
    width: i32,
    height: i32,
) -> Result<Surface<'static>> {
    let mut data = src.load_all()?;
    // (SDL_LoadFile_IO() terminates the data with a NUL)
    data.push(0);

    /* For now just use default units of pixels at 96 DPI */
    let image = nsvg_parse(&mut data, b"px", 96.0);
    drop(data);
    if image.width <= 0.0 || image.height <= 0.0 {
        return Err(Error::new("Couldn't parse SVG image"));
    }

    let mut rasterizer = nsvg_create_rasterizer();

    let scale = if width > 0 && height > 0 {
        let scale_x = width as f32 / image.width;
        let scale_y = height as f32 / image.height;

        if scale_x < scale_y {
            scale_x
        } else {
            scale_y
        }
    } else if width > 0 {
        width as f32 / image.width
    } else if height > 0 {
        height as f32 / image.height
    } else {
        1.0
    };

    let mut surface = Surface::new(
        c_f32_to_i32(ceilf(image.width * scale)),
        c_f32_to_i32(ceilf(image.height * scale)),
        PixelFormat::RGBA32,
    )?;

    let (w, h, pitch) = (surface.width(), surface.height(), surface.pitch());
    if let Some(pixels) = surface.pixels_mut() {
        nsvg_rasterize(
            &mut rasterizer,
            &image,
            0.0,
            0.0,
            scale,
            pixels,
            w,
            h,
            pitch,
        );
    }

    Ok(surface)
}

/// Load an SVG image at its own size, as an RGBA32 surface.
/// Translation of `IMG_LoadSVG_IO()`.
pub fn load_svg_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    load_sized_svg_io(src, 0, 0)
}
