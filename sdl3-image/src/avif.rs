// Rust translation of src/IMG_avif.c from SDL_image (built with LOAD_AVIF,
// libavif linked in, and SAVE_AVIF on).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! AVIF images: the detector, the loader, the saver and the animation
//! decoder and encoder, over the translation of the parts of libavif they
//! use (in `avif/`: the ISOBMFF/HEIF container parser and the
//! `avifDecoder` API, the encoder and container writer for images and
//! image sequences, `avifImage`, the YUV and RGB conversions and alpha, the
//! plane scaler with the libyuv subset libavif bundles, the OBU sequence
//! header parser, the dav1d codec glue over the translation of dav1d in
//! `crate::dav1d` and the aom encoder glue over the translation of
//! libaom's all intra and real-time encoders in `crate::aom`), as
//! SDL_image builds libavif (dav1d as the only decoder, aom as the only
//! encoder, without libyuv conversions or libsharpyuv, the experimental
//! features off).
//!
//! The animation encoder writes an image sequence whose movie, track and
//! media headers carry the current time as their creation and
//! modification times, as libavif's do (the tests compare the files with
//! those zeroed). Its default threads, half the logical CPU cores, change
//! the coded frames between one thread and more: libaom codes the rows of
//! superblocks in turn with one thread and with its row multi-threading
//! (contexts taken from the row above) with more, which the translation
//! does in turn on the calling thread.

/* This is a AVIF image file loading framework */

// (libavif's API is translated whole, header constants and functions
// SDL_image doesn't call included; the translation keeps upstream's loops
// over indices, conditions, late initializations, explicit arithmetic
// (`x * 0`, `x % 2 == 0`) and function pointer variables as written)
#[allow(clippy::needless_range_loop)]
mod alpha;
#[allow(
    dead_code,
    clippy::collapsible_if,
    clippy::manual_is_multiple_of,
    clippy::module_inception,
    clippy::needless_range_loop
)]
mod avif;
#[allow(clippy::collapsible_if, clippy::needless_range_loop)]
mod codec_aom;
#[allow(
    clippy::collapsible_if,
    clippy::field_reassign_with_default,
    clippy::type_complexity
)]
mod codec_dav1d;
mod colr;
mod diag;
mod exif;
mod internal;
#[allow(dead_code)]
mod io;
#[allow(
    clippy::explicit_counter_loop,
    clippy::needless_late_init,
    clippy::type_complexity
)]
pub(crate) mod libyuv;
#[allow(clippy::manual_is_multiple_of, clippy::needless_late_init)]
mod obu;
#[allow(dead_code)]
mod rawdata;
#[allow(
    dead_code,
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::manual_clamp,
    clippy::manual_is_multiple_of,
    clippy::needless_late_init,
    clippy::needless_range_loop
)]
mod read;
#[allow(
    dead_code,
    clippy::erasing_op,
    clippy::excessive_precision,
    clippy::identity_op,
    clippy::manual_clamp,
    clippy::manual_div_ceil,
    clippy::manual_is_multiple_of,
    clippy::needless_late_init,
    clippy::needless_range_loop
)]
mod reformat;
mod scale;
#[allow(dead_code)]
mod stream;
mod utils;
#[allow(
    dead_code,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]
mod write;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStatus, IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::video::pixels::PackedOrder;
use sdl3::video::{
    Colorspace, PixelFormat, Surface, PROP_SURFACE_HDR_HEADROOM_FLOAT,
    PROP_SURFACE_SDR_WHITE_POINT_FLOAT,
};

use crate::anim_decoder::{
    DecoderCore, PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_INCREMENTAL_BOOLEAN,
    PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_PROGRESSIVE_BOOLEAN,
    PROP_ANIMATION_DECODER_CREATE_AVIF_MAX_THREADS_NUMBER, PROP_METADATA_AUTHOR_STRING,
    PROP_METADATA_COPYRIGHT_STRING, PROP_METADATA_CREATION_TIME_STRING,
    PROP_METADATA_DESCRIPTION_STRING, PROP_METADATA_FRAME_COUNT_NUMBER,
    PROP_METADATA_IGNORE_PROPS_BOOLEAN, PROP_METADATA_LOOP_COUNT_NUMBER,
    PROP_METADATA_TITLE_STRING,
};
use crate::anim_encoder::{
    EncoderCore, PROP_ANIMATION_ENCODER_CREATE_AVIF_KEYFRAME_INTERVAL_NUMBER,
    PROP_ANIMATION_ENCODER_CREATE_AVIF_MAX_THREADS_NUMBER,
};
use crate::util::read_ok;
use crate::xmlman;
use avif::{
    avif_image_create, avif_image_set_metadata_xmp, avif_result_to_string,
    avif_rgb_image_set_defaults, AvifChromaUpsampling, AvifEncoder, AvifImage, AvifPixelFormat,
    AvifRange, AvifResult, AvifRgbFormat, AvifRgbImage, AVIF_ADD_IMAGE_FLAG_FORCE_KEYFRAME,
    AVIF_ADD_IMAGE_FLAG_NONE, AVIF_ADD_IMAGE_FLAG_SINGLE, AVIF_MATRIX_COEFFICIENTS_BT709,
    AVIF_MATRIX_COEFFICIENTS_IDENTITY, AVIF_QUALITY_DEFAULT, AVIF_QUALITY_LOSSLESS,
    AVIF_REPETITION_COUNT_UNKNOWN, AVIF_SPEED_FASTEST, AVIF_STRICT_DISABLED,
    AVIF_TRANSFER_CHARACTERISTICS_HLG, AVIF_TRANSFER_CHARACTERISTICS_SMPTE2084,
};
use io::AvifIo;
use read::{
    avif_decoder_next_image, avif_decoder_parse, avif_peek_compatible_file_type, AvifDecoder,
};
use reformat::{avif_image_rgb_to_yuv, avif_image_yuv_to_rgb};
use write::{avif_encoder_add_image, avif_encoder_create, avif_encoder_finish};

/*
 * - `SDL_PROP_SURFACE_MAXCLL_NUMBER`: MaxCLL (Maximum Content Light Level)
 *   indicates the maximum light level of any single pixel (in cd/m2 or nits)
 *   of the content. MaxCLL is usually measured off the final delivered
 *   content after mastering. If one uses the full light level of the HDR
 *   mastering display and adds a hard clip at its maximum value, MaxCLL would
 *   be equal to the peak luminance of the mastering monitor.
 * - `SDL_PROP_SURFACE_MAXFALL_NUMBER`: MaxFALL (Maximum Frame Average Light
 *   Level) indicates the maximum value of the frame average light level (in
 *   cd/m2 or nits) of the content. MaxFALL is calculated by averaging the
 *   decoded luminance values of all the pixels within a frame. MaxFALL is
 *   usually much lower than MaxCLL.
 */
/// The MaxCLL of an HDR image (a number). Translation of
/// `SDL_PROP_SURFACE_MAXCLL_NUMBER` (IMG_avif.c's).
pub const PROP_SURFACE_MAXCLL_NUMBER: &str = "SDL.surface.maxCLL";
/// The MaxFALL of an HDR image (a number). Translation of
/// `SDL_PROP_SURFACE_MAXFALL_NUMBER` (IMG_avif.c's).
pub const PROP_SURFACE_MAXFALL_NUMBER: &str = "SDL.surface.maxFALL";

// (IMG_InitAVIF(): libavif is linked in, there is nothing to load; the
// function table `lib` is the translation's functions)

