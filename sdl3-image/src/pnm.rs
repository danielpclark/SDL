// Rust translation of src/IMG_pnm.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! PNM (portable anymap) image loader:
//!
//! Supports: PBM, PGM and PPM, ASCII and binary formats
//! (PBM and PGM are loaded as 8bpp surfaces)
//! Does not support: maximum component value > 255

use sdl3::error::Result;
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{share_palette, Palette, PixelFormat, Surface};

use crate::util::{error, isdigit, isspace, read_byte, read_error, read_ok};

/* See if an image is contained in a data source */

/// Whether `src` holds a PBM, PGM or PPM image; the stream position is
/// unchanged. Translation of `IMG_isPNM()`.
pub fn is_pnm(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_pnm = false;
    let mut magic = [0u8; 2];
    if read_ok(src, &mut magic) {
        /*
         * PNM magic signatures:
         * P1   PBM, ascii format
         * P2   PGM, ascii format
         * P3   PPM, ascii format
         * P4   PBM, binary format
         * P5   PGM, binary format
         * P6   PPM, binary format
         * P7   PAM, a general wrapper for PNM data
         */
        if magic[0] == b'P' && magic[1] >= b'1' && magic[1] <= b'6' {
            is_pnm = true;
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_pnm
}

/// read a non-negative integer from the source. return -1 upon error
/// Translation of `ReadNumber()`.
fn read_number(src: &mut IoStream<'_>) -> i32 {
    /* Initialize return value */
    let mut number: i32 = 0;

    /* Skip leading whitespace */
    let mut ch;
    loop {
        let Some(c) = read_byte(src) else {
            return -1;
        };
        ch = c;
        /* Eat comments as whitespace */
        if ch == b'#' {
            /* Comment is '#' to end of line */
            loop {
                let Some(c) = read_byte(src) else {
                    return -1;
                };
                ch = c;
                if ch == b'\r' || ch == b'\n' {
                    break;
                }
            }
        }
        if !isspace(ch) {
            break;
        }
    }

    /* Add up the number */
    if !isdigit(ch) {
        return -1;
    }
    loop {
        /* Protect from possible overflow */
        if number >= (i32::MAX / 10) {
            return -1;
        }
        number *= 10;
        number += (ch - b'0') as i32;

        let Some(c) = read_byte(src) else {
            return -1;
        };
        ch = c;
        if !isdigit(ch) {
            break;
        }
    }

    number
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pbm,
    Pgm,
    Ppm,
    #[allow(dead_code)]
    Pam,
}

/// Load a PBM, PGM or PPM image (ASCII or binary, a maximum value of at
/// most 255): PPM as RGB24, PBM and PGM as INDEX8 with a black and white or
/// gray palette. Translation of `IMG_LoadPNM_IO()`.
pub fn load_pnm_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);

    let mut magic = [0u8; 2];
    if !read_ok(src, &mut magic) {
        // Note (upstream): NULL is returned here without an error message.
        return Err(read_error(src));
    }

    let result = load_pnm(src, magic);
    if result.is_err() {
        let _ = src.seek(start, IoWhence::Set);
    }
    result
}

fn load_pnm(src: &mut IoStream<'_>, magic: [u8; 2]) -> Result<Surface<'static>> {
    let mut kind = magic[1].wrapping_sub(b'1') as u32;
    let mut ascii = true;
    if kind >= 3 {
        ascii = false;
        kind -= 3;
    }
    // (a `kind` past PAM is none of the cases below, as in upstream's enum)
    let kind = match kind {
        0 => Some(Kind::Pbm),
        1 => Some(Kind::Pgm),
        2 => Some(Kind::Ppm),
        3 => Some(Kind::Pam),
        _ => None,
    };

    let width = read_number(src);
    let height = read_number(src);
    if width <= 0 || height <= 0 {
        return error("Unable to read image width and height");
    }

    let maxval = if kind != Some(Kind::Pbm) {
        let maxval = read_number(src);
        if maxval <= 0 || maxval > 255 {
            return error("unsupported PNM format");
        }
        maxval
    } else {
        255 /* never scale PBMs */
    };

    /* binary PNM allows just a single character of whitespace after
    the last parameter, and we've already consumed it */

    let format = if kind == Some(Kind::Ppm) {
        /* 24-bit surface in R,G,B byte order */
        PixelFormat::RGB24
    } else {
        /* load PBM/PGM as 8-bit indexed images */
        PixelFormat::INDEX8
    };
    let Ok(mut surface) = Surface::new(width, height, format) else {
        return error("Out of memory");
    };
    let mut bpl = width as usize * surface.format().bytes_per_pixel() as usize;
    let mut buf = Vec::new();
    if kind == Some(Kind::Pgm) {
        let Ok(palette) = surface.create_palette() else {
            return error("Couldn't create palette");
        };
        let mut palette = palette.write().unwrap_or_else(|e| e.into_inner());
        for (i, c) in palette.colors_mut().iter_mut().enumerate().take(256) {
            c.r = i as u8;
            c.g = i as u8;
            c.b = i as u8;
        }
    } else if kind == Some(Kind::Pbm) {
        /* for some reason PBM has 1=black, 0=white */
        let Ok(mut palette) = Palette::new(2) else {
            return error("Couldn't create palette");
        };
        {
            let c = palette.colors_mut();
            c[0].r = 255;
            c[0].g = 255;
            c[0].b = 255;
            c[1].r = 0;
            c[1].g = 0;
            c[1].b = 0;
        }
        let _ = surface.set_palette(Some(share_palette(palette)));

        bpl = (width as usize + 7) >> 3;
        buf = vec![0u8; bpl];
    }

    /* Read the image into the surface */
    let pitch = surface.pitch() as usize;
    let Some(pixels) = surface.pixels_mut() else {
        return error("Out of memory");
    };
    for y in 0..height as usize {
        let row = &mut pixels[y * pitch..(y + 1) * pitch];
        if ascii {
            if kind == Some(Kind::Pbm) {
                for p in row.iter_mut().take(width as usize) {
                    let mut ch;
                    loop {
                        let Some(c) = read_byte(src) else {
                            return error("file truncated");
                        };
                        ch = c.wrapping_sub(b'0');
                        if ch <= 1 {
                            break;
                        }
                    }
                    *p = ch;
                }
            } else {
                for p in row.iter_mut().take(bpl) {
                    let c = read_number(src);
                    if c < 0 {
                        return error("file truncated");
                    }
                    *p = c as u8;
                }
            }
        } else {
            if kind == Some(Kind::Pbm) {
                if !read_ok(src, &mut buf) {
                    return error("file truncated");
                }
            } else if !read_ok(src, &mut row[..bpl]) {
                return error("file truncated");
            }
            if kind == Some(Kind::Pbm) {
                /* expand bitmap to 8bpp */
                for i in 0..width as usize {
                    let bit = 7 - (i & 7);
                    row[i] = (buf[i >> 3] >> bit) & 1;
                }
            }
        }
        if maxval < 255 {
            /* scale up to full dynamic range (slow) */
            for p in row.iter_mut().take(bpl) {
                *p = (*p as i32 * 255 / maxval) as u8;
            }
        }
    }

    Ok(surface)
}
