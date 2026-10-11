// Rust translation of source/plutovg-surface.c (and the surface parts of
// source/plutovg-private.h) from PlutoVG (0.0.12, as SDL_ttf's
// external/plutovg pins it).
// Copyright (c) 2020-2024 Samuel Ugochukwu <sammycageagle@gmail.com>
// This is an altered (translated) version of the original software; it is
// used under the MIT License (see PLUTOVG-LICENSE and LICENSE.txt).

//! Surfaces: ARGB32 premultiplied pixel buffers, and the decoding of
//! images (base64 data) into them.
//!
//! PlutoVG decodes images with its bundled stb_image (every format it
//! supports); this translation decodes them with the stb_image of the
//! `sdl3` crate, SDL's configuration of the same stb_image release (2.30),
//! which has its PNG and JPEG decoders only: images of stb_image's other
//! formats (BMP, GIF, PSD, TGA, HDR, PIC, PNM) are not decoded (they are
//! not drawn).

use sdl3::video::stb_image;

use super::blend::plutovg_memfill32;
use super::paint::plutovg_color_to_argb32;
use super::*;

/// `STBI_MAX_DIMENSIONS`
const STBI_MAX_DIMENSIONS: i32 = 1 << 24;

/// `struct plutovg_surface`
#[derive(Debug, Clone, Default)]
pub struct Surface {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub data: Vec<u8>,
}

impl Surface {
    /// The pixel at `index` (in pixels from the start of the data).
    #[inline]
    pub fn pixel(&self, index: usize) -> u32 {
        let i = index * 4;
        match self.data.get(i..i + 4) {
            Some(b) => u32::from_ne_bytes([b[0], b[1], b[2], b[3]]),
            None => 0,
        }
    }
}

/// `plutovg_surface_create_uninitialized`
fn plutovg_surface_create_uninitialized(width: i32, height: i32) -> Option<Surface> {
    if width > STBI_MAX_DIMENSIONS || height > STBI_MAX_DIMENSIONS {
        return None;
    }
    /* FIXME (upstream): C computes `width * height * 4` in int, which
    overflows (and then allocates less than it writes) for the large sizes
    the check above lets through; the size is computed without overflow
    here, and the allocation fails gracefully instead. */
    let size = (width.max(0) as usize)
        .checked_mul(height.max(0) as usize)?
        .checked_mul(4)?;
    let mut data = Vec::new();
    data.try_reserve_exact(size).ok()?;
    data.resize(size, 0);
    Some(Surface {
        width,
        height,
        stride: width.wrapping_mul(4),
        data,
    })
}

/// Creates a new image surface with the specified dimensions (cleared).
pub fn plutovg_surface_create(width: i32, height: i32) -> Option<Surface> {
    plutovg_surface_create_uninitialized(width, height)
}

/// Creates an image surface using existing pixel data (taking ownership
/// of it: [`plutovg_surface_into_data`] gives it back).
pub fn plutovg_surface_create_for_data(
    data: Vec<u8>,
    width: i32,
    height: i32,
    stride: i32,
) -> Surface {
    Surface {
        width,
        height,
        stride,
        data,
    }
}

/// The pixel data of a surface made by [`plutovg_surface_create_for_data`]
/// (`plutovg_surface_destroy`, for a caller-owned buffer).
pub fn plutovg_surface_into_data(surface: Surface) -> Vec<u8> {
    surface.data
}

/// `plutovg_surface_load_from_image`: an RGBA image to a surface.
fn plutovg_surface_load_from_image(image: &[u8], width: i32, height: i32) -> Option<Surface> {
    let mut surface = plutovg_surface_create_uninitialized(width, height)?;
    let (w, h, stride) = (surface.width, surface.height, surface.stride);
    plutovg_convert_rgba_to_argb_from(&mut surface.data, image, w, h, stride);
    Some(surface)
}

/// A memory buffer as stb_image's I/O callbacks (`stbi_load_from_memory`).
struct MemoryCallbacks<'a> {
    data: &'a [u8],
    pos: usize,
}

impl stb_image::Callbacks for MemoryCallbacks<'_> {
    fn read(&mut self, data: &mut [u8]) -> usize {
        let n = data.len().min(self.data.len() - self.pos);
        data[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        n
    }
    fn skip(&mut self, n: i32) {
        let pos = self.pos as i64 + n as i64;
        self.pos = pos.clamp(0, self.data.len() as i64) as usize;
    }
    fn eof(&mut self) -> bool {
        self.pos >= self.data.len()
    }
}

/// Loads an image surface from in-memory image data (PNG and JPEG: see
/// the module documentation).
pub fn plutovg_surface_load_from_image_data(data: &[u8]) -> Option<Surface> {
    let mut io = MemoryCallbacks { data, pos: 0 };
    let image = stb_image::load_from_callbacks(&mut io, 4 /* STBI_rgb_alpha */).ok()?;
    plutovg_surface_load_from_image(&image.data, image.x, image.y)
}