/// Read the file type box at the start of an AVIF file. Translation of
/// `ReadAVIFHeader()` (`None` for false).
fn read_avif_header(src: &mut IoStream<'_>) -> Option<Vec<u8>> {
    let mut magic = [0u8; 16];
    let mut read: u64 = 0;

    if !read_ok(src, &mut magic[..8]) {
        return None;
    }
    read += 8;

    if &magic[4..8] != b"ftyp" {
        return None;
    }

    let mut size = u32::from_be_bytes([magic[0], magic[1], magic[2], magic[3]]) as u64;
    if size == 1 {
        /* 64-bit header size */
        if !read_ok(src, &mut magic[8..16]) {
            return None;
        }
        read += 8;

        size = u64::from_be_bytes([
            magic[8], magic[9], magic[10], magic[11], magic[12], magic[13], magic[14], magic[15],
        ]);
    }

    if size > usize::MAX as u64 {
        return None;
    }
    if size <= read {
        return None;
    }

    /* Read in the header */
    // (read in pieces rather than allocated up front, so that a box size
    // past the end of the stream fails like upstream's short read without
    // a huge allocation first)
    let mut data = magic[..read as usize].to_vec();
    let mut remaining = size - read;
    let mut chunk = [0u8; 4096];
    while remaining > 0 {
        let n = remaining.min(chunk.len() as u64) as usize;
        if !read_ok(src, &mut chunk[..n]) {
            return None;
        }
        data.extend_from_slice(&chunk[..n]);
        remaining -= n as u64;
    }
    Some(data)
}

/* See if an image is contained in a data source */

/// Whether `src` holds an AVIF image (or image sequence); the stream
/// position is unchanged. Translation of `IMG_isAVIF()`.
pub fn is_avif(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_avif = false;
    if let Some(data) = read_avif_header(src) {
        /* This might be AVIF, do more thorough checks */
        is_avif = avif_peek_compatible_file_type(&data);
    }
    let _ = src.seek(start, IoWhence::Set);
    is_avif
}

/* Context for AFIF I/O operations */
/// Translation of `avifIOContext` (the stream is borrowed for each call,
/// see [`AvifSdlIo`]).
#[derive(Debug, Default)]
struct AvifIoContext {
    start: u64,
    data: Vec<u8>,
    size: i64,
}

/// The `avifIO` SDL_image gives libavif: its context and the stream.
struct AvifSdlIo<'c, 's, 'a> {
    context: &'c mut AvifIoContext,
    src: &'s mut IoStream<'a>,
}

impl AvifIo for AvifSdlIo<'_, '_, '_> {
    /// Translation of `ReadAVIFIO()`.
    fn read(
        &mut self,
        _read_flags: u32,
        offset: u64,
        size: usize,
    ) -> std::result::Result<&[u8], AvifResult> {
        let context = &mut *self.context;

        /* The AVIF reader bounces all over, so always seek to the correct offset */
        if self
            .src
            .seek(context.start.wrapping_add(offset) as i64, IoWhence::Set)
            .is_err()
        {
            return Err(AvifResult::IoError);
        }

        if size as u64 > context.size as u64 {
            // (SDL_realloc(): the buffer is reserved, and filled below as
            // the data arrives)
            context.data.clear();
            if context.data.try_reserve_exact(size).is_err() {
                return Err(AvifResult::IoError);
            }
            context.size = size as i64;
        }

        // (SDL_ReadIO(), in pieces)
        const CHUNK: usize = 64 * 1024;
        context.data.clear();
        while context.data.len() < size {
            let old = context.data.len();
            let chunk = CHUNK.min(size - old);
            context.data.resize(old + chunk, 0);
            let n = self.src.read(&mut context.data[old..]);
            context.data.truncate(old + n);
            if n < chunk {
                break;
            }
        }
        if context.data.is_empty() {
            if self.src.status() == IoStatus::NotReady {
                return Err(AvifResult::WaitingOnIo);
            } else {
                return Err(AvifResult::IoError);
            }
        }

        Ok(&context.data)
    }

    fn size_hint(&self) -> u64 {
        0
    }

    fn persistent(&self) -> bool {
        false
    }
}

// (DestroyAVIFIO(): dropping the context frees its buffer)

/// `SDL_DEFINE_COLORSPACE()`
fn define_colorspace(
    type_: u32,
    range: u32,
    primaries: u32,
    transfer: u32,
    matrix: u32,
    chroma: u32,
) -> Colorspace {
    Colorspace(
        (type_ << 28)
            | (range << 24)
            | (chroma << 20)
            | (primaries << 10)
            | (transfer << 5)
            | matrix,
    )
}
/// `SDL_COLOR_TYPE_RGB`
const SDL_COLOR_TYPE_RGB: u32 = 1;
/// `SDL_COLOR_RANGE_FULL`
const SDL_COLOR_RANGE_FULL: u32 = 2;
/// `SDL_MATRIX_COEFFICIENTS_IDENTITY`
const SDL_MATRIX_COEFFICIENTS_IDENTITY: u32 = 0;
/// `SDL_CHROMA_LOCATION_NONE`
const SDL_CHROMA_LOCATION_NONE: u32 = 0;

/// Write a 32-bit pixel (native-endian) at pixel index `at` of `pixels`.
fn put_u32(pixels: &mut [u8], at: usize, v: u32) {
    if let Some(p) = pixels.get_mut(at * 4..at * 4 + 4) {
        p.copy_from_slice(&v.to_ne_bytes());
    }
}

/// Translation of `ConvertGBR444toXBGR2101010()` (`Err` for -1).
fn convert_gbr444_to_xbgr2101010(
    image: &AvifImage,
    surface: &mut Surface<'static>,
) -> std::result::Result<(), ()> {
    let plane = |i: usize| {
        image.yuv_planes[i]
            .as_ref()
            .map(|p| p.u16s())
            .unwrap_or(&[])
    };
    // (the skips are computed in size_t, then converted to int)
    let skip = |i: usize| {
        ((image.yuv_row_bytes[i] as usize).wrapping_sub(image.width as usize * 2) / 2) as i32
    };

    let src_r = plane(2);
    let srcskip_r = skip(2);
    let src_g = plane(0);
    let srcskip_g = skip(0);
    let src_b = plane(1);
    let srcskip_b = skip(1);
    if srcskip_r < 0 || srcskip_g < 0 || srcskip_b < 0 {
        return Err(());
    }

    let pitch = surface.pitch();
    let dstskip = ((pitch as usize).wrapping_sub(image.width as usize * 4) / 4) as isize;
    let Some(dst) = surface.pixels_mut() else {
        return Ok(());
    };
    let (mut r, mut g, mut b) = (0usize, 0usize, 0usize);
    let mut d: isize = 0;

    let at = |s: &[u16], i: usize| s.get(i).copied().unwrap_or(0) as i32;
    for _y in 0..image.height {
        for _x in 0..image.width {
            let mut s_r = at(src_r, r);
            r += 1;
            let mut s_g = at(src_g, g);
            g += 1;
            let mut s_b = at(src_b, b);
            b += 1;
            s_r = s_r.min(1023);
            s_g = s_g.min(1023);
            s_b = s_b.min(1023);
            put_u32(
                dst,
                d as usize,
                (0x03 << 30) | ((s_b as u32) << 20) | ((s_g as u32) << 10) | s_r as u32,
            );
            d += 1;
        }
        r += srcskip_r as usize;
        g += srcskip_g as usize;
        b += srcskip_b as usize;
        d += dstskip;
    }
    Ok(())
}

/// Translation of `ConvertRGB16toXBGR2101010()`.
fn convert_rgb16_to_xbgr2101010(image: &AvifRgbImage<'_>, surface: &mut Surface<'static>) {
    let src = image.pixels.as_deref().unwrap_or(&[]);
    let mut s = 0usize;
    let pitch = surface.pitch();
    let dstskip = ((pitch as usize).wrapping_sub(image.width as usize * 4) / 4) as isize;
    let Some(dst) = surface.pixels_mut() else {
        return;
    };
    let mut d: isize = 0;

    let mut next = || {
        let v = src
            .get(s * 2..s * 2 + 2)
            .map(|b| u16::from_ne_bytes([b[0], b[1]]))
            .unwrap_or(0) as u32;
        s += 1;
        v
    };
    for _y in 0..image.height {
        for _x in 0..image.width {
            let s_r = next() >> 6;
            let s_g = next() >> 6;
            let s_b = next() >> 6;
            put_u32(
                dst,
                d as usize,
                (0x03 << 30) | (s_b << 20) | (s_g << 10) | s_r,
            );
            d += 1;
        }
        d += dstskip;
    }
}

