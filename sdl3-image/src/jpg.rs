// Rust translation of src/IMG_jpg.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a JPEG image file loading framework
//!
//! As upstream built with `USE_STBIMAGE`: loading through stb_image (see
//! [`crate::stb`]) and saving through tiny_jpeg (see [`crate::tiny_jpeg`]).
//! The libjpeg backend isn't translated.

use std::path::Path;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{PixelFormat, Surface};

use crate::util::read_ok;

/* FIXME: This is a copypaste from JPEGLIB! Pull that out of the ifdefs */
/* Define this for quicker (but less perfect) JPEG identification */
const FAST_IS_JPEG: bool = true;

/* See if an image is contained in a data source */

/// Whether `src` holds a JPEG image; the stream position is unchanged.
/// Translation of `IMG_isJPG()`.
#[allow(clippy::if_same_then_else)] // the branches mirror upstream's
pub fn is_jpg(src: &mut IoStream<'_>) -> bool {
    /* This detection code is by Steaphan Greene <stea@cs.binghamton.edu> */
    /* Blame me, not Sam, if this doesn't work right. */
    /* And don't forget to report the problem to the the sdl list too! */

    let start = src.tell().unwrap_or(-1);
    let mut is_jpg = false;
    let mut in_scan = false;
    let mut magic = [0u8; 4];
    if read_ok(src, &mut magic[..2]) && magic[0] == 0xFF && magic[1] == 0xD8 {
        is_jpg = true;
        while is_jpg {
            if !read_ok(src, &mut magic[..2]) {
                is_jpg = false;
            } else if magic[0] != 0xFF && !in_scan {
                is_jpg = false;
            } else if magic[0] != 0xFF || magic[1] == 0xFF {
                /* Extra padding in JPEG (legal) */
                /* or this is data and we are scanning */
                let _ = src.seek(-1, IoWhence::Cur);
            } else if magic[1] == 0xD9 {
                /* Got to end of good JPEG */
                break;
            } else if in_scan && magic[1] == 0x00 {
                /* This is an encoded 0xFF within the data */
            } else if magic[1] >= 0xD0 && magic[1] < 0xD9 {
                /* These have nothing else */
            } else if !read_ok(src, &mut magic[2..4]) {
                is_jpg = false;
            } else {
                /* Yes, it's big-endian */
                let inner_start = src.tell().unwrap_or(-1);
                let size = ((magic[2] as u32) << 8) + magic[3] as u32;
                // (upstream's unsigned `size-2` wraps for a size below 2)
                let end = src
                    .seek(size.wrapping_sub(2) as i64, IoWhence::Cur)
                    .unwrap_or(-1);
                if end != inner_start + size as i64 - 2 {
                    is_jpg = false;
                }
                if magic[1] == 0xDA {
                    /* Now comes the actual JPEG meat */
                    if FAST_IS_JPEG {
                        /* Ok, I'm convinced.  It is a JPEG. */
                        break;
                    } else {
                        /* I'm not convinced.  Prove it! */
                        in_scan = true;
                    }
                }
            }
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_jpg
}

/// Load a JPEG image (through stb_image, which also decodes PNG data
/// passed here). Translation of `IMG_LoadJPG_IO()`.
pub fn load_jpg_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    crate::stb::load_stb_io(src)
}

/* Use tinyjpeg as a fallback if we don't have a hard dependency on libjpeg */

/// Translation of `IMG_SaveJPG_IO_tinyjpeg()`.
fn save_jpg_io_tinyjpeg(
    surface: &mut Surface<'_>,
    dst: &mut IoStream<'_>,
    mut quality: i32,
) -> Result<()> {
    /* The JPEG library reads bytes in R,G,B order, so this is the right
     * encoding for either endianness */
    const JPG_FORMAT: PixelFormat = PixelFormat::RGB24;

    /* Convert surface to format we can save */
    let converted;
    let jpeg_surface: &Surface<'_> = if surface.format() != JPG_FORMAT {
        converted = surface.convert(JPG_FORMAT)?;
        &converted
    } else {
        surface
    };

    /* Quality for tinyjpeg is from 1-3:
     * 0  - 33  - Lowest quality
     * 34 - 66  - Middle quality
     * 67 - 100 - Highest quality
     */
    quality = if quality < 34 {
        1
    } else if quality < 67 {
        2
    } else {
        3
    };

    let pixels = jpeg_surface.pixels().unwrap_or(&[]);
    // (IMG_SaveJPG_IO_tinyjpeg_callback(): upstream ignores the result of
    // SDL_WriteIO())
    let result = crate::tiny_jpeg::encode_with_func(
        &mut |data| {
            let _ = dst.write(data);
        },
        quality,
        jpeg_surface.width(),
        jpeg_surface.height(),
        3,
        pixels,
        jpeg_surface.pitch(),
    );

    if !result {
        return Err(Error::new("tinyjpeg error"));
    }
    Ok(())
}

/// Save a surface in JPEG format; `quality` is 0 to 100 (tiny_jpeg uses
/// three levels: below 34, below 67, and the rest).
/// Translation of `IMG_SaveJPG_IO()`.
pub fn save_jpg_io(surface: &mut Surface<'_>, dst: &mut IoStream<'_>, quality: i32) -> Result<()> {
    save_jpg_io_tinyjpeg(surface, dst, quality)
}

/// Save a surface to a JPEG file. Translation of `IMG_SaveJPG()`.
pub fn save_jpg(surface: &mut Surface<'_>, file: impl AsRef<Path>, quality: i32) -> Result<()> {
    let mut dst = IoStream::from_file(file, "wb")?;
    let result = save_jpg_io(surface, &mut dst, quality);
    let closed = dst.close();
    result.and(closed)
}
