// Rust translation of src/video/SDL_surface.c and SDL_surface_c.h from
// Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Surfaces: a block of pixels in a [`PixelFormat`], with the blending,
//! colorkey and modulation state used when it is blitted somewhere.
//! Translation of `SDL_Surface` and `SDL_surface.c`.
//!
//! A [`Surface`] owns its pixels (`Surface<'static>`, from [`Surface::new`]
//! or [`Surface::from_vec`]) or borrows them from the caller
//! ([`Surface::from_pixels`], the `SDL_CreateSurfaceFrom()` case, which
//! upstream marks `SDL_SURFACE_PREALLOCATED`). Destroying a surface is
//! `Drop`; the reference count is Rust ownership.
//!
//! Differences in shape from the C API, none in behaviour:
//!
//! * Palettes are shared as [`SharedPalette`] (`Arc<RwLock<Palette>>`),
//!   upstream's reference-counted `SDL_Palette *`. Changing the colors bumps
//!   the palette's version, which invalidates blit maps exactly as upstream.
//! * Alternate images are owned by the surface. Upstream shares them by
//!   reference count, so converting or duplicating a surface here copies
//!   its alternate images instead of sharing them.
//! * Blits take the source as `&mut self` (blitting updates the source's
//!   cached blit map) and the destination as `&mut Surface`, so a surface
//!   can't be blitted onto itself.
//! * [`Surface::pixels_mut`] is `None` while [`Surface::must_lock`] is true;
//!   use [`Surface::lock`] then, as the C documentation requires.

mod blit;
mod convert;
mod fill;

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::error::{Error, Result};
use crate::properties::Properties;
use crate::video::blendmode::BlendMode;
use crate::video::blit::{
    BlitMap, COPY_ADD, COPY_ADD_PREMULTIPLIED, COPY_BLEND, COPY_BLEND_MASK,
    COPY_BLEND_PREMULTIPLIED, COPY_COLORKEY, COPY_MOD, COPY_MODULATE_ALPHA, COPY_MODULATE_COLOR,
    COPY_MUL, COPY_RLE_DESIRED,
};
use crate::video::pixels::{
    Color, Colorspace, FColor, Palette, PixelFormat, PixelFormatDetails, TransferCharacteristics,
};
use crate::video::rect::Rect;

#[allow(unused_imports)] // for the window and clipboard code
pub(crate) use convert::duplicate_pixels;
pub use convert::{convert_pixels, convert_pixels_and_colorspace, premultiply_alpha};

/// A palette shared between surfaces (upstream's reference-counted
/// `SDL_Palette *`).
pub type SharedPalette = Arc<RwLock<Palette>>;

/// Wrap a palette for sharing between surfaces.
pub fn share_palette(palette: Palette) -> SharedPalette {
    Arc::new(RwLock::new(palette))
}

pub(crate) fn read_palette(p: &SharedPalette) -> RwLockReadGuard<'_, Palette> {
    p.read().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn write_palette(p: &SharedPalette) -> RwLockWriteGuard<'_, Palette> {
    p.write().unwrap_or_else(|e| e.into_inner())
}

/// The surface's SDR white point, in nits; a float. Translation of `SDL_PROP_SURFACE_SDR_WHITE_POINT_FLOAT`.
pub const PROP_SURFACE_SDR_WHITE_POINT_FLOAT: &str = "SDL.surface.SDR_white_point";
/// The surface's maximum luminance as a multiple of the SDR white point; a float.
/// Translation of `SDL_PROP_SURFACE_HDR_HEADROOM_FLOAT`.
pub const PROP_SURFACE_HDR_HEADROOM_FLOAT: &str = "SDL.surface.HDR_headroom";
/// The tone mapping operator used when compressing from HDR to SDR ("chrome",
/// "*=N" or "none"). Translation of `SDL_PROP_SURFACE_TONEMAP_OPERATOR_STRING`.
pub const PROP_SURFACE_TONEMAP_OPERATOR_STRING: &str = "SDL.surface.tonemap";
/// The hotspot x coordinate for a cursor surface. Translation of `SDL_PROP_SURFACE_HOTSPOT_X_NUMBER`.
pub const PROP_SURFACE_HOTSPOT_X_NUMBER: &str = "SDL.surface.hotspot.x";
/// The hotspot y coordinate for a cursor surface. Translation of `SDL_PROP_SURFACE_HOTSPOT_Y_NUMBER`.
pub const PROP_SURFACE_HOTSPOT_Y_NUMBER: &str = "SDL.surface.hotspot.y";
/// The number of degrees the surface should be rotated clockwise to display
/// correctly. Translation of `SDL_PROP_SURFACE_ROTATION_FLOAT`.
pub const PROP_SURFACE_ROTATION_FLOAT: &str = "SDL.surface.rotation";

/// The flags on a surface. Translation of `SDL_SurfaceFlags`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SurfaceFlags(pub u32);

impl SurfaceFlags {
    /// Surface uses preallocated pixel memory
    pub const PREALLOCATED: SurfaceFlags = SurfaceFlags(0x00000001);
    /// Surface needs to be locked to access pixels
    pub const LOCK_NEEDED: SurfaceFlags = SurfaceFlags(0x00000002);
    /// Surface is currently locked
    pub const LOCKED: SurfaceFlags = SurfaceFlags(0x00000004);
    /// Surface uses pixel memory aligned for SIMD (`SDL_aligned_alloc()`)
    pub const SIMD_ALIGNED: SurfaceFlags = SurfaceFlags(0x00000008);

    pub const fn contains(self, other: SurfaceFlags) -> bool {
        self.0 & other.0 == other.0
    }
    fn insert(&mut self, other: SurfaceFlags) {
        self.0 |= other.0;
    }
    fn remove(&mut self, other: SurfaceFlags) {
        self.0 &= !other.0;
    }
}

impl std::fmt::Debug for SurfaceFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = [
            (SurfaceFlags::PREALLOCATED, "PREALLOCATED"),
            (SurfaceFlags::LOCK_NEEDED, "LOCK_NEEDED"),
            (SurfaceFlags::LOCKED, "LOCKED"),
            (SurfaceFlags::SIMD_ALIGNED, "SIMD_ALIGNED"),
        ];
        let set: Vec<&str> = names
            .iter()
            .filter(|(v, _)| self.contains(*v))
            .map(|(_, n)| *n)
            .collect();
        write!(
            f,
            "SurfaceFlags({})",
            if set.is_empty() {
                "0".to_string()
            } else {
                set.join(" | ")
            }
        )
    }
}

/// The scaling mode. Translation of `SDL_ScaleMode`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ScaleMode {
    /// nearest pixel sampling
    Nearest,
    /// linear filtering
    #[default]
    Linear,
    /// nearest pixel sampling with improved scaling for pixel art
    PixelArt,
}

/// The flip mode. Translation of `SDL_FlipMode`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum FlipMode {
    /// Do not flip
    #[default]
    None,
    /// flip horizontally
    Horizontal,
    /// flip vertically
    Vertical,
    /// flip horizontally and vertically (not a diagonal flip)
    HorizontalAndVertical,
}

// Surface internal flags
/// Surface is RLE encoded. Translation of `SDL_INTERNAL_SURFACE_RLEACCEL`.
pub(crate) const INTERNAL_SURFACE_RLEACCEL: u32 = 0x00000004;

