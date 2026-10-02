// Rust translation of the conversion functions of src/video/SDL_surface.c
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::*;
use crate::video::blit::{COPY_RLE_ALPHAKEY, COPY_RLE_COLORKEY};
use crate::video::pixels::{ALPHA_OPAQUE, ALPHA_TRANSPARENT};

impl Surface<'_> {
    /// Translation of `SDL_ConvertSurfaceRectAndColorspace()`.
    ///
    /// Upstream temporarily resets the source's blit settings (and tweaks
    /// its palette's alpha) around the conversion blit. The settings are
    /// reset on a view of the source here, so the source itself is not
    /// touched; the palette tweak is the same, through the shared palette.
    fn convert_rect_and_colorspace(
        &self,
        rect: Option<&Rect>,
        format: PixelFormat,
        palette: Option<&SharedPalette>,
        mut colorspace: Colorspace,
        props: Option<&Properties>,
    ) -> Result<Surface<'static>> {
        if format == PixelFormat::UNKNOWN {
            return Err(Error::invalid_param("format"));
        }

        // Set the bounds of the new surface
        let mut bounds = Rect::new(0, 0, self.w, self.h);
        let rect = match rect {
            Some(r) => {
                bounds.w = r.w;
                bounds.h = r.h;
                *r
            }
            None => bounds,
        };

        // Check for empty destination palette! (results in empty image)
        let mut palette: Option<SharedPalette> = palette.cloned();
        if let Some(p) = &palette {
            let pal = read_palette(p);
            if pal
                .colors()
                .iter()
                .all(|c| c.r == 0xFF && c.g == 0xFF && c.b == 0xFF)
            {
                return Err(Error::new("Empty destination palette"));
            }
        } else if format.is_indexed() {
            // Create a dither palette for conversion
            if let Ok(mut temp_palette) = Palette::new(1 << format.bits_per_pixel()) {
                temp_palette.dither();
                palette = Some(share_palette(temp_palette));
            }
        }

        let src_colorspace = self.colorspace;
        let src_properties = self.props.as_ref();

        // Create a new surface with the desired format
        let mut convert = if self.pixels.is_some() || self.must_lock() {
            Surface::new_uninitialized(rect.w, rect.h, format)?
        } else {
            Surface::without_pixels(rect.w, rect.h, format)?
        };
        if format.is_indexed() {
            let _ = convert.set_palette(palette.clone());
        }

        if colorspace == Colorspace::UNKNOWN {
            colorspace = src_colorspace;
        }
        convert.set_colorspace(colorspace);

        let copy_flags;
        if format.is_fourcc() || self.format.is_fourcc() {
            if self.format == PixelFormat::MJPG && format == PixelFormat::MJPG {
                // Just do a straight pixel copy of the JPEG image
                let size = self.pitch as usize;
                let src = self.raw_pixels().unwrap_or(&[]);
                let data = src[..size.min(src.len())].to_vec();
                convert.pixels = Pixels::Owned {
                    len: data.len(),
                    buf: data,
                    offset: 0,
                };
                convert.flags.remove(SurfaceFlags::PREALLOCATED);
                convert.pitch = self.pitch;
            } else {
                let (cw, cformat, cpitch) = (convert.w, convert.format, convert.pitch);
                let _ = cw;
                let dst = convert.pixels.bytes_mut().unwrap_or(&mut []);
                convert_pixels_and_colorspace(
                    self.w,
                    self.h,
                    self.format,
                    src_colorspace,
                    src_properties,
                    self.pixels.bytes().unwrap_or(&[]),
                    self.pitch,
                    cformat,
                    colorspace,
                    props,
                    dst,
                    cpitch,
                )?;
            }

            // Save the original copy flags
            copy_flags = self.map.flags;
        } else {
            // Save the original copy flags
            copy_flags = self.map.flags;
            let copy_color = Color::new(self.map.r, self.map.g, self.map.b, self.map.a);
            let mut source = self.view(None)?;
            source.map.r = 0xFF;
            source.map.g = 0xFF;
            source.map.b = 0xFF;
            source.map.a = 0xFF;
            source.map.flags = copy_flags & (COPY_RLE_COLORKEY | COPY_RLE_ALPHAKEY);
            source.map.colorkey = self.map.colorkey;

            /* Source surface has a palette with no real alpha (0 or OPAQUE).
             * Destination format has alpha.
             * -> set alpha channel to be opaque */
            let mut palette_saved_alpha: Option<Vec<u8>> = None;
            if let Some(sp) = &self.palette {
                if format.has_alpha() {
                    let mut set_opaque = false;

                    let info = read_palette(sp).alpha_info();
                    if info.is_opaque && !info.has_alpha_channel {
                        set_opaque = true;
                    }

                    // Set opaque and backup palette alpha values
                    if set_opaque {
                        let mut pal = write_palette(sp);
                        let colors = pal.colors_mut_unversioned();
                        palette_saved_alpha = Some(colors.iter().map(|c| c.a).collect());
                        for c in colors.iter_mut() {
                            c.a = ALPHA_OPAQUE;
                        }
                    }
                }
            }

            // Transform colorkey to alpha. for cases where source palette has duplicate values, and colorkey is one of them
            let mut palette_ck_transform = false;
            let mut palette_ck_value = 0u8;
            if copy_flags & COPY_COLORKEY != 0 {
                if let (Some(sp), None) = (&self.palette, &palette) {
                    let mut pal = write_palette(sp);
                    let ck = self.map.colorkey as usize;
                    if let Some(c) = pal.colors_mut_unversioned().get_mut(ck) {
                        palette_ck_transform = true;
                        palette_ck_value = c.a;
                        c.a = ALPHA_TRANSPARENT;
                    }
                }
            }

            let result = if self.pixels.is_some() || self.must_lock() {
                source.blit_unchecked(&rect, &mut convert, &bounds)
            } else {
                Ok(())
            };
            drop(source);

            // Restore colorkey alpha value
            if palette_ck_transform {
                if let Some(sp) = &self.palette {
                    let mut pal = write_palette(sp);
                    pal.colors_mut_unversioned()[self.map.colorkey as usize].a = palette_ck_value;
                }
            }

            // Restore palette alpha values
            if let (Some(saved), Some(sp)) = (palette_saved_alpha, &self.palette) {
                let mut pal = write_palette(sp);
                for (c, a) in pal.colors_mut_unversioned().iter_mut().zip(saved) {
                    c.a = a;
                }
            }

            // Clean up the original surface, and update converted surface
            convert.map.r = copy_color.r;
            convert.map.g = copy_color.g;
            convert.map.b = copy_color.b;
            convert.map.a = copy_color.a;
            convert.map.flags = copy_flags
                & !(COPY_COLORKEY
                    | COPY_BLEND
                    | COPY_RLE_DESIRED
                    | COPY_RLE_COLORKEY
                    | COPY_RLE_ALPHAKEY);

            // SDL_BlitSurfaceUnchecked failed, and so the conversion
            result?;

            if copy_flags & COPY_COLORKEY != 0 {
                let mut set_colorkey_by_color = false;
                let mut convert_colorkey = true;

                if let Some(sp) = &self.palette {
                    let identical = palette.as_ref().is_some_and(|p| {
                        Arc::ptr_eq(sp, p) || read_palette(sp).is_prefix_of(&read_palette(p))
                    });
                    if identical {
                        // The palette is identical, just set the same colorkey
                        let _ = convert.set_color_key(Some(self.map.colorkey));
                    } else if palette.is_none() {
                        if format.has_alpha() {
                            // No need to add the colorkey, transparency is in the alpha channel
                        } else {
                            // Only set the colorkey information
                            set_colorkey_by_color = true;
                            convert_colorkey = false;
                        }
                    } else {
                        set_colorkey_by_color = true;
                    }
                } else {
                    set_colorkey_by_color = true;
                }

                if set_colorkey_by_color {
                    // Create a dummy surface to get the colorkey converted
                    let mut tmp = Surface::new_uninitialized(1, 1, self.format)?;

                    // Share the palette, if any
                    if self.palette.is_some() {
                        let _ = tmp.set_palette(self.palette.clone());
                    }

                    // The result is ignored upstream (the fill fails for formats it
                    // doesn't support, leaving the zeroed pixel).
                    let _ = tmp.fill_rect(None, self.map.colorkey);

                    tmp.map.flags &= !COPY_COLORKEY;

                    // Conversion of the colorkey
                    let tmp2 =
                        tmp.convert_with_colorspace(format, palette.as_ref(), colorspace, props)?;

                    // Get the converted colorkey
                    // FIXME (upstream): this memcpy()s bytes_per_pixel bytes into an
                    // int, overflowing it for 8- and 16-byte formats; the first four
                    // bytes are what ends up in the int on a little endian machine.
                    let bpp = tmp2.fmt.bytes_per_pixel as usize;
                    let mut ck = [0u8; 4];
                    let n = bpp.min(4);
                    if let Some(px) = tmp2.pixels.bytes() {
                        ck[..n].copy_from_slice(&px[..n]);
                    }
                    let converted_colorkey = u32::from_ne_bytes(ck);

                    // Set the converted colorkey on the new surface
                    let _ = convert.set_color_key(Some(converted_colorkey));

                    // This is needed when converting for 3D texture upload
                    if convert_colorkey {
                        convert.convert_colorkey_to_alpha(true);
                    }
                }
            }
        }

        // end:
        convert.set_clip_rect(Some(&self.clip_rect));

        /* Enable alpha blending by default if the new surface has an
         * alpha channel or alpha modulation */
        if format.has_alpha() || (copy_flags & COPY_MODULATE_ALPHA) != 0 {
            let _ = convert.set_blend_mode(BlendMode::BLEND);
        }
        if copy_flags & COPY_RLE_DESIRED != 0 {
            let _ = convert.set_rle(true);
        }

        // Copy alternate images (owned here, so duplicated)
        for image in &self.images {
            convert.add_alternate_image(image.duplicate()?);
        }

        // Copy properties
        if let Some(sp) = &self.props {
            convert.properties().copy_from(sp)?;

            // Make sure the new surface doesn't reference an old SDL2 surface.
            convert.properties().remove("sdl2-compat.surface2");
        }

        // We're ready to go!
        Ok(convert)
    }

    /// Copy this surface into a new one of `format`, with the given palette
    /// (for indexed formats), colorspace and extra properties.
    /// Translation of `SDL_ConvertSurfaceAndColorspace()`.
    pub fn convert_with_colorspace(
        &self,
        format: PixelFormat,
        palette: Option<&SharedPalette>,
        colorspace: Colorspace,
        props: Option<&Properties>,
    ) -> Result<Surface<'static>> {
        self.convert_rect_and_colorspace(None, format, palette, colorspace, props)
    }

    /// Copy this surface, keeping its format, palette, colorspace,
    /// properties and blit settings. Translation of `SDL_DuplicateSurface()`.
    pub fn duplicate(&self) -> Result<Surface<'static>> {
        let palette = self.palette.clone();
        self.convert_with_colorspace(
            self.format,
            palette.as_ref(),
            self.colorspace,
            self.props.as_ref(),
        )
    }

    /// A copy of this surface scaled to `width` x `height`.
    /// Translation of `SDL_ScaleSurface()`.
    pub fn scale(
        &self,
        width: i32,
        height: i32,
        mut scale_mode: ScaleMode,
    ) -> Result<Surface<'static>> {
        if self.format.is_fourcc() {
            // We can't directly scale a YUV surface (yet!)
            let tmp = self.convert(PixelFormat::ARGB8888)?;

            let scaled = tmp.scale(width, height, scale_mode)?;

            return scaled.convert_with_colorspace(
                self.format,
                None,
                self.colorspace,
                self.props.as_ref(),
            );
        }

        if self.format.is_indexed() {
            // Linear scaling requires conversion to RGBA and then slow pixel color lookup
            scale_mode = ScaleMode::Nearest;
        }

        // Create a new surface with the desired size
        let mut convert = if self.pixels.is_some() || self.must_lock() {
            Surface::new_uninitialized(width, height, self.format)?
        } else {
            Surface::without_pixels(width, height, self.format)?
        };
        let _ = convert.set_palette(self.palette.clone());
        convert.set_colorspace(self.colorspace);
        let _ = convert.set_rle(self.has_rle());

        if self.pixels.is_some() || self.must_lock() {
            // Save the original copy flags
            let copy_flags = self.map.flags;
            let copy_color = Color::new(self.map.r, self.map.g, self.map.b, self.map.a);
            let mut source = self.view(None)?;
            source.map.r = 0xFF;
            source.map.g = 0xFF;
            source.map.b = 0xFF;
            source.map.a = 0xFF;
            source.map.flags = copy_flags & (COPY_RLE_COLORKEY | COPY_RLE_ALPHAKEY);

            let rc = source.blit_scaled(None, &mut convert, None, scale_mode);

            // Clean up the original surface, and update converted surface
            convert.map.r = copy_color.r;
            convert.map.g = copy_color.g;
            convert.map.b = copy_color.b;
            convert.map.a = copy_color.a;
            convert.map.flags = copy_flags & !(COPY_RLE_COLORKEY | COPY_RLE_ALPHAKEY);

            // SDL_BlitSurfaceScaled failed, and so the conversion
            rc?;
        }

        // We're ready to go!
        Ok(convert)
    }

    /// Copy `rect` of this surface into a new surface of `format`.
    /// Translation of `SDL_ConvertSurfaceRect()`.
    pub(crate) fn convert_rect(
        &self,
        rect: Option<&Rect>,
        format: PixelFormat,
    ) -> Result<Surface<'static>> {
        self.convert_rect_and_colorspace(
            rect,
            format,
            None,
            format.default_colorspace(),
            self.props.as_ref(),
        )
    }

    /// Copy this surface into a new surface of `format`. Translation of `SDL_ConvertSurface()`.
    pub fn convert(&self, format: PixelFormat) -> Result<Surface<'static>> {
        self.convert_rect(None, format)
    }

    /// Premultiply the alpha in this surface's pixels, in place.
    /// Translation of `SDL_PremultiplySurfaceAlpha()`.
    pub fn premultiply_alpha(&mut self, linear: bool) -> Result<()> {
        let colorspace = self.colorspace;
        let (w, h, format, pitch) = (self.w, self.h, self.format, self.pitch);
        let props = self.props.clone();
        let pixels = match self.pixels.bytes_mut() {
            Some(p) => p,
            None => return Err(Error::invalid_param("src")),
        };
        premultiply_alpha_pixels_and_colorspace(
            w,
            h,
            format,
            colorspace,
            props.as_ref(),
            PremultiplyBuffers::InPlace(pixels),
            pitch,
            format,
            colorspace,
            props.as_ref(),
            pitch,
            linear,
        )
    }

    /// Clear the surface (ignoring the clip rectangle) to a color given as
    /// floats in 0..1 (or beyond, for HDR formats). Translation of `SDL_ClearSurface()`.
    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) -> Result<()> {
        let clip_rect = self.clip_rect();
        self.set_clip_rect(None);

        let result = if !self.format.is_fourcc() && self.format.bytes_per_pixel() <= 4 {
            let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let color = self.map_rgba(to8(r), to8(g), to8(b), to8(a));
            self.fill_rect(None, color)
        } else if self.format.is_fourcc() {
            // We can't directly set an RGB value on a YUV surface
            (|| {
                let mut tmp = Surface::new_uninitialized(self.w, self.h, PixelFormat::ARGB8888)?;
                tmp.clear(r, g, b, a)?;
                let (w, h, format, colorspace, pitch) =
                    (self.w, self.h, self.format, self.colorspace, self.pitch);
                let props = self.props.clone();
                let dst = self
                    .pixels
                    .bytes_mut()
                    .ok_or_else(|| Error::invalid_param("dst"))?;
                convert_pixels_and_colorspace(
                    w,
                    h,
                    tmp.format,
                    tmp.colorspace,
                    tmp.props.as_ref(),
                    tmp.pixels.bytes().unwrap_or(&[]),
                    tmp.pitch,
                    format,
                    colorspace,
                    props.as_ref(),
                    dst,
                    pitch,
                )
            })()
        } else {
            // Take advantage of blit color conversion
            (|| {
                let mut tmp = Surface::new_uninitialized(1, 1, PixelFormat::RGBA128_FLOAT)?;
                tmp.set_colorspace(self.colorspace);
                tmp.set_blend_mode(BlendMode::NONE)?;

                if let Some(px) = tmp.pixels.bytes_mut() {
                    for (k, v) in [r, g, b, a].iter().enumerate() {
                        px[4 * k..4 * k + 4].copy_from_slice(&v.to_ne_bytes());
                    }
                }

                tmp.blit_scaled(None, self, None, ScaleMode::Nearest)
            })()
        };

        self.set_clip_rect(Some(&clip_rect));

        result
    }
}

