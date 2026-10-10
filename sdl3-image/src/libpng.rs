// Rust translation of src/IMG_libpng.c from SDL_image (its APNG animation
// decoder and encoder).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Animated PNG: the APNG animation decoder and encoder, over the
//! translation of the parts of libpng they use (in `libpng/`: the
//! sequential reader with the palette expansion, 16-bit stripping and
//! filler transformations, and the sequential writer with its row
//! filters), which in turn use the translation of zlib's inflate and
//! deflate (in `zlib/`), as upstream builds them with libpng linked in and
//! `SAVE_PNG` (no dynamic loading: `IMG_InitPNG()` has nothing to load).
//!
//! The decoder reads the chunks itself and hands libpng one synthetic PNG
//! per frame (its IHDR, the file's PLTE and tRNS, and the frame's data as
//! one IDAT); the encoder has libpng compress each frame into a temporary
//! PNG, takes the IDAT data out of it and writes the APNG's chunks itself.
//!
//! Still PNG images stay on the stb_image loader and the miniz saver (as
//! upstream builds them with `SDL_IMAGE_LIBPNG` for APNG only and
//! `LOAD_PNG`'s default stb backend): `IMG_LoadPNG_LIBPNG()` and
//! `IMG_SavePNG_LIBPNG()` aren't translated.

/*                    This is a PNG image file framework                        */
/********************************************************************************
 * *                                                                           **
 * Initially written entirely by Xen (@lordofxen) on 7/28/2025.                 *
 * Improvements made by: slouken, sezero, madebr, and smcv.                     *
 * *                                                                           **
 *******************************************************************************/
// The libpng translation keeps upstream's names (png_write_IHDR(),
// png_IDAT, IDAT_read_size, chunk_PLTE), declarations, loops, conditions
// and clamps as written, as does the translation of IMG_libpng.c.
#![allow(
    non_snake_case,
    non_upper_case_globals,
    clippy::blocks_in_conditions,
    clippy::collapsible_match,
    clippy::enum_variant_names,
    clippy::field_reassign_with_default,
    clippy::manual_clamp,
    clippy::manual_is_multiple_of,
    clippy::manual_range_contains,
    clippy::needless_late_init
)]

#[allow(clippy::module_inception)]
mod png;
mod pngerror;
mod pnginfo;
mod pngmem;
// (the parts of pngpriv.h's and pngstruct.h's definitions the translated
// code doesn't use are kept, as in the headers)
#[allow(dead_code)]
mod pngpriv;
mod pngread;
mod pngrio;
mod pngrtran;
#[allow(
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::needless_range_loop
)]
mod pngrutil;
mod pngset;
#[allow(dead_code)]
mod pngstruct;
mod pngtrans;
mod pngwio;
mod pngwrite;
#[allow(clippy::needless_range_loop)]
mod pngwutil;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::stdlib::crc32::crc32 as sdl_crc32;
use sdl3::stdlib::string::strcasecmp;
use sdl3::video::{share_palette, BlendMode, Palette, PixelFormat, Rect, Surface};

use crate::anim_decoder::{
    AnimationDecoderStatus, DecoderCore, PROP_METADATA_AUTHOR_STRING,
    PROP_METADATA_COPYRIGHT_STRING, PROP_METADATA_CREATION_TIME_STRING,
    PROP_METADATA_DESCRIPTION_STRING, PROP_METADATA_FRAME_COUNT_NUMBER,
    PROP_METADATA_IGNORE_PROPS_BOOLEAN, PROP_METADATA_LOOP_COUNT_NUMBER,
    PROP_METADATA_TITLE_STRING,
};
use crate::anim_encoder::{has_metadata, EncoderCore};
use crate::util::{read_error, read_ok, read_up_to};
use png::{
    png_create_info_struct, png_get_io_ptr, png_sig_cmp, PngColor, PngError, PngResult,
    PNG_COLOR_MASK_ALPHA, PNG_COLOR_TYPE_PALETTE, PNG_COLOR_TYPE_RGBA, PNG_COMPRESSION_TYPE_BASE,
    PNG_COMPRESSION_TYPE_DEFAULT, PNG_FILLER_AFTER, PNG_FILTER_TYPE_BASE, PNG_FILTER_TYPE_DEFAULT,
    PNG_INTERLACE_NONE, PNG_LIBPNG_VER_STRING,
};
use pngerror::png_error;
use pngread::{png_create_read_struct, png_read_image, png_read_info, png_read_update_info};
use pngrio::png_set_read_fn;
use pngrtran::{png_set_palette_to_rgb, png_set_strip_16};
use pngrutil::png_get_uint_32 as libpng_get_uint_32;
use pngset::{png_set_IHDR, png_set_PLTE, png_set_tRNS};
use pngstruct::PngStruct;
use pngtrans::png_set_filler;
use pngwio::png_set_write_fn;
use pngwrite::{
    png_create_write_struct, png_set_compression_level, png_set_filter, png_write_end,
    png_write_flush, png_write_image, png_write_info,
};

const PNG_DISPOSE_OP_NONE: u8 = 0;
const PNG_DISPOSE_OP_BACKGROUND: u8 = 1;
const PNG_DISPOSE_OP_PREVIOUS: u8 = 2;

const PNG_BLEND_OP_SOURCE: u8 = 0;
const PNG_BLEND_OP_OVER: u8 = 1;

const APNG_DEFAULT_DENOMINATOR: u16 = 100;

/* (SAVE_PNG: we will have the PNG saving feature by default) */

/* (Check for the older version of libpng: this is 1.6) */

/* (the lib struct and IMG_InitPNG()/IMG_QuitPNG(): libpng is linked in) */

const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

// Custom implementation of png_save_uint_32 to ensure network byte order (big-endian) writing.
fn custom_png_save_uint_32(buf: &mut [u8], i: u32) {
    buf[0] = ((i >> 24) & 0xff) as u8;
    buf[1] = ((i >> 16) & 0xff) as u8;
    buf[2] = ((i >> 8) & 0xff) as u8;
    buf[3] = (i & 0xff) as u8;
}

// Custom implementation of png_save_uint_16 to ensure network byte order (big-endian) writing.
fn custom_png_save_uint_16(buf: &mut [u8], i: u16) {
    buf[0] = ((i >> 8) & 0xff) as u8;
    buf[1] = (i & 0xff) as u8;
}

/// The read function SDL_image gives libpng (`png_read_data`).
fn png_read_data(png_ptr: &mut PngStruct<'_, '_>, area: &mut [u8]) -> PngResult<()> {
    let read = match png_get_io_ptr(png_ptr) {
        Some(src) => src.read(area),
        None => 0,
    };
    if read != area.len() {
        return Err(png_error(
            png_ptr,
            "Failed to read all expected data from SDL_IOStream for PNG image.",
        ));
    }
    Ok(())
}

/// The write function SDL_image gives libpng (`png_write_data`).
fn png_write_data(png_ptr: &mut PngStruct<'_, '_>, src: &[u8]) -> PngResult<()> {
    let written = match png_get_io_ptr(png_ptr) {
        Some(dst) => dst.write(src),
        None => 0,
    };
    if written != src.len() {
        return Err(png_error(
            png_ptr,
            "Failed to write all expected data to SDL_IOStream for PNG image.",
        ));
    }
    Ok(())
}

/// The flush function SDL_image gives libpng (`png_flush_data`).
fn png_flush_data(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    // FIXME (upstream): png_write_flush() flushes through this function,
    // which would call png_write_flush() again, without end; libpng never
    // flushes here, though (SDL_image sets no flush interval, and libpng
    // isn't built to flush after IEND).
    png_write_flush(png_ptr)
}

