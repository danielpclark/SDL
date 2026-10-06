// Rust translation of src/IMG_lbm.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a ILBM image file loading framework
//! Load IFF pictures, PBM & ILBM packing methods, with or without stencil
//! Written by Daniel Morais ( Daniel AT Morais DOT com ) in September 2001.
//! 24 bits ILBM files support added by Marc Le Douarain (http://www.multimania.com/mavati)
//! in December 2002.
//! EHB and HAM (specific Amiga graphic chip modes) support added by Marc Le Douarain
//! (http://www.multimania.com/mavati) in December 2003.
//! Stencil and colorkey fixes by David Raulo (david.raulo AT free DOT fr) in February 2004.
//! Buffer overflow fix in RLE decompression by David Raulo in January 2008.

// The plane loops index several buffers at once, as upstream's do.
#![allow(clippy::needless_range_loop)]

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{share_palette, Palette, PixelFormat, Surface};

use crate::util::{read_byte, read_ok};

const MAXCOLORS: usize = 256;

/* Structure for an IFF picture ( BMHD = Bitmap Header ) */

/// Translation of `BMHD`, decoded from its 20 big-endian bytes.
#[derive(Default)]
struct Bmhd {
    w: u16, /* width & height of the bitmap in pixels */
    h: u16,
    _x: i16, /* screen coordinates of the bitmap */
    _y: i16,
    planes: u8,    /* number of planes of the bitmap */
    mask: u8,      /* mask type ( 0 => no mask ) */
    tcomp: u8,     /* compression type */
    _pad1: u8,     /* dummy value, for padding */
    tcolor: u16,   /* transparent color */
    _x_aspect: u8, /* pixel aspect ratio */
    _y_aspect: u8,
    _lpage: i16, /* width of the screen in pixels */
    _hpage: i16, /* height of the screen in pixels */
}

/// `sizeof(BMHD)`.
const BMHD_SIZE: usize = 20;

impl Bmhd {
    fn parse(b: &[u8; BMHD_SIZE]) -> Bmhd {
        let be16 = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        Bmhd {
            w: be16(0),
            h: be16(2),
            _x: be16(4) as i16,
            _y: be16(6) as i16,
            planes: b[8],
            mask: b[9],
            tcomp: b[10],
            _pad1: b[11],
            tcolor: be16(12),
            _x_aspect: b[14],
            _y_aspect: b[15],
            _lpage: be16(16) as i16,
            _hpage: be16(18) as i16,
        }
    }
}

/// Whether `src` holds an IFF `PBM ` or `ILBM` picture; the stream
/// position is unchanged. Translation of `IMG_isLBM()`.
pub fn is_lbm(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_lbm = false;
    let mut magic = [0u8; 4 + 4 + 4];
    if read_ok(src, &mut magic)
        && &magic[..4] == b"FORM"
        && (&magic[8..12] == b"PBM " || &magic[8..12] == b"ILBM")
    {
        is_lbm = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_lbm
}

/// How the loader fails: with one of its messages (`error`, after which
/// the stream is rewound), or with an error SDL already set (`goto done`
/// with no `error`: the stream is left where it is).
enum Failure {
    Error(&'static str),
    Sdl(Error),
}

/// Load an IFF picture: `PBM ` (chunky) or `ILBM` (interleaved planes),
/// uncompressed or ByteRun1, with or without a stencil, as an INDEX8
/// surface; 24-plane and HAM (and HAM8) pictures as 24-bit RGB. EHB
/// palettes are extended. Translation of `IMG_LoadLBM_IO()`.
pub fn load_lbm_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);
    match load_lbm(src) {
        Ok(image) => Ok(image),
        Err(Failure::Error(error)) => {
            let _ = src.seek(start, IoWhence::Set);
            Err(Error::new(error))
        }
        Err(Failure::Sdl(e)) => Err(e),
    }
}

