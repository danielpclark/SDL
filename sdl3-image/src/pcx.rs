// Rust translation of src/IMG_pcx.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! PCX file reader:
//! Supports:
//!  1..4 bits/pixel in multiplanar format (1 bit/plane/pixel)
//!  8 bits/pixel in single-planar format (8 bits/plane/pixel)
//!  24 bits/pixel in 3-plane format (8 bits/plane/pixel)
//!
//! (The <8bpp formats are expanded to 8bpp surfaces)
//!
//! Doesn't support:
//!  single-planar packed-pixel formats other than 8bpp
//!  4-plane 32bpp format with a fourth "intensity" plane

use sdl3::error::Result;
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{share_palette, Palette, PixelFormat, Surface};

use crate::util::{error, read_byte, read_ok};

/// The size of `struct PCXheader`.
const PCX_HEADER_SIZE: usize = 128;

/// Translation of `struct PCXheader`, decoded from its 128 little-endian
/// bytes.
struct PcxHeader {
    manufacturer: u8,
    version: u8,
    encoding: u8,
    bits_per_pixel: u8,
    xmin: i16,
    ymin: i16,
    xmax: i16,
    ymax: i16,
    // HDpi, VDpi
    colormap: [u8; 48],
    // Reserved
    nplanes: u8,
    bytes_per_line: i16,
    // PaletteInfo, HscreenSize, VscreenSize, Filler[54]
}

impl PcxHeader {
    fn parse(b: &[u8; PCX_HEADER_SIZE]) -> PcxHeader {
        let le16 = |i: usize| i16::from_le_bytes([b[i], b[i + 1]]);
        let mut colormap = [0u8; 48];
        colormap.copy_from_slice(&b[16..64]);
        PcxHeader {
            manufacturer: b[0],
            version: b[1],
            encoding: b[2],
            bits_per_pixel: b[3],
            xmin: le16(4),
            ymin: le16(6),
            xmax: le16(8),
            ymax: le16(10),
            colormap,
            nplanes: b[65],
            bytes_per_line: le16(66),
        }
    }
}

/* See if an image is contained in a data source */

/// Whether `src` holds a PCX image; the stream position is unchanged.
/// Translation of `IMG_isPCX()`.
pub fn is_pcx(src: &mut IoStream<'_>) -> bool {
    const ZSOFT_MANUFACTURER: u8 = 10;
    const PC_PAINTBRUSH_VERSION: u8 = 5;
    const PCX_UNCOMPRESSED_ENCODING: u8 = 0;
    const PCX_RUN_LENGTH_ENCODING: u8 = 1;

    let start = src.tell().unwrap_or(-1);
    let mut is_pcx = false;
    let mut raw = [0u8; PCX_HEADER_SIZE];
    if read_ok(src, &mut raw) {
        let pcxh = PcxHeader::parse(&raw);
        if pcxh.manufacturer == ZSOFT_MANUFACTURER
            && pcxh.version == PC_PAINTBRUSH_VERSION
            && (pcxh.encoding == PCX_RUN_LENGTH_ENCODING
                || pcxh.encoding == PCX_UNCOMPRESSED_ENCODING)
        {
            is_pcx = true;
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_pcx
}

/// Load a PCX image: 1 to 4 planes of 1 bit and 8-bit images as INDEX8,
/// 3 planes of 8 bits as RGB24. Translation of `IMG_LoadPCX_IO()`.
pub fn load_pcx_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);
    let result = load_pcx(src);
    if result.is_err() {
        let _ = src.seek(start, IoWhence::Set);
    }
    result
}