/// Where a surface's pixels live.
pub(crate) enum Pixels<'a> {
    /// `pixels == NULL`.
    None,
    /// Allocated by SDL. `offset` aligns the start for SIMD when requested.
    Owned {
        buf: Vec<u8>,
        offset: usize,
        len: usize,
    },
    /// The application's memory (`SDL_SURFACE_PREALLOCATED`).
    Borrowed(&'a mut [u8]),
    /// Read-only memory: the source side of `SDL_ConvertPixels()` and
    /// internal views of another surface's pixels.
    ReadOnly(&'a [u8]),
}

impl<'a> Pixels<'a> {
    pub(crate) fn bytes(&self) -> Option<&[u8]> {
        match self {
            Pixels::None => None,
            Pixels::Owned { buf, offset, len } => Some(&buf[*offset..*offset + *len]),
            Pixels::Borrowed(b) => Some(b),
            Pixels::ReadOnly(b) => Some(b),
        }
    }

    pub(crate) fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        match self {
            Pixels::None | Pixels::ReadOnly(_) => None,
            Pixels::Owned { buf, offset, len } => Some(&mut buf[*offset..*offset + *len]),
            Pixels::Borrowed(b) => Some(b),
        }
    }

    pub(crate) fn is_some(&self) -> bool {
        !matches!(self, Pixels::None)
    }

    /// Allocate `size` zeroed bytes, the start aligned to `align` (0 = no alignment).
    fn allocate(size: usize, align: usize) -> Pixels<'static> {
        if align <= 1 {
            return Pixels::Owned {
                buf: vec![0u8; size],
                offset: 0,
                len: size,
            };
        }
        let buf = vec![0u8; size + align - 1];
        let addr = buf.as_ptr() as usize;
        let offset = (align - addr % align) % align;
        Pixels::Owned {
            buf,
            offset,
            len: size,
        }
    }
}

/// A collection of pixels used in software blitting. Translation of `SDL_Surface`.
pub struct Surface<'a> {
    /// The flags of the surface, read-only
    pub(crate) flags: SurfaceFlags,
    /// The format of the surface, read-only
    pub(crate) format: PixelFormat,
    pub(crate) w: i32,
    pub(crate) h: i32,
    /// The distance in bytes between rows of pixels, read-only
    pub(crate) pitch: i32,
    pub(crate) pixels: Pixels<'a>,

    /// flags for this surface
    pub(crate) internal_flags: u32,
    /// properties for this surface
    pub(crate) props: Option<Properties>,
    /// detailed format for this surface
    pub(crate) fmt: PixelFormatDetails,
    /// Pixel colorspace
    pub(crate) colorspace: Colorspace,
    /// palette for indexed surfaces
    pub(crate) palette: Option<SharedPalette>,
    /// Alternate representation of images
    pub(crate) images: Vec<Surface<'static>>,
    /// information needed for surfaces requiring locks
    pub(crate) locked: i32,
    /// clipping information
    pub(crate) clip_rect: Rect,
    /// info for fast blit mapping to other surfaces
    pub(crate) map: BlitMap,
    /// Original pixels when RLE is enabled
    pub(crate) saved_pixels: Pixels<'a>,
}

impl std::fmt::Debug for Surface<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Surface")
            .field("format", &self.format)
            .field("w", &self.w)
            .field("h", &self.h)
            .field("pitch", &self.pitch)
            .field("flags", &self.flags)
            .field("colorspace", &self.colorspace)
            .field("clip_rect", &self.clip_rect)
            .field("images", &self.images.len())
            .finish_non_exhaustive()
    }
}

/// The image [`Surface::image_for_scale`] picked.
#[derive(Debug)]
pub enum SurfaceImage<'s, 'a> {
    /// The surface itself.
    Original(&'s mut Surface<'a>),
    /// One of its alternate images, at exactly the wanted size.
    Alternate(&'s mut Surface<'static>),
    /// A new surface scaled from the closest image.
    Scaled(Box<Surface<'static>>),
}

/// `SDL_UpdateSurfaceLockFlag()` and the `SDL_MUSTLOCK()` macro's state.
impl Surface<'_> {
    /// Translation of `SDL_UpdateSurfaceLockFlag()`.
    pub(crate) fn update_lock_flag(&mut self) {
        // We need to mark the surface as needing unlock while locked
        if self.flags.contains(SurfaceFlags::LOCKED)
            || (self.internal_flags & INTERNAL_SURFACE_RLEACCEL) != 0
        {
            self.flags.insert(SurfaceFlags::LOCK_NEEDED);
        } else {
            self.flags.remove(SurfaceFlags::LOCK_NEEDED);
        }
    }

    pub(crate) fn is_rle_encoded(&self) -> bool {
        self.internal_flags & INTERNAL_SURFACE_RLEACCEL != 0
    }
}

/*
 * Calculate the pad-aligned scanline width of a surface.
 *
 * for FOURCC, use SDL_CalculateYUVSize()
 */
/// Translation of `SDL_CalculateRGBSize()`: `(size, pitch)`.
fn calculate_rgb_size(
    format: PixelFormat,
    width: usize,
    height: usize,
    minimal: bool,
) -> Result<(usize, usize)> {
    let mut pitch;
    if format.bits_per_pixel() >= 8 {
        pitch = width
            .checked_mul(format.bytes_per_pixel() as usize)
            .ok_or_else(|| Error::new("width * bpp would overflow"))?;
    } else {
        pitch = width
            .checked_mul(format.bits_per_pixel() as usize)
            .ok_or_else(|| Error::new("width * bpp would overflow"))?;
        pitch = pitch
            .checked_add(7)
            .ok_or_else(|| Error::new("aligning pitch would overflow"))?;
        pitch /= 8;
    }
    if !minimal {
        // 4-byte aligning for speed
        pitch = pitch
            .checked_add(3)
            .ok_or_else(|| Error::new("aligning pitch would overflow"))?;
        pitch &= !3;
    }

    let size = height
        .checked_mul(pitch)
        .ok_or_else(|| Error::new("height * pitch would overflow"))?;

    Ok((size, pitch))
}

/// The byte size and pitch of a `width` x `height` surface in `format`.
/// Translation of `SDL_CalculateSurfaceSize()`: `(size, pitch)`.
pub(crate) fn calculate_surface_size(
    format: PixelFormat,
    width: i32,
    height: i32,
    minimal_pitch: bool,
) -> Result<(usize, usize)> {
    if format.is_fourcc() {
        if format == PixelFormat::MJPG {
            // We don't know in advance what it will be, we'll figure it out later.
            return Ok((0, 0));
        }

        calculate_yuv_size(format, width, height)
    } else {
        calculate_rgb_size(format, width as usize, height as usize, minimal_pitch)
    }
}

/*
 * Calculate YUV size and pitch. Check for overflow.
 * Output 'pitch' that can be used with SDL_ConvertPixels()
 */