/// Copy rows of pixels into a new surface. Translation of `SDL_DuplicatePixels()`.
#[allow(dead_code)] // for the window and clipboard code
pub(crate) fn duplicate_pixels(
    width: i32,
    height: i32,
    format: PixelFormat,
    colorspace: Colorspace,
    pixels: Option<&[u8]>,
    pitch: i32,
) -> Result<Surface<'static>> {
    let mut surface = if pixels.is_some() {
        Surface::new_uninitialized(width, height, format)?
    } else {
        Surface::without_pixels(width, height, format)?
    };
    surface.set_colorspace(colorspace);

    let dst_pitch = surface.pitch as usize;
    if let (Some(dst), Some(src)) = (surface.pixels.bytes_mut(), pixels) {
        let length = (width * format.bytes_per_pixel() as i32) as usize;
        for row in 0..height as usize {
            let s = row * pitch as usize;
            dst[row * dst_pitch..row * dst_pitch + length].copy_from_slice(&src[s..s + length]);
        }
    }
    Ok(surface)
}

/// Convert a block of pixels from one format and colorspace to another.
/// Translation of `SDL_ConvertPixelsAndColorspace()`.
#[allow(clippy::too_many_arguments)]
pub fn convert_pixels_and_colorspace(
    width: i32,
    height: i32,
    src_format: PixelFormat,
    mut src_colorspace: Colorspace,
    src_properties: Option<&Properties>,
    src: &[u8],
    src_pitch: i32,
    dst_format: PixelFormat,
    mut dst_colorspace: Colorspace,
    dst_properties: Option<&Properties>,
    dst: &mut [u8],
    dst_pitch: i32,
) -> Result<()> {
    if src.is_empty() {
        return Err(Error::invalid_param("src"));
    }
    if src_pitch == 0 {
        return Err(Error::invalid_param("src_pitch"));
    }
    if dst.is_empty() {
        return Err(Error::invalid_param("dst"));
    }
    if dst_pitch == 0 {
        return Err(Error::invalid_param("dst_pitch"));
    }

    if src_colorspace == Colorspace::UNKNOWN {
        src_colorspace = src_format.default_colorspace();
    }
    if dst_colorspace == Colorspace::UNKNOWN {
        dst_colorspace = dst_format.default_colorspace();
    }

    if src_format == PixelFormat::MJPG {
        // SDL_ConvertPixels_STB() in a build without stb_image.
        return Err(Error::new("SDL not built with STB image support"));
    }

    if src_format.is_fourcc() || dst_format.is_fourcc() {
        return Err(Error::new("SDL not built with YUV support"));
    }

    // Fast path for same format copy
    if src_format == dst_format && src_colorspace == dst_colorspace {
        if src_pitch == dst_pitch {
            let n = height as usize * src_pitch as usize;
            dst[..n].copy_from_slice(&src[..n]);
        } else {
            let bpp = src_format.bytes_per_pixel() as usize;
            let width = width as usize * bpp;
            for row in 0..height as usize {
                let s = row * src_pitch as usize;
                let d = row * dst_pitch as usize;
                dst[d..d + width].copy_from_slice(&src[s..s + width]);
            }
        }
        return Ok(());
    }

    let mut src_surface = Surface::from_const_pixels(
        width,
        height,
        src_format,
        src_colorspace,
        src_properties,
        src,
        src_pitch,
    )?;
    src_surface.set_blend_mode(BlendMode::NONE)?;

    let mut dst_surface = Surface::from_mut_pixels_with(
        width,
        height,
        dst_format,
        dst_colorspace,
        dst_properties,
        dst,
        dst_pitch,
    )?;

    // Set up the rect and go!
    let rect = Rect::new(0, 0, width, height);
    src_surface.blit_unchecked(&rect, &mut dst_surface, &rect)
}

