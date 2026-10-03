// Rust translation of the texture parts of src/render/SDL_render.c from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Textures: creation, modulation, updates, locking, render targets, and
//! drawing textures, geometry and debug text.

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::properties::Properties;
use crate::render::debug_font::{DEBUG_FONT_NUM_GLYPHS, DEBUG_TEXT_FONT_DATA};
use crate::render::sysrender::{
    invalid_texture, CopyEx, Geometry, Indices, RenderViewState, TextureCreateProps, TextureData,
    TexturePalette,
};
use crate::render::yuv_sw::SwYuvTexture;
use crate::render::{
    fcolor_floats, intersect_float, update_pixel_clip_rect, update_pixel_viewport, Renderer,
    Texture, TextureAccess, TextureAddressMode, Vertex, DEBUG_TEXT_FONT_CHARACTER_SIZE,
    PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER, PROP_TEXTURE_ACCESS_NUMBER,
    PROP_TEXTURE_COLORSPACE_NUMBER, PROP_TEXTURE_FORMAT_NUMBER, PROP_TEXTURE_HDR_HEADROOM_FLOAT,
    PROP_TEXTURE_HEIGHT_NUMBER, PROP_TEXTURE_SDR_WHITE_POINT_FLOAT, PROP_TEXTURE_WIDTH_NUMBER,
    RECT_INDEX_ORDER,
};
use crate::video::pixels::{Color, Colorspace, FColor, PixelFormat, TransferCharacteristics};
use crate::video::rect::{FPoint, FRect, Rect};
use crate::video::surface::{
    convert_pixels, convert_pixels_and_colorspace, default_hdr_headroom, default_sdr_white_point,
    read_palette, write_palette, ScaleMode, SharedPalette, Surface,
};
use crate::video::{BlendMode, FlipMode};

/// Options for creating a texture (the `SDL_PROP_TEXTURE_CREATE_*`
/// properties).
#[derive(Clone, Debug, Default)]
pub struct TextureCreateInfo {
    /// The pixel format (`SDL_PROP_TEXTURE_CREATE_FORMAT_NUMBER`);
    /// [`PixelFormat::UNKNOWN`] picks the renderer's preferred format
    pub format: PixelFormat,
    /// The access pattern (`SDL_PROP_TEXTURE_CREATE_ACCESS_NUMBER`)
    pub access: TextureAccess,
    /// The width in pixels (`SDL_PROP_TEXTURE_CREATE_WIDTH_NUMBER`)
    pub width: i32,
    /// The height in pixels (`SDL_PROP_TEXTURE_CREATE_HEIGHT_NUMBER`)
    pub height: i32,
    /// The colorspace (`SDL_PROP_TEXTURE_CREATE_COLORSPACE_NUMBER`), the
    /// format's default colorspace by default
    pub colorspace: Option<Colorspace>,
    /// The palette of an indexed texture (`SDL_PROP_TEXTURE_CREATE_PALETTE_POINTER`)
    pub palette: Option<SharedPalette>,
    /// The SDR white point (`SDL_PROP_TEXTURE_CREATE_SDR_WHITE_POINT_FLOAT`)
    pub sdr_white_point: Option<f32>,
    /// The HDR headroom (`SDL_PROP_TEXTURE_CREATE_HDR_HEADROOM_FLOAT`)
    pub hdr_headroom: Option<f32>,
}

/// The number of glyphs in a row of the debug text atlas.
/// Translation of `SDL_DEBUG_FONT_GLYPHS_PER_ROW`.
const DEBUG_FONT_GLYPHS_PER_ROW: usize = 14;

/// `((w * bpp) + 3) & ~3`: the 4 byte aligned pitch of the temporary buffers.
fn aligned_pitch(w: i32, format: PixelFormat) -> i32 {
    ((w * format.bytes_per_pixel() as i32) + 3) & !3
}

/// `SDL_modff()`: the fractional and integral parts of `x`.
fn modff(x: f32) -> (f32, f32) {
    if x.is_infinite() {
        return (0.0f32.copysign(x), x);
    }
    let int = x.trunc();
    ((x - int).copysign(x), int)
}

/// `(int)x` as x86 computes it: values out of range (and NaN), for which
/// the conversion is undefined in C, become `INT_MIN`.
fn c_float_to_int(x: f32) -> i32 {
    if !(-2147483648.0..2147483648.0).contains(&x) {
        i32::MIN
    } else {
        x as i32
    }
}

/// Translation of `IsNPOT()`.
fn is_npot(x: i32) -> bool {
    (x <= 0) || ((x & (x - 1)) != 0)
}

/// The flip mode of horizontal and vertical flip flags.
fn flip_mode(horizontal: bool, vertical: bool) -> FlipMode {
    match (horizontal, vertical) {
        (false, false) => FlipMode::None,
        (true, false) => FlipMode::Horizontal,
        (false, true) => FlipMode::Vertical,
        (true, true) => FlipMode::HorizontalAndVertical,
    }
}

/// The key of a public palette in the renderer's palette cache (upstream
/// hashes the palette pointer).
fn palette_key(palette: &SharedPalette) -> usize {
    Arc::as_ptr(palette) as *const () as usize
}

/// The number of bytes `rows` rows of `row_bytes` bytes, `pitch` apart, span.
fn rows_len(rows: i32, pitch: i32, row_bytes: usize) -> usize {
    if rows <= 0 {
        0
    } else {
        (rows as usize - 1) * pitch as usize + row_bytes
    }
}

/// `memcmp()` of two `SDL_FColor`s.
fn same_color(a: FColor, b: FColor) -> bool {
    [a.r, a.g, a.b, a.a].map(f32::to_bits) == [b.r, b.g, b.b, b.a].map(f32::to_bits)
}

/// A lock of a streaming texture's pixels; the texture is unlocked (and
/// the changes uploaded) when the lock is dropped. Returned by
/// [`Renderer::lock_texture`].
pub struct TextureLock<'r> {
    renderer: &'r mut Renderer,
    texture: Texture,
    offset: usize,
    pitch: i32,
}

impl std::fmt::Debug for TextureLock<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextureLock")
            .field("texture", &self.texture)
            .field("pitch", &self.pitch)
            .finish_non_exhaustive()
    }
}

impl TextureLock<'_> {
    /// The locked pixels: the first byte is the top left pixel of the
    /// locked area, and rows are [`TextureLock::pitch`] bytes apart. The
    /// contents are write-only (they are not necessarily the texture's
    /// current pixels).
    pub fn pixels(&mut self) -> &mut [u8] {
        let offset = self.offset;
        match self.renderer.locked_pixels(self.texture) {
            Some(p) if offset <= p.len() => &mut p[offset..],
            _ => &mut [],
        }
    }

    /// The length of a row of the locked pixels, in bytes.
    pub fn pitch(&self) -> i32 {
        self.pitch
    }
}

impl Drop for TextureLock<'_> {
    fn drop(&mut self) {
        self.renderer.unlock_texture_internal(self.texture);
    }
}

/// A lock of a streaming texture's pixels, seen as a surface; the texture
/// is unlocked when the lock is dropped. Returned by
/// [`Renderer::lock_texture_to_surface`].
pub struct TextureSurfaceLock<'r> {
    lock: TextureLock<'r>,
    w: i32,
    h: i32,
    format: PixelFormat,
    palette: Option<SharedPalette>,
}

impl std::fmt::Debug for TextureSurfaceLock<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextureSurfaceLock")
            .field("lock", &self.lock)
            .field("w", &self.w)
            .field("h", &self.h)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl TextureSurfaceLock<'_> {
    /// A surface over the locked pixels (with the texture's palette). Each
    /// call creates a new surface, without the blit settings of earlier ones.
    pub fn surface(&mut self) -> Result<Surface<'_>> {
        let (w, h, format, pitch) = (self.w, self.h, self.format, self.lock.pitch);
        let palette = self.palette.clone();
        let mut surface = Surface::from_pixels(w, h, format, self.lock.pixels(), pitch)?;
        if let Some(palette) = palette {
            surface.set_palette(Some(palette))?;
        }
        Ok(surface)
    }
}

impl Renderer {
    /// The data of texture `t` of this renderer.
    pub(crate) fn tex(&self, t: Texture) -> Result<&TextureData> {
        if t.renderer != self.id {
            return Err(Error::new("Texture was not created with this renderer"));
        }
        self.textures.get(t).ok_or_else(invalid_texture)
    }

    pub(crate) fn tex_mut(&mut self, t: Texture) -> Result<&mut TextureData> {
        if t.renderer != self.id {
            return Err(Error::new("Texture was not created with this renderer"));
        }
        self.textures.get_mut(t).ok_or_else(invalid_texture)
    }

    /// Translation of `IsSupportedFormat()`.
    fn is_supported_format(&self, format: PixelFormat) -> bool {
        self.texture_formats.contains(&format)
    }

    /// Translation of `GetClosestSupportedFormat()`.
    fn closest_supported_format(&self, format: PixelFormat, mut has_alpha: bool) -> PixelFormat {
        let formats = &self.texture_formats;

        if format == PixelFormat::MJPG {
            // We'll decode to SDL_PIXELFORMAT_NV12 or SDL_PIXELFORMAT_RGBA32
            if let Some(f) = formats.iter().find(|f| **f == PixelFormat::NV12) {
                return *f;
            }
            if let Some(f) = formats.iter().find(|f| **f == PixelFormat::RGBA32) {
                return *f;
            }
        } else if format.is_fourcc() {
            // Look for an exact match
            if let Some(f) = formats.iter().find(|f| **f == format) {
                return *f;
            }
        } else if format.is_10bit() || format.is_float() {
            if format.is_10bit() {
                if let Some(f) = formats.iter().find(|f| f.is_10bit()) {
                    return *f;
                }
            }
            if let Some(f) = formats.iter().find(|f| f.is_float()) {
                return *f;
            }
        } else if format.is_indexed() {
            // Converting between <8bpp formats is not supported yet
            if let Some(f) = formats.iter().find(|f| **f == PixelFormat::INDEX8) {
                return *f;
            }
        } else {
            let pixel_type = format.pixel_type();
            let layout = format.pixel_layout();
            has_alpha = has_alpha || format.has_alpha();

            // We just want to match the first format that has the same channels
            if let Some(f) = formats.iter().find(|f| {
                !f.is_fourcc()
                    && f.pixel_type() == pixel_type
                    && f.pixel_layout() == layout
                    && f.has_alpha() == has_alpha
            }) {
                return *f;
            }
        }

        if has_alpha {
            // The default format may still lack an alpha channel
            if let Some(f) = formats.iter().find(|f| !f.is_fourcc() && f.has_alpha()) {
                return *f;
            }
        }
        formats[0]
    }

    /// Create a texture. Translation of `SDL_CreateTexture()`.
    pub fn create_texture(
        &mut self,
        format: PixelFormat,
        access: TextureAccess,
        w: i32,
        h: i32,
    ) -> Result<Texture> {
        self.create_texture_with(&TextureCreateInfo {
            format,
            access,
            width: w,
            height: h,
            ..TextureCreateInfo::default()
        })
    }