/// Translation of `SDL_CalculateYUVSize()` (from `SDL_yuv.c`): `(size, pitch)`.
pub(crate) fn calculate_yuv_size(format: PixelFormat, w: i32, h: i32) -> Result<(usize, usize)> {
    // The C code keeps intermediate sizes in ints and assigns `(int)` casts to size_t.
    let int_to_size = |v: usize| v as i32 as isize as usize;
    let (w, h) = (w as isize as usize, h as isize as usize);
    let bpp = format.bytes_per_pixel() as usize;
    let mul =
        |a: usize, b: usize, msg: &'static str| a.checked_mul(b).ok_or_else(|| Error::new(msg));
    let add =
        |a: usize, b: usize, msg: &'static str| a.checked_add(b).ok_or_else(|| Error::new(msg));

    let is_planar1x1 = format == PixelFormat::I444 || format == PixelFormat::I4FL;
    let is_planar2x2 = matches!(
        format,
        PixelFormat::YV12
            | PixelFormat::IYUV
            | PixelFormat::NV12
            | PixelFormat::NV21
            | PixelFormat::P010
            | PixelFormat::I0FL
    );
    let is_packed4 = matches!(
        format,
        PixelFormat::YUY2 | PixelFormat::UYVY | PixelFormat::YVYU
    );

    let (mut sz_plane, mut sz_plane_chroma, mut sz_plane_packed) = (0usize, 0usize, 0usize);
    if is_planar1x1 {
        /* sz_plane == w * h * bpp; */
        let s1 = mul(w, h, "width * height would overflow")?;
        let s1 = mul(s1, bpp, "width * height * bpp would overflow")?;
        sz_plane = int_to_size(s1);
        sz_plane_chroma = sz_plane;
    } else if is_planar2x2 {
        {
            /* sz_plane == w * h * bpp; */
            let s1 = mul(w, h, "width * height would overflow")?;
            let s1 = mul(s1, bpp, "width * height * bpp would overflow")?;
            sz_plane = int_to_size(s1);
        }
        {
            /* sz_plane_chroma == ((w + 1) / 2) * ((h + 1) / 2) * bpp; */
            let s1 = add(w, 1, "width + 1 would overflow")? / 2;
            let s2 = add(h, 1, "height + 1 would overflow")? / 2;
            let s3 = mul(s1, s2, "width * height would overflow")?;
            let s3 = mul(s3, bpp, "width * height * bpp would overflow")?;
            sz_plane_chroma = int_to_size(s3);
        }
    } else if is_packed4 {
        /* sz_plane_packed == ((w + 1) / 2) * h; */
        let s1 = add(w, 1, "width + 1 would overflow")? / 2;
        let s2 = mul(s1, h, "width * height would overflow")?;
        sz_plane_packed = int_to_size(s2);
    } else {
        return Err(Error::unsupported());
    }

    match format {
        // Planar mode: Y + V + U  (3 planes), Y + U + V  (3 planes)
        PixelFormat::YV12 | PixelFormat::IYUV | PixelFormat::I444 | PixelFormat::I0FL | PixelFormat::I4FL
        // Planar mode: Y + U/V interleaved  (2 planes), 10 bit
        | PixelFormat::NV12 | PixelFormat::NV21 | PixelFormat::P010 => {
            let pitch = w.wrapping_mul(bpp) as i32 as isize as usize;
            // dst_size == sz_plane + sz_plane_chroma + sz_plane_chroma;
            let s1 = add(sz_plane, sz_plane_chroma, "Y + U would overflow")?;
            let s2 = add(s1, sz_plane_chroma, "Y + U + V would overflow")?;
            Ok((int_to_size(s2), pitch))
        }
        // Packed mode: Y0+U0+Y1+V0 (1 plane), U0+Y0+V0+Y1, Y0+V0+Y1+U0
        PixelFormat::YUY2 | PixelFormat::UYVY | PixelFormat::YVYU => {
            /* pitch == ((w + 1) / 2) * 4; */
            let p1 = add(w, 1, "width + 1 would overflow")? / 2;
            let pitch = mul(p1, 4, "width * 4 would overflow")?;
            /* dst_size == 4 * sz_plane_packed; */
            let s1 = mul(sz_plane_packed, 4, "plane * 4 would overflow")?;
            Ok((int_to_size(s1), pitch))
        }
        _ => Err(Error::unsupported()),
    }
}

/// The default SDR white point for a colorspace. Translation of `SDL_GetDefaultSDRWhitePoint()`.
pub fn default_sdr_white_point(colorspace: Colorspace) -> f32 {
    get_sdr_white_point(None, colorspace)
}

/// Translation of the body of `SDL_GetSurfaceSDRWhitePoint()`.
pub(crate) fn get_sdr_white_point(props: Option<&Properties>, colorspace: Colorspace) -> f32 {
    let transfer = colorspace.transfer();

    if transfer == TransferCharacteristics::Linear || transfer == TransferCharacteristics::Pq {
        let mut default_value = 1.0f32;

        if transfer == TransferCharacteristics::Pq {
            /* The older standards use an SDR white point of 100 nits.
             * ITU-R BT.2408-6 recommends using an SDR white point of 203 nits.
             * This is the default Chrome uses, and what a lot of game content
             * assumes, so we'll go with that.
             */
            const DEFAULT_PQ_SDR_WHITE_POINT: f32 = 203.0;
            default_value = DEFAULT_PQ_SDR_WHITE_POINT;
        }
        return props
            .and_then(|p| p.get_float(PROP_SURFACE_SDR_WHITE_POINT_FLOAT))
            .unwrap_or(default_value);
    }
    1.0
}

/// The default HDR headroom for a colorspace. Translation of `SDL_GetDefaultHDRHeadroom()`.
pub fn default_hdr_headroom(colorspace: Colorspace) -> f32 {
    get_hdr_headroom(None, colorspace)
}

/// Translation of the body of `SDL_GetSurfaceHDRHeadroom()`.
pub(crate) fn get_hdr_headroom(props: Option<&Properties>, colorspace: Colorspace) -> f32 {
    let transfer = colorspace.transfer();

    if transfer == TransferCharacteristics::Linear || transfer == TransferCharacteristics::Pq {
        let default_value = 0.0f32;
        return props
            .and_then(|p| p.get_float(PROP_SURFACE_HDR_HEADROOM_FLOAT))
            .unwrap_or(default_value);
    }
    1.0
}

impl<'a> Surface<'a> {
    /// Translation of `SDL_InitializeSurface()`.
    fn initialize(
        width: i32,
        height: i32,
        format: PixelFormat,
        colorspace: Colorspace,
        props: Option<&Properties>,
        pixels: Pixels<'a>,
        pitch: i32,
    ) -> Result<Surface<'a>> {
        let fmt = PixelFormatDetails::new(format)?;
        let mut surface = Surface {
            flags: SurfaceFlags::PREALLOCATED,
            format,
            w: width,
            h: height,
            pitch,
            pixels,
            internal_flags: 0,
            props: None,
            fmt,
            colorspace: Colorspace::UNKNOWN,
            palette: None,
            images: Vec::new(),
            locked: 0,
            // Initialize the clip rect
            clip_rect: Rect::new(0, 0, width, height),
            // Allocate an empty mapping
            map: BlitMap::default(),
            saved_pixels: Pixels::None,
        };

        if colorspace == Colorspace::UNKNOWN {
            surface.colorspace = format.default_colorspace();
        } else {
            surface.colorspace = colorspace;
        }

        if let Some(props) = props {
            surface.properties().copy_from(props)?;
        }

        // By default surfaces with an alpha mask are set up for blending
        if surface.format.has_alpha() {
            let _ = surface.set_blend_mode(BlendMode::BLEND);
        }

