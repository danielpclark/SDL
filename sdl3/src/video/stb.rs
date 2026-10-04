// Rust translation of src/video/SDL_stb.c and of SDL_LoadSurface_IO() /
// SDL_LoadSurface() from src/video/SDL_surface.c, from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! PNG and JPEG detection, loading and saving (through the bundled
//! stb_image and miniz codecs, see [`super::image`]), MJPG conversion, and
//! loading a surface from any supported image file.

use std::path::Path;

use crate::error::{Error, Result};
use crate::io::{IoStatus, IoStream, IoWhence};
use crate::properties::Properties;
use crate::video::bmp::is_bmp;
use crate::video::image::jpeg::{jpeg_load, Nv12};
use crate::video::image::miniz::write_image_to_png_file_in_memory_ex;
use crate::video::image::stb_image::{
    load_from_callbacks, load_from_callbacks_with_palette, load_from_memory, Callbacks, Context,
};
use crate::video::pixels::{Colorspace, PixelFormat};
use crate::video::surface::{calculate_yuv_size, write_palette, Surface};
use crate::video::BlendMode;

/// MJPG to NV12, decoding straight into `dst`.
/// Translation of `SDL_ConvertPixels_MJPG_to_NV12()`.
fn convert_pixels_mjpg_to_nv12(
    width: i32,
    height: i32,
    src: &[u8],
    src_pitch: i32,
    dst: &mut [u8],
    dst_pitch: i32,
) -> Result<()> {
    let len = (src_pitch.max(0) as usize).min(src.len());
    let mut s = Context::from_mem(&src[..len]);

    // (stb writes the planes without checking the buffer)
    let needed =
        height as usize * dst_pitch as usize + (height as usize).div_ceil(2) * dst_pitch as usize;
    if dst.len() < needed {
        return Err(Error::invalid_param("dst"));
    }
    let mut nv12 = Nv12 {
        w: width,
        h: height,
        pitch: dst_pitch,
        dst,
    };

    jpeg_load(&mut s, 4, Some(&mut nv12))?;
    Ok(())
}

/// Decode compressed pixels (MJPG) to another format.
/// Translation of `SDL_ConvertPixels_STB()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_pixels_stb(
    width: i32,
    height: i32,
    src_format: PixelFormat,
    src_colorspace: Colorspace,
    _src_properties: Option<&Properties>,
    src: &[u8],
    src_pitch: i32,
    dst_format: PixelFormat,
    dst_colorspace: Colorspace,
    dst_properties: Option<&Properties>,
    dst: &mut [u8],
    dst_pitch: i32,
) -> Result<()> {
    if src_format == PixelFormat::MJPG {
        if dst_format == PixelFormat::NV12 {
            return convert_pixels_mjpg_to_nv12(width, height, src, src_pitch, dst, dst_pitch);
        } else if matches!(
            dst_format,
            PixelFormat::YV12
                | PixelFormat::IYUV
                | PixelFormat::YUY2
                | PixelFormat::UYVY
                | PixelFormat::YVYU
                | PixelFormat::NV21
                | PixelFormat::P010
        ) {
            let (temp_size, temp_pitch) = calculate_yuv_size(dst_format, width, height)?;
            // FIXME (upstream): the temporary buffer is sized for the
            // destination format but filled as NV12 with its pitch, which
            // for the packed formats (YUY2/UYVY/YVYU) writes past its end;
            // here it is made large enough for the NV12 planes.
            let nv12_size = temp_pitch * (height as usize + (height as usize).div_ceil(2));
            let mut temp_pixels = vec![0u8; temp_size.max(nv12_size)];

            convert_pixels_mjpg_to_nv12(
                width,
                height,
                src,
                src_pitch,
                &mut temp_pixels,
                temp_pitch as i32,
            )?;

            return crate::video::surface::convert_pixels_and_colorspace(
                width,
                height,
                PixelFormat::NV12,
                src_colorspace,
                None, /*props*/
                &temp_pixels,
                temp_pitch as i32,
                dst_format,
                dst_colorspace,
                dst_properties,
                dst,
                dst_pitch,
            );
        }
    }

    let len = if src_format == PixelFormat::MJPG {
        src_pitch as usize
    } else {
        height as usize * src_pitch as usize
    };
    let image = load_from_memory(&src[..len.min(src.len())], 4)?;

    if image.x == width && image.y == height {
        crate::video::surface::convert_pixels_and_colorspace(
            image.x,
            image.y,
            PixelFormat::RGBA32,
            Colorspace::SRGB,
            None,
            &image.data,
            width * 4,
            dst_format,
            dst_colorspace,
            dst_properties,
            dst,
            dst_pitch,
        )
    } else {
        Err(Error::new(format!(
            "Expected image size {width}x{height}, actual size {}x{}",
            image.x, image.y
        )))
    }
}