    /// Create a texture with creation options.
    /// Translation of `SDL_CreateTextureWithProperties()`.
    pub fn create_texture_with(&mut self, info: &TextureCreateInfo) -> Result<Texture> {
        let mut format = info.format;
        let access = info.access;
        let (w, h) = (info.width, info.height);

        if format == PixelFormat::UNKNOWN {
            format = self.texture_formats[0];
        }

        if format.bytes_per_pixel() == 0 {
            return Err(Error::new("Invalid texture format"));
        }
        if format.is_indexed() && access == TextureAccess::Target {
            return Err(Error::new("Palettized textures can't be render targets"));
        }
        if format.is_fourcc() && access == TextureAccess::Target {
            return Err(Error::new(format!(
                "{} textures can't be render targets",
                format.name()
            )));
        }
        if w <= 0 || h <= 0 {
            return Err(Error::new("Texture dimensions can't be 0"));
        }
        let max_texture_size = self
            .props
            .get_number(PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER)
            .unwrap_or(0) as i32;
        if max_texture_size != 0 && (w > max_texture_size || h > max_texture_size) {
            return Err(Error::new(format!(
                "Texture dimensions are limited to {max_texture_size}x{max_texture_size}"
            )));
        }

        let default_colorspace = format.default_colorspace();
        let colorspace = info.colorspace.unwrap_or(default_colorspace);

        let mut view = RenderViewState::new(w, h);
        update_pixel_viewport(&mut view);
        update_pixel_clip_rect(&mut view);

        let t = self.textures.insert(
            self.id,
            TextureData {
                format,
                w,
                h,
                colorspace,
                sdr_white_point: info
                    .sdr_white_point
                    .unwrap_or_else(|| default_sdr_white_point(colorspace)),
                hdr_headroom: info
                    .hdr_headroom
                    .unwrap_or_else(|| default_hdr_headroom(colorspace)),
                access,
                blend_mode: if format.has_alpha() {
                    BlendMode::BLEND
                } else {
                    BlendMode::NONE
                },
                scale_mode: self.scale_mode,
                color: FColor::new(1.0, 1.0, 1.0, 1.0),
                view,
                public_palette: None,
                palette: None,
                palette_version: 0,
                palette_surface: None,
                native: None,
                parent: None,
                yuv: None,
                pixels: Vec::new(),
                pitch: 0,
                locked_rect: Rect::default(),
                last_command_generation: 0,
                props: None,
                internal: None,
            },
        );

        if let Err(e) = self.create_texture_storage(t, info) {
            self.destroy_texture_internal(t, false);
            return Err(e);
        }

        // Now set the properties for the new texture
        let props = self.texture_properties(t)?;
        let tex = self.tex(t)?;
        props.set(PROP_TEXTURE_COLORSPACE_NUMBER, tex.colorspace.0 as i64)?;
        props.set(PROP_TEXTURE_FORMAT_NUMBER, tex.format.0 as i64)?;
        props.set(PROP_TEXTURE_ACCESS_NUMBER, tex.access as i64)?;
        props.set(PROP_TEXTURE_WIDTH_NUMBER, tex.w as i64)?;
        props.set(PROP_TEXTURE_HEIGHT_NUMBER, tex.h as i64)?;
        props.set(PROP_TEXTURE_SDR_WHITE_POINT_FLOAT, tex.sdr_white_point)?;
        if tex.hdr_headroom > 0.0 {
            props.set(PROP_TEXTURE_HDR_HEADROOM_FLOAT, tex.hdr_headroom)?;
        }
        Ok(t)
    }

    /// The backend texture, or the native texture and the conversion
    /// storage, of a new texture (the middle of
    /// `SDL_CreateTextureWithProperties()`).
    fn create_texture_storage(&mut self, t: Texture, info: &TextureCreateInfo) -> Result<()> {
        let tex = self.tex(t)?;
        let (format, access, w, h, colorspace) =
            (tex.format, tex.access, tex.w, tex.h, tex.colorspace);

        // FOURCC format cannot be used directly by renderer back-ends for target texture
        let texture_is_fourcc_and_target = access == TextureAccess::Target && format.is_fourcc();

        if !texture_is_fourcc_and_target && self.is_supported_format(format) {
            let Renderer {
                backend, textures, ..
            } = self;
            let tex = textures.get_mut(t).ok_or_else(invalid_texture)?;
            backend.create_texture(
                tex,
                &TextureCreateProps {
                    colorspace: info.colorspace,
                },
            )?;
        } else {
            let closest_format = if !texture_is_fourcc_and_target {
                self.closest_supported_format(format, false)
            } else {
                self.texture_formats[0]
            };

            let native_colorspace =
                if format == PixelFormat::MJPG && closest_format == PixelFormat::NV12 {
                    Colorspace::JPEG
                } else {
                    let default_colorspace = closest_format.default_colorspace();
                    if colorspace.color_type() == default_colorspace.color_type()
                        && colorspace.transfer() == default_colorspace.transfer()
                    {
                        colorspace
                    } else {
                        default_colorspace
                    }
                };
            let native_access = if format.is_indexed() {
                // We're going to be uploading pixels frequently as the palette changes
                TextureAccess::Streaming
            } else {
                access
            };

            let native = self.create_texture_with(&TextureCreateInfo {
                format: closest_format,
                access: native_access,
                width: w,
                height: h,
                colorspace: Some(native_colorspace),
                ..TextureCreateInfo::default()
            })?;

            let scale_mode = {
                let tex = self.tex_mut(t)?;
                tex.native = Some(native);
                tex.scale_mode
            };
            let native_tex = self.tex_mut(native)?;
            native_tex.parent = Some(t);
            native_tex.scale_mode = scale_mode;

            let tex = self.tex_mut(t)?;
            if format == PixelFormat::MJPG {
                // We have a custom decode + upload path for this
            } else if format.is_fourcc() {
                tex.yuv = Some(SwYuvTexture::new(format, colorspace, w, h)?);
            } else if format.is_indexed() {
                tex.palette_surface = Some(Surface::new(w, h, format)?);
            } else if access == TextureAccess::Streaming {
                // The pitch is 4 byte aligned
                tex.pitch = aligned_pitch(w, format);
                tex.pixels = vec![0; tex.pitch as usize * h as usize];
            }
        }

        if format.is_indexed() {
            if let Some(palette) = &info.palette {
                let _ = self.set_texture_palette(t, Some(palette.clone()));
            }
        }
        Ok(())
    }

    /// Translation of `SDL_UpdateTextureFromSurface()`.
    fn update_texture_from_surface(&mut self, t: Texture, surface: &mut Surface<'_>) -> Result<()> {
        let tex = self.tex(t)?;
        let (format, colorspace, public_palette) =
            (tex.format, tex.colorspace, tex.public_palette.clone());

        let direct_update = if surface.format() == format && surface.colorspace() == colorspace {
            if surface.format().is_indexed() {
                // Update Texture directly - the color key is handled later
                true
            } else {
                /* If the formats are identical but the surface has a color key,
                 * an intermediate conversion is needed to convert the color key
                 * to alpha (SDL_ConvertColorkeyToAlpha()); otherwise update the
                 * texture directly. */
                !(surface.format().has_alpha() && surface.has_color_key())
            }
        } else {
            // Surface and Renderer formats are different, it needs an intermediate conversion.
            false
        };

        if direct_update {
            if surface.must_lock() {
                if surface.lock_raw().is_ok() {
                    let _ = self.update_texture(
                        t,
                        None,
                        surface.raw_pixels().unwrap_or(&[]),
                        surface.pitch(),
                    );
                    surface.unlock_raw();
                }
            } else {
                let _ = self.update_texture(
                    t,
                    None,
                    surface.raw_pixels().unwrap_or(&[]),
                    surface.pitch(),
                );
            }
        } else {
            // Set up a destination surface for the texture update
            let props = surface.properties();
            let temp = surface.convert_with_colorspace(
                format,
                public_palette.as_ref(),
                colorspace,
                Some(&props),
            )?;
            let _ = self.update_texture(t, None, temp.raw_pixels().unwrap_or(&[]), temp.pitch());
        }

        if format.is_indexed() && surface.has_color_key() {
            let palette = self.texture_palette(t)?;
            let key = surface.color_key();
            let (Some(palette), Some(key)) = (palette, key) else {
                return Err(Error::new(
                    "Texture has no palette for the surface color key",
                ));
            };
            let mut palette = write_palette(&palette);
            if key as usize >= palette.len() {
                return Err(Error::new(
                    "Texture has no palette for the surface color key",
                ));
            }
            let mut col = palette.colors()[key as usize];
            col.a = crate::video::pixels::ALPHA_TRANSPARENT;
            let _ = palette.set_colors(key as usize, &[col]);
        }

        let (r, g, b) = surface.color_mod();
        let _ = self.set_texture_color_mod(t, r, g, b);

        let a = surface.alpha_mod();
        let _ = self.set_texture_alpha_mod(t, a);

        if surface.has_color_key() {
            // We converted to a texture with alpha format
            let _ = self.set_texture_blend_mode(t, BlendMode::BLEND);
        } else {
            let _ = self.set_texture_blend_mode(t, surface.blend_mode());
        }

        Ok(())
    }

    /// Create a texture with the contents of a surface.
    /// Translation of `SDL_CreateTextureFromSurface()`.
    pub fn create_texture_from_surface(&mut self, surface: &mut Surface<'_>) -> Result<Texture> {
        use PixelFormat as F;
        let mut format = F::UNKNOWN;

        // Try to have the best pixel format for the texture
        // No alpha, but a colorkey => promote to alpha
        if !surface.format().has_alpha() && surface.has_color_key() {
            let promoted = match surface.format() {
                F::XRGB8888 => Some(F::ARGB8888),
                F::XBGR8888 => Some(F::ABGR8888),
                F::XRGB4444 => Some(F::ARGB4444),
                _ => None,
            };
            if let Some(p) = promoted {
                if self.texture_formats.contains(&p) {
                    format = p;
                }
            }
        } else if self.texture_formats.contains(&surface.format()) {
            // Exact match would be fine
            format = surface.format();
        }

        // Fallback, choose a valid pixel format
        if format == F::UNKNOWN {
            let mut need_alpha = surface.has_color_key();

            // If palette contains alpha values, promotes to alpha format
            if let Some(palette) = surface.palette() {
                if !read_palette(palette).alpha_info().is_opaque {
                    need_alpha = true;
                }
            }

            format = self.closest_supported_format(surface.format(), need_alpha);
        }

        let surface_colorspace = surface.colorspace();
        let mut texture_colorspace = surface_colorspace;

        if surface_colorspace == Colorspace::SRGB_LINEAR
            || surface_colorspace.transfer() == TransferCharacteristics::Pq
        {
            texture_colorspace = if format.is_float() {
                Colorspace::SRGB_LINEAR
            } else if format.is_10bit() {
                Colorspace::HDR10
            } else {
                Colorspace::SRGB
            };
        }

        let info = TextureCreateInfo {
            format,
            access: TextureAccess::Static,
            width: surface.width(),
            height: surface.height(),
            colorspace: Some(texture_colorspace),
            palette: surface.palette().cloned(),
            sdr_white_point: (surface_colorspace == texture_colorspace)
                .then(|| surface.sdr_white_point(surface_colorspace)),
            hdr_headroom: Some(surface.hdr_headroom(surface_colorspace)),
        };
        let texture = self.create_texture_with(&info)?;

        if let Err(e) = self.update_texture_from_surface(texture, surface) {
            self.destroy_texture(texture);
            return Err(e);
        }

        Ok(texture)
    }

    /// The texture's properties. Translation of `SDL_GetTextureProperties()`.
    pub fn texture_properties(&mut self, t: Texture) -> Result<Properties> {
        let tex = self.tex_mut(t)?;
        Ok(tex.props.get_or_insert_with(Properties::new).clone())
    }

    /// The size of a texture. Translation of `SDL_GetTextureSize()`.
    pub fn texture_size(&self, t: Texture) -> Result<(f32, f32)> {
        let tex = self.tex(t)?;
        Ok((tex.w as f32, tex.h as f32))
    }

    /// Set the palette of an indexed texture (`None` removes it).
    /// Translation of `SDL_SetTexturePalette()`.
    pub fn set_texture_palette(
        &mut self,
        t: Texture,
        palette: Option<SharedPalette>,
    ) -> Result<()> {
        let tex = self.tex(t)?;

        if !tex.format.is_indexed() {
            return Err(Error::new("Texture isn't palettized format"));
        }

        if let Some(p) = &palette {
            if read_palette(p).len() > (1usize << tex.format.bits_per_pixel()) {
                return Err(Error::new("Palette doesn't match surface format"));
            }
        }

        let unchanged = match (&palette, &tex.public_palette) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return Ok(());
        }

        let has_native = tex.native.is_some();
        if tex.public_palette.is_some() && !has_native {
            // Clean up the texture palette
            if let Some(key) = self.tex_mut(t)?.palette.take() {
                let refcount = self.palettes.get_mut(&key).map(|p| {
                    p.refcount -= 1;
                    p.refcount
                });
                if refcount == Some(0) {
                    let _ = self.flush_if_palette_needed(key);
                    if let Some(p) = self.palettes.remove(&key) {
                        self.backend.destroy_palette(p.internal);
                    }
                }
            }
        }