        // The surface is ready to go
        Ok(surface)
    }

    fn check_create_params(width: i32, height: i32, format: PixelFormat) -> Result<()> {
        if width < 0 {
            return Err(Error::invalid_param("width"));
        }
        if height < 0 {
            return Err(Error::invalid_param("height"));
        }
        if format == PixelFormat::UNKNOWN {
            return Err(Error::invalid_param("format"));
        }
        Ok(())
    }

    /// Create a surface with zeroed pixels. Translation of
    /// `SDL_CreateSurface()` (and of `SDL_CreateSurfaceInternal()`).
    ///
    /// Pixel memory is aligned to [`simd_alignment`](crate::cpuinfo::simd_alignment)
    /// (`SurfaceFlags::SIMD_ALIGNED`) unless the `SDL_SURFACE_MALLOC` hint is set.
    pub fn new(width: i32, height: i32, format: PixelFormat) -> Result<Surface<'static>> {
        Surface::check_create_params(width, height, format)?;

        // Overflow...
        let (size, pitch) =
            calculate_surface_size(format, width, height, false /* not minimal pitch */)?;

        // Allocate and initialize the surface
        let mut surface = Surface::initialize(
            width,
            height,
            format,
            Colorspace::UNKNOWN,
            None,
            Pixels::None,
            pitch as i32,
        )?;

        if surface.w != 0 && surface.h != 0 && format != PixelFormat::MJPG {
            surface.flags.remove(SurfaceFlags::PREALLOCATED);
            if crate::hints::get_bool("SDL_SURFACE_MALLOC", false) {
                surface.pixels = Pixels::allocate(size, 0);
            } else {
                surface.flags.insert(SurfaceFlags::SIMD_ALIGNED);
                surface.pixels = Pixels::allocate(size, crate::cpuinfo::simd_alignment());
            }
        }
        Ok(surface)
    }

    /// Translation of `SDL_CreateSurfaceUninitialized()`; Rust memory is
    /// always initialized, so this is [`Surface::new`].
    pub(crate) fn new_uninitialized(
        width: i32,
        height: i32,
        format: PixelFormat,
    ) -> Result<Surface<'static>> {
        Surface::new(width, height, format)
    }

    fn check_from_params(
        width: i32,
        height: i32,
        format: PixelFormat,
        len: Option<usize>,
        pitch: i32,
    ) -> Result<()> {
        Surface::check_create_params(width, height, format)?;

        if pitch == 0 && len.is_none() {
            // The application will fill these in later with valid values
        } else {
            // Overflow...
            let (_, minimal_pitch) =
                calculate_surface_size(format, width, height, true /* minimal pitch */)?;

            if pitch < 0 || (pitch as usize) < minimal_pitch {
                return Err(Error::invalid_param("pitch"));
            }
            // Rust-only check: C trusts the caller's buffer to be big enough.
            if let Some(len) = len {
                let needed = if height > 0 && !format.is_fourcc() {
                    (height as usize - 1) * pitch as usize + minimal_pitch
                } else {
                    0
                };
                if len < needed {
                    return Err(Error::invalid_param("pixels"));
                }
            }
        }
        Ok(())
    }

    /// Relabel the pixels as `format` (of the same size), as
    /// `SDL_RenderReadPixels()` does by setting `surface->format`.
    pub(crate) fn reinterpret_format(&mut self, format: PixelFormat) -> Result<()> {
        self.fmt = PixelFormatDetails::new(format)?;
        self.format = format;
        Ok(())
    }

    /// Create a surface over existing pixel memory. Translation of
    /// `SDL_CreateSurfaceFrom()`; the surface is `PREALLOCATED` and
    /// borrows `pixels` for its lifetime.
    ///
    /// Besides upstream's pitch check, `pixels` must hold every row
    /// (`(height - 1) * pitch` plus one minimal row).
    pub fn from_pixels(
        width: i32,
        height: i32,
        format: PixelFormat,
        pixels: &'a mut [u8],
        pitch: i32,
    ) -> Result<Surface<'a>> {
        Surface::check_from_params(width, height, format, Some(pixels.len()), pitch)?;
        Surface::initialize(
            width,
            height,
            format,
            Colorspace::UNKNOWN,
            None,
            Pixels::Borrowed(pixels),
            pitch,
        )
    }

    /// A read-only surface over `pixels` (the C code casts away `const`).
    pub(crate) fn from_const_pixels(
        width: i32,
        height: i32,
        format: PixelFormat,
        colorspace: Colorspace,
        props: Option<&Properties>,
        pixels: &'a [u8],
        pitch: i32,
    ) -> Result<Surface<'a>> {
        Surface::initialize(
            width,
            height,
            format,
            colorspace,
            props,
            Pixels::ReadOnly(pixels),
            pitch,
        )
    }

    /// A writable surface over `pixels`, set up like `SDL_InitializeSurface()`
    /// does for `SDL_ConvertPixels()`'s stack surfaces.
    pub(crate) fn from_mut_pixels_with(
        width: i32,
        height: i32,
        format: PixelFormat,
        colorspace: Colorspace,
        props: Option<&Properties>,
        pixels: &'a mut [u8],
        pitch: i32,
    ) -> Result<Surface<'a>> {
        Surface::initialize(
            width,
            height,
            format,
            colorspace,
            props,
            Pixels::Borrowed(pixels),
            pitch,
        )
    }

    /// Like [`Surface::from_pixels`], but the surface takes ownership of the
    /// buffer (and is not `PREALLOCATED`).
    pub fn from_vec(
        width: i32,
        height: i32,
        format: PixelFormat,
        pixels: Vec<u8>,
        pitch: i32,
    ) -> Result<Surface<'static>> {
        Surface::check_from_params(width, height, format, Some(pixels.len()), pitch)?;
        let len = pixels.len();
        let mut s = Surface::initialize(
            width,
            height,
            format,
            Colorspace::UNKNOWN,
            None,
            Pixels::Owned {
                buf: pixels,
                offset: 0,
                len,
            },
            pitch,
        )?;
        s.flags.remove(SurfaceFlags::PREALLOCATED);
        Ok(s)
    }

    /// A surface with no pixels: `SDL_CreateSurfaceFrom(w, h, format, NULL, 0)`.
    pub fn without_pixels(
        width: i32,
        height: i32,
        format: PixelFormat,
    ) -> Result<Surface<'static>> {
        Surface::check_from_params(width, height, format, None, 0)?;
        Surface::initialize(
            width,
            height,
            format,
            Colorspace::UNKNOWN,
            None,
            Pixels::None,
            0,
        )
    }

    /// A read-only view of this surface's pixels (or of `rect` within them,
    /// with the same pitch), with the same format, palette, colorspace,
    /// properties and blit settings. Used where upstream temporarily
    /// rewrites `surface->pixels`, `w` and `h`.
    pub(crate) fn view(&self, rect: Option<&Rect>) -> Result<Surface<'_>> {
        let bytes = self.raw_pixels();
        let (w, h, pixels) = match (rect, bytes) {
            (Some(r), Some(b)) => {
                let off = r.y as isize * self.pitch as isize
                    + r.x as isize * self.format.bytes_per_pixel() as isize;
                (r.w, r.h, Pixels::ReadOnly(&b[off as usize..]))
            }
            (Some(r), None) => (r.w, r.h, Pixels::None),
            (None, Some(b)) => (self.w, self.h, Pixels::ReadOnly(b)),
            (None, None) => (self.w, self.h, Pixels::None),
        };
        let mut v = Surface {
            flags: self.flags,
            format: self.format,
            w,
            h,
            pitch: self.pitch,
            pixels,
            internal_flags: 0,
            props: self.props.clone(),
            fmt: self.fmt,
            colorspace: self.colorspace,
            palette: self.palette.clone(),
            images: Vec::new(),
            locked: 0,
            clip_rect: Rect::new(0, 0, w, h),
            map: BlitMap::default(),
            saved_pixels: Pixels::None,
        };
        // A locked surface stays "locked" for the blit checks; the view itself is never RLE encoded.
        v.update_lock_flag();
        v.map.flags = self.map.flags
            & !(crate::video::blit::COPY_RLE_COLORKEY | crate::video::blit::COPY_RLE_ALPHAKEY);
        v.map.colorkey = self.map.colorkey;
        (v.map.r, v.map.g, v.map.b, v.map.a) = (self.map.r, self.map.g, self.map.b, self.map.a);
        Ok(v)
    }

    /// The pixel data, including pixels hidden while the surface is RLE encoded.
    pub(crate) fn raw_pixels(&self) -> Option<&[u8]> {
        self.pixels.bytes().or_else(|| self.saved_pixels.bytes())
    }

    /// The flags of the surface.
    pub fn flags(&self) -> SurfaceFlags {
        self.flags
    }

    /// The format of the surface.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// The detailed format of the surface.
    pub fn format_details(&self) -> &PixelFormatDetails {
        &self.fmt
    }

    /// The width of the surface.
    pub fn width(&self) -> i32 {
        self.w
    }

    /// The height of the surface.
    pub fn height(&self) -> i32 {
        self.h
    }

    /// The distance in bytes between rows of pixels.
    pub fn pitch(&self) -> i32 {
        self.pitch
    }

    /// The pixels, or `None` if the surface has none (or they are hidden
    /// while it is RLE encoded).
    pub fn pixels(&self) -> Option<&[u8]> {
        self.pixels.bytes()
    }

    /// Writable pixels; `None` without pixels, for read-only memory, or while
    /// [`must_lock`](Self::must_lock) is true (use [`lock`](Self::lock) then).
    pub fn pixels_mut(&mut self) -> Option<&mut [u8]> {
        if self.must_lock() {
            return None;
        }
        self.pixels.bytes_mut()
    }

    /// Whether the surface must be locked before its pixels are accessed.
    /// Translation of `SDL_MUSTLOCK()`.
    pub fn must_lock(&self) -> bool {
        self.flags.contains(SurfaceFlags::LOCK_NEEDED)
    }

    /// The surface's property group, created on first use.
    /// Translation of `SDL_GetSurfaceProperties()`.
    pub fn properties(&mut self) -> Properties {
        self.props.get_or_insert_with(Properties::new).clone()
    }

    /// Set the colorspace used by the surface. Translation of `SDL_SetSurfaceColorspace()`.
    pub fn set_colorspace(&mut self, colorspace: Colorspace) {
        self.colorspace = colorspace;
    }

    /// The colorspace used by the surface. Translation of `SDL_GetSurfaceColorspace()`.
    pub fn colorspace(&self) -> Colorspace {
        self.colorspace
    }

    /// The SDR white point for `colorspace`, from this surface's properties.
    /// Translation of `SDL_GetSurfaceSDRWhitePoint()`.
    pub fn sdr_white_point(&self, colorspace: Colorspace) -> f32 {
        get_sdr_white_point(self.props.as_ref(), colorspace)
    }

    /// The HDR headroom for `colorspace`, from this surface's properties.
    /// Translation of `SDL_GetSurfaceHDRHeadroom()`.
    pub fn hdr_headroom(&self, colorspace: Colorspace) -> f32 {
        get_hdr_headroom(self.props.as_ref(), colorspace)
    }

    /// Create a palette and associate it with the surface.
    /// Translation of `SDL_CreateSurfacePalette()`.
    pub fn create_palette(&mut self) -> Result<SharedPalette> {
        if !self.format.is_indexed() {
            return Err(Error::new("The surface is not indexed format"));
        }

        let mut palette = Palette::new(1 << self.format.bits_per_pixel())?;

        if palette.len() == 2 {
            // Create a black and white bitmap palette
            // Written directly, as upstream does: the new palette keeps version 1.
            let colors = palette.colors_mut_unversioned();
            colors[0].r = 0xFF;
            colors[0].g = 0xFF;
            colors[0].b = 0xFF;
            colors[1].r = 0x00;
            colors[1].g = 0x00;
            colors[1].b = 0x00;
        }

        let palette = share_palette(palette);
        self.set_palette(Some(palette.clone()))?;

        // The surface has retained the palette, we can remove the reference here
        Ok(palette)
    }

    /// Set (or with `None`, remove) the palette used by the surface.
    /// Translation of `SDL_SetSurfacePalette()`.
    pub fn set_palette(&mut self, palette: Option<SharedPalette>) -> Result<()> {
        if let Some(p) = &palette {
            if !self.format.is_indexed() {
                return Err(Error::new("Surface doesn't use a palette"));
            }
            if read_palette(p).len() > (1usize << self.format.bits_per_pixel()) {
                return Err(Error::new("Palette doesn't match surface format"));
            }
        }

        let same = match (&palette, &self.palette) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.palette = palette;
        }

        self.map.invalidate();

        Ok(())
    }

    /// The palette used by the surface. Translation of `SDL_GetSurfacePalette()`.
    pub fn palette(&self) -> Option<&SharedPalette> {
        self.palette.as_ref()
    }

    /// Add an alternate version of the surface, for example a higher
    /// resolution image for high DPI displays. Translation of
    /// `SDL_AddSurfaceAlternateImage()`; the surface takes ownership.
    pub fn add_alternate_image(&mut self, image: Surface<'static>) {
        self.images.push(image);
    }

    /// Whether the surface has alternate versions. Translation of `SDL_SurfaceHasAlternateImages()`.
    pub fn has_alternate_images(&self) -> bool {
        !self.images.is_empty()
    }

    /// The alternate images (not including the surface itself; upstream's
    /// `SDL_GetSurfaceImages()` returns the surface first, then these).
    pub fn alternate_images(&self) -> &[Surface<'static>] {
        &self.images
    }

    /// Mutable access to the alternate images.
    pub fn alternate_images_mut(&mut self) -> &mut [Surface<'static>] {
        &mut self.images
    }

    /// Remove (and drop) all alternate versions. Translation of `SDL_RemoveSurfaceAlternateImages()`.
    pub fn remove_alternate_images(&mut self) {
        self.images.clear();
    }

    /// The best image for a display scale: the surface itself or an
    /// alternate image of exactly the desired size, or a new surface scaled
    /// from the closest one. Translation of `SDL_GetSurfaceImage()`.
    pub fn image_for_scale(&mut self, display_scale: f32) -> SurfaceImage<'_, 'a> {
        if !self.has_alternate_images() {
            return SurfaceImage::Original(self);
        }

        // This surface has high DPI images, pick the best one available, or scale one to the correct size

        // Find closest image. Images that are larger than the
        // desired size are preferred over images that are smaller.
        let desired_w = ((self.w as f32 * display_scale) as f64).round() as i32;
        let desired_h = ((self.h as f32 * display_scale) as f64).round() as i32;
        let desired_size = desired_w.wrapping_mul(desired_h);
        let mut closest: usize = 0; // 0 = the surface itself, i = images[i - 1]
        let mut closest_distance = -1;
        let mut closest_size = -1;
        let dims: Vec<(i32, i32)> = std::iter::once((self.w, self.h))
            .chain(self.images.iter().map(|s| (s.w, s.h)))
            .collect();
        for (i, &(cw, ch)) in dims.iter().enumerate() {
            let size = cw.wrapping_mul(ch);
            let delta_w = cw - desired_w;
            let delta_h = ch - desired_h;
            let distance = delta_w
                .wrapping_mul(delta_w)
                .wrapping_add(delta_h.wrapping_mul(delta_h));
            if closest_distance < 0
                || distance < closest_distance
                || (size > desired_size && closest_size < desired_size)
            {
                closest = i;
                closest_distance = distance;
                closest_size = size;
            }
        }

        let (cw, ch) = dims[closest];
        if cw == desired_w && ch == desired_h {
            return if closest == 0 {
                SurfaceImage::Original(self)
            } else {
                SurfaceImage::Alternate(&mut self.images[closest - 1])
            };
        }

        // We need to scale the image to the correct size. To maintain good image quality, downscaling
        // is done in steps, never reducing the width and height by more than half each time.
        let scaled = {
            let source: &Surface<'_> = if closest == 0 {
                self
            } else {
                &self.images[closest - 1]
            };
            let mut scaled: Option<Surface<'static>> = None;
            loop {
                let cur: &Surface<'_> = match &scaled {
                    Some(s) => s,
                    None => source,
                };
                let next_scaled_w = desired_w.max((cur.w + 1) / 2);
                let next_scaled_h = desired_h.max((cur.h + 1) / 2);
                match cur.scale(next_scaled_w, next_scaled_h, ScaleMode::Linear) {
                    Ok(next) => {
                        let done = next.w == desired_w && next.h == desired_h;
                        scaled = Some(next);
                        if done {
                            break scaled;
                        }
                    }
                    // Failure, fall back to the closest surface
                    Err(_) => break None,
                }
            }
        };
        match scaled {
            Some(s) => SurfaceImage::Scaled(Box::new(s)),
            None if closest == 0 => SurfaceImage::Original(self),
            None => SurfaceImage::Alternate(&mut self.images[closest - 1]),
        }
    }

    /// Ask for (or stop asking for) RLE acceleration when this surface is
    /// blitted. Translation of `SDL_SetSurfaceRLE()`.
    pub fn set_rle(&mut self, enabled: bool) -> Result<()> {
        if self.format.is_fourcc() {
            return Err(Error::invalid_param("surface"));
        }

        let flags = self.map.flags;
        if enabled {
            self.map.flags |= COPY_RLE_DESIRED;
        } else {
            self.map.flags &= !COPY_RLE_DESIRED;
        }
        if self.map.flags != flags {
            self.map.invalidate();
        }
        Ok(())
    }

    /// Whether RLE is enabled. Translation of `SDL_SurfaceHasRLE()`.
    pub fn has_rle(&self) -> bool {
        self.map.flags & COPY_RLE_DESIRED != 0
    }

    /// Set (`Some(pixel)`) or clear (`None`) the color key (transparent
    /// pixel value). Translation of `SDL_SetSurfaceColorKey()`.
    pub fn set_color_key(&mut self, key: Option<u32>) -> Result<()> {
        let check_key = key.unwrap_or(0);
        if let Some(p) = &self.palette {
            if check_key >= read_palette(p).len() as u32 {
                return Err(Error::invalid_param("key"));
            }
        }

        let flags = self.map.flags;
        if let Some(key) = key {
            self.map.flags |= COPY_COLORKEY;
            self.map.colorkey = key;
        } else {
            self.map.flags &= !COPY_COLORKEY;
        }
        if self.map.flags != flags {
            self.map.invalidate();
        }

        Ok(())
    }

    /// Whether the surface has a color key. Translation of `SDL_SurfaceHasColorKey()`.
    pub fn has_color_key(&self) -> bool {
        self.map.flags & COPY_COLORKEY != 0
    }

    /// The color key, if enabled. Translation of `SDL_GetSurfaceColorKey()`
    /// (whose "Surface doesn't have a colorkey" error is `None`).
    pub fn color_key(&self) -> Option<u32> {
        self.has_color_key().then_some(self.map.colorkey)
    }

    /* This is a fairly slow function to switch from colorkey to alpha
    NB: it doesn't handle bpp 1 or 3, because they have no alpha channel */
    /// Translation of `SDL_ConvertColorkeyToAlpha()`.
    pub(crate) fn convert_colorkey_to_alpha(&mut self, ignore_alpha: bool) {
        if self.map.flags & COPY_COLORKEY == 0 || !self.format.has_alpha() {
            return;
        }

        let bpp = self.format.bytes_per_pixel();

        let _ = self.lock_raw();

        let (w, h, pitch) = (self.w as usize, self.h as usize, self.pitch as usize);
        let amask = self.fmt.Amask;
        let colorkey = self.map.colorkey;
        if let Some(pixels) = self.pixels.bytes_mut() {
            if bpp == 2 {
                let mut ckey = colorkey as u16;
                let mask = !amask as u16;
                let rd = |p: &[u8], i: usize| u16::from_ne_bytes([p[i], p[i + 1]]);

                // Ignore, or not, alpha in colorkey comparison
                if ignore_alpha {
                    ckey &= mask;
                }
                for y in 0..h {
                    for x in 0..w {
                        let i = y * (pitch / 2) * 2 + x * 2;
                        let spot = rd(pixels, i);
                        let hit = if ignore_alpha {
                            (spot & mask) == ckey
                        } else {
                            spot == ckey
                        };
                        if hit {
                            pixels[i..i + 2].copy_from_slice(&(spot & mask).to_ne_bytes());
                        }
                    }
                }
            } else if bpp == 4 {
                let mut ckey = colorkey;
                let mask = !amask;

                // Ignore, or not, alpha in colorkey comparison
                if ignore_alpha {
                    ckey &= mask;
                }
                for y in 0..h {
                    for x in 0..w {
                        let i = y * (pitch / 4) * 4 + x * 4;
                        let spot = crate::video::blit::rd32(pixels, i);
                        let hit = if ignore_alpha {
                            (spot & mask) == ckey
                        } else {
                            spot == ckey
                        };
                        if hit {
                            crate::video::blit::wr32(pixels, i, spot & mask);
                        }
                    }
                }
            }
        }

        self.unlock_raw();

        let _ = self.set_color_key(None);
        let _ = self.set_blend_mode(BlendMode::BLEND);
    }

    /// Set the color multiplied into blit operations.
    /// Translation of `SDL_SetSurfaceColorMod()`.
    pub fn set_color_mod(&mut self, r: u8, g: u8, b: u8) {
        self.map.r = r;
        self.map.g = g;
        self.map.b = b;

        let flags = self.map.flags;
        if r != 0xFF || g != 0xFF || b != 0xFF {
            self.map.flags |= COPY_MODULATE_COLOR;
        } else {
            self.map.flags &= !COPY_MODULATE_COLOR;
        }
        if self.map.flags != flags {
            self.map.invalidate();
        }
    }

    /// The color multiplied into blit operations. Translation of `SDL_GetSurfaceColorMod()`.
    pub fn color_mod(&self) -> (u8, u8, u8) {
        (self.map.r, self.map.g, self.map.b)
    }

    /// Set the alpha multiplied into blit operations.
    /// Translation of `SDL_SetSurfaceAlphaMod()`.
    pub fn set_alpha_mod(&mut self, alpha: u8) {
        self.map.a = alpha;

        let flags = self.map.flags;
        if alpha != 0xFF {
            self.map.flags |= COPY_MODULATE_ALPHA;
        } else {
            self.map.flags &= !COPY_MODULATE_ALPHA;
        }
        if self.map.flags != flags {
            self.map.invalidate();
        }
    }

    /// The alpha multiplied into blit operations. Translation of `SDL_GetSurfaceAlphaMod()`.
    pub fn alpha_mod(&self) -> u8 {
        self.map.a
    }

    /// Set the blend mode used for blit operations.
    /// Translation of `SDL_SetSurfaceBlendMode()`.
    pub fn set_blend_mode(&mut self, blend_mode: BlendMode) -> Result<()> {
        if blend_mode == BlendMode::INVALID {
            return Err(Error::invalid_param("blendMode"));
        }

        let mut result = Ok(());
        let flags = self.map.flags;
        self.map.flags &= !COPY_BLEND_MASK;
        match blend_mode {
            BlendMode::NONE => {}
            BlendMode::BLEND => self.map.flags |= COPY_BLEND,
            BlendMode::BLEND_PREMULTIPLIED => self.map.flags |= COPY_BLEND_PREMULTIPLIED,
            BlendMode::ADD => self.map.flags |= COPY_ADD,
            BlendMode::ADD_PREMULTIPLIED => self.map.flags |= COPY_ADD_PREMULTIPLIED,
            BlendMode::MOD => self.map.flags |= COPY_MOD,
            BlendMode::MUL => self.map.flags |= COPY_MUL,
            _ => result = Err(Error::unsupported()),
        }

        if self.map.flags != flags {
            self.map.invalidate();
        }

        result
    }

    /// The blend mode used for blit operations. Translation of `SDL_GetSurfaceBlendMode()`.
    pub fn blend_mode(&self) -> BlendMode {
        match self.map.flags & COPY_BLEND_MASK {
            COPY_BLEND => BlendMode::BLEND,
            COPY_BLEND_PREMULTIPLIED => BlendMode::BLEND_PREMULTIPLIED,
            COPY_ADD => BlendMode::ADD,
            COPY_ADD_PREMULTIPLIED => BlendMode::ADD_PREMULTIPLIED,
            COPY_MOD => BlendMode::MOD,
            COPY_MUL => BlendMode::MUL,
            _ => BlendMode::NONE,
        }
    }

    /// Set the clipping rectangle (`None` = the whole surface). Returns
    /// whether the rectangle intersects the surface; if not, the clip
    /// rectangle becomes empty and blits are clipped away entirely.
    /// Translation of `SDL_SetSurfaceClipRect()`.
    pub fn set_clip_rect(&mut self, rect: Option<&Rect>) -> bool {
        // Set up the full surface rectangle
        let full_rect = Rect::new(0, 0, self.w, self.h);

        // Set the clipping rectangle
        match rect {
            None => {
                self.clip_rect = full_rect;
                true
            }
            Some(rect) => rect.intersect_into(&full_rect, &mut self.clip_rect),
        }
    }

    /// The clipping rectangle. Translation of `SDL_GetSurfaceClipRect()`.
    pub fn clip_rect(&self) -> Rect {
        self.clip_rect
    }

    /// Increment the lock count (un-RLE-ing the surface on the first lock).
    pub(crate) fn lock_raw(&mut self) -> Result<()> {
        if self.locked == 0 {
            // Perform the lock
            if self.is_rle_encoded() {
                crate::video::rle::un_rle_surface(self);
            }
        }

        // Increment the surface lock count, for recursive locks
        self.locked += 1;
        self.flags.insert(SurfaceFlags::LOCKED);
        self.update_lock_flag();

        // Ready to go..
        Ok(())
    }

    /// Decrement the lock count. Translation of `SDL_UnlockSurface()`.
    pub(crate) fn unlock_raw(&mut self) {
        // Only perform an unlock if we are locked
        if self.locked == 0 {
            return;
        }
        self.locked -= 1;
        if self.locked > 0 {
            return;
        }

        self.flags.remove(SurfaceFlags::LOCKED);
        self.update_lock_flag();
    }

    /// Lock the surface for direct pixel access; it unlocks when the guard
    /// drops. Translation of `SDL_LockSurface()` / `SDL_UnlockSurface()`.
    pub fn lock(&mut self) -> Result<SurfaceLock<'_, 'a>> {
        self.lock_raw()?;
        Ok(SurfaceLock { surface: self })
    }

    /// Translation of `SDL_FlipSurfaceHorizontal()`.
    fn flip_horizontal(&mut self) -> Result<()> {
        if self.format.bits_per_pixel() < 8 {
            // We could implement this if needed, but we'd have to flip sets of bits within a byte
            return Err(Error::unsupported());
        }

        if self.h <= 0 {
            return Ok(());
        }

        if self.w <= 1 {
            return Ok(());
        }

        let bpp = self.format.bytes_per_pixel() as usize;
        let (w, h, pitch) = (self.w as usize, self.h as usize, self.pitch as usize);
        let Some(pixels) = self.pixels.bytes_mut() else {
            return Ok(());
        };
        for i in 0..h {
            let row = i * pitch;
            let mut a = row;
            let mut b = a + (w - 1) * bpp;
            for _ in 0..w / 2 {
                for k in 0..bpp {
                    pixels.swap(a + k, b + k);
                }
                a += bpp;
                b -= bpp;
            }
        }
        Ok(())
    }

    /// Translation of `SDL_FlipSurfaceVertical()`.
    fn flip_vertical(&mut self) -> Result<()> {
        if self.h <= 1 {
            return Ok(());
        }

        let (h, pitch) = (self.h as usize, self.pitch as usize);
        let Some(pixels) = self.pixels.bytes_mut() else {
            return Ok(());
        };
        let mut a = 0;
        let mut b = (h - 1) * pitch;
        for _ in 0..h / 2 {
            let (top, bottom) = pixels.split_at_mut(b);
            top[a..a + pitch].swap_with_slice(&mut bottom[..pitch]);
            a += pitch;
            b -= pitch;
        }
        Ok(())
    }

    /// Flip the surface in place. Translation of `SDL_FlipSurface()`.
    ///
    /// `FlipMode::None` is rejected (upstream's switch has no case for it).
    pub fn flip(&mut self, flip: FlipMode) -> Result<()> {
        if !self.pixels.is_some() {
            return Ok(());
        }

        match flip {
            FlipMode::Horizontal => self.flip_horizontal(),
            FlipMode::Vertical => self.flip_vertical(),
            FlipMode::HorizontalAndVertical => {
                let h = self.flip_horizontal();
                let v = self.flip_vertical();
                h.and(v)
            }
            FlipMode::None => Err(Error::invalid_param("flip")),
        }
    }

    /// Map an opaque color to a pixel value for this surface.
    /// Translation of `SDL_MapSurfaceRGB()`.
    pub fn map_rgb(&self, r: u8, g: u8, b: u8) -> u32 {
        self.map_rgba(r, g, b, crate::video::pixels::ALPHA_OPAQUE)
    }

    /// Map a color to a pixel value for this surface. Translation of
    /// `SDL_MapSurfaceRGBA()` (an indexed surface without a palette maps to 0).
    pub fn map_rgba(&self, r: u8, g: u8, b: u8, a: u8) -> u32 {
        let pal = self.palette.as_ref().map(|p| read_palette(p));
        self.fmt
            .map_rgba(pal.as_deref(), Color::new(r, g, b, a))
            .unwrap_or(0)
    }

    fn check_pixel_access(&self, x: i32, y: i32) -> Result<()> {
        if self.format == PixelFormat::UNKNOWN || !self.pixels.is_some() {
            return Err(Error::invalid_param("surface"));
        }
        if x < 0 || x >= self.w {
            return Err(Error::invalid_param("x"));
        }
        if y < 0 || y >= self.h {
            return Err(Error::invalid_param("y"));
        }
        Ok(())
    }

    /// Read a single pixel. Translation of `SDL_ReadSurfacePixel()`.
    // This function Copyright 2023 Collabora Ltd., contributed to SDL under the ZLib license
    pub fn read_pixel(&self, x: i32, y: i32) -> Result<Color> {
        self.check_pixel_access(x, y)?;

        let bytes_per_pixel = self.format.bytes_per_pixel() as usize;
        let pixels = self.pixels.bytes().unwrap_or(&[]);
        let p = y as usize * self.pitch as usize + x as usize * bytes_per_pixel;

        if bytes_per_pixel <= 4
            && !self.format.is_fourcc()
            && self.colorspace.primaries() == crate::video::pixels::ColorPrimaries::Bt709
        {
            /* Fill the appropriate number of least-significant bytes of pixel,
             * leaving the most-significant bytes set to zero */
            let pixel = load_low_bytes(&pixels[p..p + bytes_per_pixel]);
            let pal = self.palette.as_ref().map(|p| read_palette(p));
            Ok(self.fmt.get_rgba(pixel, pal.as_deref()))
        } else if self.format.is_fourcc() {
            // FIXME: We need code to extract a single macroblock from a YUV surface
            let converted = self.convert(PixelFormat::ARGB8888)?;
            converted.read_pixel(x, y)
        } else {
            // This is really slow, but it gets the job done
            let mut rgba = [0u8; 4];
            convert_pixels_and_colorspace(
                1,
                1,
                self.format,
                self.colorspace,
                self.props.as_ref(),
                &pixels[p..],
                self.pitch,
                PixelFormat::RGBA32,
                Colorspace::SRGB,
                None,
                &mut rgba,
                4,
            )?;
            Ok(Color::new(rgba[0], rgba[1], rgba[2], rgba[3]))
        }
    }

    /// Read a single pixel as floating point values.
    /// Translation of `SDL_ReadSurfacePixelFloat()`.
    pub fn read_pixel_float(&self, x: i32, y: i32) -> Result<FColor> {
        self.check_pixel_access(x, y)?;

        if self.format.bytes_per_pixel() <= 4
            && !self.format.is_fourcc()
            && self.colorspace.primaries() == crate::video::pixels::ColorPrimaries::Bt709
        {
            let c = self.read_pixel(x, y)?;
            Ok(FColor::new(
                c.r as f32 / 255.0,
                c.g as f32 / 255.0,
                c.b as f32 / 255.0,
                c.a as f32 / 255.0,
            ))
        } else if self.format.is_fourcc() {
            // FIXME: We need code to extract a single macroblock from a YUV surface
            let converted = self.convert(PixelFormat::ARGB8888)?;
            converted.read_pixel_float(x, y)
        } else {
            // This is really slow, but it gets the job done
            let pixels = self.pixels.bytes().unwrap_or(&[]);
            let p = y as usize * self.pitch as usize
                + x as usize * self.format.bytes_per_pixel() as usize;

            let mut rgba = [0f32; 4];
            if self.format == PixelFormat::RGBA128_FLOAT {
                for (k, v) in rgba.iter_mut().enumerate() {
                    *v = f32::from_bits(crate::video::blit::rd32(pixels, p + 4 * k));
                }
            } else {
                let src_colorspace = self.colorspace;
                let dst_colorspace = if src_colorspace == Colorspace::SRGB_LINEAR {
                    Colorspace::SRGB_LINEAR
                } else {
                    Colorspace::SRGB
                };

                let mut bytes = [0u8; 16];
                convert_pixels_and_colorspace(
                    1,
                    1,
                    self.format,
                    src_colorspace,
                    self.props.as_ref(),
                    &pixels[p..],
                    self.pitch,
                    PixelFormat::RGBA128_FLOAT,
                    dst_colorspace,
                    None,
                    &mut bytes,
                    16,
                )?;
                for (k, v) in rgba.iter_mut().enumerate() {
                    *v = f32::from_ne_bytes([
                        bytes[4 * k],
                        bytes[4 * k + 1],
                        bytes[4 * k + 2],
                        bytes[4 * k + 3],
                    ]);
                }
            }
            Ok(FColor::new(rgba[0], rgba[1], rgba[2], rgba[3]))
        }
    }

    /// Write a single pixel. Translation of `SDL_WriteSurfacePixel()`.
    pub fn write_pixel(&mut self, x: i32, y: i32, c: Color) -> Result<()> {
        self.check_pixel_access(x, y)?;

        let bytes_per_pixel = self.format.bytes_per_pixel() as usize;

        let must_lock = self.must_lock();
        if must_lock {
            self.lock_raw()?;
        }

        let p = y as usize * self.pitch as usize + x as usize * bytes_per_pixel;

        let result = if bytes_per_pixel <= 4 && !self.format.is_fourcc() {
            let pixel = self.map_rgba(c.r, c.g, c.b, c.a);
            match self.pixels.bytes_mut() {
                Some(pixels) => {
                    store_low_bytes(&mut pixels[p..p + bytes_per_pixel], pixel);
                    Ok(())
                }
                None => Err(Error::invalid_param("surface")),
            }
        } else if self.format.is_fourcc() {
            Err(Error::unsupported())
        } else {
            // This is really slow, but it gets the job done
            let rgba = [c.r, c.g, c.b, c.a];
            let (format, colorspace, props, pitch) =
                (self.format, self.colorspace, self.props.clone(), self.pitch);
            match self.pixels.bytes_mut() {
                Some(pixels) => convert_pixels_and_colorspace(
                    1,
                    1,
                    PixelFormat::RGBA32,
                    Colorspace::SRGB,
                    None,
                    &rgba,
                    4,
                    format,
                    colorspace,
                    props.as_ref(),
                    &mut pixels[p..],
                    pitch,
                ),
                None => Err(Error::invalid_param("surface")),
            }
        };

        if must_lock {
            self.unlock_raw();
        }
        result
    }

    /// Write a single pixel from floating point values.
    /// Translation of `SDL_WriteSurfacePixelFloat()`.
    pub fn write_pixel_float(&mut self, x: i32, y: i32, c: FColor) -> Result<()> {
        self.check_pixel_access(x, y)?;

        if self.format.bytes_per_pixel() <= 4 && !self.format.is_fourcc() {
            let to8 = |v: f32| ((v.clamp(0.0, 1.0) * 255.0) as f64).round() as u8;
            self.write_pixel(x, y, Color::new(to8(c.r), to8(c.g), to8(c.b), to8(c.a)))
        } else if self.format.is_fourcc() {
            Err(Error::unsupported())
        } else {
            // This is really slow, but it gets the job done
            let must_lock = self.must_lock();
            if must_lock {
                self.lock_raw()?;
            }

            let p = y as usize * self.pitch as usize
                + x as usize * self.format.bytes_per_pixel() as usize;

            let rgba = [c.r, c.g, c.b, c.a];
            let mut bytes = [0u8; 16];
            for (k, v) in rgba.iter().enumerate() {
                bytes[4 * k..4 * k + 4].copy_from_slice(&v.to_ne_bytes());
            }

            let (format, dst_colorspace, props, pitch) =
                (self.format, self.colorspace, self.props.clone(), self.pitch);
            let result = match self.pixels.bytes_mut() {
                Some(pixels) if format == PixelFormat::RGBA128_FLOAT => {
                    pixels[p..p + 16].copy_from_slice(&bytes);
                    Ok(())
                }
                Some(pixels) => {
                    let src_colorspace = if dst_colorspace == Colorspace::SRGB_LINEAR {
                        Colorspace::SRGB_LINEAR
                    } else {
                        Colorspace::SRGB
                    };
                    convert_pixels_and_colorspace(
                        1,
                        1,
                        PixelFormat::RGBA128_FLOAT,
                        src_colorspace,
                        None,
                        &bytes,
                        16,
                        format,
                        dst_colorspace,
                        props.as_ref(),
                        &mut pixels[p..],
                        pitch,
                    )
                }
                None => Err(Error::invalid_param("surface")),
            };

            if must_lock {
                self.unlock_raw();
            }
            result
        }
    }
}