fn load_lbm(src: &mut IoStream<'_>) -> std::result::Result<Surface<'static>, Failure> {
    use Failure::Error as E;

    let mut id = [0u8; 4];
    let mut colormap = [0u8; MAXCOLORS * 3];

    if !read_ok(src, &mut id) {
        return Err(E("error reading IFF chunk"));
    }

    /* Should be the size of the file minus 4+4 ( 'FORM'+size ) */
    let mut size_bytes = [0u8; 4];
    if !read_ok(src, &mut size_bytes) {
        return Err(E("error reading IFF chunk size"));
    }

    /* As size is not used here, no need to swap it */

    if &id != b"FORM" {
        return Err(E("not a IFF file"));
    }

    if !read_ok(src, &mut id) {
        return Err(E("error reading IFF chunk"));
    }

    let mut pbm = false;

    /* File format : PBM=Packed Bitmap, ILBM=Interleaved Bitmap */
    if &id == b"PBM " {
        pbm = true;
    } else if &id != b"ILBM" {
        return Err(E("not a IFF picture"));
    }

    let mut nbcolors: u32 = 0;

    let mut bmhd = Bmhd::default();
    let mut flag_ham = false;
    let mut flag_ehb = false;

    while &id != b"BODY" {
        if !read_ok(src, &mut id) {
            return Err(E("error reading IFF chunk"));
        }

        if !read_ok(src, &mut size_bytes) {
            return Err(E("error reading IFF chunk size"));
        }

        let mut bytesloaded: u32 = 0;

        let mut size = u32::from_be_bytes(size_bytes);

        if &id == b"BMHD" {
            /* Bitmap header */
            let mut raw = [0u8; BMHD_SIZE];
            if !read_ok(src, &mut raw) {
                return Err(E("error reading BMHD chunk"));
            }

            bytesloaded = BMHD_SIZE as u32;

            bmhd = Bmhd::parse(&raw);
        }

        if &id == b"CMAP" {
            /* palette ( Color Map ) */
            if size as usize > colormap.len() {
                return Err(E("colormap size is too large"));
            }

            if !read_ok(src, &mut colormap[..size as usize]) {
                return Err(E("error reading CMAP chunk"));
            }

            bytesloaded = size;
            nbcolors = size / 3;
        }

        if &id == b"CAMG" {
            /* Amiga ViewMode  */
            let mut raw = [0u8; 4];
            if !read_ok(src, &mut raw) {
                return Err(E("error reading CAMG chunk"));
            }

            // FIXME (upstream): the whole chunk counts as loaded, so the
            // bytes of a chunk longer than 4 aren't skipped.
            bytesloaded = size;
            let viewmodes = u32::from_be_bytes(raw);
            if viewmodes & 0x0800 != 0 {
                flag_ham = true;
            }
            if viewmodes & 0x0080 != 0 {
                flag_ehb = true;
            }
        }

        if &id != b"BODY" {
            if size & 1 != 0 {
                size = size.wrapping_add(1); /* padding ! */
            }
            size = size.wrapping_sub(bytesloaded);
            /* skip the remaining bytes of this chunk */
            if size != 0 {
                let _ = src.seek(size as i64, IoWhence::Cur);
            }
        }
    }

    /* compute some useful values, based on the bitmap header */

    let width: u32 = (bmhd.w as u32 + 15) & 0xFFFFFFF0; /* Width in pixels modulo 16 */

    let mut bytesperline: u32 = (bmhd.w as u32).div_ceil(16) * 2;

    let mut nbplanes: u32 = bmhd.planes as u32;

    /* Sanity check: nbplanes must not exceed 8 for paletted images.
    Higher values cause 1<<nbplanes to exceed the 256-entry palette. */
    if !pbm && nbplanes > 8 && nbplanes != 24 && !flag_ham {
        return Err(Failure::Sdl(Error::new(format!(
            "LBM: invalid number of bitplanes ({nbplanes})"
        ))));
    }

    if pbm {
        /* File format : 'Packed Bitmap' */
        bytesperline *= 8;
        nbplanes = 1;
    }

    let stencil: u32 = (bmhd.mask & 1) as u32; /* There is a mask ( 'stencil' ) */

    /* Allocate memory for a temporary buffer ( used for
    decompression/deinterleaving ) */

    let mut mini_buf = vec![0u8; (bytesperline * (nbplanes + stencil)) as usize];

    let mut image = {
        let mut format = PixelFormat::INDEX8;
        if nbplanes == 24 || flag_ham {
            format = if cfg!(target_endian = "big") {
                PixelFormat::RGB24
            } else {
                PixelFormat::BGR24
            };
        }
        Surface::new(width as i32, bmhd.h as i32, format).map_err(Failure::Sdl)?
    };

    if bmhd.mask & 2 != 0 {
        /* There is a transparent color */
        let _ = image.set_color_key(Some(bmhd.tcolor as u32));
    }

    /* Update palette information */

    /* There is no palette in 24 bits ILBM file */
    if nbcolors > 0 && !flag_ham {
        /* FIXME: Should this include the stencil? See comment below */
        let mut nbrcolorsfinal: i32 = 1i32.wrapping_shl(nbplanes + stencil);

        let shared = image.create_palette().map_err(Failure::Sdl)?;
        let mut palette = shared.write().unwrap_or_else(|e| e.into_inner());
        let colors = palette.colors_mut();

        let mut ptr = 0;
        for i in 0..nbcolors as usize {
            colors[i].r = colormap[ptr];
            colors[i].g = colormap[ptr + 1];
            colors[i].b = colormap[ptr + 2];
            ptr += 3;
        }

        /* Amiga EHB mode (Extra-Half-Bright) */
        /* 6 bitplanes mode with a 32 colors palette */
        /* The 32 last colors are the same but divided by 2 */
        /* Some Amiga pictures save 64 colors with 32 last wrong colors, */
        /* they shouldn't !, and here we overwrite these 32 bad colors. */
        if nbplanes == 6 && (flag_ehb || nbcolors <= 32) {
            nbcolors = 64;
            let mut ptr = 0;
            for i in 32..64 {
                colors[i].r = colormap[ptr] / 2;
                colors[i].g = colormap[ptr + 1] / 2;
                colors[i].b = colormap[ptr + 2] / 2;
                ptr += 3;
            }
        }

        /* If nbcolors < 2^nbplanes, repeat the colormap */
        /* This happens when pictures have a stencil mask */
        if nbrcolorsfinal > (1 << nbplanes) {
            nbrcolorsfinal = 1 << nbplanes;
        }
        for i in nbcolors as usize..nbrcolorsfinal as usize {
            let from = colors[i % nbcolors as usize];
            colors[i].r = from.r;
            colors[i].g = from.g;
            colors[i].b = from.b;
        }
        if !pbm {
            // (`palette->ncolors = nbrcolorsfinal`)
            let kept = colors[..nbrcolorsfinal as usize].to_vec();
            drop(palette);
            let shrunk = Palette::from_colors(kept).map_err(Failure::Sdl)?;
            image
                .set_palette(Some(share_palette(shrunk)))
                .map_err(Failure::Sdl)?;
        }
    }

    /* Get the bitmap */

    let pixels: &mut [u8] = image.pixels_mut().unwrap_or(&mut []);
    let width = width as usize;
    let bytesperline = bytesperline as usize;
    for h in 0..bmhd.h as usize {
        /* uncompress the datas of each planes */

        for plane in 0..(nbplanes + stencil) as usize {
            let mut ptr = plane * bytesperline;

            let mut remainingbytes = bytesperline;

            if bmhd.tcomp == 1 {
                /* Datas are compressed */
                loop {
                    let Some(mut count) = read_byte(src) else {
                        return Err(E("error reading BODY chunk"));
                    };

                    if count & 0x80 != 0 {
                        count ^= 0xFF;
                        count += 2; /* now it */

                        if count as usize > remainingbytes {
                            return Err(E("error reading BODY chunk"));
                        }
                        let Some(color) = read_byte(src) else {
                            return Err(E("error reading BODY chunk"));
                        };
                        mini_buf[ptr..ptr + count as usize].fill(color);
                    } else {
                        count += 1;

                        if count as usize > remainingbytes
                            || !read_ok(src, &mut mini_buf[ptr..ptr + count as usize])
                        {
                            return Err(E("error reading BODY chunk"));
                        }
                    }

                    ptr += count as usize;
                    remainingbytes -= count as usize;

                    if remainingbytes == 0 {
                        break;
                    }
                }
            } else if !read_ok(src, &mut mini_buf[ptr..ptr + bytesperline]) {
                return Err(E("error reading BODY chunk"));
            }
        }

        /* One line has been read, store it ! */

        let mut ptr = if nbplanes == 24 || flag_ham {
            h * width * 3
        } else {
            h * width
        };

        if pbm {
            /* File format : 'Packed Bitmap' */
            pixels[ptr..ptr + width].copy_from_slice(&mini_buf[..width]);
        } else {
            /* We have to un-interlace the bits ! */
            if nbplanes != 24 && !flag_ham {
                let size = width.div_ceil(8);

                for i in 0..size {
                    pixels[ptr..ptr + 8].fill(0);

                    for plane in 0..(nbplanes + stencil) as usize {
                        let color = mini_buf[i + plane * bytesperline];
                        let mut msk: u8 = 0x80;

                        for j in 0..8 {
                            let bit = (color & msk) as u32;
                            if plane + j <= 7 {
                                pixels[ptr + j] |= (bit >> (7 - plane - j)) as u8;
                            } else {
                                pixels[ptr + j] |= (bit << (plane + j - 7)) as u8;
                            }

                            msk >>= 1;
                        }
                    }
                    ptr += 8;
                }
            } else {
                let mut finalcolor: u32 = 0;
                let size = width.div_ceil(8);
                /* 24 bitplanes ILBM : R0...R7,G0...G7,B0...B7 */
                /* or HAM (6 bitplanes) or HAM8 (8 bitplanes) modes */
                for i in (0..width).step_by(8) {
                    let mut mask_bit: u8 = 0x80;
                    for _j in 0..8 {
                        let mut pixelcolor: u32 = 0;
                        let mut mask_color: u32 = 1;
                        for plane in 0..nbplanes as usize {
                            let data_body = mini_buf[plane * size + i / 8];
                            if data_body & mask_bit != 0 {
                                pixelcolor |= mask_color;
                            }
                            mask_color <<= 1;
                        }
                        /* HAM : 12 bits RGB image (4 bits per color component) */
                        /* HAM8 : 18 bits RGB image (6 bits per color component) */
                        if flag_ham {
                            // FIXME (upstream): with fewer than 2 or more
                            // than 26 planes the shift counts are out of
                            // range (here, as on x86, taken modulo 32), and
                            // with more than 10 a direct color can index
                            // past the colormap (here, reading 0).
                            match pixelcolor.wrapping_shr(nbplanes.wrapping_sub(2)) {
                                0 => {
                                    /* take direct color from palette */
                                    let c = |i: usize| -> u32 {
                                        colormap.get(i).copied().unwrap_or(0) as u32
                                    };
                                    let p = pixelcolor as usize * 3;
                                    finalcolor = c(p) + (c(p + 1) << 8) + (c(p + 2) << 16);
                                }
                                1 => {
                                    /* modify only blue component */
                                    finalcolor &= 0x00FFFF;
                                    finalcolor |= pixelcolor.wrapping_shl(
                                        16u32.wrapping_add(10u32.wrapping_sub(nbplanes)),
                                    );
                                }
                                2 => {
                                    /* modify only red component */
                                    finalcolor &= 0xFFFF00;
                                    finalcolor |=
                                        pixelcolor.wrapping_shl(10u32.wrapping_sub(nbplanes));
                                }
                                3 => {
                                    /* modify only green component */
                                    finalcolor &= 0xFF00FF;
                                    finalcolor |= pixelcolor.wrapping_shl(
                                        8u32.wrapping_add(10u32.wrapping_sub(nbplanes)),
                                    );
                                }
                                _ => {}
                            }
                        } else {
                            finalcolor = pixelcolor;
                        }
                        if cfg!(target_endian = "little") {
                            pixels[ptr] = (finalcolor >> 16) as u8;
                            pixels[ptr + 1] = (finalcolor >> 8) as u8;
                            pixels[ptr + 2] = finalcolor as u8;
                        } else {
                            pixels[ptr] = finalcolor as u8;
                            pixels[ptr + 1] = (finalcolor >> 8) as u8;
                            pixels[ptr + 2] = (finalcolor >> 16) as u8;
                        }
                        ptr += 3;
                        mask_bit >>= 1;
                    }
                }
            }
        }
    }

    Ok(image)
}