/// `SDL_malloc()` of the 16-bit RGB buffer: `None` when it can't be
/// allocated.
fn alloc_pixels(size: usize) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(size).ok()?;
    v.resize(size, 0);
    Some(v)
}

/// The 16-bit RGB conversion of a PQ image, converted to XBGR2101010:
/// `None` when the surface couldn't be created.
fn convert_rgb16_surface(
    image: &AvifImage,
    oom_error: Option<&'static str>,
) -> Result<Option<Surface<'static>>> {
    // Convert the YUV image to 10-bit RGB
    let mut rgb = AvifRgbImage {
        width: image.width,
        height: image.height,
        depth: 16,
        format: AvifRgbFormat::Rgb,
        row_bytes: (image.width as usize * 3 * 2) as u32,
        ..Default::default()
    };
    let size = image.height.wrapping_mul(rgb.row_bytes) as usize;
    let Some(mut pixels) = alloc_pixels(size) else {
        return Err(match oom_error {
            Some(message) => Error::new(message),
            None => Error::out_of_memory(),
        });
    };
    rgb.pixels = Some(&mut pixels);
    let result = avif_image_yuv_to_rgb(image, &mut rgb);
    if result != AvifResult::Ok {
        return Err(Error::new(format!(
            "Couldn't convert AVIF image to RGB: {}",
            avif_result_to_string(result)
        )));
    }

    let surface = Surface::new(
        image.width as i32,
        image.height as i32,
        PixelFormat::XBGR2101010,
    );
    Ok(match surface {
        Ok(mut surface) => {
            convert_rgb16_to_xbgr2101010(&rgb, &mut surface);
            Some(surface)
        }
        Err(_) => None,
    })
}

/// Load an AVIF image (the first image of a sequence) from `src`. HDR (PQ)
/// images are converted to XBGR2101010 with their colorspace and light
/// levels in the surface's properties, others to ARGB8888. Translation of
/// `IMG_LoadAVIF_IO()`.
pub fn load_avif_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);

    // (IMG_InitAVIF(): nothing to load)

    let result = load_avif_io_internal(src, start);
    if result.is_err() {
        let _ = src.seek(start, IoWhence::Set);
    }
    result
}

