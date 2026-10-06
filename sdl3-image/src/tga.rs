// Rust translation of src/IMG_tga.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a Targa image file loading framework
//!
//! A TGA loader for the SDL library
//! Supports: Reading 8, 15, 16, 24 and 32bpp images, with alpha or colourkey,
//!           uncompressed or RLE encoded.
//!
//! 2000-06-10 Mattias Engdegård <f91-men@nada.kth.se>: initial version
//! 2000-06-26 Mattias Engdegård <f91-men@nada.kth.se>: read greyscale TGAs
//! 2000-08-09 Mattias Engdegård <f91-men@nada.kth.se>: alpha inversion removed

use std::path::Path;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{share_palette, Palette, PixelFormat, Surface};

use crate::img::verify_can_save_surface;
use crate::util::{error, read_ok, write_error};

/// The size of `struct TGAheader`.
const TGA_HEADER_SIZE: usize = 18;

/// Translation of `struct TGAheader`, as its 18 bytes.
struct TgaHeader {
    infolen: u8,  /* length of info field */
    has_cmap: u8, /* 1 if image has colormap, 0 otherwise */
    type_: u8,

    cmap_start: [u8; 2], /* index of first colormap entry */
    cmap_len: [u8; 2],   /* number of entries in colormap */
    cmap_bits: u8,       /* bits per colormap entry */

    yorigin: [u8; 2], /* image origin (ignored here) */
    xorigin: [u8; 2],
    width: [u8; 2], /* image size */
    height: [u8; 2],
    pixel_bits: u8, /* bits/pixel */
    flags: u8,
}

impl TgaHeader {
    fn parse(b: &[u8; TGA_HEADER_SIZE]) -> TgaHeader {
        TgaHeader {
            infolen: b[0],
            has_cmap: b[1],
            type_: b[2],
            cmap_start: [b[3], b[4]],
            cmap_len: [b[5], b[6]],
            cmap_bits: b[7],
            yorigin: [b[8], b[9]],
            xorigin: [b[10], b[11]],
            width: [b[12], b[13]],
            height: [b[14], b[15]],
            pixel_bits: b[16],
            flags: b[17],
        }
    }

    fn to_bytes(&self) -> [u8; TGA_HEADER_SIZE] {
        [
            self.infolen,
            self.has_cmap,
            self.type_,
            self.cmap_start[0],
            self.cmap_start[1],
            self.cmap_len[0],
            self.cmap_len[1],
            self.cmap_bits,
            self.yorigin[0],
            self.yorigin[1],
            self.xorigin[0],
            self.xorigin[1],
            self.width[0],
            self.width[1],
            self.height[0],
            self.height[1],
            self.pixel_bits,
            self.flags,
        ]
    }
}

/* enum tga_type */
const TGA_TYPE_INDEXED: u8 = 1;
const TGA_TYPE_RGB: u8 = 2;
const TGA_TYPE_BW: u8 = 3;
const TGA_TYPE_RLE_INDEXED: u8 = 9;
const TGA_TYPE_RLE_RGB: u8 = 10;
const TGA_TYPE_RLE_BW: u8 = 11;

const TGA_INTERLEAVE_MASK: u8 = 0xc0;
const TGA_INTERLEAVE_NONE: u8 = 0x00;
#[allow(dead_code)]
const TGA_INTERLEAVE_2WAY: u8 = 0x40;
#[allow(dead_code)]
const TGA_INTERLEAVE_4WAY: u8 = 0x80;

#[allow(dead_code)]
const TGA_ORIGIN_MASK: u8 = 0x30;
#[allow(dead_code)]
const TGA_ORIGIN_LEFT: u8 = 0x00;
const TGA_ORIGIN_RIGHT: u8 = 0x10;
#[allow(dead_code)]
const TGA_ORIGIN_LOWER: u8 = 0x00;
const TGA_ORIGIN_UPPER: u8 = 0x20;

/* read/write unaligned little-endian 16-bit ints */
/// Translation of `LE16()`.
fn le16(p: [u8; 2]) -> i32 {
    p[0] as i32 + ((p[1] as i32) << 8)
}
/// Translation of `SETLE16()`.
fn setle16(p: &mut [u8; 2], v: i32) {
    p[0] = v as u8;
    p[1] = (v >> 8) as u8;
}

