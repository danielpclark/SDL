// Rust translation of src/IMG_stb.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loading through stb_image (`USE_STBIMAGE`). Upstream compiles its own
//! copy of `stb_image.h` here, configured like SDL's (PNG and JPEG only,
//! user error messages, no SIMD); this crate uses the translation of that
//! decoder in `sdl3` ([`sdl3::video::stb_image`]).

use sdl3::error::{Error, Result};
use sdl3::io::{IoStatus, IoStream, IoWhence};
use sdl3::video::stb_image::{load_from_callbacks, load_from_callbacks_with_palette, Callbacks};
use sdl3::video::{BlendMode, PixelFormat, Surface};

/// stb_image's I/O callbacks over an [`IoStream`].
struct StreamCallbacks<'s, 'a>(&'s mut IoStream<'a>);

impl Callbacks for StreamCallbacks<'_, '_> {
    /// Translation of `IMG_LoadSTB_IO_read()`.
    fn read(&mut self, data: &mut [u8]) -> usize {
        self.0.read(data)
    }

    /// Translation of `IMG_LoadSTB_IO_skip()`.
    fn skip(&mut self, n: i32) {
        let _ = self.0.seek(n as i64, IoWhence::Cur);
    }

    /// Translation of `IMG_LoadSTB_IO_eof()`.
    fn eof(&mut self) -> bool {
        self.0.status() == IoStatus::Eof
    }
}

const STBI_GREY: i32 = 1;
const STBI_GREY_ALPHA: i32 = 2;
const STBI_RGB: i32 = 3;
const STBI_RGB_ALPHA: i32 = 4;

/// Decode the PNG or JPEG image at the stream's position: paletted PNGs as
/// INDEX8 (with a color key or blending from the palette's alpha), gray as
/// INDEX8 with a gray palette, gray+alpha as RGBA32, others as RGB24 or
/// RGBA32. On failure the stream is rewound. Translation of
/// `IMG_LoadSTB_IO()`.
pub(crate) fn load_stb_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let mut use_palette = false;
    let mut palette_colors = [0u8; 1024];

    let start = src.tell().unwrap_or(-1);

    let mut magic = [0u8; 26];
    if src.read(&mut magic) == magic.len() {
        const PNG_COLOR_INDEXED: u8 = 3;
        if magic[0] == 0x89
            && &magic[1..4] == b"PNG"
            && &magic[12..16] == b"IHDR"
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

            load_from_callbacks_with_palette(&mut callbacks, &mut palette_colors)
                .map(|image| (image, STBI_GREY))
        } else {
            load_from_callbacks(&mut callbacks, 0 /* STBI_default */).map(|image| {
                let format = image.comp;
                (image, format)
            })
        }
    };
    let (image, format) = match loaded {
        Ok(loaded) => loaded,
        Err(e) => {
            let _ = src.seek(start, IoWhence::Set);
            return Err(e);
        }
    };
    let (w, h, pixels) = (image.x, image.y, image.data);

    let surface = if use_palette {
        (|| {
            let mut surface = Surface::from_vec(w, h, PixelFormat::INDEX8, pixels, w)?;
            let mut has_colorkey = false;
            let mut colorkey_index = 0usize;
            let mut has_alpha = false;
            let palette = surface.create_palette()?;
            {
                let mut palette = palette.write().unwrap_or_else(|e| e.into_inner());
                for (i, (c, b)) in palette
                    .colors_mut()
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
                            colorkey_index = i;
                        } else {
                            /* Partial opacity or multiple colorkeys */
                            has_alpha = true;
                        }
                    }
                }
                if !has_alpha && has_colorkey {
                    /* remove redundant pixel alpha before setting colorkey */
                    palette.colors_mut()[colorkey_index].a = 255;
                }
            }
            if has_alpha {
                surface.set_blend_mode(BlendMode::BLEND)?;
            } else if has_colorkey {
                surface.set_color_key(Some(colorkey_index as u32))?;
            }

            /* FIXME: This sucks. It'd be better to allocate the surface first, then
             * write directly to the pixel buffer:
             * https://github.com/nothings/stb/issues/58
             * -flibit
             */
            Ok(surface)
        })()
    } else if format == STBI_GREY || format == STBI_RGB || format == STBI_RGB_ALPHA {
        (|| {
            let pixel_format = if format == STBI_RGB_ALPHA {
                PixelFormat::RGBA32
            } else if format == STBI_RGB {
                PixelFormat::RGB24
            } else {
                PixelFormat::INDEX8
            };
            let mut surface = Surface::from_vec(w, h, pixel_format, pixels, w * format)?;
            /* Set a grayscale palette for gray images */
            if surface.format() == PixelFormat::INDEX8 {
                let palette = surface.create_palette()?;
                let mut palette = palette.write().unwrap_or_else(|e| e.into_inner());
                for (i, c) in palette.colors_mut().iter_mut().enumerate() {
                    c.r = i as u8;
                    c.g = i as u8;
                    c.b = i as u8;
                }
            }

            /* FIXME: This sucks. It'd be better to allocate the surface first, then
             * write directly to the pixel buffer:
             * https://github.com/nothings/stb/issues/58
             * -flibit
             */
            Ok(surface)
        })()
    } else if format == STBI_GREY_ALPHA {
        (|| {
            let mut surface = Surface::new(w, h, PixelFormat::RGBA32)?;
            let pitch = surface.pitch() as usize;
            let w = w as usize;
            if let (Some(dst), true) = (surface.pixels_mut(), w > 0) {
                for (row, src_row) in pixels.chunks_exact(w * 2).enumerate() {
                    let dst_row = &mut dst[row * pitch..row * pitch + w * 4];
                    for (d, s) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(2)) {
                        let (c, a) = (s[0], s[1]);
                        d.copy_from_slice(&[c, c, c, a]);
                    }
                }
            }
            Ok(surface)
        })()
    } else {
        Err(Error::new(format!("Unknown image format: {format}")))
    };

    if surface.is_err() {
        /* The error message should already be set */
        let _ = src.seek(start, IoWhence::Set);
    }
    surface
}