        {
            let tex = self.tex_mut(t)?;
            tex.public_palette = palette.clone();
            tex.palette_version = 0;
        }

        if let Some(palette) = &palette {
            if !has_native {
                let key = palette_key(palette);
                if let Some(p) = self.palettes.get_mut(&key) {
                    p.refcount += 1;
                } else {
                    match self.backend.create_palette() {
                        Ok(internal) => {
                            self.palettes.insert(
                                key,
                                TexturePalette {
                                    refcount: 1,
                                    version: 0,
                                    last_command_generation: 0,
                                    internal,
                                },
                            );
                        }
                        Err(e) => {
                            let _ = self.set_texture_palette(t, None);
                            return Err(e);
                        }
                    }
                }
                self.tex_mut(t)?.palette = Some(key);

                let Renderer {
                    backend,
                    textures,
                    palettes,
                    ..
                } = self;
                if let (Some(tex), Some(p)) = (textures.get_mut(t), palettes.get(&key)) {
                    let _ = backend.change_texture_palette(tex, Some(p.internal.as_ref()));
                }
            }
        }

        let tex = self.tex_mut(t)?;
        if let Some(surface) = &mut tex.palette_surface {
            let _ = surface.set_palette(palette);
        }
        Ok(())
    }

    /// The palette of an indexed texture. Translation of `SDL_GetTexturePalette()`.
    pub fn texture_palette(&self, t: Texture) -> Result<Option<SharedPalette>> {
        Ok(self.tex(t)?.public_palette.clone())
    }

    /// Set the color multiplied into render copies of a texture.
    /// Translation of `SDL_SetTextureColorMod()`.
    pub fn set_texture_color_mod(&mut self, t: Texture, r: u8, g: u8, b: u8) -> Result<()> {
        self.set_texture_color_mod_float(t, r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }

    /// Set the color multiplied into render copies of a texture.
    /// Translation of `SDL_SetTextureColorModFloat()`.
    pub fn set_texture_color_mod_float(
        &mut self,
        t: Texture,
        r: f32,
        g: f32,
        b: f32,
    ) -> Result<()> {
        let tex = self.tex_mut(t)?;
        tex.color.r = r;
        tex.color.g = g;
        tex.color.b = b;
        match tex.native {
            Some(native) => self.set_texture_color_mod_float(native, r, g, b),
            None => Ok(()),
        }
    }

    /// The color multiplied into render copies of a texture.
    /// Translation of `SDL_GetTextureColorMod()`.
    pub fn texture_color_mod(&self, t: Texture) -> Result<(u8, u8, u8)> {
        let (r, g, b) = self.texture_color_mod_float(t)?;
        let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Ok((to_byte(r), to_byte(g), to_byte(b)))
    }

    /// The color multiplied into render copies of a texture.
    /// Translation of `SDL_GetTextureColorModFloat()`.
    pub fn texture_color_mod_float(&self, t: Texture) -> Result<(f32, f32, f32)> {
        let color = self.tex(t)?.color;
        Ok((color.r, color.g, color.b))
    }

    /// Set the alpha multiplied into render copies of a texture.
    /// Translation of `SDL_SetTextureAlphaMod()`.
    pub fn set_texture_alpha_mod(&mut self, t: Texture, alpha: u8) -> Result<()> {
        self.set_texture_alpha_mod_float(t, alpha as f32 / 255.0)
    }

    /// Set the alpha multiplied into render copies of a texture.
    /// Translation of `SDL_SetTextureAlphaModFloat()`.
    pub fn set_texture_alpha_mod_float(&mut self, t: Texture, alpha: f32) -> Result<()> {
        let tex = self.tex_mut(t)?;
        tex.color.a = alpha;
        match tex.native {
            Some(native) => self.set_texture_alpha_mod_float(native, alpha),
            None => Ok(()),
        }
    }

    /// The alpha multiplied into render copies of a texture.
    /// Translation of `SDL_GetTextureAlphaMod()`.
    pub fn texture_alpha_mod(&self, t: Texture) -> Result<u8> {
        let a = self.texture_alpha_mod_float(t)?;
        Ok((a.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    /// The alpha multiplied into render copies of a texture.
    /// Translation of `SDL_GetTextureAlphaModFloat()`.
    pub fn texture_alpha_mod_float(&self, t: Texture) -> Result<f32> {
        Ok(self.tex(t)?.color.a)
    }

    /// Set the blend mode of render copies of a texture.
    /// Translation of `SDL_SetTextureBlendMode()`.
    pub fn set_texture_blend_mode(&mut self, t: Texture, blend_mode: BlendMode) -> Result<()> {
        self.tex(t)?;

        if blend_mode == BlendMode::INVALID {
            return Err(Error::invalid_param("blendMode"));
        }

        if !self.is_supported_blend_mode(blend_mode) {
            return Err(Error::unsupported());
        }
        let tex = self.tex_mut(t)?;
        tex.blend_mode = blend_mode;
        match tex.native {
            Some(native) => self.set_texture_blend_mode(native, blend_mode),
            None => Ok(()),
        }
    }

    /// The blend mode of render copies of a texture.
    /// Translation of `SDL_GetTextureBlendMode()`.
    pub fn texture_blend_mode(&self, t: Texture) -> Result<BlendMode> {
        Ok(self.tex(t)?.blend_mode)
    }

    /// Set the scale mode of scaled render copies of a texture.
    /// Translation of `SDL_SetTextureScaleMode()`.
    pub fn set_texture_scale_mode(&mut self, t: Texture, scale_mode: ScaleMode) -> Result<()> {
        let tex = self.tex_mut(t)?;
        tex.scale_mode = scale_mode;
        match tex.native {
            Some(native) => self.set_texture_scale_mode(native, scale_mode),
            None => Ok(()),
        }
    }

    /// The scale mode of scaled render copies of a texture.
    /// Translation of `SDL_GetTextureScaleMode()`.
    pub fn texture_scale_mode(&self, t: Texture) -> Result<ScaleMode> {
        Ok(self.tex(t)?.scale_mode)
    }

    /// The pixels of a lock of texture `t` (see `lock_texture_raw()`).
    pub(crate) fn locked_pixels(&mut self, t: Texture) -> Option<&mut [u8]> {
        let Renderer {
            textures, backend, ..
        } = self;
        let tex = textures.get_mut(t)?;
        if tex.yuv.is_some() {
            return tex.yuv.as_mut().map(|y| &mut y.pixels[..]);
        }
        if tex.palette_surface.is_some() {
            return tex.palette_surface.as_mut().and_then(|s| s.pixels_mut());
        }
        if tex.native.is_some() {
            return Some(&mut tex.pixels[..]);
        }
        backend.texture_pixels_mut(tex)
    }

    /// Run `f` on texture `t` and the locked pixels of `rect` of its native
    /// texture (`SDL_LockTexture(native)`, then `SDL_UnlockTexture(native)`).
    fn with_native_lock(
        &mut self,
        t: Texture,
        rect: &Rect,
        f: impl FnOnce(&mut TextureData, &NativeView, &mut [u8], i32) -> Result<()>,
    ) -> Result<()> {
        let native = self.tex(t)?.native.ok_or_else(invalid_texture)?;
        let (offset, pitch) = self.lock_texture_raw(native, Some(rect))?;
        let result = (|| {
            let Renderer {
                textures, backend, ..
            } = self;
            let (tex, native_tex) = textures
                .get_two_mut(t, native)
                .ok_or_else(invalid_texture)?;
            let native_view = NativeView {
                format: native_tex.format,
                colorspace: native_tex.colorspace,
                w: native_tex.w,
                h: native_tex.h,
            };
            let pixels = backend
                .texture_pixels_mut(native_tex)
                .ok_or_else(|| Error::new("Texture pixels are not accessible"))?;
            let pixels = pixels
                .get_mut(offset..)
                .ok_or_else(|| Error::invalid_param("rect"))?;
            f(tex, &native_view, pixels, pitch)
        })();
        self.unlock_texture_internal(native);
        result
    }

    /// Copy the YUV planes of texture `t` to its native texture, after an
    /// update (the end of `SDL_UpdateTextureYUV()` and its planar variants).
    fn upload_yuv(&mut self, t: Texture) -> Result<()> {
        let tex = self.tex(t)?;
        let full_rect = Rect::new(0, 0, tex.w, tex.h);
        let rect = full_rect;

        if rect.w == 0 || rect.h == 0 {
            return Ok(()); // nothing to do.
        }

        if tex.access == TextureAccess::Streaming {
            // We can lock the texture and copy to it
            self.with_native_lock(t, &rect, |tex, native, pixels, pitch| {
                let yuv = tex.yuv.as_mut().ok_or_else(invalid_texture)?;
                yuv.copy_to_rgb(&rect, native.format, rect.w, rect.h, pixels, pitch)
            })
        } else {
            // Use a temporary buffer for updating
            let native = tex.native.ok_or_else(invalid_texture)?;
            let native_format = self.tex(native)?.format;
            let temp_pitch = aligned_pitch(rect.w, native_format);
            let alloclen = rect.h as usize * temp_pitch as usize;
            let mut result = Ok(());
            if alloclen > 0 {
                let mut temp_pixels = vec![0u8; alloclen];
                let tex = self.tex_mut(t)?;
                let yuv = tex.yuv.as_mut().ok_or_else(invalid_texture)?;
                result = yuv.copy_to_rgb(
                    &rect,
                    native_format,
                    rect.w,
                    rect.h,
                    &mut temp_pixels,
                    temp_pitch,
                );
                if result.is_ok() {
                    let _ = self.update_texture(native, Some(&rect), &temp_pixels, temp_pitch);
                }
            }
            result
        }
    }

    /// Translation of `SDL_UpdateTextureYUV()`.
    fn update_texture_yuv_internal(
        &mut self,
        t: Texture,
        rect: &Rect,
        pixels: &[u8],
        pitch: i32,
    ) -> Result<()> {
        let tex = self.tex_mut(t)?;
        tex.yuv
            .as_mut()
            .ok_or_else(invalid_texture)?
            .update(rect, pixels, pitch as usize)?;
        self.upload_yuv(t)
    }

    /// Translation of `SDL_UpdateTexturePaletteSurface()`.
    fn update_texture_palette_surface(
        &mut self,
        t: Texture,
        rect: &Rect,
        pixels: &[u8],
        pitch: i32,
    ) -> Result<()> {
        let tex = self.tex_mut(t)?;
        let bits = tex.format.bits_per_pixel() as i32;
        let surface = tex.palette_surface.as_mut().ok_or_else(invalid_texture)?;
        let spitch = surface.pitch() as usize;
        let dst_pixels = surface.pixels_mut().ok_or_else(invalid_texture)?;

        let w = (((rect.w * bits) + 7) / 8) as usize;
        let mut src = 0;
        let mut dst = rect.y as usize * spitch + ((rect.x * bits) / 8) as usize;
        for _ in 0..rect.h {
            dst_pixels[dst..dst + w].copy_from_slice(&pixels[src..src + w]);
            src += pitch as usize;
            dst += spitch;
        }
        tex.palette_version = 0;
        Ok(())
    }

    /// Translation of `SDL_UpdateTextureNative()`.
    fn update_texture_native(
        &mut self,
        t: Texture,
        rect: &Rect,
        pixels: &[u8],
        pitch: i32,
    ) -> Result<()> {
        let tex = self.tex(t)?;
        let (format, colorspace) = (tex.format, tex.colorspace);

        if tex.access == TextureAccess::Streaming {
            // We can lock the texture and copy to it
            self.with_native_lock(t, rect, |_, native, native_pixels, native_pitch| {
                let _ = convert_pixels_and_colorspace(
                    rect.w,
                    rect.h,
                    format,
                    colorspace,
                    None,
                    pixels,
                    pitch,
                    native.format,
                    native.colorspace,
                    None,
                    native_pixels,
                    native_pitch,
                );
                Ok(())
            })
        } else {
            // Use a temporary buffer for updating
            let native = tex.native.ok_or_else(invalid_texture)?;
            let native_tex = self.tex(native)?;
            let (native_format, native_colorspace) = (native_tex.format, native_tex.colorspace);
            let temp_pitch = aligned_pitch(rect.w, native_format);
            let alloclen = rect.h as usize * temp_pitch as usize;
            if alloclen > 0 {
                let mut temp_pixels = vec![0u8; alloclen];
                let _ = convert_pixels_and_colorspace(
                    rect.w,
                    rect.h,
                    format,
                    colorspace,
                    None,
                    pixels,
                    pitch,
                    native_format,
                    native_colorspace,
                    None,
                    &mut temp_pixels,
                    temp_pitch,
                );
                let _ = self.update_texture(native, Some(rect), &temp_pixels, temp_pitch);
            }
            Ok(())
        }
    }

    /// Update `rect` of a texture (`None` = all of it) with new pixels,
    /// `pitch` bytes per row. Translation of `SDL_UpdateTexture()`.
    pub fn update_texture(
        &mut self,
        t: Texture,
        rect: Option<&Rect>,
        pixels: &[u8],
        pitch: i32,
    ) -> Result<()> {
        let tex = self.tex(t)?;

        if pixels.is_empty() {
            return Err(Error::invalid_param("pixels"));
        }
        if pitch == 0 {
            return Err(Error::invalid_param("pitch"));
        }
        // (the rows are read forwards from `pixels`)
        if pitch < 0 {
            return Err(Error::invalid_param("pitch"));
        }

        let full_rect = Rect::new(0, 0, tex.w, tex.h);
        let mut real_rect = full_rect;
        if let Some(rect) = rect {
            if !rect.intersect_into(&full_rect, &mut real_rect) {
                return Ok(());
            }
        }

        if real_rect.w == 0 || real_rect.h == 0 {
            return Ok(()); // nothing to do.
        }

        if tex.yuv.is_none() {
            let row_bytes =
                (real_rect.w as usize * tex.format.bits_per_pixel() as usize).div_ceil(8);
            if pixels.len() < rows_len(real_rect.h, pitch, row_bytes) {
                return Err(Error::invalid_param("pixels"));
            }
        }

        if tex.yuv.is_some() {
            self.update_texture_yuv_internal(t, &real_rect, pixels, pitch)
        } else if tex.palette_surface.is_some() {
            self.update_texture_palette_surface(t, &real_rect, pixels, pitch)
        } else if tex.native.is_some() {
            self.update_texture_native(t, &real_rect, pixels, pitch)
        } else {
            self.flush_if_texture_needed(t)?;
            let Renderer {
                backend, textures, ..
            } = self;
            let tex = textures.get_mut(t).ok_or_else(invalid_texture)?;
            backend.update_texture(tex, &real_rect, pixels, pitch as usize)
        }
    }

    /// The part of a texture an update of `rect` covers (the start of the
    /// YUV and NV update functions, which don't check the intersection).
    fn yuv_update_rect(&self, t: Texture, rect: Option<&Rect>) -> Result<Rect> {
        let tex = self.tex(t)?;
        let full_rect = Rect::new(0, 0, tex.w, tex.h);
        let mut real_rect = full_rect;
        if let Some(rect) = rect {
            // FIXME (upstream): the result of the intersection is ignored,
            // so a rect outside the texture updates a rect with a negative
            // size; that is treated as an empty rect here.
            if !rect.intersect_into(&full_rect, &mut real_rect) {
                real_rect.w = 0;
                real_rect.h = 0;
            }
        }
        Ok(real_rect)
    }

    /// Update `rect` of a planar YUV texture (`None` = all of it) with new
    /// Y, U and V planes. Translation of `SDL_UpdateYUVTexture()`.
    #[allow(clippy::too_many_arguments)]
    pub fn update_yuv_texture(
        &mut self,
        t: Texture,
        rect: Option<&Rect>,
        y_plane: &[u8],
        y_pitch: i32,
        u_plane: &[u8],
        u_pitch: i32,
        v_plane: &[u8],
        v_pitch: i32,
    ) -> Result<()> {
        let tex = self.tex(t)?;

        if y_plane.is_empty() {
            return Err(Error::invalid_param("Yplane"));
        }
        if y_pitch <= 0 {
            return Err(Error::invalid_param("Ypitch"));
        }
        if u_plane.is_empty() {
            return Err(Error::invalid_param("Uplane"));
        }
        if u_pitch <= 0 {
            return Err(Error::invalid_param("Upitch"));
        }
        if v_plane.is_empty() {
            return Err(Error::invalid_param("Vplane"));
        }
        if v_pitch <= 0 {
            return Err(Error::invalid_param("Vpitch"));
        }

        use PixelFormat as F;
        if !matches!(tex.format, F::YV12 | F::IYUV | F::I444 | F::I0FL | F::I4FL) {
            return Err(Error::new(
                "Texture format must be YV12, IYUV, I444, I0FL, or I4FL",
            ));
        }

        let real_rect = self.yuv_update_rect(t, rect)?;
        if real_rect.w <= 0 || real_rect.h <= 0 {
            return Ok(()); // nothing to do.
        }

        if tex.yuv.is_some() {
            // Translation of `SDL_UpdateTextureYUVPlanar()`.
            let tex = self.tex_mut(t)?;
            tex.yuv
                .as_mut()
                .ok_or_else(invalid_texture)?
                .update_planar(
                    &real_rect,
                    y_plane,
                    y_pitch as usize,
                    u_plane,
                    u_pitch as usize,
                    v_plane,
                    v_pitch as usize,
                )?;
            self.upload_yuv(t)
        } else {
            self.flush_if_texture_needed(t)?;
            let Renderer {
                backend, textures, ..
            } = self;
            let tex = textures.get_mut(t).ok_or_else(invalid_texture)?;
            backend
                .update_texture_yuv(
                    tex,
                    &real_rect,
                    (y_plane, y_pitch as usize),
                    (u_plane, u_pitch as usize),
                    (v_plane, v_pitch as usize),
                )
                .unwrap_or_else(|| Err(Error::unsupported()))
        }
    }

    /// Update `rect` of an NV12, NV21 or P010 texture (`None` = all of it)
    /// with new Y and interleaved UV planes. Translation of `SDL_UpdateNVTexture()`.
    pub fn update_nv_texture(
        &mut self,
        t: Texture,
        rect: Option<&Rect>,
        y_plane: &[u8],
        y_pitch: i32,
        uv_plane: &[u8],
        uv_pitch: i32,
    ) -> Result<()> {
        let tex = self.tex(t)?;

        if y_plane.is_empty() {
            return Err(Error::invalid_param("Yplane"));
        }
        if y_pitch <= 0 {
            return Err(Error::invalid_param("Ypitch"));
        }
        if uv_plane.is_empty() {
            return Err(Error::invalid_param("UVplane"));
        }
        if uv_pitch <= 0 {
            return Err(Error::invalid_param("UVpitch"));
        }

        use PixelFormat as F;
        if !matches!(tex.format, F::NV12 | F::NV21 | F::P010) {
            return Err(Error::new("Texture format must be NV12, NV21, or P010"));
        }

        let real_rect = self.yuv_update_rect(t, rect)?;
        if real_rect.w <= 0 || real_rect.h <= 0 {
            return Ok(()); // nothing to do.
        }

        if tex.yuv.is_some() {
            // Translation of `SDL_UpdateTextureNVPlanar()`.
            let tex = self.tex_mut(t)?;
            tex.yuv
                .as_mut()
                .ok_or_else(invalid_texture)?
                .update_nv_planar(
                    &real_rect,
                    y_plane,
                    y_pitch as usize,
                    uv_plane,
                    uv_pitch as usize,
                )?;
            self.upload_yuv(t)
        } else {
            self.flush_if_texture_needed(t)?;
            let Renderer {
                backend, textures, ..
            } = self;
            let tex = textures.get_mut(t).ok_or_else(invalid_texture)?;
            backend
                .update_texture_nv(
                    tex,
                    &real_rect,
                    (y_plane, y_pitch as usize),
                    (uv_plane, uv_pitch as usize),
                )
                .unwrap_or_else(|| Err(Error::unsupported()))
        }
    }

    /// `SDL_LockTexture()` without the guard: the byte offset of `rect` in
    /// [`Renderer::locked_pixels`] and the pitch.
    fn lock_texture_raw(&mut self, t: Texture, rect: Option<&Rect>) -> Result<(usize, i32)> {
        let tex = self.tex(t)?;

        if tex.access != TextureAccess::Streaming {
            return Err(Error::new("SDL_LockTexture(): texture must be streaming"));
        }

        let full_rect = Rect::new(0, 0, tex.w, tex.h);
        let rect = rect.copied().unwrap_or(full_rect);
        // (upstream hands out a pointer for any rect; the pixels here are
        // a slice, so the rect must be inside the texture)
        if rect.x < 0
            || rect.y < 0
            || rect.w < 0
            || rect.h < 0
            || rect.x + rect.w > tex.w
            || rect.y + rect.h > tex.h
        {
            return Err(Error::invalid_param("rect"));
        }

        if tex.yuv.is_some() {
            self.flush_if_texture_needed(t)?;
            // Translation of `SDL_LockTextureYUV()`.
            let yuv = self.tex(t)?.yuv.as_ref().ok_or_else(invalid_texture)?;
            let (offset, pitch) = yuv.lock(Some(&rect))?;
            Ok((offset, pitch as i32))
        } else if let Some(surface) = &tex.palette_surface {
            // Translation of `SDL_LockTexturePaletteSurface()`.
            let offset = rect.y as usize * surface.pitch() as usize
                + ((rect.x * tex.format.bits_per_pixel() as i32) / 8) as usize;
            Ok((offset, surface.pitch()))
        } else if tex.native.is_some() {
            // Calls a real SDL_LockTexture/SDL_UnlockTexture on unlock, flushing then.
            // Translation of `SDL_LockTextureNative()`.
            let tex = self.tex_mut(t)?;
            tex.locked_rect = rect;
            let offset = rect.y as usize * tex.pitch as usize
                + rect.x as usize * tex.format.bytes_per_pixel() as usize;
            Ok((offset, tex.pitch))
        } else {
            self.flush_if_texture_needed(t)?;
            let Renderer {
                backend, textures, ..
            } = self;
            let tex = textures.get_mut(t).ok_or_else(invalid_texture)?;
            backend.lock_texture(tex, &rect)
        }
    }

    /// Lock `rect` of a streaming texture (`None` = all of it) for writing
    /// pixels; the texture is updated when the lock is dropped.
    /// Translation of `SDL_LockTexture()` and `SDL_UnlockTexture()`.
    pub fn lock_texture(&mut self, t: Texture, rect: Option<&Rect>) -> Result<TextureLock<'_>> {
        let (offset, pitch) = self.lock_texture_raw(t, rect)?;
        Ok(TextureLock {
            renderer: self,
            texture: t,
            offset,
            pitch,
        })
    }

    /// Lock `rect` of a streaming texture (`None` = all of it) for drawing
    /// into it as a surface; the texture is updated when the lock is
    /// dropped. Translation of `SDL_LockTextureToSurface()`.
    pub fn lock_texture_to_surface(
        &mut self,
        t: Texture,
        rect: Option<&Rect>,
    ) -> Result<TextureSurfaceLock<'_>> {
        let tex = self.tex(t)?;
        let full_rect = Rect::new(0, 0, tex.w, tex.h);
        let mut real_rect = full_rect;
        if let Some(rect) = rect {
            // FIXME (upstream): the result of the intersection is ignored.
            if !rect.intersect_into(&full_rect, &mut real_rect) {
                real_rect.w = 0;
                real_rect.h = 0;
            }
        }
        let (format, palette) = (tex.format, tex.public_palette.clone());

        let lock = self.lock_texture(t, Some(&real_rect))?;
        let mut lock = TextureSurfaceLock {
            lock,
            w: real_rect.w,
            h: real_rect.h,
            format,
            palette,
        };
        // (upstream creates the surface here, unlocking if that fails)
        lock.surface().map(|_| ())?;
        Ok(lock)
    }

    /// Translation of `SDL_UnlockTexture()`.
    pub(crate) fn unlock_texture_internal(&mut self, t: Texture) {
        let Ok(tex) = self.tex(t) else { return };

        if tex.access != TextureAccess::Streaming {
            return;
        }

        if tex.yuv.is_some() {
            // Translation of `SDL_UnlockTextureYUV()`.
            let rect = Rect::new(0, 0, tex.w, tex.h);
            let _ = self.with_native_lock(t, &rect, |tex, native, pixels, pitch| {
                let yuv = tex.yuv.as_mut().ok_or_else(invalid_texture)?;
                yuv.copy_to_rgb(&rect, native.format, rect.w, rect.h, pixels, pitch)
            });
        } else if tex.palette_surface.is_some() {
            // Translation of `SDL_UnlockTexturePaletteSurface()`.
            if let Ok(tex) = self.tex_mut(t) {
                tex.palette_version = 0;
            }
        } else if tex.native.is_some() {
            // Translation of `SDL_UnlockTextureNative()`.
            let rect = tex.locked_rect;
            let _ = self.with_native_lock(t, &rect, |tex, native, native_pixels, native_pitch| {
                let offset = rect.y as usize * tex.pitch as usize
                    + rect.x as usize * tex.format.bytes_per_pixel() as usize;
                let _ = convert_pixels(
                    rect.w,
                    rect.h,
                    tex.format,
                    &tex.pixels[offset..],
                    tex.pitch,
                    native.format,
                    native_pixels,
                    native_pitch,
                );
                Ok(())
            });
        } else {
            let Renderer {
                backend, textures, ..
            } = self;
            if let Some(tex) = textures.get_mut(t) {
                backend.unlock_texture(tex);
            }
        }
    }

    /// Set the target of drawing (`None` = the output).
    /// Translation of `SDL_SetRenderTarget()`.
    pub fn set_render_target(&mut self, texture: Option<Texture>) -> Result<()> {
        // texture == NULL is valid and means reset the target to the window
        let mut target = None;
        if let Some(t) = texture {
            let tex = self.tex(t)?;
            if tex.access != TextureAccess::Target {
                return Err(Error::new(
                    "Texture not created with SDL_TEXTUREACCESS_TARGET",
                ));
            }

            // Always render to the native texture
            target = Some(tex.native.unwrap_or(t));
        }

        if target == self.target {
            // Nothing to do!
            return Ok(());
        }

        let _ = self.flush_render_commands(); // time to send everything to the GPU!

        self.target = target;
        self.current_colorspace = match target {
            Some(t) => self.tex(t)?.colorspace,
            None => self.output_colorspace,
        };
        self.update_color_scale();

        self.backend.set_render_target(target)?;

        self.queue_cmd_set_viewport()?;
        self.queue_cmd_set_clip_rect()?;

        // All set!
        Ok(())
    }

    /// The current render target (`None` = the output).
    /// Translation of `SDL_GetRenderTarget()`.
    pub fn render_target(&self) -> Option<Texture> {
        let target = self.target?;
        Some(
            self.textures
                .get(target)
                .and_then(|t| t.parent)
                .unwrap_or(target),
        )
    }

    /// Destroy a texture. Translation of `SDL_DestroyTexture()`.
    pub fn destroy_texture(&mut self, t: Texture) {
        if self.tex(t).is_ok() {
            self.destroy_texture_internal(t, false);
        }
    }

    /// Translation of `SDL_DestroyTextureInternal()`.
    pub(crate) fn destroy_texture_internal(&mut self, t: Texture, is_destroying: bool) {
        let Some(tex) = self.textures.get(t) else {
            return;
        };

        if tex.public_palette.is_some() {
            let _ = self.set_texture_palette(t, None);
        }

        if is_destroying {
            // Renderer get destroyed, avoid to queue more commands
        } else if Some(t) == self.target {
            let _ = self.set_render_target(None); // implies command queue flush
        } else {
            let _ = self.flush_if_texture_needed(t);
        }

        let Some(mut data) = self.textures.remove(t) else {
            return;
        };

        if let Some(native) = data.native {
            self.destroy_texture_internal(native, is_destroying);
        }

        self.backend.destroy_texture(&mut data);
    }

    /// Translation of `UpdateTexturePalette()`.
    fn update_texture_palette(&mut self, t: Texture) -> Result<()> {
        let tex = self.tex(t)?;

        if !tex.format.is_indexed() {
            return Ok(());
        }

        let Some(public) = tex.public_palette.clone() else {
            return Err(Error::new("Texture doesn't have a palette"));
        };
        let public_version = read_palette(&public).version();

        if let Some(native) = tex.native {
            // Keep the native texture in sync with palette updates
            if tex.palette_version == public_version {
                return Ok(());
            }

            self.flush_if_texture_needed(native)?;

            let full_rect = {
                let n = self.tex(native)?;
                Rect::new(0, 0, n.w, n.h)
            };
            self.with_native_lock(t, &full_rect, |tex, native, pixels, pitch| {
                let mut surface =
                    Surface::from_pixels(native.w, native.h, native.format, pixels, pitch)?;
                let palette_surface = tex.palette_surface.as_mut().ok_or_else(invalid_texture)?;
                palette_surface.blit(None, &mut surface, None)
            })?;
            self.tex_mut(t)?.palette_version = public_version;
            return Ok(());
        }

        let key = tex
            .palette
            .ok_or_else(|| Error::new("Texture doesn't have a palette"))?;
        let palette_version = self.palettes.get(&key).map_or(0, |p| p.version);
        if palette_version != public_version {
            // Keep the native palette in sync with palette updates
            self.flush_if_palette_needed(key)?;

            let Renderer {
                backend, palettes, ..
            } = self;
            let palette = palettes.get_mut(&key).ok_or_else(invalid_texture)?;
            backend.update_palette(palette.internal.as_mut(), read_palette(&public).colors())?;

            palette.version = public_version;
        }

        let generation = self.render_command_generation;
        if let Some(palette) = self.palettes.get_mut(&key) {
            palette.last_command_generation = generation;
        }
        Ok(())
    }

    /// The texture drawn for texture `t` (its native texture, if it has
    /// one), marked as used by the current command queue.
    fn texture_for_drawing(&mut self, t: Texture) -> Result<Texture> {
        let t = self.tex(t)?.native.unwrap_or(t);
        let generation = self.render_command_generation;
        self.tex_mut(t)?.last_command_generation = generation;
        Ok(t)
    }

    /// A quad with texture coordinates, queued as geometry.
    #[allow(clippy::too_many_arguments)]
    fn queue_textured_quad(
        &mut self,
        t: Texture,
        xy: &[f32; 8],
        uv: &[f32; 8],
        scale_x: f32,
        scale_y: f32,
        mode_u: TextureAddressMode,
        mode_v: TextureAddressMode,
    ) -> Result<()> {
        let color = fcolor_floats(self.tex(t)?.color);
        let geometry = Geometry {
            xy,
            xy_stride: 2,
            color: &color,
            color_stride: 0,
            uv,
            uv_stride: 2,
            num_vertices: 4,
            indices: Some(Indices::I32(&RECT_INDEX_ORDER)),
        };
        self.queue_cmd_geometry(Some(t), &geometry, scale_x, scale_y, mode_u, mode_v)
    }

    /// Translation of `SDL_RenderTextureInternal()`.
    fn render_texture_internal(
        &mut self,
        t: Texture,
        srcrect: &FRect,
        dstrect: &FRect,
    ) -> Result<()> {
        let view = self.view();
        let scale_x = view.current_scale.x;
        let scale_y = view.current_scale.y;
        let use_rendergeometry = !self.backend.has_queue_copy();

        if use_rendergeometry {
            let tex = self.tex(t)?;
            let (tw, th) = (tex.w as f32, tex.h as f32);
            let minu = srcrect.x / tw;
            let minv = srcrect.y / th;
            let maxu = (srcrect.x + srcrect.w) / tw;
            let maxv = (srcrect.y + srcrect.h) / th;

            let minx = dstrect.x;
            let miny = dstrect.y;
            let maxx = dstrect.x + dstrect.w;
            let maxy = dstrect.y + dstrect.h;

            let uv = [minu, minv, maxu, minv, maxu, maxv, minu, maxv];
            let xy = [minx, miny, maxx, miny, maxx, maxy, minx, maxy];

            self.queue_textured_quad(
                t,
                &xy,
                &uv,
                scale_x,
                scale_y,
                TextureAddressMode::Clamp,
                TextureAddressMode::Clamp,
            )
        } else {
            let rect = FRect {
                x: dstrect.x * scale_x,
                y: dstrect.y * scale_y,
                w: dstrect.w * scale_x,
                h: dstrect.h * scale_y,
            };
            self.queue_cmd_copy(t, srcrect, &rect)
        }
    }

    /// The source rect of a texture draw: `srcrect` clipped to the
    /// texture, or `None` when nothing is left.
    fn real_srcrect(&self, t: Texture, srcrect: Option<&FRect>) -> Result<Option<FRect>> {
        let tex = self.tex(t)?;
        let full = FRect {
            x: 0.0,
            y: 0.0,
            w: tex.w as f32,
            h: tex.h as f32,
        };
        let mut real_srcrect = full;
        if let Some(srcrect) = srcrect {
            if !intersect_float(srcrect, &full, &mut real_srcrect) {
                return Ok(None);
            }
        }
        Ok(Some(real_srcrect))
    }

    /// Copy (part of) a texture to (part of) the target (`None` = the
    /// whole texture, the whole viewport). Translation of `SDL_RenderTexture()`.
    pub fn render_texture(
        &mut self,
        t: Texture,
        srcrect: Option<&FRect>,
        dstrect: Option<&FRect>,
    ) -> Result<()> {
        let Some(real_srcrect) = self.real_srcrect(t, srcrect)? else {
            return Ok(());
        };

        let dstrect = dstrect
            .copied()
            .unwrap_or_else(|| self.render_viewport_size());

        self.update_texture_palette(t)?;

        let t = self.texture_for_drawing(t)?;
        self.render_texture_internal(t, &real_srcrect, &dstrect)
    }

    /// Copy (part of) a texture to the parallelogram with corners `origin`,
    /// `right` and `down` (`None` = the viewport's top left, top right and
    /// bottom left corner). Translation of `SDL_RenderTextureAffine()`.
    pub fn render_texture_affine(
        &mut self,
        t: Texture,
        srcrect: Option<&FRect>,
        origin: Option<&FPoint>,
        right: Option<&FPoint>,
        down: Option<&FPoint>,
    ) -> Result<()> {
        self.tex(t)?;
        if !self.backend.has_queue_copy_ex() && !self.backend.has_queue_geometry() {
            return Err(Error::new("Renderer does not support RenderCopyEx"));
        }

        let Some(real_srcrect) = self.real_srcrect(t, srcrect)? else {
            return Ok(());
        };

        let real_dstrect = self.render_viewport_size();

        self.update_texture_palette(t)?;

        let t = self.texture_for_drawing(t)?;

        let view = self.view();
        let scale_x = view.current_scale.x;
        let scale_y = view.current_scale.y;

        let tex = self.tex(t)?;
        let (tw, th) = (tex.w as f32, tex.h as f32);
        let minu = real_srcrect.x / tw;
        let minv = real_srcrect.y / th;
        let maxu = (real_srcrect.x + real_srcrect.w) / tw;
        let maxv = (real_srcrect.y + real_srcrect.h) / th;

        let uv = [minu, minv, maxu, minv, maxu, maxv, minu, maxv];
        let mut xy = [0.0f32; 8];

        // (minx, miny)
        (xy[0], xy[1]) = match origin {
            Some(o) => (o.x, o.y),
            None => (real_dstrect.x, real_dstrect.y),
        };

        // (maxx, miny)
        (xy[2], xy[3]) = match right {
            Some(r) => (r.x, r.y),
            None => (real_dstrect.x + real_dstrect.w, real_dstrect.y),
        };

        // (minx, maxy)
        (xy[6], xy[7]) = match down {
            Some(d) => (d.x, d.y),
            None => (real_dstrect.x, real_dstrect.y + real_dstrect.h),
        };

        // (maxx, maxy)
        if origin.is_some() || right.is_some() || down.is_some() {
            xy[4] = xy[2] + xy[6] - xy[0];
            xy[5] = xy[3] + xy[7] - xy[1];
        } else {
            xy[4] = real_dstrect.x + real_dstrect.w;
            xy[5] = real_dstrect.y + real_dstrect.h;
        }

        self.queue_textured_quad(
            t,
            &xy,
            &uv,
            scale_x,
            scale_y,
            TextureAddressMode::Clamp,
            TextureAddressMode::Clamp,
        )
    }

    /// Copy (part of) a texture to (part of) the target, rotated `angle`
    /// degrees clockwise around `center` (`None` = the middle of the
    /// destination) and flipped. Translation of `SDL_RenderTextureRotated()`.
    pub fn render_texture_rotated(
        &mut self,
        t: Texture,
        srcrect: Option<&FRect>,
        dstrect: Option<&FRect>,
        angle: f64,
        center: Option<&FPoint>,
        flip: FlipMode,
    ) -> Result<()> {
        if flip == FlipMode::None && ((angle / 360.0) as i32) as f64 == angle / 360.0 {
            // fast path when we don't need rotation or flipping
            return self.render_texture(t, srcrect, dstrect);
        }

        self.tex(t)?;
        if !self.backend.has_queue_copy_ex() && !self.backend.has_queue_geometry() {
            return Err(Error::new("Renderer does not support RenderCopyEx"));
        }

        let Some(real_srcrect) = self.real_srcrect(t, srcrect)? else {
            return Ok(());
        };

        // We don't intersect the dstrect with the viewport as RenderCopy does because of potential rotation clipping issues... TODO: should we?
        let dstrect = dstrect
            .copied()
            .unwrap_or_else(|| self.render_viewport_size());

        self.update_texture_palette(t)?;

        let real_center = match center {
            Some(c) => *c,
            None => FPoint {
                x: dstrect.w / 2.0,
                y: dstrect.h / 2.0,
            },
        };

        let t = self.texture_for_drawing(t)?;

        let view = self.view();
        let scale_x = view.current_scale.x;
        let scale_y = view.current_scale.y;

        let use_rendergeometry = !self.backend.has_queue_copy_ex();
        if use_rendergeometry {
            let radian_angle = ((std::f64::consts::PI * angle) / 180.0) as f32;
            let s = radian_angle.sin();
            let c = radian_angle.cos();

            let tex = self.tex(t)?;
            let (tw, th) = (tex.w as f32, tex.h as f32);
            let minu = real_srcrect.x / tw;
            let minv = real_srcrect.y / th;
            let maxu = (real_srcrect.x + real_srcrect.w) / tw;
            let maxv = (real_srcrect.y + real_srcrect.h) / th;

            let centerx = real_center.x + dstrect.x;
            let centery = real_center.y + dstrect.y;

            let (minx, maxx) =
                if matches!(flip, FlipMode::Horizontal | FlipMode::HorizontalAndVertical) {
                    (dstrect.x + dstrect.w, dstrect.x)
                } else {
                    (dstrect.x, dstrect.x + dstrect.w)
                };

            let (miny, maxy) =
                if matches!(flip, FlipMode::Vertical | FlipMode::HorizontalAndVertical) {
                    (dstrect.y + dstrect.h, dstrect.y)
                } else {
                    (dstrect.y, dstrect.y + dstrect.h)
                };

            let uv = [minu, minv, maxu, minv, maxu, maxv, minu, maxv];

            /* apply rotation with 2x2 matrix ( c -s )
             *                                ( s  c ) */
            let s_minx = s * (minx - centerx);
            let s_miny = s * (miny - centery);
            let s_maxx = s * (maxx - centerx);
            let s_maxy = s * (maxy - centery);
            let c_minx = c * (minx - centerx);
            let c_miny = c * (miny - centery);
            let c_maxx = c * (maxx - centerx);
            let c_maxy = c * (maxy - centery);

            let xy = [
                // (minx, miny)
                (c_minx - s_miny) + centerx,
                (s_minx + c_miny) + centery,
                // (maxx, miny)
                (c_maxx - s_miny) + centerx,
                (s_maxx + c_miny) + centery,
                // (maxx, maxy)
                (c_maxx - s_maxy) + centerx,
                (s_maxx + c_maxy) + centery,
                // (minx, maxy)
                (c_minx - s_maxy) + centerx,
                (s_minx + c_maxy) + centery,
            ];

            self.queue_textured_quad(
                t,
                &xy,
                &uv,
                scale_x,
                scale_y,
                TextureAddressMode::Clamp,
                TextureAddressMode::Clamp,
            )
        } else {
            self.queue_cmd_copy_ex(
                t,
                &CopyEx {
                    srcquad: real_srcrect,
                    dstrect,
                    angle,
                    center: real_center,
                    flip,
                    scale_x,
                    scale_y,
                },
            )
        }
    }

    /// Translation of `SDL_RenderTextureTiled_Wrap()`.
    fn render_texture_tiled_wrap(
        &mut self,
        t: Texture,
        srcrect: &FRect,
        scale: f32,
        dstrect: &FRect,
    ) -> Result<()> {
        let minu = 0.0;
        let minv = 0.0;
        let maxu = dstrect.w / (srcrect.w * scale);
        let maxv = dstrect.h / (srcrect.h * scale);

        let minx = dstrect.x;
        let miny = dstrect.y;
        let maxx = dstrect.x + dstrect.w;
        let maxy = dstrect.y + dstrect.h;

        let uv = [minu, minv, maxu, minv, maxu, maxv, minu, maxv];
        let xy = [minx, miny, maxx, miny, maxx, maxy, minx, maxy];

        let view = self.view();
        let (scale_x, scale_y) = (view.current_scale.x, view.current_scale.y);
        self.queue_textured_quad(
            t,
            &xy,
            &uv,
            scale_x,
            scale_y,
            TextureAddressMode::Wrap,
            TextureAddressMode::Wrap,
        )
    }

    /// Translation of `SDL_RenderTextureTiled_Iterate()`.
    fn render_texture_tiled_iterate(
        &mut self,
        t: Texture,
        srcrect: &FRect,
        scale: f32,
        dstrect: &FRect,
    ) -> Result<()> {
        let tile_width = srcrect.w * scale;
        let tile_height = srcrect.h * scale;
        let (remaining_w, float_cols) = modff(dstrect.w / tile_width);
        let (remaining_h, float_rows) = modff(dstrect.h / tile_height);
        let remaining_src_w = remaining_w * srcrect.w;
        let remaining_src_h = remaining_h * srcrect.h;
        let remaining_dst_w = remaining_w * tile_width;
        let remaining_dst_h = remaining_h * tile_height;
        // (an empty tile makes infinitely many: no tiles, as x86 converts them)
        let rows = c_float_to_int(float_rows);
        let cols = c_float_to_int(float_cols);

        let mut curr_src = *srcrect;
        let mut curr_dst = FRect {
            x: 0.0,
            y: dstrect.y,
            w: tile_width,
            h: tile_height,
        };
        for _ in 0..rows {
            curr_dst.x = dstrect.x;
            for _ in 0..cols {
                self.render_texture_internal(t, &curr_src, &curr_dst)?;
                curr_dst.x += curr_dst.w;
            }
            if remaining_dst_w > 0.0 {
                curr_src.w = remaining_src_w;
                curr_dst.w = remaining_dst_w;
                self.render_texture_internal(t, &curr_src, &curr_dst)?;
                curr_src.w = srcrect.w;
                curr_dst.w = tile_width;
            }
            curr_dst.y += curr_dst.h;
        }
        if remaining_dst_h > 0.0 {
            curr_src.h = remaining_src_h;
            curr_dst.h = remaining_dst_h;
            curr_dst.x = dstrect.x;
            for _ in 0..cols {
                self.render_texture_internal(t, &curr_src, &curr_dst)?;
                curr_dst.x += curr_dst.w;
            }
            if remaining_dst_w > 0.0 {
                curr_src.w = remaining_src_w;
                curr_dst.w = remaining_dst_w;
                self.render_texture_internal(t, &curr_src, &curr_dst)?;
            }
        }
        Ok(())
    }

    /// Tile (part of) a texture over (part of) the target, each tile
    /// scaled by `scale`. Translation of `SDL_RenderTextureTiled()`.
    pub fn render_texture_tiled(
        &mut self,
        t: Texture,
        srcrect: Option<&FRect>,
        scale: f32,
        dstrect: Option<&FRect>,
    ) -> Result<()> {
        self.tex(t)?;

        if scale <= 0.0 {
            return Err(Error::invalid_param("scale"));
        }

        let Some(real_srcrect) = self.real_srcrect(t, srcrect)? else {
            return Ok(());
        };

        let dstrect = dstrect
            .copied()
            .unwrap_or_else(|| self.render_viewport_size());

        self.update_texture_palette(t)?;

        let t = self.texture_for_drawing(t)?;
        let tex = self.tex(t)?;

        let mut do_wrapping = !self.software
            && (srcrect.is_none()
                || (real_srcrect.x == 0.0
                    && real_srcrect.y == 0.0
                    && real_srcrect.w == tex.w as f32
                    && real_srcrect.h == tex.h as f32));
        if do_wrapping && self.npot_texture_wrap_unsupported && (is_npot(tex.w) || is_npot(tex.h)) {
            do_wrapping = false;
        }

        // See if we can use geometry with repeating texture coordinates
        if do_wrapping {
            self.render_texture_tiled_wrap(t, &real_srcrect, scale, &dstrect)
        } else {
            self.render_texture_tiled_iterate(t, &real_srcrect, scale, &dstrect)
        }
    }

    /// Draw the nine parts of a 9-grid (`draw_center` for the center and
    /// edges: a stretched copy or tiles).
    #[allow(clippy::too_many_arguments)]
    fn render_9grid(
        &mut self,
        t: Texture,
        srcrect: Option<&FRect>,
        left_width: f32,
        right_width: f32,
        top_height: f32,
        bottom_height: f32,
        scale: f32,
        dstrect: Option<&FRect>,
        tile_scale: Option<f32>,
    ) -> Result<()> {
        let tex = self.tex(t)?;

        let srcrect = srcrect.copied().unwrap_or(FRect {
            x: 0.0,
            y: 0.0,
            w: tex.w as f32,
            h: tex.h as f32,
        });

        let dstrect = dstrect
            .copied()
            .unwrap_or_else(|| self.render_viewport_size());

        let (dst_left_width, dst_right_width, dst_top_height, dst_bottom_height) =
            if scale <= 0.0 || scale == 1.0 {
                (
                    left_width.ceil(),
                    right_width.ceil(),
                    top_height.ceil(),
                    bottom_height.ceil(),
                )
            } else {
                (
                    (left_width * scale).ceil(),
                    (right_width * scale).ceil(),
                    (top_height * scale).ceil(),
                    (bottom_height * scale).ceil(),
                )
            };

        // The center and edges are tiled in a tiled 9-grid.
        let draw_stretchable = |r: &mut Renderer, src: &FRect, dst: &FRect| match tile_scale {
            Some(tile_scale) => r.render_texture_tiled(t, Some(src), tile_scale, Some(dst)),
            None => r.render_texture(t, Some(src), Some(dst)),
        };

        // Center
        let mut curr_src = FRect {
            x: srcrect.x + left_width,
            y: srcrect.y + top_height,
            w: srcrect.w - left_width - right_width,
            h: srcrect.h - top_height - bottom_height,
        };
        let mut curr_dst = FRect {
            x: dstrect.x + dst_left_width,
            y: dstrect.y + dst_top_height,
            w: dstrect.w - dst_left_width - dst_right_width,
            h: dstrect.h - dst_top_height - dst_bottom_height,
        };
        draw_stretchable(self, &curr_src, &curr_dst)?;

        // Upper-left corner
        curr_src.x = srcrect.x;
        curr_src.y = srcrect.y;
        curr_src.w = left_width;
        curr_src.h = top_height;
        curr_dst.x = dstrect.x;
        curr_dst.y = dstrect.y;
        curr_dst.w = dst_left_width;
        curr_dst.h = dst_top_height;
        self.render_texture(t, Some(&curr_src), Some(&curr_dst))?;

        // Upper-right corner
        curr_src.x = srcrect.x + srcrect.w - right_width;
        curr_src.w = right_width;
        curr_dst.x = dstrect.x + dstrect.w - dst_right_width;
        curr_dst.w = dst_right_width;
        self.render_texture(t, Some(&curr_src), Some(&curr_dst))?;

        // Lower-right corner
        curr_src.y = srcrect.y + srcrect.h - bottom_height;
        curr_src.h = bottom_height;
        curr_dst.y = dstrect.y + dstrect.h - dst_bottom_height;
        curr_dst.h = dst_bottom_height;
        self.render_texture(t, Some(&curr_src), Some(&curr_dst))?;

        // Lower-left corner
        curr_src.x = srcrect.x;
        curr_src.w = left_width;
        curr_dst.x = dstrect.x;
        curr_dst.w = dst_left_width;
        self.render_texture(t, Some(&curr_src), Some(&curr_dst))?;

        // Left
        curr_src.y = srcrect.y + top_height;
        curr_src.h = srcrect.h - top_height - bottom_height;
        curr_dst.y = dstrect.y + dst_top_height;
        curr_dst.h = dstrect.h - dst_top_height - dst_bottom_height;
        draw_stretchable(self, &curr_src, &curr_dst)?;

        // Right
        curr_src.x = srcrect.x + srcrect.w - right_width;
        curr_src.w = right_width;
        curr_dst.x = dstrect.x + dstrect.w - dst_right_width;
        curr_dst.w = dst_right_width;
        draw_stretchable(self, &curr_src, &curr_dst)?;

        // Top
        curr_src.x = srcrect.x + left_width;
        curr_src.y = srcrect.y;
        curr_src.w = srcrect.w - left_width - right_width;
        curr_src.h = top_height;
        curr_dst.x = dstrect.x + dst_left_width;
        curr_dst.y = dstrect.y;
        curr_dst.w = dstrect.w - dst_left_width - dst_right_width;
        curr_dst.h = dst_top_height;
        draw_stretchable(self, &curr_src, &curr_dst)?;

        // Bottom
        curr_src.y = srcrect.y + srcrect.h - bottom_height;
        curr_src.h = bottom_height;
        curr_dst.y = dstrect.y + dstrect.h - dst_bottom_height;
        curr_dst.h = dst_bottom_height;
        draw_stretchable(self, &curr_src, &curr_dst)
    }

    /// Draw a texture as a 9-grid: the corners (`left_width`, `right_width`,
    /// `top_height` and `bottom_height` pixels of the source, scaled by
    /// `scale`) are copied as they are, the edges and the center stretched.
    /// Translation of `SDL_RenderTexture9Grid()`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_texture_9grid(
        &mut self,
        t: Texture,
        srcrect: Option<&FRect>,
        left_width: f32,
        right_width: f32,
        top_height: f32,
        bottom_height: f32,
        scale: f32,
        dstrect: Option<&FRect>,
    ) -> Result<()> {
        self.render_9grid(
            t,
            srcrect,
            left_width,
            right_width,
            top_height,
            bottom_height,
            scale,
            dstrect,
            None,
        )
    }

    /// Draw a texture as a 9-grid whose edges and center are tiled, each
    /// tile scaled by `tile_scale`. Translation of `SDL_RenderTexture9GridTiled()`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_texture_9grid_tiled(
        &mut self,
        t: Texture,
        srcrect: Option<&FRect>,
        left_width: f32,
        right_width: f32,
        top_height: f32,
        bottom_height: f32,
        scale: f32,
        dstrect: Option<&FRect>,
        tile_scale: f32,
    ) -> Result<()> {
        self.render_9grid(
            t,
            srcrect,
            left_width,
            right_width,
            top_height,
            bottom_height,
            scale,
            dstrect,
            Some(tile_scale),
        )
    }

    /// Draw triangles: every three vertices (or `indices` into them) are a
    /// triangle, colored and optionally textured.
    /// Translation of `SDL_RenderGeometry()`.
    pub fn render_geometry(
        &mut self,
        texture: Option<Texture>,
        vertices: &[Vertex],
        indices: Option<&[i32]>,
    ) -> Result<()> {
        // The vertices, interleaved as SDL_Vertex: position, color, texture coordinates.
        let floats: Vec<f32> = vertices
            .iter()
            .flat_map(|v| {
                [
                    v.position.x,
                    v.position.y,
                    v.color.r,
                    v.color.g,
                    v.color.b,
                    v.color.a,
                    v.tex_coord.x,
                    v.tex_coord.y,
                ]
            })
            .collect();
        const STRIDE: usize = 8;
        if vertices.is_empty() {
            return Err(Error::invalid_param("vertices"));
        }
        let geometry = Geometry {
            xy: &floats,
            xy_stride: STRIDE,
            color: &floats[2..],
            color_stride: STRIDE,
            uv: &floats[6..],
            uv_stride: STRIDE,
            num_vertices: vertices.len(),
            indices: indices.map(Indices::I32),
        };
        self.render_geometry_internal(texture, &geometry)
    }

    /// Draw triangles from separate position, color and texture coordinate
    /// arrays: vertex `i` is at `xy[i * xy_stride..]` (two floats), with
    /// color `color[i * color_stride]` (a stride of 0 uses one color for
    /// all vertices) and texture coordinates `uv[i * uv_stride..]` (two
    /// floats; `uv` may be empty without a texture). Every three vertices
    /// (or `indices` into them) are a triangle.
    /// Translation of `SDL_RenderGeometryRaw()`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_geometry_raw(
        &mut self,
        texture: Option<Texture>,
        xy: &[f32],
        xy_stride: usize,
        color: &[FColor],
        color_stride: usize,
        uv: &[f32],
        uv_stride: usize,
        num_vertices: usize,
        indices: Option<Indices<'_>>,
    ) -> Result<()> {
        if xy.is_empty() {
            return Err(Error::invalid_param("xy"));
        }
        if color.is_empty() {
            return Err(Error::invalid_param("color"));
        }
        if texture.is_some() && uv.is_empty() {
            return Err(Error::invalid_param("uv"));
        }

        // (upstream reads through the pointers; the slices must hold every vertex)
        let last = num_vertices.saturating_sub(1);
        if num_vertices > 0 {
            if xy.len() < last * xy_stride + 2 {
                return Err(Error::invalid_param("xy"));
            }
            if color.len() < last * color_stride + 1 {
                return Err(Error::invalid_param("color"));
            }
            if !uv.is_empty() && uv.len() < last * uv_stride + 2 {
                return Err(Error::invalid_param("uv"));
            }
        }

        let color_floats: Vec<f32> = color.iter().flat_map(|c| fcolor_floats(*c)).collect();
        let geometry = Geometry {
            xy,
            xy_stride,
            color: &color_floats,
            // (in floats, as the other strides)
            color_stride: color_stride * 4,
            uv,
            uv_stride,
            num_vertices,
            indices,
        };
        self.render_geometry_internal(texture, &geometry)
    }

    /// The body of `SDL_RenderGeometryRaw()`.
    fn render_geometry_internal(
        &mut self,
        texture: Option<Texture>,
        g: &Geometry<'_>,
    ) -> Result<()> {
        let count = g.count();
        let mut texture_address_mode_u = TextureAddressMode::Clamp;
        let mut texture_address_mode_v = TextureAddressMode::Clamp;

        if let Some(t) = texture {
            self.tex(t)?;
        }

        if count % 3 != 0 {
            return Err(Error::invalid_param(if g.indices.is_some() {
                "num_indices"
            } else {
                "num_vertices"
            }));
        }

        if !self.backend.has_queue_geometry() {
            return Err(Error::unsupported());
        }

        if g.num_vertices < 3 {
            return Ok(());
        }

        let mut texture = texture;
        if let Some(t) = texture {
            self.update_texture_palette(t)?;

            let t = self.tex(t)?.native.unwrap_or(t);
            texture = Some(t);
            let tex = self.tex(t)?;

            texture_address_mode_u = if self.npot_texture_wrap_unsupported && is_npot(tex.w) {
                TextureAddressMode::Clamp
            } else {
                self.texture_address_mode_u
            };
            texture_address_mode_v = if self.npot_texture_wrap_unsupported && is_npot(tex.h) {
                TextureAddressMode::Clamp
            } else {
                self.texture_address_mode_v
            };

            if texture_address_mode_u == TextureAddressMode::Auto
                || texture_address_mode_v == TextureAddressMode::Auto
            {
                for i in 0..g.num_vertices {
                    let (u, v) = g.uv(i);
                    if (!(0.0..=1.0).contains(&u))
                        && texture_address_mode_u == TextureAddressMode::Auto
                    {
                        texture_address_mode_u = TextureAddressMode::Wrap;
                        if texture_address_mode_v != TextureAddressMode::Auto {
                            break;
                        }
                    }
                    if (!(0.0..=1.0).contains(&v))
                        && texture_address_mode_v == TextureAddressMode::Auto
                    {
                        texture_address_mode_v = TextureAddressMode::Wrap;
                        if texture_address_mode_u != TextureAddressMode::Auto {
                            break;
                        }
                    }
                }
                if texture_address_mode_u == TextureAddressMode::Auto {
                    texture_address_mode_u = TextureAddressMode::Clamp;
                }
                if texture_address_mode_v == TextureAddressMode::Auto {
                    texture_address_mode_v = TextureAddressMode::Clamp;
                }
            }
        }

        if let Some(indices) = g.indices {
            for i in 0..indices.len() {
                let j = indices.get(i);
                if j < 0 || j as usize >= g.num_vertices {
                    return Err(Error::new("Values of 'indices' out of bounds"));
                }
            }
        }

        if let Some(t) = texture {
            let generation = self.render_command_generation;
            self.tex_mut(t)?.last_command_generation = generation;
        }

        // For the software renderer, try to reinterpret triangles as SDL_Rect
        if self.software
            && texture_address_mode_u == TextureAddressMode::Clamp
            && texture_address_mode_v == TextureAddressMode::Clamp
        {
            return self.sw_render_geometry_raw(texture, g);
        }

        let view = self.view();
        let (scale_x, scale_y) = (view.current_scale.x, view.current_scale.y);
        self.queue_cmd_geometry(
            texture,
            g,
            scale_x,
            scale_y,
            texture_address_mode_u,
            texture_address_mode_v,
        )
    }

    /// Translation of `SDL_SW_RenderGeometryRaw()`: draw the quads made of
    /// two triangles as rects, the other triangles as geometry.
    fn sw_render_geometry_raw(&mut self, texture: Option<Texture>, g: &Geometry<'_>) -> Result<()> {
        // Save
        let blend_mode = self.draw_blend_mode();
        let color = self.draw_color_float();

        let result = self.sw_render_geometry_raw_body(texture, g);

        // Restore
        let _ = self.set_draw_blend_mode(blend_mode);
        self.set_draw_color_float(color.r, color.g, color.b, color.a);

        result
    }

    fn sw_render_geometry_raw_body(
        &mut self,
        texture: Option<Texture>,
        g: &Geometry<'_>,
    ) -> Result<()> {
        let count = g.count() as i32;
        let mut prev: [i32; 3] = [-1, -1, -1]; // Previous triangle vertex indices
        let view = self.view();
        let scale_x = view.current_scale.x;
        let scale_y = view.current_scale.y;

        let (texw, texh) = match texture {
            Some(t) => self.texture_size(t)?,
            None => (0.0, 0.0),
        };

        let xy = |k: i32| g.xy(k as usize);
        let color = |k: i32| g.color(k as usize);
        let has_texture = texture.is_some();

        // Translation of `remap_one_indice()`.
        let remap_one_indice = |prev: i32, k: i32| -> i32 {
            let (x0, y0) = xy(prev);
            let (x1, y1) = xy(k);
            if x0 != x1 {
                return k;
            }
            if y0 != y1 {
                return k;
            }
            if has_texture {
                let (u0, v0) = g.uv(prev as usize);
                let (u1, v1) = g.uv(k as usize);
                if u0 != u1 {
                    return k;
                }
                if v0 != v1 {
                    return k;
                }
            }
            if !same_color(color(prev), color(k)) {
                return k;
            }
            prev
        };

        // Translation of `remap_indices()`.
        let remap_indices = |prev: &[i32; 3], k: i32| -> i32 {
            if prev[0] == -1 {
                return k;
            }
            for p in prev {
                let new_k = remap_one_indice(*p, k);
                if new_k != k {
                    return new_k;
                }
            }
            k
        };

        let queue_triangle = |r: &mut Renderer, prev: &[i32; 3]| {
            let triangle = Geometry {
                indices: Some(Indices::I32(prev)),
                ..*g
            };
            r.queue_cmd_geometry(
                texture,
                &triangle,
                scale_x,
                scale_y,
                TextureAddressMode::Clamp,
                TextureAddressMode::Clamp,
            )
        };

        let mut i = 0;
        while i < count {
            let (k0, k1, k2); // Current triangle indices
            let mut is_quad;
            let mut a = -1; // Top left vertex
            let mut b = -1; // Bottom right vertex
            let mut c = -1; // Third vertex of current triangle
            let mut c2 = -1; // Last, vertex of previous triangle

            match g.indices {
                Some(indices) => {
                    k0 = indices.get(i as usize);
                    k1 = indices.get(i as usize + 1);
                    k2 = indices.get(i as usize + 2);
                }
                None => {
                    /* Vertices were not provided by indices. Maybe some are duplicated.
                     * We try to indentificate the duplicates by comparing with the previous three vertices */
                    k0 = remap_indices(&prev, i);
                    k1 = remap_indices(&prev, i + 1);
                    k2 = remap_indices(&prev, i + 2);
                }
            }

            if prev[0] == -1 {
                prev = [k0, k1, k2];
                i += 3;
                continue;
            }

            /* Two triangles forming a quadrilateral,
             * prev and current triangles must have exactly 2 common vertices */
            {
                let cnt = prev
                    .iter()
                    .filter(|p| **p == k0 || **p == k1 || **p == k2)
                    .count();
                is_quad = cnt == 2;
            }

            // Identify vertices
            if is_quad {
                let (x0, y0) = xy(k0);
                let (x1, y1) = xy(k1);
                let (x2, y2) = xy(k2);

                // Find top-left
                if x0 <= x1 && y0 <= y1 {
                    a = if x0 <= x2 && y0 <= y2 { k0 } else { k2 };
                } else {
                    a = if x1 <= x2 && y1 <= y2 { k1 } else { k2 };
                }

                // Find bottom-right
                if x0 >= x1 && y0 >= y1 {
                    b = if x0 >= x2 && y0 >= y2 { k0 } else { k2 };
                } else {
                    b = if x1 >= x2 && y1 >= y2 { k1 } else { k2 };
                }

                // Find C
                c = if k0 != a && k0 != b {
                    k0
                } else if k1 != a && k1 != b {
                    k1
                } else {
                    k2
                };

                // Find C2
                c2 = if prev[0] != a && prev[0] != b {
                    prev[0]
                } else if prev[1] != a && prev[1] != b {
                    prev[1]
                } else {
                    prev[2]
                };

                let (x0, y0) = xy(a);
                let (x1, y1) = xy(b);
                let (x2, y2) = xy(c);

                // Check if triangle A B C is rectangle
                if (x0 == x2 && y1 == y2) || (y0 == y2 && x1 == x2) {
                    // ok
                } else {
                    is_quad = false;
                }

                let (x2, y2) = xy(c2);

                // Check if triangle A B C2 is rectangle
                if (x0 == x2 && y1 == y2) || (y0 == y2 && x1 == x2) {
                    // ok
                } else {
                    is_quad = false;
                }
            }

            // Check if uniformly colored
            if is_quad {
                let col0 = color(a);
                if same_color(col0, color(b))
                    && same_color(col0, color(c))
                    && same_color(col0, color(c2))
                {
                    // ok
                } else {
                    is_quad = false;
                }
            }

            // Check if UVs within range
            if is_quad && !g.uv.is_empty() {
                // FIXME (upstream): the texture coordinates are looked up
                // with the color stride instead of the texture coordinate
                // stride (the same for SDL_Vertex arrays). A lookup outside
                // the coordinates counts as out of range here.
                let u = |k: i32| g.uv.get(k as usize * g.color_stride).copied();
                let in_range = |k: i32| u(k).is_some_and(|u| (0.0..=1.0).contains(&u));
                if in_range(a) && in_range(b) && in_range(c) && in_range(c2) {
                    // ok
                } else {
                    is_quad = false;
                }
            }

            // Start rendering rect
            if is_quad {
                let col0 = color(k0);

                let mut s = FRect::default();
                if has_texture {
                    let (u0, v0) = g.uv(a as usize);
                    let (u1, v1) = g.uv(b as usize);
                    s.x = u0 * texw;
                    s.y = v0 * texh;
                    s.w = u1 * texw - s.x;
                    s.h = v1 * texh - s.y;
                }

                let (dx0, dy0) = xy(a);
                let (dx1, dy1) = xy(b);
                let d = FRect {
                    x: dx0,
                    y: dy0,
                    w: dx1 - dx0,
                    h: dy1 - dy0,
                };

                match texture {
                    // Rect + texture
                    Some(t) if s.w != 0.0 && s.h != 0.0 => {
                        let _ = self.set_texture_alpha_mod_float(t, col0.a);
                        let _ = self.set_texture_color_mod_float(t, col0.r, col0.g, col0.b);
                        if s.w > 0.0 && s.h > 0.0 {
                            let _ = self.render_texture(t, Some(&s), Some(&d));
                        } else {
                            let (mut flip_h, mut flip_v) = (false, false);
                            if s.w < 0.0 {
                                flip_h = true;
                                s.w *= -1.0;
                                s.x -= s.w;
                            }
                            if s.h < 0.0 {
                                flip_v = true;
                                s.h *= -1.0;
                                s.y -= s.h;
                            }
                            let _ = self.render_texture_rotated(
                                t,
                                Some(&s),
                                Some(&d),
                                0.0,
                                None,
                                flip_mode(flip_h, flip_v),
                            );
                        }
                    }
                    _ => {
                        if d.w != 0.0 && d.h != 0.0 {
                            // Rect, no texture
                            let _ = self.set_draw_blend_mode(BlendMode::BLEND);
                            self.set_draw_color_float(col0.r, col0.g, col0.b, col0.a);
                            let _ = self.render_fill_rect(Some(&d));
                        }
                    }
                }

                prev[0] = -1;
            } else {
                // Render triangles
                if prev[0] != -1 {
                    queue_triangle(self, &prev)?;
                }

                prev = [k0, k1, k2];
            }
            i += 3;
        } // End for (), next triangle

        if prev[0] != -1 {
            // flush the last triangle
            queue_triangle(self, &prev)?;
        }

        Ok(())
    }

    /// Translation of `CreateDebugTextAtlas()`.
    fn create_debug_text_atlas(&mut self) -> Result<()> {
        const COLORS: [Color; 2] = [Color::new(255, 255, 255, 0), Color::new(255, 255, 255, 255)];

        crate::sdl_assert!(self.debug_char_texture_atlas.is_none()); // don't double-create it!

        let char_width = DEBUG_TEXT_FONT_CHARACTER_SIZE as usize;
        let char_height = DEBUG_TEXT_FONT_CHARACTER_SIZE as usize;

        // actually make each glyph two pixels taller/wider, to prevent scaling artifacts.
        let rows = (DEBUG_FONT_NUM_GLYPHS / DEBUG_FONT_GLYPHS_PER_ROW) + 1;
        let mut atlas = Surface::new(
            ((char_width + 2) * DEBUG_FONT_GLYPHS_PER_ROW) as i32,
            (rows * (char_height + 2)) as i32,
            PixelFormat::INDEX8,
        )?;

        let palette = atlas.create_palette()?;
        write_palette(&palette).set_colors(0, &COLORS)?;

        let pitch = atlas.pitch() as usize;
        let pixels = atlas
            .pixels_mut()
            .ok_or_else(|| Error::new("Couldn't access the atlas pixels"))?;
        pixels.fill(0);

        let mut column = 0;
        let mut row = 0;
        for glyph in 0..DEBUG_FONT_NUM_GLYPHS {
            // find top-left of this glyph in destination surface. The +2's account for glyph padding.
            let mut linepos =
                ((row * (char_height + 2) + 1) * pitch) + (column * (char_width + 2) + 1);
            let charpos = &DEBUG_TEXT_FONT_DATA[glyph * 8..glyph * 8 + 8];

            // Draw the glyph to the surface...
            for bits in charpos.iter().take(char_height) {
                for ix in 0..char_width {
                    pixels[linepos + ix] = (bits >> ix) & 1;
                }
                linepos += pitch;
            }

            // move to next position (and if too far, start the next row).
            column += 1;
            if column >= DEBUG_FONT_GLYPHS_PER_ROW {
                row += 1;
                column = 0;
            }
        }

        crate::sdl_assert!((row < rows) || ((row == rows) && (column == 0))); // make sure we didn't overflow the surface.

        // Convert temp surface into texture
        let texture = self.create_texture_from_surface(&mut atlas)?;
        let _ = self.set_texture_scale_mode(texture, ScaleMode::PixelArt);
        let _ = self.set_texture_blend_mode(texture, BlendMode::BLEND);
        self.debug_char_texture_atlas = Some(texture);
        Ok(())
    }

    /// Translation of `DrawDebugCharacter()`.
    fn draw_debug_character(&mut self, x: f32, y: f32, c: u32) -> Result<()> {
        let atlas = self.debug_char_texture_atlas.ok_or_else(invalid_texture)?; // should have been created by now!

        let char_width = DEBUG_TEXT_FONT_CHARACTER_SIZE as u32;
        let char_height = DEBUG_TEXT_FONT_CHARACTER_SIZE as u32;

        // Character index in cache
        let mut ci = c;
        if (ci <= 32) || ((127..=160).contains(&ci)) {
            return Ok(()); // these are just completely blank chars, don't bother doing anything.
        } else if ci >= DEBUG_FONT_NUM_GLYPHS as u32 {
            ci = DEBUG_FONT_NUM_GLYPHS as u32 - 1; // use our "not a valid/supported character" glyph.
        } else if ci < 127 {
            ci -= 33; // adjust for the 33 blank glyphs at the start
        } else {
            ci -= 67; // adjust for the 33 blank glyphs at the start AND the 34 gap in the middle.
        }

        let per_row = DEBUG_FONT_GLYPHS_PER_ROW as u32;
        let src_x = (((ci % per_row) * (char_width + 2)) + 1) as f32;
        let src_y = (((ci / per_row) * (char_height + 2)) + 1) as f32;

        // Draw texture onto destination
        let srect = FRect {
            x: src_x,
            y: src_y,
            w: char_width as f32,
            h: char_height as f32,
        };
        let drect = FRect {
            x,
            y,
            w: char_width as f32,
            h: char_height as f32,
        };
        self.render_texture(atlas, Some(&srect), Some(&drect))
    }

    /// Draw text with the built in 8x8 debug font, in the draw color, with
    /// its top left corner at (`x`, `y`). Translation of
    /// `SDL_RenderDebugText()` (format text with `format!` for
    /// `SDL_RenderDebugTextFormat()`).
    pub fn render_debug_text(&mut self, x: f32, y: f32, s: &str) -> Result<()> {
        // Allocate a texture atlas for this renderer if needed.
        if self.debug_char_texture_atlas.is_none() {
            self.create_debug_text_atlas()?;
        }
        let atlas = self.debug_char_texture_atlas.ok_or_else(invalid_texture)?;

        let (r, g, b, a) = self.draw_color();
        self.set_texture_color_mod(atlas, r, g, b)?;
        self.set_texture_alpha_mod(atlas, a)?;

        let mut curx = x;
        // (the C string ends at a NUL character)
        for ch in s.chars().take_while(|c| *c != '\0') {
            self.draw_debug_character(curx, y, ch as u32)?;
            curx += DEBUG_TEXT_FONT_CHARACTER_SIZE as f32;
        }

        Ok(())
    }
}

/// What the callbacks of `with_native_lock()` see of the native texture.
struct NativeView {
    format: PixelFormat,
    colorspace: Colorspace,
    w: i32,
    h: i32,
}