fn load_avif_io_internal(src: &mut IoStream<'_>, start: i64) -> Result<Surface<'static>> {
    let mut context = AvifIoContext::default();

    let mut decoder = Box::new(AvifDecoder::new());

    /* Be permissive so we can load as many images as possible */
    decoder.strict_flags = AVIF_STRICT_DISABLED;

    context.start = start as u64;
    let mut io = AvifSdlIo {
        context: &mut context,
        src,
    };

    let result = avif_decoder_parse(&mut decoder, &mut io);
    if result != AvifResult::Ok {
        return Err(Error::new(format!(
            "Couldn't parse AVIF image: {}",
            avif_result_to_string(result)
        )));
    }

    let result = avif_decoder_next_image(&mut decoder, &mut io);
    if result != AvifResult::Ok {
        return Err(Error::new(format!(
            "Couldn't get AVIF image: {}",
            avif_result_to_string(result)
        )));
    }

    let Some(image) = decoder.image.as_deref() else {
        return Err(Error::new("Couldn't get AVIF image"));
    };
    let mut surface: Option<Surface<'static>> = None;
    if image.transfer_characteristics == AVIF_TRANSFER_CHARACTERISTICS_SMPTE2084 {
        // This is an HDR PQ image

        if image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_IDENTITY
            && image.yuv_format == AvifPixelFormat::Yuv444
        {
            // This image uses identity GBR channel ordering
            if image.depth == 10 {
                if let Ok(mut s) = Surface::new(
                    image.width as i32,
                    image.height as i32,
                    PixelFormat::XBGR2101010,
                ) {
                    if convert_gbr444_to_xbgr2101010(image, &mut s).is_ok() {
                        surface = Some(s);
                    }
                    // Invalid image, let avif take care of it
                    // Note (upstream): the surface is freed with SDL_free()
                    // instead of SDL_DestroySurface(), leaking its pixels
                    // (dropped here).
                }
            }
        }

        if surface.is_none() {
            surface = convert_rgb16_surface(image, None)?;
        }

        if let Some(surface) = &mut surface {
            // Set HDR properties

            // The older standards use an SDR white point of 100 nits.
            // ITU-R BT.2408-6 recommends using an SDR white point of 203 nits.
            // This is the default Chrome uses, and what a lot of game content
            // assumes, so we'll go with that.
            const DEFAULT_PQ_SDR_WHITE_POINT: f32 = 203.0;

            // The official definition is 10000, but PQ game content is often mastered for 400 or 1000 nits
            const DEFAULT_PQ_MAXCLL: u16 = 1000;
            let mut max_cll = DEFAULT_PQ_MAXCLL;

            let props = surface.properties();
            let colorspace = define_colorspace(
                SDL_COLOR_TYPE_RGB,
                SDL_COLOR_RANGE_FULL,
                image.color_primaries as u32,
                image.transfer_characteristics as u32,
                SDL_MATRIX_COEFFICIENTS_IDENTITY,
                SDL_CHROMA_LOCATION_NONE,
            );
            surface.set_colorspace(colorspace);
            if image.clli.max_cll > 0 {
                max_cll = image.clli.max_cll;
                let _ = props.set(PROP_SURFACE_MAXCLL_NUMBER, image.clli.max_cll as i64);
            }
            if image.clli.max_pall > 0 {
                let _ = props.set(PROP_SURFACE_MAXFALL_NUMBER, image.clli.max_pall as i64);
            }
            let _ = props.set(
                PROP_SURFACE_SDR_WHITE_POINT_FLOAT,
                DEFAULT_PQ_SDR_WHITE_POINT,
            );
            let _ = props.set(
                PROP_SURFACE_HDR_HEADROOM_FLOAT,
                max_cll as f32 / DEFAULT_PQ_SDR_WHITE_POINT,
            );
        }
    }

    let surface = match surface {
        Some(surface) => surface,
        None => {
            let mut surface = Surface::new(
                image.width as i32,
                image.height as i32,
                PixelFormat::ARGB8888,
            )?;

            /* Convert the YUV image to RGB */
            let (w, h, pitch) = (surface.width(), surface.height(), surface.pitch());
            let mut rgb = AvifRgbImage {
                width: w as u32,
                height: h as u32,
                depth: 8,
                #[cfg(target_endian = "little")]
                format: AvifRgbFormat::Bgra,
                #[cfg(target_endian = "big")]
                format: AvifRgbFormat::Argb,
                row_bytes: pitch as u32,
                ..Default::default()
            };
            rgb.pixels = surface.pixels_mut();
            let result = avif_image_yuv_to_rgb(image, &mut rgb);
            if result != AvifResult::Ok {
                return Err(Error::new(format!(
                    "Couldn't convert AVIF image to RGB: {}",
                    avif_result_to_string(result)
                )));
            }
            surface
        }
    };

    // (done: dropping the decoder is avifDecoderDestroy())
    Ok(surface)
}

/// Translation of `IMG_SaveAVIF_IO_libavif()`.
fn save_avif_io_libavif(
    surface: &mut Surface<'_>,
    dst: &mut IoStream<'_>,
    quality: i32,
) -> Result<()> {
    // (IMG_InitAVIF(): nothing to load)

    /* Get the colorspace and light level properties, if any */
    let colorspace = surface.colorspace();
    let props = surface.properties();
    let max_cll = props.get_number(PROP_SURFACE_MAXCLL_NUMBER).unwrap_or(0) as u16;
    let max_fall = props.get_number(PROP_SURFACE_MAXFALL_NUMBER).unwrap_or(0) as u16;

    let Some(mut image) = avif_image_create(
        surface.width() as u32,
        surface.height() as u32,
        10,
        AvifPixelFormat::Yuv444,
    ) else {
        return Err(Error::new("Couldn't create AVIF YUV image"));
    };
    image.yuv_range = AvifRange::Full;
    image.color_primaries = colorspace.primaries() as u16;
    image.transfer_characteristics = colorspace.transfer() as u16;
    image.clli.max_cll = max_cll;
    image.clli.max_pall = max_fall;

    let mut rgb = AvifRgbImage::default();
    avif_rgb_image_set_defaults(&mut rgb, &image);

    let format = surface.format();
    if format.is_10bit() {
        const EXPAND_ALPHA: [u16; 4] = [0, 0x155, 0x2aa, 0x3ff];

        /* We are not actually using YUV, but storing raw GBR (yes not RGB) data */
        image.yuv_format = AvifPixelFormat::Yuv444;
        image.depth = 10;
        image.matrix_coefficients = AVIF_MATRIX_COEFFICIENTS_IDENTITY;

        if format.pixel_order() == PackedOrder::Xrgb as u32
            || format.pixel_order() == PackedOrder::Argb as u32
        {
            rgb.format = AvifRgbFormat::Rgba;
        } else {
            rgb.format = AvifRgbFormat::Bgra;
        }
        rgb.ignore_alpha = !format.has_alpha();
        rgb.depth = 10;
        rgb.row_bytes = image.width * 4 * std::mem::size_of::<u16>() as u32;
        let size = image.height as usize * rgb.row_bytes as usize;
        let mut pixels = Vec::new();
        if pixels.try_reserve_exact(size).is_err() {
            return Err(Error::new("Out of memory"));
        }
        pixels.resize(size, 0u8);

        let src = surface.pixels().unwrap_or(&[]);
        let pitch = surface.pitch() as usize;
        let mut dst16 = 0;
        for y in 0..image.height as usize {
            for x in 0..image.width as usize {
                let o = y * pitch + x * 4;
                let pixelvalue = u32::from_ne_bytes([src[o], src[o + 1], src[o + 2], src[o + 3]]);

                for v in [
                    ((pixelvalue >> 20) & 0x3FF) as u16,
                    ((pixelvalue >> 10) & 0x3FF) as u16,
                    (pixelvalue & 0x3FF) as u16,
                    EXPAND_ALPHA[((pixelvalue >> 30) & 0x3) as usize],
                ] {
                    pixels[dst16..dst16 + 2].copy_from_slice(&v.to_ne_bytes());
                    dst16 += 2;
                }
            }
        }
        rgb.pixels = Some(&mut pixels);

        let rc = avif_image_rgb_to_yuv(&mut image, &rgb);
        if rc != AvifResult::Ok {
            return Err(Error::new(format!(
                "Couldn't convert to YUV: {}",
                avif_result_to_string(rc)
            )));
        }
    } else {
        let mut temp = None;

        rgb.depth = 8;
        if format == PixelFormat::RGBX32 || format == PixelFormat::RGBA32 {
            rgb.format = AvifRgbFormat::Rgba;
        } else if format == PixelFormat::XRGB32 || format == PixelFormat::ARGB32 {
            rgb.format = AvifRgbFormat::Argb;
        } else if format == PixelFormat::BGRX32 || format == PixelFormat::BGRA32 {
            rgb.format = AvifRgbFormat::Bgra;
        } else if format == PixelFormat::XBGR32 || format == PixelFormat::ABGR32 {
            rgb.format = AvifRgbFormat::Abgr;
        } else {
            /* Need to convert to a format libavif understands */
            rgb.format = AvifRgbFormat::Rgba;
            if format.has_alpha() {
                temp = Some(surface.convert(PixelFormat::RGBA32)?);
            } else {
                temp = Some(surface.convert(PixelFormat::RGBX32)?);
            }
        }
        rgb.ignore_alpha = !format.has_alpha();
        // (the pixels are only read: a copy when the surface's own can't
        // be borrowed writable)
        let temp_surface: &Surface<'_> = temp.as_ref().unwrap_or(surface);
        let mut pixels = temp_surface.pixels().map(|p| p.to_vec());
        rgb.pixels = pixels.as_deref_mut();
        rgb.row_bytes = temp_surface.pitch() as u32;

        /* Convert to YUV */
        let rc = avif_image_rgb_to_yuv(&mut image, &rgb);

        /* Do any cleanup */
        // Note (upstream): the converted surface is released with
        // SDL_free(), which leaks its pixels; it is dropped here.
        drop(temp);
        // (rgb.pixels = NULL: the pixels aren't rgb's to free)

        /* Check the result of the conversion */
        if rc != AvifResult::Ok {
            return Err(Error::new(format!(
                "Couldn't convert to YUV: {}",
                avif_result_to_string(rc)
            )));
        }
    }

    // Note (upstream): C doesn't check avifEncoderCreate() for NULL (a
    // failed allocation) before it uses the encoder; here that's an error.
    let Some(mut encoder) = avif_encoder_create() else {
        return Err(Error::new("Out of memory"));
    };
    encoder.quality = quality;
    encoder.quality_alpha = AVIF_QUALITY_LOSSLESS;
    encoder.speed = AVIF_SPEED_FASTEST;

    let rc = avif_encoder_add_image(&mut encoder, &image, 1, AVIF_ADD_IMAGE_FLAG_SINGLE);
    if rc != AvifResult::Ok {
        return Err(Error::new(format!(
            "Failed to add image to avif encoder: {}",
            avif_result_to_string(rc)
        )));
    }

    let mut avif_output = Vec::new();
    let rc = avif_encoder_finish(&mut encoder, &mut avif_output);
    if rc != AvifResult::Ok {
        return Err(Error::new(format!(
            "Failed to finish encoder: {}",
            avif_result_to_string(rc)
        )));
    }

    if dst.write(&avif_output) == avif_output.len() {
        Ok(())
    } else {
        Err(crate::util::write_error(dst))
    }
    // (done: dropping the image, the encoder and the output frees them)
}

/// Save `surface` as AVIF at `quality` (0 to 100; 100 is lossless): 10-bit
/// YUV 4:4:4 (GBR for 10-bit surfaces), coded with libaom's all intra
/// encoder at its fastest speed, with lossless alpha.
/// Translation of `IMG_SaveAVIF_IO()` (without `closeio`: the caller keeps
/// `dst`).
pub fn save_avif_io(surface: &mut Surface<'_>, dst: &mut IoStream<'_>, quality: i32) -> Result<()> {
    crate::img::verify_can_save_surface(surface)?;
    save_avif_io_libavif(surface, dst, quality)
}

/// Save `surface` to an AVIF file (see [`save_avif_io`]). Translation of
/// `IMG_SaveAVIF()`.
pub fn save_avif(
    surface: &mut Surface<'_>,
    file: impl AsRef<std::path::Path>,
    quality: i32,
) -> Result<()> {
    crate::img::verify_can_save_surface(surface)?;
    let mut dst = IoStream::from_file(file, "wb")?;
    let result = save_avif_io(surface, &mut dst, quality);
    let closed = dst.close();
    result.and(closed)
}

/// Translation of `SetHDRProperties()`.
fn set_hdr_properties(surface: &mut Surface<'static>, image: &AvifImage) {
    // Standard HDR constants
    const DEFAULT_PQ_SDR_WHITE_POINT: f32 = 203.0;
    const DEFAULT_PQ_MAXCLL: u16 = 1000;
    let mut max_cll = DEFAULT_PQ_MAXCLL;

    let props = surface.properties();

    // Set colorspace first
    let colorspace = define_colorspace(
        SDL_COLOR_TYPE_RGB,
        SDL_COLOR_RANGE_FULL,
        image.color_primaries as u32,
        image.transfer_characteristics as u32,
        image.matrix_coefficients as u32,
        SDL_CHROMA_LOCATION_NONE,
    );
    surface.set_colorspace(colorspace);

    // Check if this is an HDR image by transfer function
    let is_hdr = image.transfer_characteristics == AVIF_TRANSFER_CHARACTERISTICS_SMPTE2084
        || image.transfer_characteristics == AVIF_TRANSFER_CHARACTERISTICS_HLG;

    if is_hdr {
        // Use metadata if available
        if image.clli.max_cll > 0 {
            max_cll = image.clli.max_cll;
            let _ = props.set(PROP_SURFACE_MAXCLL_NUMBER, image.clli.max_cll as i64);
        } else {
            // Default values based on common HDR mastering practices
            let _ = props.set(PROP_SURFACE_MAXCLL_NUMBER, DEFAULT_PQ_MAXCLL as i64);
        }

        if image.clli.max_pall > 0 {
            let _ = props.set(PROP_SURFACE_MAXFALL_NUMBER, image.clli.max_pall as i64);
        } else {
            // A reasonable default for MaxFALL
            let _ = props.set(PROP_SURFACE_MAXFALL_NUMBER, (DEFAULT_PQ_MAXCLL / 4) as i64);
        }

        // HDR properties needed for proper tone mapping
        let _ = props.set(
            PROP_SURFACE_SDR_WHITE_POINT_FLOAT,
            DEFAULT_PQ_SDR_WHITE_POINT,
        );
        let _ = props.set(
            PROP_SURFACE_HDR_HEADROOM_FLOAT,
            max_cll as f32 / DEFAULT_PQ_SDR_WHITE_POINT,
        );
    }
}

/// The AVIF animation decoder's state. Translation of IMG_avif.c's
/// `struct IMG_AnimationDecoderContext` (`io` is made from `io_context`
/// and the decoder's stream for each call).
pub(crate) struct AvifDecoderContext {
    /// AVIF decoder instance
    decoder: Option<Box<AvifDecoder>>,
    /// Context data for IO operations
    io_context: AvifIoContext,
    /// Starting position in the stream
    start_pos: i64,

    /// Current frame index
    current_frame: i32,
    /// Total number of frames in the animation
    total_frames: i32,

    /// Width of the animation
    #[allow(dead_code)]
    width: i32,
    /// Height of the animation
    #[allow(dead_code)]
    height: i32,

    /// The decoder's thread count, progressive and incremental modes and
    /// metadata setting, for the one a reset creates
    max_threads: i32,
    allow_progressive: bool,
    allow_incremental: bool,
    ignore_props: bool,
}

impl AvifDecoderContext {
    /// Translation of `IMG_AnimationDecoderReset_Internal()`.
    pub(crate) fn reset(&mut self, d: &mut DecoderCore<'_, '_>) -> Result<()> {
        // Reset the decoder
        self.decoder = None;

        // Reset stream position
        if d.src().seek(self.start_pos, IoWhence::Set).is_err() {
            return Err(Error::new("Failed to seek to beginning of AVIF file"));
        }

        // Recreate the decoder
        let mut decoder = Box::new(AvifDecoder::new());

        // Be permissive to decode as many frames as possible
        decoder.strict_flags = AVIF_STRICT_DISABLED;
        // Note (upstream): upstream's new decoder doesn't get the thread
        // count, progressive and incremental modes and metadata settings
        // the first one was created with (libavif's defaults, without the
        // progressive mode, are used); the translation fixes it.
        decoder.max_threads = self.max_threads;
        decoder.allow_progressive = self.allow_progressive;
        decoder.allow_incremental = self.allow_incremental;
        decoder.ignore_exif = self.ignore_props;
        decoder.ignore_xmp = self.ignore_props;

        // Set up IO
        self.io_context = AvifIoContext {
            start: self.start_pos as u64,
            data: Vec::new(),
            size: 0,
        };

        // Reset state
        self.current_frame = 0;

        // Need to re-parse the animation if we reset
        let mut io = AvifSdlIo {
            context: &mut self.io_context,
            src: d.src(),
        };
        let result = avif_decoder_parse(&mut decoder, &mut io);
        self.decoder = Some(decoder);
        if result != AvifResult::Ok {
            return Err(Error::new(format!(
                "Couldn't re-parse AVIF animation after reset: {}",
                avif_result_to_string(result)
            )));
        }

        Ok(())
    }

    /// Translation of `IMG_AnimationDecoderGetNextFrame_Internal()`:
    /// `Ok(None)` (with the COMPLETE status) when there are no more frames.
    pub(crate) fn get_next_frame(
        &mut self,
        d: &mut DecoderCore<'_, '_>,
    ) -> Result<Option<(Surface<'static>, u64)>> {
        if self.total_frames - self.current_frame < 1 {
            d.status = crate::AnimationDecoderStatus::Complete;
            return Ok(None);
        }

        // Note (upstream): after a failed reset there is no decoder, which
        // upstream dereferences; here that's an error.
        let Some(decoder) = self.decoder.as_deref_mut() else {
            return Err(Error::invalid_param("decoder"));
        };
        let mut io = AvifSdlIo {
            context: &mut self.io_context,
            src: d.src(),
        };
        let result = avif_decoder_next_image(decoder, &mut io);
        if result != AvifResult::Ok {
            if result == AvifResult::NoImagesRemaining {
                // This shouldn't happen here, but handle it gracefully
                d.status = crate::AnimationDecoderStatus::Complete;
                return Ok(None);
            }

            return Err(Error::new(format!(
                "Couldn't get AVIF frame {}: {}",
                self.current_frame + 1,
                avif_result_to_string(result)
            )));
        }

        let Some(image) = decoder.image.as_deref() else {
            return Err(Error::invalid_param("image"));
        };
        let frame_surface: Option<Surface<'static>> = if image.depth == 16 {
            // Handle 16-bit depth
            let Ok(mut s) =
                Surface::new(image.width as i32, image.height as i32, PixelFormat::RGBA64)
            else {
                return Err(Error::new("Couldn't create 16-bit surface for AVIF frame"));
            };

            let pitch = s.pitch();
            let mut rgb = AvifRgbImage {
                width: image.width,
                height: image.height,
                depth: 16,
                format: AvifRgbFormat::Rgba,
                row_bytes: pitch as u32,
                ignore_alpha: false,
                ..Default::default()
            };
            rgb.pixels = s.pixels_mut();

            let result = avif_image_yuv_to_rgb(image, &mut rgb);
            if result != AvifResult::Ok {
                return Err(Error::new(format!(
                    "Couldn't convert 16-bit AVIF image to RGB: {}",
                    avif_result_to_string(result)
                )));
            }

            // Set HDR properties if needed
            set_hdr_properties(&mut s, image);
            Some(s)
        } else if image.transfer_characteristics == AVIF_TRANSFER_CHARACTERISTICS_SMPTE2084 {
            // Handle HDR PQ image
            let mut surface = None;
            if image.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_IDENTITY
                && image.yuv_format == AvifPixelFormat::Yuv444
                && image.depth == 10
            {
                if let Ok(mut s) = Surface::new(
                    image.width as i32,
                    image.height as i32,
                    PixelFormat::XBGR2101010,
                ) {
                    if convert_gbr444_to_xbgr2101010(image, &mut s).is_ok() {
                        surface = Some(s);
                    }
                }
            }

            if surface.is_none() {
                surface = convert_rgb16_surface(image, Some("Out of memory for AVIF RGB pixels"))?;
            }

            if let Some(s) = &mut surface {
                set_hdr_properties(s, image);
            }
            surface
        } else {
            let Ok(mut s) =
                Surface::new(image.width as i32, image.height as i32, PixelFormat::RGBA32)
            else {
                return Err(Error::new("Couldn't create surface for AVIF frame"));
            };

            let (w, h, pitch) = (s.width(), s.height(), s.pitch());
            let mut rgb = AvifRgbImage {
                width: w as u32,
                height: h as u32,
                depth: 8,
                #[cfg(target_endian = "little")]
                format: AvifRgbFormat::Rgba,
                #[cfg(target_endian = "big")]
                format: AvifRgbFormat::Abgr,
                ignore_alpha: false,
                row_bytes: pitch as u32,
                ..Default::default()
            };
            rgb.pixels = s.pixels_mut();

            let result = avif_image_yuv_to_rgb(image, &mut rgb);
            if result != AvifResult::Ok {
                return Err(Error::new(format!(
                    "Couldn't convert AVIF image to RGB: {}",
                    avif_result_to_string(result)
                )));
            }

            let colorspace = define_colorspace(
                SDL_COLOR_TYPE_RGB,
                SDL_COLOR_RANGE_FULL,
                image.color_primaries as u32,
                image.transfer_characteristics as u32,
                image.matrix_coefficients as u32,
                SDL_CHROMA_LOCATION_NONE,
            );
            s.set_colorspace(colorspace);
            Some(s)
        };

        // Note (upstream): upstream takes the duration, which is in the
        // file's timescale, as one in the decoder's time base (only
        // multiplying it by the numerator), so the delays are wrong unless
        // the timescale is the time base's denominator; the translation
        // fixes it, converting it from the timescale (and taking no
        // duration for a zero one, which libavif leaves without a duration
        // too).
        let duration = if decoder.timescale != 0 {
            d.decoder_duration(
                decoder.image_timing.duration_in_timescales,
                decoder.timescale,
            )
        } else {
            0
        };

        self.current_frame += 1;

        // (upstream returns success without a frame when the HDR surface
        // couldn't be created; that is an error here)
        match frame_surface {
            Some(frame) => Ok(Some((frame, duration))),
            None => Err(Error::new("Couldn't create surface for AVIF frame")),
        }
    }
}