/// stb_image's I/O callbacks over an [`IoStream`] (`IMG_LoadSTB_IO_read()`,
/// `IMG_LoadSTB_IO_skip()` and `IMG_LoadSTB_IO_eof()`).
struct StreamCallbacks<'s, 'a>(&'s mut IoStream<'a>);

impl Callbacks for StreamCallbacks<'_, '_> {
    fn read(&mut self, data: &mut [u8]) -> usize {
        self.0.read(data)
    }

    fn skip(&mut self, n: i32) {
        let _ = self.0.seek(n as i64, IoWhence::Cur);
    }

    fn eof(&mut self) -> bool {
        self.0.status() == IoStatus::Eof
    }
}

/// Decode the PNG or JPEG image at the stream's position into a surface;
/// on failure the stream is rewound. Translation of `SDL_LoadSTB_IO()`.
fn load_stb_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let mut use_palette = false;
    let mut palette_colors = [0u8; 1024];

    // src has already been validated
    let start = src.tell().unwrap_or(-1);

    let mut magic = [0u8; 26];
    if src.read(&mut magic) == magic.len() {
        const PNG_COLOR_INDEXED: u8 = 3;
        if magic[0] == 0x89
            && magic[1] == b'P'
            && magic[2] == b'N'
            && magic[3] == b'G'
            && magic[12] == b'I'
            && magic[13] == b'H'
            && magic[14] == b'D'
            && magic[15] == b'R'
            && magic[25] == PNG_COLOR_INDEXED
        {
            use_palette = true;
        }
    }
    let _ = src.seek(start, IoWhence::Set);

    /* Load the image data */
    let loaded = {
        let mut callbacks = StreamCallbacks(&mut *src);
        if use_palette {
            /* Unused palette entries will be opaque white */
            palette_colors.fill(0xff);

            load_from_callbacks_with_palette(&mut callbacks, &mut palette_colors).map(|i| (i, 1))
        } else {
            load_from_callbacks(&mut callbacks, 0).map(|i| {
                let comp = i.comp;
                (i, comp)
            })
        }
    };
    let surface = loaded.and_then(|(image, format)| {
        let (w, h, pixels) = (image.x, image.y, image.data);
        if use_palette {
            let mut surface = Surface::from_vec(w, h, PixelFormat::INDEX8, pixels, w)?;
            let mut has_colorkey = false;
            let mut colorkey_index = 0u32;
            let mut has_alpha = false;
            if let Ok(palette) = surface.create_palette() {
                let mut p = write_palette(&palette);
                for (i, (c, b)) in p
                    .colors_mut_unversioned()
                    .iter_mut()
                    .zip(palette_colors.chunks_exact(4))
                    .enumerate()
                {
                    c.r = b[0];
                    c.g = b[1];
                    c.b = b[2];
                    c.a = b[3];
                    if c.a != 255 {
                        if c.a == 0 && !has_colorkey {
                            has_colorkey = true;
                            colorkey_index = i as u32;
                        } else {
                            /* Partial opacity or multiple colorkeys */
                            has_alpha = true;
                        }
                    }
                }
            }
            if has_alpha {
                surface.set_blend_mode(BlendMode::BLEND)?;
            } else if has_colorkey {
                surface.set_color_key(Some(colorkey_index))?;
            }
            Ok(surface)
        } else if format == 1 || format == 3 || format == 4 {
            let pixel_format = match format {
                4 => PixelFormat::RGBA32,
                3 => PixelFormat::RGB24,
                _ => PixelFormat::INDEX8,
            };
            let mut surface = Surface::from_vec(w, h, pixel_format, pixels, w * format)?;
            /* Set a grayscale palette for gray images */
            if surface.format() == PixelFormat::INDEX8 {
                if let Ok(palette) = surface.create_palette() {
                    for (i, c) in write_palette(&palette)
                        .colors_mut_unversioned()
                        .iter_mut()
                        .enumerate()
                    {
                        c.r = i as u8;
                        c.g = i as u8;
                        c.b = i as u8;
                    }
                }
            }
            Ok(surface)
        } else if format == 2 {
            let mut surface = Surface::new_uninitialized(w, h, PixelFormat::RGBA32)?;
            let pitch = surface.pitch() as usize;
            let dst = surface
                .pixels_mut()
                .ok_or_else(|| Error::invalid_param("surface"))?;
            for (row, src_row) in pixels.chunks_exact(w as usize * 2).enumerate() {
                let dst_row = &mut dst[row * pitch..row * pitch + w as usize * 4];
                for (d, s) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(2)) {
                    let (c, a) = (s[0], s[1]);
                    d.copy_from_slice(&[c, c, c, a]);
                }
            }
            Ok(surface)
        } else {
            Err(Error::new(format!("Unknown image format: {format}")))
        }
    });

    if surface.is_err() {
        /* The error message should already be set */
        let _ = src.seek(start, IoWhence::Set);
    }
    surface
}

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
        load_stb_io(src)
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
        load_stb_io(src)
    }

    /// Load a PNG image from a file. Translation of `SDL_LoadPNG()`.
    pub fn load_png(file: impl AsRef<Path>) -> Result<Surface<'static>> {
        let mut stream = IoStream::from_file(file, "rb")?;
        Surface::load_png_io(&mut stream)
    }

    /// Save the surface in PNG format: indexed surfaces as 8-bit paletted
    /// images (with the palette's alpha as `tRNS`), others as RGBA.
    /// Translation of `SDL_SavePNG_IO()`.
    pub fn save_png_io(&mut self, dst: &mut IoStream<'_>) -> Result<()> {
        let mut plte = Vec::new();
        let mut trns = Vec::new();
        let converted;
        let mut surface: &Surface<'_> = self;

        if surface.format().is_indexed() {
            if surface.palette().is_none() {
                return Err(Error::new("Indexed surfaces must have a palette"));
            }

            if surface.format() != PixelFormat::INDEX8 {
                converted = surface.convert(PixelFormat::INDEX8)?;
                surface = &converted;
            }

            let Some(palette) = surface.palette() else {
                return Err(Error::new("Indexed surfaces must have a palette"));
            };
            let colors = crate::video::surface::read_palette(palette)
                .colors()
                .to_vec();

            for c in &colors {
                plte.extend_from_slice(&[c.r, c.g, c.b]);
                trns.push(c.a);
            }
        } else if surface.format() != PixelFormat::RGBA32 {
            converted = surface.convert(PixelFormat::RGBA32)?;
            surface = &converted;
        }

        let pixels = surface.raw_pixels().unwrap_or(&[]);
        let png = write_image_to_png_file_in_memory_ex(
            pixels,
            surface.width(),
            surface.height(),
            surface.format().bytes_per_pixel() as i32,
            surface.pitch(),
            6,
            false,
            &plte,
            &trns,
        );
        match png {
            Some(png) => {
                if dst.write(&png) != 0 {
                    Ok(())
                } else {
                    Err(dst
                        .last_error()
                        .cloned()
                        .unwrap_or_else(|| Error::new("Failed to write the PNG data")))
                }
            }
            None => Err(Error::new("Failed to convert and save image")),
        }
    }

    /// Save the surface to a PNG file. Translation of `SDL_SavePNG()`.
    pub fn save_png(&mut self, file: impl AsRef<Path>) -> Result<()> {
        if self.format().is_indexed() && self.palette().is_none() {
            return Err(Error::new("Indexed surfaces must have a palette"));
        }
        let mut stream = IoStream::from_file(file, "wb")?;
        let result = self.save_png_io(&mut stream);
        let closed = stream.close();
        result.and(closed)
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
