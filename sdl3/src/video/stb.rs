// Rust translation of the image file front end of src/video/SDL_stb.c and
// of SDL_LoadSurface_IO() / SDL_LoadSurface() from src/video/SDL_surface.c,
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! PNG and JPEG detection, and loading a surface from any supported image
//! file.
//!
//! The PNG and JPEG codecs are the third-party stb_image and miniz
//! libraries upstream; they are not translated yet, so this behaves like an
//! upstream build without `SDL_HAVE_STB`: the formats are recognized, but
//! loading and saving them reports "SDL not built with STB image support".

use std::path::Path;

use crate::error::{Error, Result};
use crate::io::{IoStream, IoWhence};
use crate::video::bmp::is_bmp;
use crate::video::surface::Surface;

const NO_STB: &str = "SDL not built with STB image support";

/* FIXME: This is a copypaste from JPEGLIB! Pull that out of the ifdefs */
/* Define this for quicker (but less perfect) JPEG identification */
const FAST_IS_JPEG: bool = true;

/// Whether `src` holds a JPEG image; the stream position is unchanged.
/// Translation of `SDL_IsJPG()`.
#[allow(clippy::if_same_then_else)] // the branches mirror upstream's
pub fn is_jpg(src: &mut IoStream<'_>) -> bool {
    /* This detection code is by Steaphan Greene <stea@cs.binghamton.edu> */
    /* Blame me, not Sam, if this doesn't work right. */
    /* And don't forget to report the problem to the the sdl list too! */

    let start = src.tell().unwrap_or(-1);
    let mut is_jpg = false;
    let mut in_scan = false;
    let mut magic = [0u8; 4];
    if src.read(&mut magic[..2]) == 2 && magic[0] == 0xFF && magic[1] == 0xD8 {
        is_jpg = true;
        while is_jpg {
            if src.read(&mut magic[..2]) != 2 {
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
            } else if src.read(&mut magic[2..4]) != 2 {
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

/// Whether `src` holds a PNG image; the stream position is unchanged.
/// Translation of `SDL_IsPNG()`.
pub fn is_png(src: &mut IoStream<'_>) -> bool {
    let mut is_png = false;
    if let Ok(start) = src.tell() {
        if start >= 0 {
            let mut magic = [0u8; 4];
            if src.read(&mut magic) == magic.len()
                && magic[0] == 0x89
                && magic[1] == b'P'
                && magic[2] == b'N'
                && magic[3] == b'G'
            {
                is_png = true;
            }
            let _ = src.seek(start, IoWhence::Set);
        }
    }
    is_png
}

impl Surface<'_> {
    /// Load a JPEG image. Translation of `SDL_LoadJPG_IO()`.
    pub fn load_jpg_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
        if !is_jpg(src) {
            return Err(Error::new("File is not a JPEG file"));
        }
        Err(Error::new(NO_STB))
    }

    /// Load a JPEG image from a file. Translation of `SDL_LoadJPG()`.
    pub fn load_jpg(file: impl AsRef<Path>) -> Result<Surface<'static>> {
        let mut stream = IoStream::from_file(file, "rb")?;
        Surface::load_jpg_io(&mut stream)
    }

    /// Load a PNG image. Translation of `SDL_LoadPNG_IO()`.
    pub fn load_png_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
        if !is_png(src) {
            return Err(Error::new("File is not a PNG file"));
        }
        Err(Error::new(NO_STB))
    }

    /// Load a PNG image from a file. Translation of `SDL_LoadPNG()`.
    pub fn load_png(file: impl AsRef<Path>) -> Result<Surface<'static>> {
        let mut stream = IoStream::from_file(file, "rb")?;
        Surface::load_png_io(&mut stream)
    }

    /// Save the surface in PNG format. Translation of `SDL_SavePNG_IO()`.
    pub fn save_png_io(&mut self, _dst: &mut IoStream<'_>) -> Result<()> {
        Err(Error::new(NO_STB))
    }

    /// Save the surface to a PNG file. Translation of `SDL_SavePNG()`.
    pub fn save_png(&mut self, _file: impl AsRef<Path>) -> Result<()> {
        Err(Error::new(NO_STB))
    }

    /// Load a BMP, PNG or JPEG image. Translation of `SDL_LoadSurface_IO()`.
    pub fn load_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
        if is_bmp(src) {
            Surface::load_bmp_io(src)
        } else if is_png(src) {
            Surface::load_png_io(src)
        } else if is_jpg(src) {
            Surface::load_jpg_io(src)
        } else {
            Err(Error::new("Unsupported image format"))
        }
    }

    /// Load a BMP, PNG or JPEG image file. Translation of `SDL_LoadSurface()`.
    pub fn load(file: impl AsRef<Path>) -> Result<Surface<'static>> {
        let mut stream = IoStream::from_file(file, "rb")?;
        Surface::load_io(&mut stream)
    }
}