/* (struct png_load_vars, LIBPNG_LoadPNG_IO_Internal(), IMG_LoadPNG_LIBPNG(),
 * struct png_save_vars, LIBPNG_SavePNG_IO_Internal() and
 * IMG_SavePNG_LIBPNG(): not translated, as still PNG images are loaded and
 * saved by the stb_image and miniz backends) */

/// Translation of `apng_acTL_chunk`.
#[derive(Clone, Copy, Debug, Default)]
struct ApngAcTLChunk {
    num_frames: u32,
    num_plays: u32,
}

/// Translation of `apng_fcTL_chunk`.
#[derive(Debug, Default)]
struct ApngFcTLChunk {
    sequence_number: u32,
    width: u32,
    height: u32,
    x_offset: u32,
    y_offset: u32,
    delay_num: u16,
    delay_den: u16,
    dispose_op: u8,
    blend_op: u8,
    raw_idat_data: Vec<u8>, /* (raw_idat_size is its length) */
}

/* (apng_read_context: unused) */

/// A whole chunk as read_png_chunk() reads it: its header (length and
/// type), then its data and CRC.
#[derive(Debug)]
struct RawChunk {
    header: [u8; 8],
    rest: Vec<u8>,
}

impl RawChunk {
    /// The chunk's size (`chunk_size`).
    fn size(&self) -> usize {
        self.header.len() + self.rest.len()
    }
}

/// The error of a libpng read (a png_error()'s longjmp() to the setjmp()).
fn png_read_failed(_: PngError) -> Error {
    Error::new("Error during PNG read")
}

/// Translation of `decompress_png_frame_data()` (its `DecompressionContext`
/// is the locals, released as they are dropped).
#[allow(clippy::too_many_arguments)]
fn decompress_png_frame_data(
    compressed_data: &[u8],
    width: i32,
    height: i32,
    png_color_type: i32,
    bit_depth: i32,
    chunk_PLTE: Option<&RawChunk>,
    chunk_tRNS: Option<&RawChunk>,
) -> Result<Surface<'static>> {
    /*
     * Usually you'd directly decompress zlib but then we have to do defiltering and deinterlacing ourselves.
     * We can decompress zlib then pass those jobs to libpng but then libpng expects a fully compressed data,
     * therefore, we manually add chunks for header and other parts only for this given compressed data,
     * tricking libpng to believe this is a normal PNG file, then make it defilter (if any) and deinterlace
     * (if any) for us.
     */
    // FIXME (upstream): the synthetic IHDR always says PNG_INTERLACE_NONE,
    // so an interlaced APNG's frames are read as if they weren't
    // interlaced (garbled, or failing to decode).

    // Create a memory stream to hold our synthetic PNG
    // (a buffer here: the size is known, and SDL_IOFromDynamicMem()'s
    // contents are read back into a buffer below anyway)
    let data_size = 8
        + 8
        + 13
        + 4
        + chunk_PLTE.map_or(0, RawChunk::size)
        + chunk_tRNS.map_or(0, RawChunk::size)
        + 8
        + compressed_data.len()
        + 4
        + 12;
    if data_size >= i32::MAX as usize {
        return Err(Error::new("data size >= INT32_MAX"));
    }
    let mut buffer: Vec<u8> = Vec::new();
    if buffer.try_reserve_exact(data_size).is_err() {
        return Err(Error::out_of_memory());
    }

    // Write PNG signature
    buffer.extend_from_slice(&PNG_SIG);

    // Write IHDR chunk
    {
        let mut ihdr_data = [0u8; 13];
        let ihdr_header: [u8; 8] = [0, 0, 0, 13, b'I', b'H', b'D', b'R'];

        // Write IHDR length and type
        buffer.extend_from_slice(&ihdr_header);

        // Write IHDR data
        custom_png_save_uint_32(&mut ihdr_data, width as u32);
        custom_png_save_uint_32(&mut ihdr_data[4..], height as u32);
        ihdr_data[8] = bit_depth as u8;
        ihdr_data[9] = png_color_type as u8;
        ihdr_data[10] = PNG_INTERLACE_NONE as u8;
        ihdr_data[11] = PNG_COMPRESSION_TYPE_DEFAULT as u8;
        ihdr_data[12] = PNG_FILTER_TYPE_DEFAULT as u8;

        buffer.extend_from_slice(&ihdr_data);

        // Calculate and write IHDR CRC
        let mut crc = sdl_crc32(0, b"IHDR");
        crc = sdl_crc32(crc, &ihdr_data);
        let mut crc_bytes = [0u8; 4];
        custom_png_save_uint_32(&mut crc_bytes, crc);
        buffer.extend_from_slice(&crc_bytes);
    }

    // Write PLTE chunk
    if let Some(chunk) = chunk_PLTE {
        buffer.extend_from_slice(&chunk.header);
        buffer.extend_from_slice(&chunk.rest);
    }

    // Write tRNS chunk
    if let Some(chunk) = chunk_tRNS {
        buffer.extend_from_slice(&chunk.header);
        buffer.extend_from_slice(&chunk.rest);
    }

    // Write IDAT chunk
    {
        let mut idat_header: [u8; 8] = [0, 0, 0, 0, b'I', b'D', b'A', b'T'];
        custom_png_save_uint_32(&mut idat_header, compressed_data.len() as u32);

        // Write IDAT length and type
        buffer.extend_from_slice(&idat_header);

        // Write compressed data
        buffer.extend_from_slice(compressed_data);

        // Calculate and write IDAT CRC
        let mut crc = sdl_crc32(0, b"IDAT");
        crc = sdl_crc32(crc, compressed_data);
        let mut crc_bytes = [0u8; 4];
        custom_png_save_uint_32(&mut crc_bytes, crc);
        buffer.extend_from_slice(&crc_bytes);
    }

    // Write IEND chunk
    {
        let iend_chunk: [u8; 12] = [
            0, 0, 0, 0, // Length (0)
            b'I', b'E', b'N', b'D', // Type
            0xAE, 0x42, 0x60, 0x82, // CRC (precomputed for empty IEND)
        ];

        buffer.extend_from_slice(&iend_chunk);
    }

    let mut read_stream = IoStream::from_const_mem(&buffer);

    // Now we have a proper PNG file in memory, use libpng to read it
    let Some(mut png_ptr) = png_create_read_struct(PNG_LIBPNG_VER_STRING) else {
        return Err(Error::out_of_memory());
    };

    let Some(mut info_ptr) = png_create_info_struct(&png_ptr) else {
        return Err(Error::out_of_memory());
    };

    // (the setjmp(): every libpng call's error is png_read_failed())

    png_set_read_fn(&mut png_ptr, Some(&mut read_stream), Some(png_read_data));
    png_read_info(&mut png_ptr, &mut info_ptr).map_err(png_read_failed)?;

    if png_color_type == PNG_COLOR_TYPE_PALETTE as i32 {
        png_set_palette_to_rgb(&mut png_ptr).map_err(png_read_failed)?;
    }

    if bit_depth == 16 {
        png_set_strip_16(&mut png_ptr).map_err(png_read_failed)?;
    }

    // FIXME (upstream): a gray (or gray and alpha) frame is read as gray and
    // alpha, 2 bytes a pixel, into the RGBA surface below (and the filler
    // of a gray frame under 8 bits fails with libpng's "internal row size
    // calculation error" for most widths).
    if (png_color_type & PNG_COLOR_MASK_ALPHA as i32) == 0 {
        png_set_filler(&mut png_ptr, 0xFF, PNG_FILLER_AFTER).map_err(png_read_failed)?;
    }

    png_read_update_info(&mut png_ptr, &mut info_ptr).map_err(png_read_failed)?;

    let mut surface = Surface::new(width, height, PixelFormat::RGBA32)?;

    let pitch = surface.pitch() as usize;
    let pixels = surface.pixels_mut().unwrap_or_default();
    let mut row_pointers: Vec<&mut [u8]> = Vec::new();
    if row_pointers.try_reserve_exact(height as usize).is_err() {
        return Err(Error::out_of_memory());
    }
    if pitch > 0 {
        row_pointers.extend(pixels.chunks_mut(pitch).take(height as usize));
    }

    png_read_image(&mut png_ptr, &mut row_pointers).map_err(png_read_failed)?;

    drop(row_pointers);
    // (png_destroy_read_struct(), and closing the streams: dropping them)

    Ok(surface)
}