/// Create the AVIF decoder of an animation decoder: the stream parsed
/// (an image sequence's frames, or an image's progressive layers, or the
/// image) and, unless [`PROP_METADATA_IGNORE_PROPS_BOOLEAN`] is set, the
/// frame and loop counts and the XMP metadata in the decoder's properties.
/// Translation of `IMG_CreateAVIFAnimationDecoder()` (its
/// `IMG_AnimationDecoderClose_Internal()` is dropping the context).
pub(crate) fn create_avif_animation_decoder(
    d: &mut DecoderCore<'_, '_>,
    props: &Properties,
) -> Result<Box<AvifDecoderContext>> {
    // (IMG_InitAVIF(): nothing to load)

    let start_pos = d.src().tell().unwrap_or(-1);
    if start_pos < 0 {
        return Err(Error::new("Failed to get current stream position"));
    }

    let mut decoder = Box::new(AvifDecoder::new());

    decoder.strict_flags = AVIF_STRICT_DISABLED;
    // Note (upstream): upstream sets the timescale to the time base's
    // denominator here, but it is the decoder's output, the file's, which
    // avifDecoderParse() overwrites; the translation leaves it out (the
    // durations are converted from the file's timescale in
    // get_next_frame()).

    let mut io_context = AvifIoContext {
        start: start_pos as u64,
        data: Vec::new(),
        size: 0,
    };

    let max_l_cores = sdl3::cpuinfo::num_logical_cpu_cores();
    let mut max_threads = props
        .get_number(PROP_ANIMATION_DECODER_CREATE_AVIF_MAX_THREADS_NUMBER)
        .unwrap_or((max_l_cores / 2) as i64) as i32;
    max_threads = if max_threads < 1 {
        1
    } else if max_threads > max_l_cores {
        max_l_cores
    } else {
        max_threads
    };
    decoder.max_threads = max_threads;

    let allow_progressive = props
        .get_bool(PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_PROGRESSIVE_BOOLEAN)
        .unwrap_or(true);
    decoder.allow_progressive = allow_progressive;

    // Note (upstream): upstream reads the progressive property here, not
    // IMG_PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_INCREMENTAL_BOOLEAN; the
    // translation fixes it.
    let allow_incremental = props
        .get_bool(PROP_ANIMATION_DECODER_CREATE_AVIF_ALLOW_INCREMENTAL_BOOLEAN)
        .unwrap_or(false);
    decoder.allow_incremental = allow_incremental;

    let ignore_props = props
        .get_bool(PROP_METADATA_IGNORE_PROPS_BOOLEAN)
        .unwrap_or(false);
    decoder.ignore_exif = ignore_props;
    decoder.ignore_xmp = ignore_props;

    let mut io = AvifSdlIo {
        context: &mut io_context,
        src: d.src(),
    };
    let result = avif_decoder_parse(&mut decoder, &mut io);
    if result != AvifResult::Ok {
        return Err(Error::new(format!(
            "Couldn't parse AVIF animation: {}",
            avif_result_to_string(result)
        )));
    }

    if decoder.image_count < 1 {
        return Err(Error::new("No frames found in AVIF"));
    }

    let (width, height) = decoder
        .image
        .as_deref()
        .map(|image| (image.width as i32, image.height as i32))
        .unwrap_or((0, 0));
    let total_frames = decoder.image_count;

    if !ignore_props {
        // Allow implicit properties to be set which are not globalized but specific to the decoder.
        let _ = d
            .props
            .set(PROP_METADATA_FRAME_COUNT_NUMBER, total_frames as i64);

        // Set well-defined properties.
        if decoder.repetition_count != AVIF_REPETITION_COUNT_UNKNOWN {
            let _ = d.props.set(
                PROP_METADATA_LOOP_COUNT_NUMBER,
                decoder.repetition_count.wrapping_add(1) as i64,
            );
        }

        // Get other well-defined properties and set them in our props.
        if !decoder.ignore_xmp {
            let data = decoder
                .image
                .as_deref()
                .map(|image| &image.xmp[..])
                .unwrap_or(&[]);
            if !data.is_empty() {
                let desc = xmlman::get_xmp_description(data);
                let rights = xmlman::get_xmp_copyright(data);
                let title = xmlman::get_xmp_title(data);
                let creator = xmlman::get_xmp_creator(data);
                let create_date = xmlman::get_xmp_create_date(data);
                if let Some(desc) = desc {
                    let _ = d.props.set(PROP_METADATA_DESCRIPTION_STRING, desc);
                }
                if let Some(rights) = rights {
                    let _ = d.props.set(PROP_METADATA_COPYRIGHT_STRING, rights);
                }
                if let Some(title) = title {
                    let _ = d.props.set(PROP_METADATA_TITLE_STRING, title);
                }
                if let Some(creator) = creator {
                    let _ = d.props.set(PROP_METADATA_AUTHOR_STRING, creator);
                }
                if let Some(create_date) = create_date {
                    let _ = d.props.set(PROP_METADATA_CREATION_TIME_STRING, create_date);
                }
            }
        }
    }

    Ok(Box::new(AvifDecoderContext {
        decoder: Some(decoder),
        io_context,
        start_pos,
        current_frame: 0,
        total_frames,
        width,
        height,
        max_threads,
        allow_progressive,
        allow_incremental,
        ignore_props,
    }))
}

