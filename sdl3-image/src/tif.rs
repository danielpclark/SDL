// Rust translation of src/IMG_tif.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a TIFF image file loading framework
//!
//! The decoder is libtiff (as SDL_image's external/libtiff pins it,
//! 4.7.2), translated in the submodules: its reading path, with the
//! codecs SDL_image's build enables (none, CCITT RLE/RLEW/Group 3/Group 4,
//! PackBits, LZW, ThunderScan, NeXT and SGI LogL/LogLuv; the zlib, JPEG,
//! JBIG, LERC, LZMA, ZSTD, WebP and PixarLog codecs are off, and files
//! using them fail as "not configured").

// (the translation of libtiff keeps upstream's switches, loops over
// indices, range checks and NaN-aware comparisons as they are)
#![allow(
    clippy::collapsible_match,
    clippy::manual_is_multiple_of,
    clippy::manual_range_contains,
    clippy::needless_range_loop,
    clippy::neg_cmp_op_on_partial_ord
)]

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{PixelFormat, Surface};

use crate::util::read_ok;

#[allow(non_snake_case)]
mod tif_aux;
mod tif_codec;
mod tif_color;
mod tif_compress;
#[allow(non_snake_case)]
mod tif_dir;
mod tif_dirinfo;
mod tif_dirread;
mod tif_dumpmode;
pub(crate) mod tif_error;
#[allow(non_snake_case)]
mod tif_fax3;
mod tif_fax3sm;
mod tif_getimage;
mod tif_luv;
mod tif_lzw;
mod tif_next;
mod tif_open;
mod tif_packbits;
mod tif_predict;
mod tif_read;
mod tif_strip;
mod tif_swab;
mod tif_thunder;
mod tif_tile;
mod tiff;
#[allow(non_snake_case)]
mod tiffio;
mod tiffiop;

use tiff::{
    ORIENTATION_TOPLEFT, ORIENTATION_TOPRIGHT, TIFFTAG_IMAGELENGTH, TIFFTAG_IMAGEWIDTH,
    TIFFTAG_ORIENTATION,
};
use tiffiop::{TiffClient, TmSize};

/*
 * These are the thunking routine to use the SDL_IOStream* routines from
 * libtiff's internals.
 */

/// The client procedures over the stream (`tiff_read()`, `tiff_seek()`
/// and `tiff_size()`; `tiff_write()` isn't used for reading, and
/// `tiff_close()`, `tiff_map()` and `tiff_unmap()` do nothing).
struct Client<'s, 'io> {
    src: &'s mut IoStream<'io>,
}

impl TiffClient for Client<'_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> TmSize {
        self.src.read(buf) as TmSize
    }

    fn seek(&mut self, offset: u64, origin: i32) -> u64 {
        let whence = match origin {
            0 => IoWhence::Set,
            1 => IoWhence::Cur,
            _ => IoWhence::End,
        };
        match self.src.seek(offset as i64, whence) {
            Ok(p) => p as u64,
            Err(_) => u64::MAX,
        }
    }

    fn size(&mut self) -> u64 {
        let save_pos = self.src.tell().unwrap_or(-1);
        let _ = self.src.seek(0, IoWhence::End);
        let size = self.src.tell().unwrap_or(-1);
        let _ = self.src.seek(save_pos, IoWhence::Set);
        size as u64
    }
}

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

/// The error of a failure libtiff reported: its last error's text, where
/// upstream leaves `SDL_GetError()` as it was (libtiff only prints it).
fn libtiff_error() -> Error {
    match tif_error::take_last_error() {
        Some(message) => Error::new(message),
        None => Error::new("Couldn't load TIFF image"),
    }
}

/// Load a TIFF image (its first directory) as ABGR8888, rotated as its
/// orientation says; on failure the stream is back at its start.
/// Translation of `IMG_LoadTIF_IO()`.
pub fn load_tif_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);
    let result = load(src);
    if result.is_err() {
        let _ = src.seek(start, IoWhence::Set);
    }
    result
}

/// The body of `IMG_LoadTIF_IO()` up to its `error:` label.
fn load(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let mut img_orientation: u16 = 1;

    let _ = tif_error::take_last_error();
    let mut client = Client { src };
    /* turn off memory mapped access with the m flag */
    let Some(mut tiff) = tif_open::tiff_client_open("SDL_image", "rm", &mut client) else {
        return Err(libtiff_error());
    };

    /* Retrieve the dimensions of the image from the TIFF tags */
    let img_width = tif_dir::tiff_get_field_int(&mut tiff, TIFFTAG_IMAGEWIDTH).unwrap_or(0) as u32;
    let img_height =
        tif_dir::tiff_get_field_int(&mut tiff, TIFFTAG_IMAGELENGTH).unwrap_or(0) as u32;
    if let Some(o) = tif_dir::tiff_get_field_int(&mut tiff, TIFFTAG_ORIENTATION) {
        img_orientation = o as u16;
    }

    let mut surface = match Surface::new(img_width as i32, img_height as i32, PixelFormat::ABGR8888)
    {
        Ok(s) => s,
        Err(e) => {
            tif_open::tiff_close(tiff);
            return Err(e);
        }
    };

    let load_orientation = match img_orientation {
        5..=8 => ORIENTATION_TOPRIGHT,
        _ => ORIENTATION_TOPLEFT,
    };
    let ok = {
        let mut empty = [0u8; 0];
        let pixels = surface.pixels_mut().unwrap_or(&mut empty);
        tif_getimage::tiff_read_rgba_image_oriented(
            &mut tiff,
            img_width,
            img_height,
            pixels,
            load_orientation as i32,
            0,
        )
    };
    tif_open::tiff_close(tiff);
    if ok == 0 {
        return Err(libtiff_error());
    }

    match img_orientation {
        5 | 7 => surface.rotate(270.0f32),
        6 | 8 => surface.rotate(90.0f32),
        _ => Ok(surface),
    }
}

/// The width and height of the first directory of a TIFF in memory, if
/// libtiff opens it (for tests to skip huge images).
#[cfg(test)]
pub(crate) fn dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut io = IoStream::from_const_mem(data);
    let mut client = Client { src: &mut io };
    let mut tiff = tif_open::tiff_client_open("SDL_image", "rm", &mut client)?;
    let w = tif_dir::tiff_get_field_int(&mut tiff, TIFFTAG_IMAGEWIDTH);
    let h = tif_dir::tiff_get_field_int(&mut tiff, TIFFTAG_IMAGELENGTH);
    tif_open::tiff_close(tiff);
    Some((w? as u32, h? as u32))
}