/// Translation of `read_png_chunk()`: the chunk and its type and data
/// length (the data is `chunk.rest[..data_length]`, the CRC after it).
fn read_png_chunk(stream: &mut IoStream<'_>) -> Result<(RawChunk, [u8; 4], u32)> {
    let mut header = [0u8; 8];

    // Read chunk header (8 bytes)
    if !read_ok(stream, &mut header) {
        return Err(read_error(stream));
    }

    // Get data length (4 bytes, big-endian)
    let data_length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);

    // Get chunk type (4 bytes)
    let chunk_type = [header[4], header[5], header[6], header[7]];

    // Allocate memory for chunk
    if data_length > (u32::MAX - (header.len() as u32 + 4)) {
        return Err(Error::new("Corrupt PNG"));
    }
    // Read chunk data
    // Read CRC (4 bytes, big-endian)
    // (both into one buffer, which grows as the data arrives)
    let want = data_length as usize + 4;
    let Some(rest) = read_up_to(stream, want) else {
        return Err(Error::out_of_memory());
    };
    if rest.len() != want {
        return Err(read_error(stream));
    }

    Ok((RawChunk { header, rest }, chunk_type, data_length))
}

/// The APNG animation decoder's state. Translation of IMG_libpng.c's
/// `struct IMG_AnimationDecoderContext` (`IMG_AnimationDecoderClose_Internal()`
/// is dropping it).
pub(crate) struct ApngDecoderContext {
    actl: ApngAcTLChunk,
    fctl_frames: Vec<ApngFcTLChunk>, /* (fctl_count and fctl_capacity are its length and capacity) */
    is_apng: bool,
    canvas: Option<Surface<'static>>,
    prev_canvas_copy: Option<Surface<'static>>,
    current_frame_index: i32,

    width: i32,
    height: i32,
    bit_depth: i32,
    png_color_type: i32,

    palette: Option<Palette>,
    chunk_PLTE: Option<RawChunk>,
    chunk_tRNS: Option<RawChunk>,
}

impl ApngDecoderContext {
    /// Translation of `IMG_AnimationDecoderReset_Internal()`.
    pub(crate) fn reset(&mut self, d: &mut DecoderCore<'_, '_>) -> Result<()> {
        self.current_frame_index = 0;
        let start = d.start;
        if d.src().seek(start, IoWhence::Set).is_err() {
            return Err(Error::new("Failed to seek to beginning of APNG animation"));
        }

        if let Some(canvas) = self.canvas.as_mut() {
            let _ = canvas.fill_rect(None, 0x00000000);
        }

        if let Some(prev_canvas_copy) = self.prev_canvas_copy.as_mut() {
            let _ = prev_canvas_copy.fill_rect(None, 0x00000000);
        }

        Ok(())
    }

    /// Translation of `IMG_AnimationDecoderGetNextFrame_Internal()`:
    /// `Ok(None)` (with the COMPLETE status) when there are no more frames.
    pub(crate) fn get_next_frame(
        &mut self,
        d: &mut DecoderCore<'_, '_>,
    ) -> Result<Option<(Surface<'static>, u64)>> {
        if !self.is_apng {
            // (never: the decoder is created for APNGs only)
            return Err(Error::new(
                "Not an APNG file or not enough frame control chunks found",
            ));
        }

        if self
            .actl
            .num_frames
            .wrapping_sub(self.current_frame_index as u32)
            < 1
        {
            d.status = AnimationDecoderStatus::Complete;
            return Ok(None);
        }

        if self.canvas.is_none() {
            let Ok(canvas) = Surface::new(self.width, self.height, PixelFormat::RGBA32) else {
                return Err(Error::new("Failed to create APNG canvas"));
            };
            let canvas = self.canvas.insert(canvas);
            if canvas.set_blend_mode(BlendMode::BLEND).is_err() {
                return Err(Error::new("Failed to set APNG canvas blend mode"));
            }
            if canvas.fill_rect(None, 0x00000000).is_err() {
                return Err(Error::new("Failed to fill APNG canvas"));
            }
        }

        if self.prev_canvas_copy.is_none() {
            let Ok(prev_canvas_copy) = Surface::new(self.width, self.height, PixelFormat::RGBA32)
            else {
                return Err(Error::new("Failed to create previous canvas copy"));
            };
            let prev_canvas_copy = self.prev_canvas_copy.insert(prev_canvas_copy);
            if prev_canvas_copy.fill_rect(None, 0x00000000).is_err() {
                return Err(Error::new("Failed to fill previous canvas copy"));
            }
        }

        let index = self.current_frame_index as usize;
        let canvas = self.canvas.as_mut().expect("the canvas");
        let prev_canvas_copy = self.prev_canvas_copy.as_mut().expect("the canvas copy");

        let fctl = &self.fctl_frames[index];
        // FIXME (upstream): a zero delay_den divides by zero in
        // IMG_TimebaseDuration() (a crash); the APNG specification's
        // denominator for it, 100 (APNG_DEFAULT_DENOMINATOR, which upstream
        // defines but doesn't use), is used here.
        let delay_den = if fctl.delay_den == 0 {
            APNG_DEFAULT_DENOMINATOR
        } else {
            fctl.delay_den
        };
        let duration = d.decoder_duration(fctl.delay_num as u64, delay_den as u64);

        if index > 0 {
            let prev_fctl = &self.fctl_frames[index - 1];
            let prev_frame_rect = Rect::new(
                prev_fctl.x_offset as i32,
                prev_fctl.y_offset as i32,
                prev_fctl.width as i32,
                prev_fctl.height as i32,
            );

            match prev_fctl.dispose_op {
                PNG_DISPOSE_OP_NONE => {
                    // Do nothing
                }
                PNG_DISPOSE_OP_BACKGROUND => {
                    if canvas
                        .fill_rect(Some(&prev_frame_rect), 0x00000000)
                        .is_err()
                    {
                        return Err(Error::new(
                            "Failed to fill canvas for background dispose operation",
                        ));
                    }
                }
                PNG_DISPOSE_OP_PREVIOUS => {
                    // FIXME (upstream): the copy has the blend mode of a new
                    // RGBA surface, BLEND, so it is blended over the canvas
                    // rather than copied (and the same below, when the canvas
                    // is saved into it).
                    if prev_canvas_copy.blit(None, canvas, None).is_err() {
                        return Err(Error::new(
                            "Failed to restore previous canvas copy for dispose operation",
                        ));
                    }
                }
                _ => {}
            }
        }

        if fctl.dispose_op == PNG_DISPOSE_OP_PREVIOUS
            && canvas.blit(None, prev_canvas_copy, None).is_err()
        {
            return Err(Error::new(
                "Failed to copy current canvas to previous canvas copy",
            ));
        }

        // (DecompressionContext: the function's locals)
        let temp_frame = decompress_png_frame_data(
            &fctl.raw_idat_data,
            fctl.width as i32,
            fctl.height as i32,
            self.png_color_type,
            self.bit_depth,
            self.chunk_PLTE.as_ref(),
            self.chunk_tRNS.as_ref(),
        );

        let mut temp_frame = match temp_frame {
            Ok(temp_frame) => temp_frame,
            Err(e) => {
                return Err(Error::new(format!(
                    "Failed to decompress PNG frame data: {e}"
                )))
            }
        };

        if temp_frame.format() == PixelFormat::INDEX8 {
            // (never: the frames are decoded as RGBA32)
            if let Some(palette) = &self.palette {
                let _ = temp_frame.set_palette(Some(share_palette(palette.clone())));
            }
        }

        match fctl.blend_op {
            PNG_BLEND_OP_SOURCE => {
                if let Err(e) = temp_frame.set_blend_mode(BlendMode::NONE) {
                    return Err(Error::new(format!(
                        "Failed to set blend mode for frame: {e}"
                    )));
                }
            }
            PNG_BLEND_OP_OVER => {
                if let Err(e) = temp_frame.set_blend_mode(BlendMode::BLEND) {
                    return Err(Error::new(format!(
                        "Failed to set blend mode for frame: {e}"
                    )));
                }
            }
            _ => {}
        }

        let dest_rect = Rect::new(
            fctl.x_offset as i32,
            fctl.y_offset as i32,
            fctl.width as i32,
            fctl.height as i32,
        );

        if let Err(e) = temp_frame.blit(None, canvas, Some(&dest_rect)) {
            return Err(Error::new(format!("Failed to blit frame onto canvas: {e}")));
        }
        drop(temp_frame);

        let retval = canvas.duplicate()?;

        self.current_frame_index += 1;

        Ok(Some((retval, duration)))
    }
}