/// Translation of `struct IMG_AnimationEncoderContext` of the AVIF
/// animation encoder.
pub(crate) struct AvifEncoderContext {
    encoder: Option<Box<AvifEncoder>>,
    first_frame_added: bool,
    metadata: Option<Properties>,
}

impl AvifEncoderContext {
    /// Translation of `AnimationEncoder_AddFrame()`.
    pub(crate) fn add_frame(
        &mut self,
        e: &mut EncoderCore<'_, '_>,
        surface: &mut Surface<'_>,
        duration: u64,
    ) -> Result<()> {
        let mut rc: AvifResult;
        let mut temp: Option<Surface<'static>> = None;
        let is_lossless = e.quality == 100;
        let format = surface.format();
        let is10bit = format.is_10bit();
        let is16bit = format == PixelFormat::RGB48
            || format == PixelFormat::BGR48
            || format == PixelFormat::RGBA64
            || format == PixelFormat::ARGB64
            || format == PixelFormat::BGRA64
            || format == PixelFormat::ABGR64;
        let has_alpha = format.has_alpha();

        let duration_in_timescales = duration.wrapping_mul(e.timebase_numerator as u64);
        let colorspace = surface.colorspace();
        let props = surface.properties();
        let max_cll = props.get_number(PROP_SURFACE_MAXCLL_NUMBER).unwrap_or(0) as u16;
        let max_fall = props.get_number(PROP_SURFACE_MAXFALL_NUMBER).unwrap_or(0) as u16;

        let depth: u32 = if is16bit {
            16
        } else if is10bit {
            10
        } else {
            8
        };
        let pixel_format = if is_lossless {
            AvifPixelFormat::None
        } else {
            AvifPixelFormat::Yuv444
        };

        let Some(mut image) = avif_image_create(
            surface.width() as u32,
            surface.height() as u32,
            depth,
            pixel_format,
        ) else {
            return Err(Error::new("Couldn't create AVIF image"));
        };

        if !self.first_frame_added {
            if let Some(metadata) = self
                .metadata
                .as_ref()
                .filter(|m| crate::anim_encoder::has_metadata(m))
            {
                let title = metadata.get_string(PROP_METADATA_TITLE_STRING);
                let author = metadata.get_string(PROP_METADATA_AUTHOR_STRING);
                let description = metadata.get_string(PROP_METADATA_DESCRIPTION_STRING);
                let copyright = metadata.get_string(PROP_METADATA_COPYRIGHT_STRING);
                let creation_time = metadata.get_string(PROP_METADATA_CREATION_TIME_STRING);
                // Note (upstream): upstream constructs the XMP for any
                // metadata property (the loop count alone, say), and without
                // one of the five XMP strings none is constructed, which
                // fails the frame; the translation fixes it, writing no XMP
                // then.
                if title.is_some()
                    || author.is_some()
                    || description.is_some()
                    || copyright.is_some()
                    || creation_time.is_some()
                {
                    let xmp_data = xmlman::construct_xmp_with_rdf_description(
                        title.as_deref(),
                        author.as_deref(),
                        description.as_deref(),
                        copyright.as_deref(),
                        creation_time.as_deref(),
                    );
                    let Some(xmp_data) = xmp_data.filter(|d| !d.is_empty()) else {
                        return Err(Error::new("Couldn't create XMP data for AVIF image"));
                    };
                    let ar = avif_image_set_metadata_xmp(&mut image, &xmp_data);
                    if ar != AvifResult::Ok {
                        return Err(Error::new(format!(
                            "Couldn't set XMP metadata for AVIF image: {}",
                            avif_result_to_string(ar)
                        )));
                    }
                }
            }
        }

        image.yuv_range = AvifRange::Full;
        image.color_primaries = colorspace.primaries() as u16;
        image.transfer_characteristics = colorspace.transfer() as u16;

        if is10bit || is16bit || is_lossless {
            image.matrix_coefficients = AVIF_MATRIX_COEFFICIENTS_IDENTITY;
        } else {
            image.matrix_coefficients = AVIF_MATRIX_COEFFICIENTS_BT709;
        }

        image.clli.max_cll = max_cll;
        image.clli.max_pall = max_fall;

        let mut rgb = AvifRgbImage::default();
        avif_rgb_image_set_defaults(&mut rgb, &image);

        // (SDL_LockSurface(): the pixels are readable here; the conversions
        // read a copy of them)
        let mut pixels: Vec<u8>;
        if is16bit {
            rgb.depth = 16;

            rgb.format = if format == PixelFormat::RGB48 {
                AvifRgbFormat::Rgb
            } else if format == PixelFormat::BGR48 {
                AvifRgbFormat::Bgr
            } else if format == PixelFormat::RGBA64 {
                AvifRgbFormat::Rgba
            } else if format == PixelFormat::ARGB64 {
                AvifRgbFormat::Argb
            } else if format == PixelFormat::BGRA64 {
                AvifRgbFormat::Bgra
            } else {
                // (SDL_PIXELFORMAT_ABGR64, the last of the 16-bit formats:
                // the default case can't be reached)
                AvifRgbFormat::Abgr
            };

            rgb.ignore_alpha = !has_alpha;
            pixels = surface.pixels().map(|p| p.to_vec()).unwrap_or_default();
            rgb.pixels = Some(&mut pixels);
            rgb.row_bytes = surface.pitch() as u32;

            if !is_lossless {
                rc = avif_image_rgb_to_yuv(&mut image, &rgb);

                if rc != AvifResult::Ok {
                    return Err(Error::new(format!(
                        "Couldn't convert 16-bit RGB to YUV: {}",
                        avif_result_to_string(rc)
                    )));
                }
            }
        } else if is10bit {
            const EXPAND_ALPHA: [u16; 4] = [0, 0x155, 0x2aa, 0x3ff];

            rgb.depth = 10;
            rgb.format = if format.pixel_order() == PackedOrder::Xrgb as u32
                || format.pixel_order() == PackedOrder::Argb as u32
            {
                AvifRgbFormat::Rgba
            } else {
                AvifRgbFormat::Bgra
            };
            rgb.ignore_alpha = !has_alpha;
            rgb.row_bytes = image.width * 4 * std::mem::size_of::<u16>() as u32;
            let size = image.height as usize * rgb.row_bytes as usize;
            pixels = Vec::new();
            if pixels.try_reserve_exact(size).is_err() {
                return Err(Error::new("Out of memory for RGB pixels"));
            }
            pixels.resize(size, 0u8);

            // Convert 10-bit packed format to planar format for libavif
            let src = surface.pixels().unwrap_or(&[]);
            let pitch = surface.pitch() as usize;
            let mut dst16 = 0;
            for y in 0..image.height as usize {
                for x in 0..image.width as usize {
                    let o = y * pitch + x * 4;
                    let pixelvalue =
                        u32::from_ne_bytes([src[o], src[o + 1], src[o + 2], src[o + 3]]);

                    // Extract 10-bit components
                    // Handle alpha - use full opacity for non-alpha formats
                    let alpha = if has_alpha {
                        EXPAND_ALPHA[((pixelvalue >> 30) & 0x3) as usize] // A
                    } else {
                        0x3FF // Full opacity
                    };
                    for v in [
                        ((pixelvalue >> 20) & 0x3FF) as u16, // R
                        ((pixelvalue >> 10) & 0x3FF) as u16, // G
                        (pixelvalue & 0x3FF) as u16,         // B
                        alpha,
                    ] {
                        pixels[dst16..dst16 + 2].copy_from_slice(&v.to_ne_bytes());
                        dst16 += 2;
                    }
                }
            }
            rgb.pixels = Some(&mut pixels);

            if !is_lossless {
                rc = avif_image_rgb_to_yuv(&mut image, &rgb);

                // (SDL_free(rgb.pixels): dropped with `pixels`)

                if rc != AvifResult::Ok {
                    return Err(Error::new(format!(
                        "Couldn't convert to YUV: {}",
                        avif_result_to_string(rc)
                    )));
                }
            }
        } else {
            let mut needs_conversion = true;

            let rgb_format = if format == PixelFormat::RGBA32 {
                needs_conversion = false;
                AvifRgbFormat::Rgba
            } else if format == PixelFormat::ARGB32 {
                needs_conversion = false;
                AvifRgbFormat::Argb
            } else if format == PixelFormat::BGRA32 {
                needs_conversion = false;
                AvifRgbFormat::Bgra
            } else if format == PixelFormat::ABGR32 {
                needs_conversion = false;
                AvifRgbFormat::Abgr
            } else if format == PixelFormat::RGB24 {
                // Note (upstream): upstream converts the surface to RGBX32
                // below, which this format reads as 3 bytes a pixel (the X
                // bytes, which it leaves uninitialized, among them); the
                // translation fixes it, reading the surface as it is.
                needs_conversion = false;
                AvifRgbFormat::Rgb
            } else if format == PixelFormat::BGR24 {
                // Note (upstream): as for RGB24
                needs_conversion = false;
                AvifRgbFormat::Bgr
            } else if format == PixelFormat::RGBX32
                || format == PixelFormat::XRGB32
                || format == PixelFormat::BGRX32
                || format == PixelFormat::XBGR32
            {
                AvifRgbFormat::Rgba
            } else {
                // Note (upstream): upstream reads the RGBA32 or RGBX32
                // surface converted below as ABGR on little-endian machines,
                // its bytes reversed (for RGBX32 the X bytes, which it
                // leaves uninitialized, as red); the translation fixes it,
                // reading those bytes, red, green, blue and alpha, as RGBA
                // on any machine, as for the four formats above.
                AvifRgbFormat::Rgba
            };

            let source: &Surface<'_> = if needs_conversion {
                let target_format = if has_alpha {
                    PixelFormat::RGBA32
                } else {
                    PixelFormat::RGBX32
                };
                match surface.convert(target_format) {
                    Ok(t) => temp = Some(t),
                    Err(_) => {
                        return Err(Error::new("Couldn't convert surface to compatible format"));
                    }
                }
                temp.as_ref().expect("the converted surface")
            } else {
                surface
            };
            rgb.format = rgb_format;
            pixels = source.pixels().map(|p| p.to_vec()).unwrap_or_default();
            rgb.row_bytes = source.pitch() as u32;
            rgb.pixels = Some(&mut pixels);

            rgb.depth = 8;
            rgb.ignore_alpha = !has_alpha;
            rgb.alpha_premultiplied = false;
            rgb.chroma_upsampling = AvifChromaUpsampling::Automatic;

            if !is_lossless {
                rc = avif_image_rgb_to_yuv(&mut image, &rgb);

                if rc != AvifResult::Ok {
                    return Err(Error::new(format!(
                        "Couldn't convert to YUV: {}",
                        avif_result_to_string(rc)
                    )));
                }
            }
        }