/// Load a TGA image, a magicless format (so the front end only tries it
/// when asked for by type): 8-bit indexed or gray as INDEX8, 15 and 16 bits
/// as XRGB1555, 24 bits as BGR24 and 32 bits as BGRA32; uncompressed or RLE.
/// Translation of `IMG_LoadTGA_IO()`.
pub fn load_tga_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);
    let result = load_tga(src);
    if result.is_err() {
        let _ = src.seek(start, IoWhence::Set);
    }
    result
}

#[allow(clippy::needless_range_loop)] // the colormap is indexed as upstream's
fn load_tga(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let unsupported = || Err(Error::new("Unsupported TGA format"));

    let rle;
    let indexed;
    let mut grey = false;
    let mut ckey: i32 = -1;

    let mut raw = [0u8; TGA_HEADER_SIZE];
    if !read_ok(src, &mut raw) {
        return error("Error reading TGA data");
    }
    let hdr = TgaHeader::parse(&raw);
    let mut ncols = le16(hdr.cmap_len);
    match hdr.type_ {
        TGA_TYPE_RLE_INDEXED | TGA_TYPE_INDEXED => {
            rle = hdr.type_ == TGA_TYPE_RLE_INDEXED;
            /* fallthrough */
            if hdr.has_cmap == 0 || hdr.pixel_bits != 8 || ncols > 256 {
                return unsupported();
            }
            indexed = true;
        }

        TGA_TYPE_RLE_RGB | TGA_TYPE_RGB => {
            rle = hdr.type_ == TGA_TYPE_RLE_RGB;
            /* fallthrough */
            indexed = false;
        }

        TGA_TYPE_RLE_BW | TGA_TYPE_BW => {
            rle = hdr.type_ == TGA_TYPE_RLE_BW;
            /* fallthrough */
            if hdr.pixel_bits != 8 {
                return unsupported();
            }
            /* Treat greyscale as 8bpp indexed images */
            indexed = true;
            grey = true;
        }

        _ => return unsupported(),
    }

    let bpp = ((hdr.pixel_bits as usize) + 7) >> 3;
    let format = match hdr.pixel_bits {
        8 => {
            if !indexed {
                return unsupported();
            }
            PixelFormat::INDEX8
        }

        15 | 16 => {
            /* 15 and 16bpp both seem to use 5 bits/plane. The extra alpha bit
            is ignored for now. */
            PixelFormat::XRGB1555
        }

        32 => PixelFormat::BGRA32,
        24 => PixelFormat::BGR24,

        _ => return unsupported(),
    };

    if (hdr.flags & TGA_INTERLEAVE_MASK) != TGA_INTERLEAVE_NONE || hdr.flags & TGA_ORIGIN_RIGHT != 0
    {
        return unsupported();
    }

    let _ = src.seek(hdr.infolen as i64, IoWhence::Cur); /* skip info field */

    let w = le16(hdr.width);
    let h = le16(hdr.height);
    if w == 0 || h == 0 {
        return error("TGA image with zero width or height");
    }
    let Ok(mut img) = Surface::new(w, h, format) else {
        return error("Out of memory");
    };

    if hdr.has_cmap != 0 {
        let palsiz = ncols as usize * ((hdr.cmap_bits as usize + 7) >> 3);
        if indexed && !grey {
            let mut pal = vec![0u8; palsiz];
            if !read_ok(src, &mut pal) {
                return error("Error reading TGA data");
            }
            // (SDL_CreateSurfacePalette() and then `palette->ncolors = ncols`)
            if ncols > 256 {
                ncols = 256;
            }
            // Note (upstream): a colormap with no entries leaves the palette
            // with none there; SDL can't make one, so it keeps one here.
            let Ok(mut palette) = Palette::new(ncols.max(1) as usize) else {
                return error("Couldn't create palette");
            };

            let mut p = 0usize;
            let colors = palette.colors_mut();
            for i in 0..ncols as usize {
                match hdr.cmap_bits {
                    15 | 16 => {
                        let c = pal[p] as u16 + ((pal[p + 1] as u16) << 8);
                        p += 2;
                        colors[i].r = ((c >> 7) & 0xf8) as u8;
                        colors[i].g = ((c >> 2) & 0xf8) as u8;
                        colors[i].b = (c << 3) as u8;
                    }
                    24 | 32 => {
                        colors[i].b = pal[p];
                        colors[i].g = pal[p + 1];
                        colors[i].r = pal[p + 2];
                        p += 3;
                        if hdr.cmap_bits == 32 {
                            let a = pal[p];
                            p += 1;
                            if a < 128 {
                                ckey = i as i32;
                            }
                        }
                    }
                    _ => {}
                }
            }
            if img.set_palette(Some(share_palette(palette))).is_err() {
                return error("Couldn't create palette");
            }

            if ckey >= 0 {
                let _ = img.set_color_key(Some(ckey as u32));
            }
        } else {
            /* skip unneeded colormap */
            let _ = src.seek(palsiz as i64, IoWhence::Cur);
        }
    }

    if grey {
        let Ok(palette) = img.create_palette() else {
            return error("Couldn't create palette");
        };
        let mut palette = palette.write().unwrap_or_else(|e| e.into_inner());
        for (i, c) in palette.colors_mut().iter_mut().enumerate().take(256) {
            c.r = i as u8;
            c.g = i as u8;
            c.b = i as u8;
        }
    }

    let pitch = img.pitch() as usize;
    let h = h as usize;
    let w = w as usize;
    let upper = hdr.flags & TGA_ORIGIN_UPPER != 0;
    let Some(pixels) = img.pixels_mut() else {
        return error("Out of memory");
    };

    /* The RLE decoding code is slightly convoluted since we can't rely on
    spans not to wrap across scan lines */
    let mut count = 0usize;
    let mut rep = 0usize;
    let mut pixelvalue = [0u8; 4];
    for i in 0..h {
        // (lstep: top-down, or bottom-up from the last row)
        let row = if upper { i } else { h - 1 - i };
        let dst = &mut pixels[row * pitch..row * pitch + w * bpp];
        if rle {
            let mut x = 0usize;
            loop {
                if count != 0 {
                    let mut n = count;
                    if n > w - x {
                        n = w - x;
                    }
                    if !read_ok(src, &mut dst[x * bpp..(x + n) * bpp]) {
                        return error("Error reading TGA data");
                    }
                    count -= n;
                    x += n;
                    if x == w {
                        break;
                    }
                } else if rep != 0 {
                    let mut n = rep;
                    if n > w - x {
                        n = w - x;
                    }
                    rep -= n;
                    for _ in 0..n {
                        dst[x * bpp..(x + 1) * bpp].copy_from_slice(&pixelvalue[..bpp]);
                        x += 1;
                    }
                    if x == w {
                        break;
                    }
                }

                let mut c = [0u8];
                if !read_ok(src, &mut c) {
                    return error("Error reading TGA data");
                }
                let c = c[0];
                if c & 0x80 != 0 {
                    if !read_ok(src, &mut pixelvalue[..bpp]) {
                        return error("Error reading TGA data");
                    }
                    rep = (c & 0x7f) as usize + 1;
                } else {
                    count = c as usize + 1;
                }
            }
        } else if !read_ok(src, dst) {
            return error("Error reading TGA data");
        }
        if cfg!(target_endian = "big") && bpp == 2 {
            /* swap byte order */
            for p in dst.chunks_exact_mut(2) {
                p.swap(0, 1);
            }
        }
    }
    Ok(img)
}