/// Create the APNG decoder of an animation decoder: every chunk read (the
/// frames' data kept), and unless [`PROP_METADATA_IGNORE_PROPS_BOOLEAN`] is
/// set, the frame and loop counts and the tEXt metadata in the decoder's
/// properties. Translation of `IMG_CreateAPNGAnimationDecoder()`.
pub(crate) fn create_apng_animation_decoder(
    d: &mut DecoderCore<'_, '_>,
    props: &Properties,
) -> Result<Box<ApngDecoderContext>> {
    // (IMG_InitPNG(): libpng is linked in)

    let mut ctx = Box::new(ApngDecoderContext {
        actl: ApngAcTLChunk::default(),
        fctl_frames: Vec::new(),
        is_apng: false,
        canvas: None,
        prev_canvas_copy: None,
        current_frame_index: 0,
        width: 0,
        height: 0,
        bit_depth: 0,
        png_color_type: 0,
        palette: None,
        chunk_PLTE: None,
        chunk_tRNS: None,
    });

    let src = d.src();
    let mut header = [0u8; 8];
    if !read_ok(src, &mut header) {
        return Err(Error::new("Failed to read PNG header"));
    }

    if png_sig_cmp(&header, 0, 8) != 0 {
        return Err(Error::new("Not a valid PNG file signature"));
    }

    // Extracted metadata will be assigned to variables below
    let mut desc: Option<String> = None;
    let mut rights: Option<String> = None;
    let mut title: Option<String> = None;
    let mut author: Option<String> = None;
    let mut creationtime: Option<String> = None;

    let mut found_iend = false;
    while !found_iend {
        let (chunk, chunk_type, chunk_length) = read_png_chunk(src)?;
        let mut chunk = Some(chunk);
        let chunk_data = &chunk.as_ref().expect("the chunk").rest[..chunk_length as usize];

        if chunk_length > i32::MAX as u32 {
            return Err(Error::new("APNG chunk too large to process"));
        }

        let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        let be16 = |b: &[u8]| u16::from_be_bytes([b[0], b[1]]);

        if &chunk_type == b"IHDR" {
            if chunk_length != 13 {
                return Err(Error::new("Invalid IHDR chunk size"));
            }

            // Extract image dimensions from IHDR
            ctx.width = be32(chunk_data) as i32;
            ctx.height = be32(&chunk_data[4..]) as i32;
            ctx.bit_depth = chunk_data[8] as i32;
            ctx.png_color_type = chunk_data[9] as i32;
        } else if &chunk_type == b"acTL" {
            if chunk_length != 8 {
                return Err(Error::new("Invalid acTL chunk size"));
            }

            ctx.is_apng = true;
            ctx.actl.num_frames = be32(chunk_data);
            ctx.actl.num_plays = be32(&chunk_data[4..]);
        } else if &chunk_type == b"PLTE" {
            let num_entries = chunk_length as i32 / 3;
            if num_entries > 0 && num_entries <= 256 {
                let mut palette = Palette::new(num_entries as usize)?;

                for (i, color) in palette.colors_mut().iter_mut().enumerate() {
                    color.r = chunk_data[i * 3];
                    color.g = chunk_data[i * 3 + 1];
                    color.b = chunk_data[i * 3 + 2];
                    color.a = 255; /* SDL_ALPHA_OPAQUE */
                }
                ctx.palette = Some(palette);
            }

            ctx.chunk_PLTE = chunk.take();
        } else if &chunk_type == b"tRNS" {
            if let Some(palette) = ctx.palette.as_mut() {
                let num_trans = (chunk_length as i32).min(palette.len() as i32);
                for (i, color) in palette
                    .colors_mut()
                    .iter_mut()
                    .take(num_trans as usize)
                    .enumerate()
                {
                    color.a = chunk_data[i];
                }
            }

            ctx.chunk_tRNS = chunk.take();
        } else if &chunk_type == b"fcTL" {
            if chunk_length != 26 {
                return Err(Error::new("Invalid fcTL chunk size"));
            }

            if ctx.fctl_frames.try_reserve(1).is_err() {
                return Err(Error::new("Out of memory for fcTL chunks"));
            }

            ctx.fctl_frames.push(ApngFcTLChunk {
                sequence_number: be32(chunk_data),
                width: be32(&chunk_data[4..]),
                height: be32(&chunk_data[8..]),
                x_offset: be32(&chunk_data[12..]),
                y_offset: be32(&chunk_data[16..]),
                delay_num: be16(&chunk_data[20..]),
                delay_den: be16(&chunk_data[22..]),
                dispose_op: chunk_data[24],
                blend_op: chunk_data[25],
                raw_idat_data: Vec::new(),
            });
        } else if &chunk_type == b"IDAT" {
            // Find fcTL with sequence number 0 (which corresponds to IDAT data)
            let matching_fctl_index = ctx
                .fctl_frames
                .iter()
                .position(|fctl| fctl.sequence_number == 0);

            if let Some(matching_fctl_index) = matching_fctl_index {
                let fctl = &mut ctx.fctl_frames[matching_fctl_index];

                // Allocate or reallocate buffer
                if fctl
                    .raw_idat_data
                    .len()
                    .checked_add(chunk_data.len())
                    .is_none()
                {
                    return Err(Error::new("IDAT size would overflow"));
                }

                if fctl.raw_idat_data.try_reserve(chunk_data.len()).is_err() {
                    return Err(Error::out_of_memory());
                }

                fctl.raw_idat_data.extend_from_slice(chunk_data);
            }
        } else if &chunk_type == b"fdAT" {
            if chunk_length < 4 {
                return Err(Error::new("Invalid fdAT chunk size"));
            }

            let sequence_number = be32(chunk_data);

            // Find matching fcTL by sequence number (fdAT sequence - 1 matches fcTL sequence)
            let matching_fctl_index = ctx
                .fctl_frames
                .iter()
                .position(|fctl| fctl.sequence_number == sequence_number.wrapping_sub(1));

            if let Some(matching_fctl_index) = matching_fctl_index {
                let fctl = &mut ctx.fctl_frames[matching_fctl_index];
                let data = &chunk_data[4..];

                if fctl.raw_idat_data.len().checked_add(data.len()).is_none() {
                    return Err(Error::new("fdAT size would overflow"));
                }

                if fctl.raw_idat_data.try_reserve(data.len()).is_err() {
                    return Err(Error::out_of_memory());
                }

                fctl.raw_idat_data.extend_from_slice(data);
            }
        } else if &chunk_type == b"IEND" {
            found_iend = true;
        } else if &chunk_type == b"tEXt" {
            if let Some(separator) = chunk_data.iter().position(|&b| b == 0) {
                let keyword_len = separator;
                let text_len = chunk_length as usize - keyword_len - 1;

                let keyword = &chunk_data[..keyword_len];
                let text = &chunk_data[separator + 1..separator + 1 + text_len];
                // (SDL_strndup(): the text ends at a NUL, if it has one)
                let text = &text[..text.iter().position(|&b| b == 0).unwrap_or(text.len())];
                let text = Some(String::from_utf8_lossy(text).into_owned());
                if strcasecmp(keyword, "description").is_eq() {
                    desc = text;
                } else if strcasecmp(keyword, "copyright").is_eq() {
                    rights = text;
                } else if strcasecmp(keyword, "title").is_eq() {
                    title = text;
                } else if strcasecmp(keyword, "author").is_eq() {
                    author = text;
                } else if strcasecmp(keyword, "creation time").is_eq() {
                    creationtime = text;
                }
            }
        }

        // (a chunk that isn't saved is freed as it is dropped)
        drop(chunk);
    }

    if !ctx.is_apng
        || ctx.fctl_frames.is_empty()
        || ctx.actl.num_frames > ctx.fctl_frames.len() as u32
    {
        return Err(Error::new(
            "Not an APNG file or not enough frame control chunks found",
        ));
    }

    // Validate what might be missing or wrong
    if ctx.bit_depth < 1 || ctx.png_color_type < 0 || ctx.width < 1 || ctx.height < 1 {
        return Err(Error::new("Received invalid APNG with either corrupt or unspecified bit depth, color type, width or height"));
    }

    let ignore_props = props
        .get_bool(PROP_METADATA_IGNORE_PROPS_BOOLEAN)
        .unwrap_or(false);
    if !ignore_props {
        // Allow implicit properties to be set which are not globalized but specific to the decoder.
        let _ = d
            .props
            .set(PROP_METADATA_FRAME_COUNT_NUMBER, ctx.actl.num_frames as i64);

        // Set well-defined properties.
        let _ = d
            .props
            .set(PROP_METADATA_LOOP_COUNT_NUMBER, ctx.actl.num_plays as i64);

        // Get other well-defined properties and set them in our props.
        if let Some(desc) = desc {
            let _ = d.props.set(PROP_METADATA_DESCRIPTION_STRING, desc);
        }
        if let Some(rights) = rights {
            let _ = d.props.set(PROP_METADATA_COPYRIGHT_STRING, rights);
        }
        if let Some(title) = title {
            let _ = d.props.set(PROP_METADATA_TITLE_STRING, title);
        }
        if let Some(author) = author {
            let _ = d.props.set(PROP_METADATA_AUTHOR_STRING, author);
        }
        if let Some(creationtime) = creationtime {
            let _ = d
                .props
                .set(PROP_METADATA_CREATION_TIME_STRING, creationtime);
        }
    }

    Ok(ctx)
}