/// `memcpy` of a 1-4 byte pixel into the least significant bytes of a `Uint32`.
fn load_low_bytes(p: &[u8]) -> u32 {
    let mut b = [0u8; 4];
    if cfg!(target_endian = "big") {
        b[4 - p.len()..].copy_from_slice(p);
    } else {
        b[..p.len()].copy_from_slice(p);
    }
    u32::from_ne_bytes(b)
}

/// `memcpy` of the least significant bytes of a `Uint32` pixel.
pub(crate) fn store_low_bytes(p: &mut [u8], pixel: u32) {
    let b = pixel.to_ne_bytes();
    let n = p.len();
    if cfg!(target_endian = "big") {
        p.copy_from_slice(&b[4 - n..]);
    } else {
        p.copy_from_slice(&b[..n]);
    }
}

/// A locked surface; unlocks on drop. Returned by [`Surface::lock`].
#[derive(Debug)]
pub struct SurfaceLock<'s, 'a> {
    surface: &'s mut Surface<'a>,
}

impl<'a> SurfaceLock<'_, 'a> {
    /// The pixels (`None` if the surface has none).
    pub fn pixels(&self) -> Option<&[u8]> {
        self.surface.pixels.bytes()
    }

    /// The pixels, writable (`None` if the surface has none or they are read-only).
    pub fn pixels_mut(&mut self) -> Option<&mut [u8]> {
        self.surface.pixels.bytes_mut()
    }

    /// The distance in bytes between rows of pixels.
    pub fn pitch(&self) -> i32 {
        self.surface.pitch
    }

    /// The locked surface.
    pub fn surface(&self) -> &Surface<'a> {
        self.surface
    }
}

impl Drop for SurfaceLock<'_, '_> {
    fn drop(&mut self) {
        self.surface.unlock_raw();
    }
}

#[cfg(test)]
pub(crate) mod tests;