static BASE64_TABLE: [u8; 128] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3E, 0x00, 0x00, 0x00, 0x3F,
    0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
    0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Loads an image surface from base64-encoded image data.
pub fn plutovg_surface_load_from_image_base64(data: &[u8]) -> Option<Surface> {
    let length = data.len();
    let mut output_data = Vec::new();
    output_data.try_reserve_exact(length).ok()?;
    output_data.resize(length, 0u8);
    let mut output_length: usize = 0;

    let mut equals_sign_count: usize = 0;
    let mut sidx: usize = 0;
    let mut didx: usize = 0;

    for &cc in data {
        if cc == b'=' {
            equals_sign_count += 1;
        } else if cc == b'+' || cc == b'/' || plutovg_is_alnum(cc) {
            if equals_sign_count > 0 {
                return None;
            }
            output_data[output_length] = BASE64_TABLE[cc as usize];
            output_length += 1;
        } else if !plutovg_is_ws(cc) {
            return None;
        }
    }

    if output_length == 0 || equals_sign_count > 2 || (output_length % 4) == 1 {
        return None;
    }
    output_length -= output_length.div_ceil(4);
    if output_length == 0 {
        return None;
    }

    let o = &mut output_data;
    if output_length > 1 {
        while didx < output_length - 2 {
            o[didx] = (o[sidx] << 2) | ((o[sidx + 1] >> 4) & 0o03);
            o[didx + 1] = (o[sidx + 1] << 4) | ((o[sidx + 2] >> 2) & 0o17);
            o[didx + 2] = (o[sidx + 2] << 6) | (o[sidx + 3] & 0o77);
            sidx += 4;
            didx += 3;
        }
    }

    if didx < output_length {
        o[didx] = (o[sidx] << 2) | ((o[sidx + 1] >> 4) & 0o03);
    }
    didx += 1;
    if didx < output_length {
        o[didx] = (o[sidx + 1] << 4) | ((o[sidx + 2] >> 2) & 0o17);
    }

    plutovg_surface_load_from_image_data(&output_data[..output_length])
}

/// Gets the width of the surface.
pub fn plutovg_surface_get_width(surface: &Surface) -> i32 {
    surface.width
}

/// Gets the height of the surface.
pub fn plutovg_surface_get_height(surface: &Surface) -> i32 {
    surface.height
}

/// Gets the stride of the surface.
pub fn plutovg_surface_get_stride(surface: &Surface) -> i32 {
    surface.stride
}

/// Clears the entire surface with the specified color.
pub fn plutovg_surface_clear(surface: &mut Surface, color: &Color) {
    let pixel = plutovg_premultiply_argb(plutovg_color_to_argb32(color));
    for y in 0..surface.height {
        let start = (surface.stride as usize) * y as usize;
        plutovg_memfill32(&mut surface.data, start / 4, surface.width, pixel);
    }
}

/// Converts pixel data from premultiplied ARGB to RGBA format (in place).
pub fn plutovg_convert_argb_to_rgba(data: &mut [u8], width: i32, height: i32, stride: i32) {
    for y in 0..height as usize {
        let row = stride as usize * y;
        for x in 0..width as usize {
            let i = row + 4 * x;
            let pixel = u32::from_ne_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
            let a = (pixel >> 24) & 0xFF;
            if a == 0 {
                data[i..i + 4].fill(0);
            } else {
                let mut r = (pixel >> 16) & 0xFF;
                let mut g = (pixel >> 8) & 0xFF;
                let mut b = pixel & 0xFF;
                if a != 255 {
                    r = (r * 255) / a;
                    g = (g * 255) / a;
                    b = (b * 255) / a;
                }

                data[i] = r as u8;
                data[i + 1] = g as u8;
                data[i + 2] = b as u8;
                data[i + 3] = a as u8;
            }
        }
    }
}

/// `plutovg_convert_rgba_to_argb` (into `dst` from `src`): converts pixel
/// data from RGBA to premultiplied ARGB format.
pub fn plutovg_convert_rgba_to_argb_from(
    dst: &mut [u8],
    src: &[u8],
    width: i32,
    height: i32,
    stride: i32,
) {
    for y in 0..height as usize {
        let row = stride as usize * y;
        for x in 0..width as usize {
            let i = row + 4 * x;
            let a = src[i + 3] as u32;
            let pixel = if a == 0 {
                0x00000000
            } else {
                let mut r = src[i] as u32;
                let mut g = src[i + 1] as u32;
                let mut b = src[i + 2] as u32;
                if a != 255 {
                    r = (r * a) / 255;
                    g = (g * a) / 255;
                    b = (b * a) / 255;
                }

                (a << 24) | (r << 16) | (g << 8) | b
            };
            dst[i..i + 4].copy_from_slice(&pixel.to_ne_bytes());
        }
    }
}