/// The APNG animation encoder's state. Translation of IMG_libpng.c's
/// `struct IMG_AnimationEncoderContext` (`SaveAPNGAnimationEnd()`'s
/// cleanup is dropping it).
pub(crate) struct ApngEncoderContext {
    // (libpng never writes through png_write_ptr: the encoder writes its
    // chunks itself; the write struct and its info struct are kept as
    // upstream keeps them, without the stream)
    png_write_ptr: Option<Box<PngStruct<'static, 'static>>>,
    info_write_ptr: Option<Box<pnginfo::PngInfo>>,
    acTL_chunk_start_pos: i64,
    current_frame_index: i32, // This also serves as the sequence number for APNG chunks
    apng_width: i32,
    apng_height: i32,
    compression_level: i32,
    output_pixel_format: PixelFormat,
    apng_palette_ptr: Option<Palette>,
    metadata: Option<Properties>,
}

/// Translation of `write_png_chunk()`.
fn write_png_chunk(stream: &mut IoStream<'_>, chunk_type_str: &str, data: &[u8]) -> Result<()> {
    let mut crc_data = [0u8; 4];
    let mut size_bytes = [0u8; 4];
    let mut chunk_type = [0u8; 4];

    chunk_type.copy_from_slice(&chunk_type_str.as_bytes()[..4]);

    // Write chunk length
    custom_png_save_uint_32(&mut size_bytes, data.len() as u32);
    if stream.write(&size_bytes) != 4 {
        return Err(Error::new(format!(
            "Failed to write chunk size for {chunk_type_str} chunk"
        )));
    }

    // Write chunk type
    if stream.write(&chunk_type) != 4 {
        return Err(Error::new(format!(
            "Failed to write chunk type for {chunk_type_str} chunk"
        )));
    }

    // Write chunk data (if any)
    if !data.is_empty() && stream.write(data) != data.len() {
        return Err(Error::new(format!(
            "Failed to write chunk data for {chunk_type_str} chunk"
        )));
    }

    // Calculate and write CRC
    let mut crc = sdl_crc32(0, &[]);
    crc = sdl_crc32(crc, &chunk_type);
    if !data.is_empty() {
        crc = sdl_crc32(crc, data);
    }
    custom_png_save_uint_32(&mut crc_data, crc);
    if stream.write(&crc_data) != 4 {
        return Err(Error::new(format!(
            "Failed to write chunk CRC for {chunk_type_str} chunk"
        )));
    }

    Ok(())
}

/// The error of a libpng write into the temporary PNG.
fn png_write_failed(_: PngError) -> Error {
    Error::new("Error during temporary PNG write operation for compression")
}