        let mut add_image_flags = AVIF_ADD_IMAGE_FLAG_NONE;
        if is_lossless || !self.first_frame_added {
            add_image_flags = AVIF_ADD_IMAGE_FLAG_FORCE_KEYFRAME;
        }

        // (the lossless 10-bit, the other lossless and the lossy branches
        // make the same call)
        let Some(encoder) = self.encoder.as_mut() else {
            return Err(Error::new("Invalid context or encoder"));
        };
        rc = avif_encoder_add_image(encoder, &image, duration_in_timescales, add_image_flags);

        // (SDL_DestroySurface(temp), SDL_UnlockSurface() and
        // avifImageDestroy(): dropped)
        drop(temp);
        drop(image);

        if rc != AvifResult::Ok {
            return Err(Error::new(format!(
                "Failed to add image to avif encoder: {}",
                avif_result_to_string(rc)
            )));
        }

        if !self.first_frame_added {
            self.first_frame_added = true;
        }

        Ok(())
    }

    /// Translation of `AnimationEncoder_End()` (the metadata, the encoder
    /// and the context are released with the context).
    pub(crate) fn end(&mut self, e: &mut EncoderCore<'_, '_>) -> Result<()> {
        self.metadata = None;
        let Some(mut encoder) = self.encoder.take() else {
            return Err(Error::new("Invalid context or encoder"));
        };

        let mut avif_output = Vec::new();
        let rc = avif_encoder_finish(&mut encoder, &mut avif_output);
        if rc != AvifResult::Ok {
            return Err(Error::new(format!(
                "Failed to finish encoder: {}",
                avif_result_to_string(rc)
            )));
        }

        let dst = e.dst();
        if dst.write(&avif_output) == avif_output.len() {
            Ok(())
        } else {
            Err(Error::new("Failed to write AVIF data to IOStream"))
        }
    }
}