/// Convert a block of pixels from one format to another (in their default
/// colorspaces). Translation of `SDL_ConvertPixels()`.
#[allow(clippy::too_many_arguments)]
pub fn convert_pixels(
    width: i32,
    height: i32,
    src_format: PixelFormat,
    src: &[u8],
    src_pitch: i32,
    dst_format: PixelFormat,
    dst: &mut [u8],
    dst_pitch: i32,
) -> Result<()> {
    convert_pixels_and_colorspace(
        width,
        height,
        src_format,
        Colorspace::UNKNOWN,
        None,
        src,
        src_pitch,
        dst_format,
        Colorspace::UNKNOWN,
        None,
        dst,
        dst_pitch,
    )
}

/*
 * Premultiply the alpha on a block of pixels
 *
 * Here are some ideas for optimization:
 * https://github.com/Wizermil/premultiply_alpha/tree/master/premultiply_alpha
 * https://developer.arm.com/documentation/101964/0201/Pre-multiplied-alpha-channel-data
 */

/// The source and destination of a premultiply pass: distinct buffers, or
/// one buffer converted in place (upstream passes the same pointer twice).
enum PremultiplyBuffers<'s, 'd> {
    Separate(&'s [u8], &'d mut [u8]),
    InPlace(&'d mut [u8]),
}

/// Run `f` on every 32-bit pixel, reading `src` rows and writing `dst` rows.
fn for_each_u32(
    width: i32,
    height: i32,
    bufs: &mut PremultiplyBuffers<'_, '_>,
    src_pitch: i32,
    dst_pitch: i32,
    f: impl Fn(u32) -> u32,
) {
    for row in 0..height as usize {
        for c in 0..width as usize {
            let si = row * src_pitch as usize + 4 * c;
            let di = row * dst_pitch as usize + 4 * c;
            match bufs {
                PremultiplyBuffers::Separate(s, d) => {
                    let v = f(crate::video::blit::rd32(s, si));
                    crate::video::blit::wr32(d, di, v);
                }
                PremultiplyBuffers::InPlace(b) => {
                    let v = f(crate::video::blit::rd32(b, si));
                    crate::video::blit::wr32(b, di, v);
                }
            }
        }
    }
}

/// Translation of `SDL_PremultiplyAlpha_AXYZ8888()`.
fn premultiply_alpha_axyz8888(
    width: i32,
    height: i32,
    bufs: &mut PremultiplyBuffers<'_, '_>,
    src_pitch: i32,
    dst_pitch: i32,
) {
    for_each_u32(width, height, bufs, src_pitch, dst_pitch, |srcpixel| {
        // Component bytes extraction.
        let (src_r, src_g, src_b, src_a) = crate::video::blit::rgba_from_argb8888(srcpixel);

        // Alpha pre-multiplication of each component.
        let dst_a = src_a;
        let dst_r = (src_a * src_r) / 255;
        let dst_g = (src_a * src_g) / 255;
        let dst_b = (src_a * src_b) / 255;

        // ARGB8888 pixel recomposition.
        crate::video::blit::argb8888_from_rgba(dst_r, dst_g, dst_b, dst_a)
    });
}

/// Translation of `SDL_PremultiplyAlpha_XYZA8888()`.
fn premultiply_alpha_xyza8888(
    width: i32,
    height: i32,
    bufs: &mut PremultiplyBuffers<'_, '_>,
    src_pitch: i32,
    dst_pitch: i32,
) {
    for_each_u32(width, height, bufs, src_pitch, dst_pitch, |srcpixel| {
        // Component bytes extraction.
        let (src_r, src_g, src_b, src_a) = crate::video::blit::rgba_from_rgba8888(srcpixel);

        // Alpha pre-multiplication of each component.
        let dst_a = src_a;
        let dst_r = (src_a * src_r) / 255;
        let dst_g = (src_a * src_g) / 255;
        let dst_b = (src_a * src_b) / 255;

        // RGBA8888 pixel recomposition.
        crate::video::blit::rgba8888_from_rgba(dst_r, dst_g, dst_b, dst_a)
    });
}

/// Translation of `SDL_PremultiplyAlpha_AXYZ128()`.
fn premultiply_alpha_axyz128(
    width: i32,
    height: i32,
    bufs: &mut PremultiplyBuffers<'_, '_>,
    src_pitch: i32,
    dst_pitch: i32,
) {
    let rdf = |b: &[u8], i: usize| f32::from_bits(crate::video::blit::rd32(b, i));
    for row in 0..height as usize {
        for c in 0..width as usize {
            let si = row * src_pitch as usize + 16 * c;
            let di = row * dst_pitch as usize + 16 * c;
            let s: &[u8] = match bufs {
                PremultiplyBuffers::Separate(s, _) => s,
                PremultiplyBuffers::InPlace(b) => b,
            };
            let fl_a = rdf(s, si);
            let mut fl_r = rdf(s, si + 4);
            let mut fl_g = rdf(s, si + 8);
            let mut fl_b = rdf(s, si + 12);

            // Alpha pre-multiplication of each component.
            fl_r *= fl_a;
            fl_g *= fl_a;
            fl_b *= fl_a;

            let d: &mut [u8] = match bufs {
                PremultiplyBuffers::Separate(_, d) => d,
                PremultiplyBuffers::InPlace(b) => b,
            };
            for (k, v) in [fl_a, fl_r, fl_g, fl_b].iter().enumerate() {
                crate::video::blit::wr32(d, di + 4 * k, v.to_bits());
            }
        }
    }
}

/// Translation of `SDL_PremultiplyAlphaPixelsAndColorspace()`.
#[allow(clippy::too_many_arguments)]
fn premultiply_alpha_pixels_and_colorspace(
    width: i32,
    height: i32,
    src_format: PixelFormat,
    src_colorspace: Colorspace,
    src_properties: Option<&Properties>,
    bufs: PremultiplyBuffers<'_, '_>,
    src_pitch: i32,
    dst_format: PixelFormat,
    dst_colorspace: Colorspace,
    dst_properties: Option<&Properties>,
    dst_pitch: i32,
    linear: bool,
) -> Result<()> {
    let (src_empty, dst_empty) = match &bufs {
        PremultiplyBuffers::Separate(s, d) => (s.is_empty(), d.is_empty()),
        PremultiplyBuffers::InPlace(b) => (b.is_empty(), b.is_empty()),
    };
    if src_empty {
        return Err(Error::invalid_param("src"));
    }
    if src_pitch == 0 {
        return Err(Error::invalid_param("src_pitch"));
    }
    if dst_empty {
        return Err(Error::invalid_param("dst"));
    }
    if dst_pitch == 0 {
        return Err(Error::invalid_param("dst_pitch"));
    }

    // Use a high precision format if we're converting to linear colorspace or using high precision pixel formats
    let format = if linear
        || src_format.is_10bit()
        || src_format.bits_per_pixel() > 32
        || dst_format.is_10bit()
        || dst_format.bits_per_pixel() > 32
    {
        if src_format == PixelFormat::ARGB128_FLOAT || src_format == PixelFormat::ABGR128_FLOAT {
            src_format
        } else {
            PixelFormat::ARGB128_FLOAT
        }
    } else if matches!(
        src_format,
        PixelFormat::ARGB8888
            | PixelFormat::ABGR8888
            | PixelFormat::RGBA8888
            | PixelFormat::BGRA8888
    ) {
        src_format
    } else {
        PixelFormat::ARGB8888
    };
    let colorspace = if linear {
        Colorspace::SRGB_LINEAR
    } else {
        Colorspace::SRGB
    };

    let run = |format: PixelFormat,
               bufs: &mut PremultiplyBuffers<'_, '_>,
               sp: i32,
               dp: i32|
     -> Result<()> {
        match format {
            PixelFormat::ARGB8888 | PixelFormat::ABGR8888 => {
                premultiply_alpha_axyz8888(width, height, bufs, sp, dp)
            }
            PixelFormat::RGBA8888 | PixelFormat::BGRA8888 => {
                premultiply_alpha_xyza8888(width, height, bufs, sp, dp)
            }
            PixelFormat::ARGB128_FLOAT | PixelFormat::ABGR128_FLOAT => {
                premultiply_alpha_axyz128(width, height, bufs, sp, dp)
            }
            _ => return Err(Error::new("Unexpected internal pixel format")),
        }
        Ok(())
    };

    let src_bytes: &[u8] = match &bufs {
        PremultiplyBuffers::Separate(s, _) => s,
        PremultiplyBuffers::InPlace(b) => b,
    };

    if src_format != format || src_colorspace != colorspace {
        let mut convert = Surface::new_uninitialized(width, height, format)?;
        let cpitch = convert.pitch;
        convert_pixels_and_colorspace(
            width,
            height,
            src_format,
            src_colorspace,
            src_properties,
            src_bytes,
            src_pitch,
            format,
            colorspace,
            None,
            convert.pixels.bytes_mut().unwrap_or(&mut []),
            cpitch,
        )?;

        // src = dst = convert->pixels
        {
            let buf = convert.pixels.bytes_mut().unwrap_or(&mut []);
            run(
                format,
                &mut PremultiplyBuffers::InPlace(buf),
                cpitch,
                cpitch,
            )?;
        }
        finish_premultiply(
            width,
            height,
            format,
            colorspace,
            &convert,
            bufs,
            dst_format,
            dst_colorspace,
            dst_properties,
            dst_pitch,
        )
    } else if dst_format != format || dst_colorspace != colorspace {
        let mut convert = Surface::new_uninitialized(width, height, format)?;
        let cpitch = convert.pitch;
        {
            let buf = convert.pixels.bytes_mut().unwrap_or(&mut []);
            run(
                format,
                &mut PremultiplyBuffers::Separate(src_bytes, buf),
                src_pitch,
                cpitch,
            )?;
        }
        finish_premultiply(
            width,
            height,
            format,
            colorspace,
            &convert,
            bufs,
            dst_format,
            dst_colorspace,
            dst_properties,
            dst_pitch,
        )
    } else {
        let mut bufs = bufs;
        run(format, &mut bufs, src_pitch, dst_pitch)
    }
}

/// The `dst != final_dst` step: convert the premultiplied pixels into the
/// caller's destination.
#[allow(clippy::too_many_arguments)]
fn finish_premultiply(
    width: i32,
    height: i32,
    format: PixelFormat,
    colorspace: Colorspace,
    convert: &Surface<'_>,
    bufs: PremultiplyBuffers<'_, '_>,
    dst_format: PixelFormat,
    dst_colorspace: Colorspace,
    dst_properties: Option<&Properties>,
    dst_pitch: i32,
) -> Result<()> {
    let final_dst: &mut [u8] = match bufs {
        PremultiplyBuffers::Separate(_, d) => d,
        PremultiplyBuffers::InPlace(b) => b,
    };
    convert_pixels_and_colorspace(
        width,
        height,
        format,
        colorspace,
        None,
        convert.pixels.bytes().unwrap_or(&[]),
        convert.pitch,
        dst_format,
        dst_colorspace,
        dst_properties,
        final_dst,
        dst_pitch,
    )
}

/// Premultiply the alpha of a block of pixels, converting between formats.
/// Translation of `SDL_PremultiplyAlpha()`.
#[allow(clippy::too_many_arguments)]
pub fn premultiply_alpha(
    width: i32,
    height: i32,
    src_format: PixelFormat,
    src: &[u8],
    src_pitch: i32,
    dst_format: PixelFormat,
    dst: &mut [u8],
    dst_pitch: i32,
    linear: bool,
) -> Result<()> {
    let src_colorspace = src_format.default_colorspace();
    let dst_colorspace = dst_format.default_colorspace();

    premultiply_alpha_pixels_and_colorspace(
        width,
        height,
        src_format,
        src_colorspace,
        None,
        PremultiplyBuffers::Separate(src, dst),
        src_pitch,
        dst_format,
        dst_colorspace,
        None,
        dst_pitch,
        linear,
    )
}