/// Translation of `compress_surface_to_png_data()`: the IDAT data (a whole
/// zlib stream) of the surface written as a PNG at the compression level
/// (its `CompressionContext` is the locals, released as they are
/// dropped).
fn compress_surface_to_png_data(
    surface: &Surface<'_>,
    compression_level: i32,
    png_color_type: u8,
) -> Result<Vec<u8>> {
    // Create a growable memory buffer for the temporary PNG
    let mut mem_stream = IoStream::from_dynamic_mem();

    let Some(mut temp_png_ptr) = png_create_write_struct(PNG_LIBPNG_VER_STRING) else {
        return Err(Error::new(
            "Couldn't allocate memory for temporary PNG write struct",
        ));
    };
    let Some(mut temp_info_ptr) = png_create_info_struct(&temp_png_ptr) else {
        return Err(Error::new(
            "Couldn't create temporary image information for PNG file",
        ));
    };

    // (the setjmp(): every libpng call's error is png_write_failed())

    // png_io_context temp_io_context = { mem_stream };
    png_set_write_fn(
        &mut temp_png_ptr,
        Some(&mut mem_stream),
        Some(png_write_data),
        Some(png_flush_data),
    );
    png_set_compression_level(&mut temp_png_ptr, compression_level);
    png_set_filter(&mut temp_png_ptr, 0, PNG_FILTER_TYPE_DEFAULT).map_err(png_write_failed)?;
    png_set_IHDR(
        &temp_png_ptr,
        &mut temp_info_ptr,
        surface.width() as u32,
        surface.height() as u32,
        8,
        png_color_type as i32, // Use specified color type
        PNG_INTERLACE_NONE,
        PNG_COMPRESSION_TYPE_DEFAULT,
        PNG_FILTER_TYPE_DEFAULT,
    )
    .map_err(png_write_failed)?;

    if png_color_type == PNG_COLOR_TYPE_PALETTE {
        let surface_palette = surface.palette();
        if surface.format() != PixelFormat::INDEX8 || surface_palette.is_none() {
            return Err(Error::new("compress_surface_to_png_data: Expected SDL_PIXELFORMAT_INDEX8 surface with palette for paletted PNG type."));
        }
        let colors: Vec<_> = match surface_palette {
            Some(p) => p
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .colors()
                .to_vec(),
            None => Vec::new(),
        };
        if !colors.is_empty() {
            // FIXME (upstream): the palette's SDL_Colors (red, green, blue
            // and alpha) are passed as png_colors (red, green and blue), so
            // libpng takes every 3 bytes of them as a color.
            let bytes: Vec<u8> = colors.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
            let png_colors: Vec<PngColor> = bytes
                .chunks_exact(3)
                .take(colors.len())
                .map(|c| PngColor {
                    red: c[0],
                    green: c[1],
                    blue: c[2],
                })
                .collect();
            png_set_PLTE(
                &mut temp_png_ptr,
                &mut temp_info_ptr,
                &png_colors,
                colors.len() as i32,
            )
            .map_err(png_write_failed)?;
        } else {
            return Err(Error::new(
                "Surface has no palette for paletted PNG compression.",
            ));
        }
        let tRNS_data: [u8; 1] = [0x00];
        png_set_tRNS(
            &mut temp_png_ptr,
            &mut temp_info_ptr,
            Some(&tRNS_data),
            1,
            None,
        )
        .map_err(png_write_failed)?;
    } else if png_color_type == PNG_COLOR_TYPE_RGBA {
        if surface.format() != PixelFormat::RGBA32 {
            return Err(Error::new("compress_surface_to_png_data: Expected SDL_PIXELFORMAT_RGBA32 surface for RGBA PNG type."));
        }
    } else {
        return Err(Error::new("Unsupported PNG color type for compression."));
    }

    png_write_info(&mut temp_png_ptr, &temp_info_ptr).map_err(png_write_failed)?;
    let pitch = surface.pitch() as usize;
    let pixels = surface.pixels().unwrap_or_default();
    let h = surface.height() as usize;
    let mut row_pointers: Vec<&[u8]> = Vec::new();
    if row_pointers.try_reserve_exact(h).is_err() {
        return Err(Error::new("Out of memory for temporary row pointers"));
    }
    for y in 0..h {
        row_pointers.push(&pixels[y * pitch..((y + 1) * pitch).min(pixels.len())]);
    }

    png_write_image(&mut temp_png_ptr, &row_pointers).map_err(png_write_failed)?;
    png_write_end(&mut temp_png_ptr, Some(&mut temp_info_ptr)).map_err(png_write_failed)?;

    drop(row_pointers);
    // (png_destroy_write_struct(): here, as the struct borrows the stream)
    drop(temp_png_ptr);

    let mem_buffer_size = match mem_stream.tell() {
        Ok(size) => size,
        Err(e) => {
            return Err(Error::new(format!(
                "Failed to get size of memory stream: {e}"
            )))
        }
    };

    // Sanity check
    if mem_buffer_size < (8 + 12 + 13 + 12 + 12) {
        // PNG_SIG + IHDR_CHUNK + IDAT_CHUNK + IEND_CHUNK
        return Err(Error::new(
            "Temporary PNG stream too small, likely corrupted during internal write.",
        ));
    }

    // (the content of the temporary PNG: the dynamic stream's memory, read
    // in place)
    let mem_buffer = mem_stream.dynamic_memory().unwrap_or_default();
    let mem_buffer = &mem_buffer[..(mem_buffer_size as usize).min(mem_buffer.len())];
    let mem_buffer_size = mem_buffer.len();

    let mut current_pos: usize = 8; // Skip PNG signature (8 bytes)
    let mut full_zlib_data_buffer: Vec<u8> = Vec::new();
    let mut iend_found = false;

    // Parse the temporary PNG in memory to extract only the IDAT chunk data (which is a full zlib stream)
    while current_pos < mem_buffer_size {
        if current_pos + 8 > mem_buffer_size {
            break;
        }
        let chunk_header = &mem_buffer[current_pos..current_pos + 8];
        let chunk_len = libpng_get_uint_32(chunk_header) as usize;
        let chunk_type = &chunk_header[4..8];

        current_pos += 8;

        if chunk_type == b"IDAT" {
            let Some(data) = mem_buffer.get(current_pos..current_pos + chunk_len) else {
                // (never: libpng wrote the chunk whole)
                break;
            };
            if full_zlib_data_buffer.try_reserve(chunk_len).is_err() {
                return Err(Error::new(
                    "Out of memory for IDAT data aggregation (full zlib stream)",
                ));
            }
            full_zlib_data_buffer.extend_from_slice(data);
        } else if chunk_type == b"IEND" {
            iend_found = true;
            break;
        }

        current_pos += chunk_len + 4;
    }

    if full_zlib_data_buffer.is_empty() {
        return Err(Error::new(
            "Could not find IDAT chunk in temporary PNG for compression",
        ));
    }
    if !iend_found {
        return Err(Error::new(
            "IEND chunk not found in temporary PNG, likely incomplete write.",
        ));
    }

    // (closing the memory stream: dropping it)

    Ok(full_zlib_data_buffer)
}

/// Translation of `writetEXtchunk()`.
fn writetEXtchunk(dst: &mut IoStream<'_>, keyword: &str, value: &str) -> Result<()> {
    // (the value is a C string: it ends at a NUL, if it has one)
    let value = value.as_bytes();
    let value = &value[..value.iter().position(|&b| b == 0).unwrap_or(value.len())];
    let total_len = keyword.len() + 1 + value.len() + 1;
    let mut buffer: Vec<u8> = Vec::new();
    if buffer.try_reserve_exact(total_len).is_err() {
        return Err(Error::new("Out of memory for tEXt chunk"));
    }
    // (SDL_snprintf("%s%c%s", keyword, '\0', value), and its terminator)
    buffer.extend_from_slice(keyword.as_bytes());
    buffer.push(0);
    buffer.extend_from_slice(value);
    buffer.push(0);
    if write_png_chunk(dst, "tEXt", &buffer).is_err() {
        return Err(Error::new("Failed to write png chunk tEXt"));
    }
    Ok(())
}