/// Save a surface in TGA format: INDEX8 with a 24-bit colormap, RGB24 and
/// BGR24 as 24 bits, RGBA32, BGRA32 and XRGB8888 as 32 bits (with alpha),
/// XRGB1555 and ARGB1555 as 16 bits; other formats are an error.
/// Translation of `IMG_SaveTGA_IO()`.
pub fn save_tga_io(surface: &mut Surface<'_>, dst: &mut IoStream<'_>) -> Result<()> {
    verify_can_save_surface(surface)?;

    let start = dst.tell().unwrap_or(-1);

    let result = write_tga(surface, dst);
    if result.is_err() && start != -1 {
        let _ = dst.seek(start, IoWhence::Set);
    }
    result
}

fn write_tga(surface: &mut Surface<'_>, dst: &mut IoStream<'_>) -> Result<()> {
    let mut hdr = TgaHeader::parse(&[0u8; TGA_HEADER_SIZE]);

    hdr.infolen = 0;
    hdr.has_cmap = 0;
    hdr.type_ = TGA_TYPE_RGB;
    setle16(&mut hdr.cmap_start, 0);
    setle16(&mut hdr.cmap_len, 0);
    hdr.cmap_bits = 0;
    setle16(&mut hdr.xorigin, 0);
    setle16(&mut hdr.yorigin, 0);
    setle16(&mut hdr.width, surface.width());
    setle16(&mut hdr.height, surface.height());
    hdr.flags = TGA_ORIGIN_UPPER;

    let mut bytes_per_pixel = surface.format().bytes_per_pixel() as usize;

    let surface_palette = surface.palette().cloned();

    match surface.format() {
        PixelFormat::INDEX8 => {
            hdr.has_cmap = 1;
            hdr.type_ = TGA_TYPE_INDEXED;
            hdr.pixel_bits = 8;
            if let Some(p) = &surface_palette {
                let ncolors = p.read().unwrap_or_else(|e| e.into_inner()).len();
                setle16(&mut hdr.cmap_len, ncolors as i32);
            } else {
                setle16(&mut hdr.cmap_len, 0);
            }
            hdr.cmap_bits = 24;
        }
        PixelFormat::BGR24 | PixelFormat::RGB24 => {
            hdr.type_ = TGA_TYPE_RGB;
            hdr.pixel_bits = 24;
        }
        PixelFormat::BGRA32 | PixelFormat::RGBA32 | PixelFormat::XRGB8888 => {
            hdr.type_ = TGA_TYPE_RGB;
            hdr.pixel_bits = 32;
            hdr.flags |= 0x08;
        }
        PixelFormat::XRGB1555 | PixelFormat::ARGB1555 => {
            hdr.type_ = TGA_TYPE_RGB;
            hdr.pixel_bits = 16;
            if surface.format() == PixelFormat::ARGB1555 {
                hdr.flags |= 0x01;
            }
        }
        _ => {
            return Err(Error::new("Unsupported SDL_Surface format for TGA saving. Supported formats are INDEX8, BGR24, RGB24, BGRA32, RGBA32, XRGB8888, XRGB1555, ARGB1555."));
        }
    }

    let header = hdr.to_bytes();
    if dst.write(&header) != header.len() {
        return Err(write_error(dst));
    }

    if hdr.has_cmap != 0 {
        if let Some(palette) = &surface_palette {
            let palette = palette.read().unwrap_or_else(|e| e.into_inner());
            let colors = palette.colors();
            let ncolors = le16(hdr.cmap_len) as usize;
            for c in colors.iter().take(ncolors) {
                match hdr.cmap_bits {
                    24 => {
                        let color_entry = [c.b, c.g, c.r];
                        if dst.write(&color_entry) != 3 {
                            return Err(write_error(dst));
                        }
                    }
                    32 => {
                        let color_entry = [c.b, c.g, c.r, c.a];
                        if dst.write(&color_entry) != 4 {
                            return Err(write_error(dst));
                        }
                    }
                    _ => {
                        return Err(Error::new("Unsupported TGA colormap bit depth for saving"));
                    }
                }
            }
        }
    }

    let target_format = match surface.format() {
        PixelFormat::RGB24 => PixelFormat::BGR24,
        PixelFormat::RGBA32 | PixelFormat::XRGB8888 => PixelFormat::BGRA32,
        PixelFormat::XRGB1555 | PixelFormat::ARGB1555 => PixelFormat::XRGB1555,
        _ => PixelFormat::UNKNOWN,
    };

    let temp_surface;
    let to_write: &Surface<'_> =
        if target_format != PixelFormat::UNKNOWN && target_format != surface.format() {
            temp_surface = surface.convert(target_format)?;
            bytes_per_pixel = temp_surface.format().bytes_per_pixel() as usize;
            &temp_surface
        } else {
            surface
        };
    let pixels_to_write = to_write.pixels().unwrap_or(&[]);
    let pitch_to_write = to_write.pitch() as usize;

    let row_len = surface.width() as usize * bytes_per_pixel;
    for y in 0..surface.height() as usize {
        let row = &pixels_to_write[y * pitch_to_write..y * pitch_to_write + row_len];
        if dst.write(row) != row_len {
            return Err(write_error(dst));
        }
    }

    Ok(())
}

/// Save a surface to a TGA file. Translation of `IMG_SaveTGA()`.
pub fn save_tga(surface: &mut Surface<'_>, file: impl AsRef<Path>) -> Result<()> {
    verify_can_save_surface(surface)?;
    let mut dst = IoStream::from_file(file, "wb")?;
    let result = save_tga_io(surface, &mut dst);
    let closed = dst.close();
    result.and(closed)
}