fn load_pcx(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let mut raw = [0u8; PCX_HEADER_SIZE];
    if !read_ok(src, &mut raw) {
        return error("file truncated");
    }
    let pcxh = PcxHeader::parse(&raw);

    /* Create the surface of the appropriate type */
    let width = (pcxh.xmax as i32 - pcxh.xmin as i32) + 1;
    let height = (pcxh.ymax as i32 - pcxh.ymin as i32) + 1;
    let src_bits = pcxh.bits_per_pixel as i32 * pcxh.nplanes as i32;
    let (bits, format) = if (pcxh.bits_per_pixel == 1 && pcxh.nplanes >= 1 && pcxh.nplanes <= 4)
        || (pcxh.bits_per_pixel == 8 && pcxh.nplanes == 1)
    {
        (8, PixelFormat::INDEX8)
    } else if pcxh.bits_per_pixel == 8 && pcxh.nplanes == 3 {
        (24, PixelFormat::RGB24)
    } else {
        return error("unsupported PCX format");
    };
    let mut surface = Surface::new(width, height, format)?;

    let bpl = pcxh.nplanes as i32 * pcxh.bytes_per_line as i32;
    if bpl < 0 {
        // (a negative BytesPerLine wraps to a huge size_t there)
        return error("Out of memory");
    }
    let bpl = bpl as usize;
    let mut buf = vec![0u8; bpl];
    let pitch = surface.pitch() as usize;
    let h = surface.height() as usize;
    let mut count = 0i32;
    let mut ch = 0u8;
    {
        let pixels: &mut [u8] = surface.pixels_mut().unwrap_or(&mut []);
        for y in 0..h {
            // (a surface of zero width has no pixels, but its rows are read)
            let row = pixels
                .get_mut(y * pitch..(y + 1) * pitch)
                .unwrap_or(&mut []);
            /* decode a scan line to a temporary buffer first */
            if pcxh.encoding == 0 {
                if !read_ok(src, &mut buf) {
                    return error("file truncated");
                }
            } else {
                for b in buf.iter_mut() {
                    if count == 0 {
                        let Some(c) = read_byte(src) else {
                            return error("file truncated");
                        };
                        ch = c;
                        if ch < 0xc0 {
                            count = 1;
                        } else {
                            count = ch as i32 - 0xc0;
                            let Some(c) = read_byte(src) else {
                                return error("file truncated");
                            };
                            ch = c;
                        }
                    }
                    *b = ch;
                    count -= 1;
                }
            }

            if src_bits <= 4 {
                /* expand planes to 1 byte/pixel */
                let mut inner_src = 0;
                for plane in 0..pcxh.nplanes as u32 {
                    let mut x = 0;
                    for j in 0..pcxh.bytes_per_line as i32 {
                        let byte = buf[inner_src];
                        inner_src += 1;
                        for k in (0..=7).rev() {
                            let bit = (byte >> k) & 1;
                            /* skip padding bits */
                            // FIXME (upstream): for a width that isn't a
                            // multiple of 8 this keeps the low bits of the
                            // row's last byte, where the pixels are its
                            // high bits.
                            if j * 8 + k >= width {
                                continue;
                            }
                            row[x] |= bit << plane;
                            x += 1;
                        }
                    }
                }
            } else if src_bits == 8 {
                /* Copy the row directly */
                let n = (width as usize).min(bpl);
                row[..n].copy_from_slice(&buf[..n]);
            } else if src_bits == 24 {
                /* de-interlace planes */
                let mut inner_src = 0usize;
                let end1 = bpl;
                for plane in 0..pcxh.nplanes as usize {
                    let mut dst = plane;
                    let end2 = pitch;
                    for x in 0..width as usize {
                        if (inner_src + x) >= end1 || dst >= end2 {
                            return error("decoding out of bounds (corrupt?)");
                        }
                        row[dst] = buf[inner_src + x];
                        dst += pcxh.nplanes as usize;
                    }
                    inner_src += pcxh.bytes_per_line as usize;
                }
            }
        }
    }

    if bits == 8 {
        let mut nc = 1usize << src_bits;

        // (SDL_CreateSurfacePalette() and then `palette->ncolors = nc`)
        let max = 1usize << surface.format().bits_per_pixel();
        if nc > max {
            nc = max;
        }
        let Ok(mut palette) = Palette::new(nc) else {
            return error("Couldn't create palette");
        };

        if src_bits == 8 {
            let mut colormap = [0u8; 768];

            /* look for a 256-colour palette */
            loop {
                match read_byte(src) {
                    None => {
                        /* Couldn't find the palette, try the end of the file */
                        let _ = src.seek(-768, IoWhence::End);
                        break;
                    }
                    Some(12) => break,
                    Some(_) => {}
                }
            }

            if !read_ok(src, &mut colormap) {
                return error("file truncated");
            }
            for (i, c) in palette.colors_mut().iter_mut().enumerate().take(256) {
                c.r = colormap[i * 3];
                c.g = colormap[i * 3 + 1];
                c.b = colormap[i * 3 + 2];
            }
        } else {
            for (i, c) in palette.colors_mut().iter_mut().enumerate().take(nc) {
                c.r = pcxh.colormap[i * 3];
                c.g = pcxh.colormap[i * 3 + 1];
                c.b = pcxh.colormap[i * 3 + 2];
            }
        }
        if surface.set_palette(Some(share_palette(palette))).is_err() {
            return error("Couldn't create palette");
        }
    }

    Ok(surface)
}