impl ApngEncoderContext {
    /// Translation of `SaveAPNGAnimationPushFrame()`.
    pub(crate) fn add_frame(
        &mut self,
        e: &mut EncoderCore<'_, '_>,
        frame: &mut Surface<'_>,
        duration: u64,
    ) -> Result<()> {
        if self.png_write_ptr.is_none() {
            // bogus call, not initialized
            return Err(Error::new("APNG animation write not started."));
        }
        // (the frame is never NULL)

        if self.current_frame_index == 0 {
            let png_color_type;

            self.output_pixel_format = frame.format();
            if self.output_pixel_format != PixelFormat::RGBA32
                && self.output_pixel_format != PixelFormat::INDEX8
            {
                self.output_pixel_format = PixelFormat::RGBA32;
            }

            if self.output_pixel_format == PixelFormat::INDEX8 {
                png_color_type = PNG_COLOR_TYPE_PALETTE; // Use palette for paletted output
            } else {
                self.output_pixel_format = PixelFormat::RGBA32;
                png_color_type = PNG_COLOR_TYPE_RGBA; // Use RGBA for other formats
            }

            // (SDL_GetPixelFormatDetails() knows both formats)
            let pfd = self.output_pixel_format;
            let bit_depth = (pfd.bits_per_pixel() / pfd.bytes_per_pixel()) as u8;

            self.apng_width = frame.width();
            self.apng_height = frame.height();

            let mut ihdr_data = [0u8; 13];
            custom_png_save_uint_32(&mut ihdr_data, self.apng_width as u32);
            custom_png_save_uint_32(&mut ihdr_data[4..], self.apng_height as u32);

            ihdr_data[8] = bit_depth;
            ihdr_data[9] = png_color_type;
            ihdr_data[10] = PNG_COMPRESSION_TYPE_BASE as u8;
            ihdr_data[11] = PNG_FILTER_TYPE_BASE as u8;
            ihdr_data[12] = PNG_INTERLACE_NONE as u8;
            write_png_chunk(e.dst(), "IHDR", &ihdr_data)?;

            self.acTL_chunk_start_pos = e.dst().tell().unwrap_or(-1);

            // Write a placeholder acTL chunk (animation control)
            // num_frames and num_plays will be updated in SaveAPNGAnimationEnd.
            let mut actl_data = [0u8; 8];
            custom_png_save_uint_32(&mut actl_data, 0); // Placeholder for num_frames
            custom_png_save_uint_32(&mut actl_data[4..], 0); // num_plays (0 for infinite loop)

            write_png_chunk(e.dst(), "acTL", &actl_data)?;
        } else if frame.width() != self.apng_width || frame.height() != self.apng_height {
            // The current API is unspecified about deciding whether to fail or resize subsequent frames according to the first frame so,
            // we will fail here as default, if API changes in the future requires us to resize the subsequent frames, please uncomment the code below.

            return Err(Error::new(format!(
                "Frame {} doesn't match the first frame's width (current={} | expected={}) and height (current={} | expected={})",
                self.current_frame_index,
                frame.width(),
                self.apng_width,
                frame.height(),
                self.apng_height
            )));

            //    current_frame_for_processing = SDL_CreateSurface(stream->ctx->apng_width, stream->ctx->apng_height, SDL_PIXELFORMAT_RGBA32);
            //    if (!current_frame_for_processing) {
            //        SDL_SetError("Failed to create resized surface for APNG frame: %s", SDL_GetError());
            //        goto error;
            //    }
            //    if (!SDL_BlitSurfaceScaled(frame, NULL, current_frame_for_processing, NULL, SDL_SCALEMODE_NEAREST)) {
            //        SDL_SetError("Failed to scale surface for APNG frame: %s", SDL_GetError());
            //        goto error;
            //    }
        }
        let current_frame_for_processing: &Surface<'_> = frame;

        // We do manually convert surface pixel format right now if it doesn't much INDEX8 and RGBA32,
        // since those two are the only ones supported by this implementation of libpng right now (usually libpng only uses palette, rgb/a or gray/with alpha).
        let converted;
        let final_frame_for_compression: &Surface<'_> =
            if self.output_pixel_format == PixelFormat::INDEX8 {
                if current_frame_for_processing.format() != PixelFormat::INDEX8 {
                    match current_frame_for_processing.convert(PixelFormat::INDEX8) {
                        Ok(s) => {
                            converted = s;
                            &converted
                        }
                        Err(err) => {
                            return Err(Error::new(format!(
                                "Failed to convert frame to INDEX8 for compression: {err}"
                            )))
                        }
                    }
                } else {
                    current_frame_for_processing
                }
            } else {
                // Default to RGBA32
                if current_frame_for_processing.format() != PixelFormat::RGBA32 {
                    match current_frame_for_processing.convert(PixelFormat::RGBA32) {
                        Ok(s) => {
                            converted = s;
                            &converted
                        }
                        Err(err) => {
                            return Err(Error::new(format!(
                                "Failed to convert frame to RGBA32 for compression: {err}"
                            )))
                        }
                    }
                } else {
                    current_frame_for_processing
                }
            };

        let png_color_type_for_compression = if self.output_pixel_format == PixelFormat::INDEX8 {
            PNG_COLOR_TYPE_PALETTE
        } else {
            PNG_COLOR_TYPE_RGBA
        };

        let delay_den = e.timebase_denominator as u16;
        let delay_num = duration.wrapping_mul(e.timebase_numerator as i64 as u64) as u16;

        // Default image + first animated frame
        if self.current_frame_index == 0 {
            // If paletted output, create and write PLTE and tRNS chunks
            if self.output_pixel_format == PixelFormat::INDEX8 {
                let first_frame_colors: Vec<_> = match final_frame_for_compression.palette() {
                    Some(p) => p
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .colors()
                        .to_vec(),
                    None => Vec::new(),
                };
                if !first_frame_colors.is_empty() {
                    let mut palette = match Palette::new(first_frame_colors.len()) {
                        Ok(palette) => palette,
                        Err(err) => {
                            return Err(Error::new(format!(
                                "Failed to allocate palette for APNG: {err}"
                            )))
                        }
                    };
                    let _ = palette.set_colors(0, &first_frame_colors);
                    self.apng_palette_ptr = Some(palette);
                } else {
                    return Err(Error::new(
                        "First frame has no palette after conversion to indexed format.",
                    ));
                }

                // Write PLTE chunk
                let colors = self
                    .apng_palette_ptr
                    .as_ref()
                    .expect("the palette")
                    .colors();
                let mut plte_data: Vec<u8> = Vec::new(); // 3 bytes per color (RGB)
                if plte_data.try_reserve_exact(colors.len() * 3).is_err() {
                    return Err(Error::new("Out of memory for PLTE data"));
                }
                for c in colors {
                    plte_data.extend_from_slice(&[c.r, c.g, c.b]);
                }
                write_png_chunk(e.dst(), "PLTE", &plte_data)?;

                // Write tRNS chunk (based on hex dump, first entry transparent)
                let tRNS_data: [u8; 1] = [0x00];
                write_png_chunk(e.dst(), "tRNS", &tRNS_data)?;
            }

            if let Some(metadata) = self.metadata.as_ref().filter(|m| has_metadata(m)) {
                let desc = metadata.get_string(PROP_METADATA_DESCRIPTION_STRING);
                let rights = metadata.get_string(PROP_METADATA_COPYRIGHT_STRING);
                let title = metadata.get_string(PROP_METADATA_TITLE_STRING);
                let author = metadata.get_string(PROP_METADATA_AUTHOR_STRING);
                let creationtime = metadata.get_string(PROP_METADATA_CREATION_TIME_STRING);

                if let Some(desc) = desc {
                    writetEXtchunk(e.dst(), "Description", &desc)?;
                }
                if let Some(rights) = rights {
                    writetEXtchunk(e.dst(), "Copyright", &rights)?;
                }
                if let Some(title) = title {
                    writetEXtchunk(e.dst(), "Title", &title)?;
                }
                if let Some(author) = author {
                    writetEXtchunk(e.dst(), "Author", &author)?;
                }
                if let Some(creationtime) = creationtime {
                    writetEXtchunk(e.dst(), "Creation Time", &creationtime)?;
                }
            }

            // Now, write the fcTL for the first animated frame (sequence 0)
            let mut fctl_data = [0u8; 26];
            custom_png_save_uint_32(&mut fctl_data, 0); // Sequence number for the first animated frame is fixed at 0.
            custom_png_save_uint_32(&mut fctl_data[4..], self.apng_width as u32); // Frame width
            custom_png_save_uint_32(&mut fctl_data[8..], self.apng_height as u32); // Frame height
            custom_png_save_uint_32(&mut fctl_data[12..], 0); // x_offset
            custom_png_save_uint_32(&mut fctl_data[16..], 0); // y_offset

            custom_png_save_uint_16(&mut fctl_data[20..], delay_num);
            custom_png_save_uint_16(&mut fctl_data[22..], delay_den);
            fctl_data[24] = PNG_DISPOSE_OP_NONE; // dispose_op
            fctl_data[25] = PNG_BLEND_OP_SOURCE; // blend_op
            write_png_chunk(e.dst(), "fcTL", &fctl_data)?;

            // Compress the current frame's surface into a full zlib stream (IDAT payload)
            let full_zlib_data = match compress_surface_to_png_data(
                final_frame_for_compression,
                self.compression_level,
                png_color_type_for_compression,
            ) {
                Ok(data) if !data.is_empty() => data,
                _ => {
                    return Err(Error::new(
                        "Failed to compress frame data for default IDAT.",
                    ))
                }
            };

            // Write IDAT chunk: This is the default image, NOT part of the animation sequence
            write_png_chunk(e.dst(), "IDAT", &full_zlib_data)?;

            // We have no fdAT chunk for the first frame, so we increase our index only by 1.
            self.current_frame_index = 1;
        } else {
            // For subsequent animated frames (current_frame_index > 0)
            // Compress the current frame's surface into a full zlib stream (IDAT payload)
            let full_zlib_data = match compress_surface_to_png_data(
                final_frame_for_compression,
                self.compression_level,
                png_color_type_for_compression,
            ) {
                Ok(data) if !data.is_empty() => data,
                _ => {
                    return Err(Error::new(
                        "Failed to compress frame data or compressed data is empty.",
                    ))
                }
            };

            // Write fcTL chunk for this frame
            let mut fctl_data = [0u8; 26];
            custom_png_save_uint_32(&mut fctl_data, self.current_frame_index as u32); // Sequence number for this fcTL
            custom_png_save_uint_32(&mut fctl_data[4..], self.apng_width as u32); // Frame width
            custom_png_save_uint_32(&mut fctl_data[8..], self.apng_height as u32); // Frame height
            custom_png_save_uint_32(&mut fctl_data[12..], 0); // x_offset
            custom_png_save_uint_32(&mut fctl_data[16..], 0); // y_offset

            custom_png_save_uint_16(&mut fctl_data[20..], delay_num);
            custom_png_save_uint_16(&mut fctl_data[22..], delay_den);
            fctl_data[24] = PNG_DISPOSE_OP_NONE;
            fctl_data[25] = PNG_BLEND_OP_SOURCE;

            write_png_chunk(e.dst(), "fcTL", &fctl_data)?;

            // Write fdAT chunk for this frame
            let mut fdat_prefix = [0u8; 4];
            custom_png_save_uint_32(
                &mut fdat_prefix,
                self.current_frame_index.wrapping_add(1) as u32,
            ); // Sequence number for fdAT

            let mut fdat_data: Vec<u8> = Vec::new();
            if fdat_data
                .try_reserve_exact(4 + full_zlib_data.len())
                .is_err()
            {
                return Err(Error::new("Out of memory for fdAT data"));
            }
            fdat_data.extend_from_slice(&fdat_prefix);
            fdat_data.extend_from_slice(&full_zlib_data);
            write_png_chunk(e.dst(), "fdAT", &fdat_data)?;

            self.current_frame_index = self.current_frame_index.wrapping_add(2);
            // Increment by 2 (one for fcTL and one for fdAT) for the next fcTL sequence number
        }

        Ok(())
    }