/// `SDL_GetNumLogicalCPUCores()`, which the encoder's default threads come
/// from. The tests pin it to 4 (2 threads), as their reference harness's
/// `FAKE_CORES=4` does: one thread and more code the frames differently
/// (rows of superblocks coded in turn, or with libaom's row
/// multi-threading).
fn logical_cpu_cores() -> i32 {
    if cfg!(test) {
        4
    } else {
        sdl3::cpuinfo::num_logical_cpu_cores()
    }
}

/// Create the AVIF encoder of an animation encoder: libavif's encoder at
/// its fastest speed (libaom's real-time usage), the quality (75 by
/// default) for color and libavif's default for alpha, the threads and
/// key frame interval of the properties, and unless ignored the loop count
/// and the metadata. Translation of `IMG_CreateAVIFAnimationEncoder()`.
pub(crate) fn create_avif_animation_encoder(
    e: &mut EncoderCore<'_, '_>,
    props: &Properties,
) -> Result<Box<AvifEncoderContext>> {
    // (IMG_InitAVIF(): nothing to load)

    let Some(mut encoder) = avif_encoder_create() else {
        return Err(Error::new("Couldn't create AVIF encoder"));
    };

    if e.quality < 0 {
        e.quality = 75;
    } else if e.quality > 100 {
        e.quality = 100;
    }

    let available_l_cores = logical_cpu_cores();
    let mut threads = props
        .get_number(PROP_ANIMATION_ENCODER_CREATE_AVIF_MAX_THREADS_NUMBER)
        .unwrap_or((available_l_cores / 2) as i64) as i32;
    threads = if threads < 1 {
        1
    } else if threads > available_l_cores {
        available_l_cores
    } else {
        threads
    };

    let key_frame_interval = props
        .get_number(PROP_ANIMATION_ENCODER_CREATE_AVIF_KEYFRAME_INTERVAL_NUMBER)
        .unwrap_or(0) as i32;

    encoder.max_threads = threads;
    encoder.quality = e.quality;

    encoder.quality_alpha = AVIF_QUALITY_DEFAULT;
    encoder.speed = AVIF_SPEED_FASTEST;

    if encoder.quality >= 100 {
        encoder.keyframe_interval = 1.min(key_frame_interval);
        encoder.min_quantizer = 0;
        encoder.max_quantizer = 0;

        encoder.min_quantizer_alpha = 0;
        encoder.max_quantizer_alpha = 0;
        encoder.auto_tiling = false;
    } else {
        encoder.keyframe_interval = key_frame_interval;
        encoder.min_quantizer = 8;
        encoder.max_quantizer = 63;

        encoder.min_quantizer_alpha = 0;
        encoder.max_quantizer_alpha = 63;
        encoder.auto_tiling = true;
    }

    encoder.tile_rows_log2 = 0;
    encoder.tile_cols_log2 = 0;

    encoder.timescale = e.timebase_denominator as u64;

    let ignore_props = props
        .get_bool(PROP_METADATA_IGNORE_PROPS_BOOLEAN)
        .unwrap_or(false);
    let mut metadata = None;
    if !ignore_props {
        encoder.repetition_count = (props
            .get_number(PROP_METADATA_LOOP_COUNT_NUMBER)
            .unwrap_or(0)
            .max(0)
            - 1) as i32;

        let m = Properties::new();
        if m.copy_from(props).is_err() {
            return Err(Error::new(
                "Couldn't copy properties to AVIF encoder metadata",
            ));
        }
        metadata = Some(m);
    }

    Ok(Box::new(AvifEncoderContext {
        encoder: Some(encoder),
        first_frame_added: false,
        metadata,
    }))
}