    /// Translation of `SaveAPNGAnimationEnd()` (the write structs, the
    /// palette and the metadata go with the context).
    pub(crate) fn end(&mut self, e: &mut EncoderCore<'_, '_>) -> Result<()> {
        if self.png_write_ptr.is_none() {
            // bogus call, not initialized
            return Err(Error::new("APNG animation write not in progress."));
        }

        let dst = e.dst();
        let current_pos = match dst.tell() {
            Ok(pos) => pos,
            Err(err) => {
                return Err(Error::new(format!(
                    "Failed to get current stream position: {err}"
                )))
            }
        };

        // FIXME (upstream): with no frames added, there is no acTL chunk and
        // its position is 0: the acTL is written over the start of the
        // stream (the PNG signature), and the IEND after it.
        if let Err(err) = dst.seek(self.acTL_chunk_start_pos, IoWhence::Set) {
            return Err(Error::new(format!("Failed to seek to acTL chunk: {err}")));
        }

        let mut actl_data = [0u8; 8];
        // Write the actual total number of frames pushed (which is current_frame_index + 1 / 2 or directly 0 if current_frame_index is 0)
        custom_png_save_uint_32(
            &mut actl_data,
            if self.current_frame_index == 0 {
                0
            } else {
                (self.current_frame_index.wrapping_add(1) / 2) as u32
            },
        );

        // num_plays
        let mut numplays: u32 = 0;
        if let Some(metadata) = self.metadata.as_ref().filter(|m| has_metadata(m)) {
            numplays = metadata
                .get_number(PROP_METADATA_LOOP_COUNT_NUMBER)
                .unwrap_or(0)
                .max(0) as u32;
        }

        custom_png_save_uint_32(&mut actl_data[4..], numplays);

        // Re-write the updated acTL chunk. write_png_chunk will recalculate its CRC.
        write_png_chunk(dst, "acTL", &actl_data)?;

        if let Err(err) = dst.seek(current_pos, IoWhence::Set) {
            return Err(Error::new(format!(
                "Failed to seek back to end of stream: {err}"
            )));
        }

        // Write the IEND chunk to finalize the PNG file
        write_png_chunk(dst, "IEND", &[])?;

        Ok(())
    }
}

/// Create the APNG encoder of an animation encoder: the PNG signature
/// written, the compression level from the quality (1 to 9, 1 by
/// default), and the metadata unless ignored. Translation of
/// `IMG_CreateAPNGAnimationEncoder()`.
pub(crate) fn create_apng_animation_encoder(
    e: &mut EncoderCore<'_, '_>,
    props: &Properties,
) -> Result<Box<ApngEncoderContext>> {
    // (IMG_InitPNG(): libpng is linked in)

    let mut ctx = Box::new(ApngEncoderContext {
        png_write_ptr: None,
        info_write_ptr: None,
        acTL_chunk_start_pos: 0,
        current_frame_index: 0,
        apng_width: 0,
        apng_height: 0,
        compression_level: 0,
        output_pixel_format: PixelFormat::UNKNOWN,
        apng_palette_ptr: None,
        metadata: None,
    });

    let ignore_props = props
        .get_bool(PROP_METADATA_IGNORE_PROPS_BOOLEAN)
        .unwrap_or(false);
    if !ignore_props {
        let metadata = Properties::new();
        metadata.copy_from(props)?;
        ctx.metadata = Some(metadata);
    }

    e.start = e.dst().tell().unwrap_or(-1);

    // Calculate compression level based on quality (1-9)
    ctx.compression_level =
        crate::util::c_f32_to_i32(e.quality as f32 / 100.0f32 * 8.0f32).wrapping_add(1);
    if ctx.compression_level < 1 {
        ctx.compression_level = 1;
    }
    if ctx.compression_level > 9 {
        ctx.compression_level = 9;
    }

    ctx.png_write_ptr = png_create_write_struct(PNG_LIBPNG_VER_STRING);
    let Some(png_write_ptr) = ctx.png_write_ptr.as_mut() else {
        return Err(Error::new("Couldn't allocate memory for PNG write struct"));
    };
    ctx.info_write_ptr = png_create_info_struct(png_write_ptr);
    if ctx.info_write_ptr.is_none() {
        return Err(Error::new("Couldn't create image information for PNG file"));
    }

    // (the setjmp(): nothing below fails in libpng)

    // png_io_context io_context = { apng_write_ctx.dst_stream };
    // (without the stream, which the struct never writes to)
    png_set_write_fn(
        png_write_ptr,
        None,
        Some(png_write_data),
        Some(png_flush_data),
    );

    // Write PNG signature (8 bytes)
    if e.dst().write(&PNG_SIG) != 8 {
        return Err(Error::new("Failed to write PNG signature"));
    }

    Ok(ctx)
}
