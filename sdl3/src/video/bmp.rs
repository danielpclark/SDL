// Rust translation of src/video/SDL_bmp.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loading and saving surfaces in Windows BMP format.
//!
//! Why support BMP format?  Well, it's a native format for Windows, and
//! most image processing programs can read and write it.  It would be nice
//! to be able to have at least one image format that we can natively load
//! and save, and since PNG is so complex that it would bloat the library,
//! BMP is a good alternative.
//!
//! Uncompressed 1, 2, 4, 8, 15, 16, 24 and 32 bpp images, bitfield masks,
//! and RLE4/RLE8 compression are supported.

use std::path::Path;

use crate::error::{Error, Result};
use crate::hints;
use crate::io::{IoStream, IoWhence};
use crate::video::blit::COPY_COLORKEY;
use crate::video::pixels::{Color, Palette, PixelFormat, PixelMasks, ALPHA_OPAQUE};
use crate::video::surface::{share_palette, Surface};

// Save 32-bit BMPs for surfaces with alpha or a colorkey (`SAVE_32BIT_BMP`).
const SAVE_32BIT_BMP: bool = true;

// Compression encodings for BMP files
const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;

// Logical color space values for BMP files
// https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-wmf/eb4bbd50-b3ce-4917-895c-be31f214797f
// 0x73524742 == "sRGB"
const LCS_SRGB: u32 = 0x73524742;

// Logical/physical color relationship
// https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-wmf/9fec0834-607d-427d-abd5-ab240fb0db38
const LCS_GM_GRAPHICS: u32 = 0x00000002;

/// The error for a short read of pixel data.
fn read_error(src: &IoStream<'_>) -> Error {
    src.last_error()
        .cloned()
        .unwrap_or_else(|| Error::new("Error reading from datastream"))
}

/// Translation of `readRlePixels()`: set the surface pixels from `src`.
/// A bmp image is upside down.
fn read_rle_pixels(
    pixels: &mut [u8],
    pitch: usize,
    height: usize,
    src: &mut IoStream<'_>,
    is_rle8: bool,
) -> Result<()> {
    let end = height * pitch;
    // The start of the current row; it moves up the image and may leave it.
    let mut bits = end as isize - pitch as isize;
    let mut ofs: isize = 0;
    let pixels_per_byte: i32 = if is_rle8 { 1 } else { 2 };

    let mut copy_pixel = |bits: isize, ofs: &mut isize, x: u8| {
        let spot = bits + *ofs;
        *ofs += 1;
        if spot >= 0 && (spot as usize) < end {
            pixels[spot as usize] = x;
        }
    };

    loop {
        let ch = src.read_u8()?;
        /*
        | encoded mode starts with a run length, and then a byte
        | with two colour indexes to alternate between for the run
        */
        if ch != 0 {
            let pixelvalue = src.read_u8()?;
            let mut ich = ch as i32;
            loop {
                copy_pixel(bits, &mut ofs, pixelvalue);
                ich -= pixels_per_byte;
                if ich <= 0 {
                    break;
                }
            }
        } else {
            /*
            | A leading zero is an escape; it may signal the end of the bitmap,
            | a cursor move, or some absolute data.
            | zero tag may be absolute mode or an escape
            */
            let ch = src.read_u8()?;
            match ch {
                0 => {
                    // end of line
                    ofs = 0;
                    bits -= pitch as isize; // go to previous
                }
                1 => {
                    // end of bitmap
                    return Ok(()); // success!
                }
                2 => {
                    // delta
                    let ch = src.read_u8()?;
                    ofs += (ch as i32 / pixels_per_byte) as isize;
                    if (ch as i32 & pixels_per_byte) != 0 {
                        ofs += 1;
                    }
                    let ch = src.read_u8()?;
                    bits -= ch as isize * pitch as isize;
                }
                _ => {
                    // no compression
                    // !!! FIXME: this needsPad calculation can probably be simpler than this.
                    let needs_pad = if pixels_per_byte == 1 {
                        (ch & 1) != 0
                    } else {
                        (((ch as i32 + (pixels_per_byte - 1)) / pixels_per_byte)
                            & (pixels_per_byte - 1))
                            != 0
                    };
                    let mut ich = ch as i32;
                    loop {
                        let pixelvalue = src.read_u8()?;
                        copy_pixel(bits, &mut ofs, pixelvalue);
                        ich -= pixels_per_byte;
                        if ich <= 0 {
                            break;
                        }
                    }
                    // pad at even boundary
                    if needs_pad {
                        src.read_u8()?;
                    }
                }
            }
        }
    }
}

/// Translation of `CorrectAlphaChannel()`: make a 32-bit image without any
/// alpha data opaque.
fn correct_alpha_channel(pixels: &mut [u8], len: usize) {
    // Check to see if there is any alpha channel data
    let alpha_channel_offset = if cfg!(target_endian = "big") { 0 } else { 3 };
    let alpha = pixels[..len].iter().skip(alpha_channel_offset).step_by(4);
    let has_alpha = alpha.into_iter().any(|&a| a != 0);
    if !has_alpha {
        for a in pixels[..len]
            .iter_mut()
            .skip(alpha_channel_offset)
            .step_by(4)
        {
            *a = ALPHA_OPAQUE;
        }
    }
}

/// Whether `src` starts with a BMP header; the stream position is
/// unchanged. Translation of `SDL_IsBMP()`.
pub fn is_bmp(src: &mut IoStream<'_>) -> bool {
    let mut is_bmp = false;
    if let Ok(start) = src.tell() {
        if start >= 0 {
            let mut magic = [0u8; 2];
            if src.read(&mut magic) == magic.len() && magic == *b"BM" {
                is_bmp = true;
            }
            let _ = src.seek(start, IoWhence::Set);
        }
    }
    is_bmp
}

impl Surface<'_> {
    /// Load a BMP image from a seekable stream. On failure the stream is
    /// put back where it was. Translation of `SDL_LoadBMP_IO()` (the
    /// caller keeps the stream; drop it to close it).
    pub fn load_bmp_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
        // Read in the BMP file header
        let fp_offset = src.tell()?;
        if fp_offset < 0 {
            return Err(Error::new("Couldn't get stream offset"));
        }
        let result = load_bmp(src, fp_offset);
        if result.is_err() {
            let _ = src.seek(fp_offset, IoWhence::Set);
        }
        result
    }

    /// Load a BMP image from a file. Translation of `SDL_LoadBMP()`.
    pub fn load_bmp(file: impl AsRef<Path>) -> Result<Surface<'static>> {
        let mut stream = IoStream::from_file(file, "rb")?;
        Surface::load_bmp_io(&mut stream)
    }

    /// Save the surface to a seekable stream in BMP format: 8-bit palettized
    /// surfaces as they are, surfaces with alpha or a colorkey as 32-bit
    /// BGRA (a v5 header unless `SDL_HINT_BMP_SAVE_LEGACY_FORMAT`), and the
    /// rest as 24-bit. Translation of `SDL_SaveBMP_IO()` (the caller keeps
    /// the stream).
    pub fn save_bmp_io(&mut self, dst: &mut IoStream<'_>) -> Result<()> {
        let mut state = init_bmp_save_state(self)?;
        match &mut state.converted {
            Some(converted) => {
                save_bmp_io_internal(converted, state.save32bit, state.save_legacy_bmp, dst)
            }
            None => save_bmp_io_internal(self, state.save32bit, state.save_legacy_bmp, dst),
        }
    }

    /// Save the surface to a file in BMP format. Translation of `SDL_SaveBMP()`.
    pub fn save_bmp(&mut self, file: impl AsRef<Path>) -> Result<()> {
        let mut state = init_bmp_save_state(self)?;
        let mut stream = IoStream::from_file(file, "wb")?;
        let result = match &mut state.converted {
            Some(converted) => save_bmp_io_internal(
                converted,
                state.save32bit,
                state.save_legacy_bmp,
                &mut stream,
            ),
            None => save_bmp_io_internal(self, state.save32bit, state.save_legacy_bmp, &mut stream),
        };
        let closed = stream.close();
        result.and(closed)
    }
}

/// The body of `SDL_LoadBMP_IO()` after the stream offset is known.
fn load_bmp(src: &mut IoStream<'_>, fp_offset: i64) -> Result<Surface<'static>> {
    let mut rmask: u32 = 0;
    let mut gmask: u32 = 0;
    let mut bmask: u32 = 0;
    let mut amask: u32 = 0;
    let mut have_rgb_masks = false;
    let mut have_alpha_mask = false;
    let mut correct_alpha = false;

    // The Win32 BITMAPINFOHEADER struct (40 bytes)
    let bi_width: i32;
    let mut bi_height: i32;
    let bi_bit_count: u16;
    let mut bi_compression: u32 = BI_RGB;
    let mut bi_clr_used: u32 = 0;

    if !is_bmp(src) {
        return Err(Error::new("File is not a Windows BMP file"));
    }

    // The Win32 BMP file header (14 bytes)
    src.read_u16_le()?; // magic (already checked)
    src.read_u32_le()?; // bfSize
    src.read_u16_le()?; // bfReserved1
    src.read_u16_le()?; // bfReserved2
    let bf_off_bits = src.read_u32_le()?;

    // Read the Win32 BITMAPINFOHEADER
    let bi_size = src.read_u32_le()?;
    if bi_size == 12 {
        // really old BITMAPCOREHEADER
        bi_width = src.read_u16_le()? as i32;
        bi_height = src.read_u16_le()? as i32;
        src.read_u16_le()?; // biPlanes
        bi_bit_count = src.read_u16_le()?;
    } else if bi_size >= 40 {
        // some version of BITMAPINFOHEADER
        bi_width = src.read_s32_le()?;
        bi_height = src.read_s32_le()?;
        src.read_u16_le()?; // biPlanes
        bi_bit_count = src.read_u16_le()?;
        bi_compression = src.read_u32_le()?;
        src.read_u32_le()?; // biSizeImage
        src.read_u32_le()?; // biXPelsPerMeter
        src.read_u32_le()?; // biYPelsPerMeter
        bi_clr_used = src.read_u32_le()?;
        src.read_u32_le()?; // biClrImportant

        // 64 == BITMAPCOREHEADER2, an incompatible OS/2 2.x extension. Skip this stuff for now.
        if bi_size != 64 {
            /* This is complicated. If compression is BI_BITFIELDS, then
            we have 3 DWORDS that specify the RGB masks. This is either
            stored here in an BITMAPV2INFOHEADER (which only differs in
            that it adds these RGB masks) and biSize >= 52, or we've got
            these masks stored in the exact same place, but strictly
            speaking, this is the bmiColors field in BITMAPINFO immediately
            following the legacy v1 info header, just past biSize. */
            if bi_compression == BI_BITFIELDS {
                have_rgb_masks = true;
                rmask = src.read_u32_le()?;
                gmask = src.read_u32_le()?;
                bmask = src.read_u32_le()?;

                // ...v3 adds an alpha mask.
                if bi_size >= 56 {
                    // BITMAPV3INFOHEADER; adds alpha mask
                    have_alpha_mask = true;
                    amask = src.read_u32_le()?;
                }
            } else {
                // the mask fields are ignored for v2+ headers if not BI_BITFIELD.
                if bi_size >= 52 {
                    // BITMAPV2INFOHEADER; adds RGB masks
                    src.read_u32_le()?; // Rmask
                    src.read_u32_le()?; // Gmask
                    src.read_u32_le()?; // Bmask
                }
                if bi_size >= 56 {
                    // BITMAPV3INFOHEADER; adds alpha mask
                    src.read_u32_le()?; // Amask
                }
            }
            /* Insert other fields here; Wikipedia and MSDN say we're up to
            v5 of this header, but we ignore those for now (they add gamma,
            color spaces, etc). Ignoring the weird OS/2 2.x format, we
            currently parse up to v3 correctly (hopefully!). */
        }

        // skip any header bytes we didn't handle...
        let header_size = (src.tell()? - (fp_offset + 14)) as u32;
        if bi_size > header_size {
            src.seek((bi_size - header_size) as i64, IoWhence::Cur)?;
        }
    } else {
        // Upstream reads no further and fails on the dimensions below.
        bi_width = 0;
        bi_height = 0;
        bi_bit_count = 0;
    }
    if bi_width <= 0 || bi_height == 0 {
        return Err(Error::new(format!(
            "BMP file with bad dimensions ({bi_width}x{bi_height})"
        )));
    }
    let top_down = if bi_height < 0 {
        bi_height = bi_height.wrapping_neg();
        true
    } else {
        false
    };

    // Reject invalid bit depths
    if matches!(bi_bit_count, 0 | 3 | 5 | 6 | 7) {
        return Err(Error::new(format!(
            "{bi_bit_count} bpp BMP images are not supported"
        )));
    }

    // RLE4 and RLE8 BMP compression is supported
    if bi_compression == BI_RGB {
        // If there are no masks, use the defaults
        crate::sdl_assert!(!have_rgb_masks);
        crate::sdl_assert!(!have_alpha_mask);
        // Default values for the BMP format
        match bi_bit_count {
            15 | 16 => {
                // SDL_PIXELFORMAT_XRGB1555 or SDL_PIXELFORMAT_ARGB1555 if Amask
                rmask = 0x7C00;
                gmask = 0x03E0;
                bmask = 0x001F;
            }
            24 => {
                if cfg!(target_endian = "big") {
                    // SDL_PIXELFORMAT_RGB24
                    rmask = 0x000000FF;
                    gmask = 0x0000FF00;
                    bmask = 0x00FF0000;
                } else {
                    // SDL_PIXELFORMAT_BGR24
                    rmask = 0x00FF0000;
                    gmask = 0x0000FF00;
                    bmask = 0x000000FF;
                }
            }
            32 => {
                // We don't know if this has alpha channel or not
                correct_alpha = true;
                // SDL_PIXELFORMAT_RGBA8888
                amask = 0xFF000000;
                rmask = 0x00FF0000;
                gmask = 0x0000FF00;
                bmask = 0x000000FF;
            }
            _ => {}
        }
    }
    // BI_BITFIELDS: we handled this in the info header.

    // Create a compatible surface, note that the colors are RGB ordered
    let format = PixelFormat::from_masks(PixelMasks {
        bpp: bi_bit_count as u32,
        r: rmask,
        g: gmask,
        b: bmask,
        a: amask,
    })
    .unwrap_or(PixelFormat::UNKNOWN);
    let mut surface = Surface::new_uninitialized(bi_width, bi_height, format)?;

    // Load the palette, if any
    if surface.format().is_indexed() {
        let ncolors = 1usize << surface.format().bits_per_pixel();

        if src
            .seek(fp_offset + 14 + bi_size as i64, IoWhence::Set)
            .is_err()
        {
            return Err(Error::new("Error seeking in datastream"));
        }

        if bi_bit_count >= 32 {
            // we shift biClrUsed by this value later.
            return Err(Error::new("Unsupported or incorrect biBitCount field"));
        }
        if bi_clr_used == 0 {
            bi_clr_used = 1 << bi_bit_count;
        }
        if bi_clr_used as usize > ncolors {
            bi_clr_used = 1 << bi_bit_count; // try forcing it?
            if bi_clr_used as usize > ncolors {
                return Err(Error::new("Unsupported or incorrect biClrUsed field"));
            }
        }

        let mut colors = Vec::with_capacity(bi_clr_used as usize);
        for _ in 0..bi_clr_used {
            let b = src.read_u8()?;
            let g = src.read_u8()?;
            let r = src.read_u8()?;
            if bi_size != 12 {
                /* According to Microsoft documentation, the fourth element
                   is reserved and must be zero, so we shouldn't treat it as
                   alpha.
                */
                src.read_u8()?;
            }
            colors.push(Color::new(r, g, b, ALPHA_OPAQUE));
        }
        surface.set_palette(Some(share_palette(Palette::from_colors(colors)?)))?;
    }

    // Read the surface pixels.  Note that the bmp image is upside down
    if src
        .seek(fp_offset + bf_off_bits as i64, IoWhence::Set)
        .is_err()
    {
        return Err(Error::new("Error seeking in datastream"));
    }

    let (w, h, pitch) = (
        surface.width() as usize,
        surface.height() as usize,
        surface.pitch() as usize,
    );
    let has_palette = surface.palette().is_some();
    let pixels = surface
        .pixels_mut()
        .ok_or_else(|| Error::new("BMP surface has no pixels"))?;

    if bi_compression == BI_RLE4 || bi_compression == BI_RLE8 {
        if read_rle_pixels(pixels, pitch, h, src, bi_compression == BI_RLE8).is_err() {
            return Err(Error::new("Error reading from datastream"));
        }
        // Success!
        return Ok(surface);
    }

    let end = h * pitch;
    let pad = if pitch % 4 != 0 { 4 - (pitch % 4) } else { 0 };
    for row in 0..h {
        let bits = if top_down {
            row * pitch
        } else {
            end - (row + 1) * pitch
        };
        let line = &mut pixels[bits..bits + pitch];
        if src.read(line) != pitch {
            return Err(read_error(src));
        }
        if bi_bit_count == 8
            && has_palette
            && bi_clr_used < (1u32 << bi_bit_count)
            && line[..w].iter().any(|&p| p as u32 >= bi_clr_used)
        {
            return Err(Error::new(
                "A BMP image contains a pixel with a color out of the palette",
            ));
        }
        if cfg!(target_endian = "big") {
            /* Byte-swap the pixels if needed. Note that the 24bpp
            case has already been taken care of above. */
            match bi_bit_count {
                15 | 16 => line[..w * 2].chunks_exact_mut(2).for_each(|p| p.reverse()),
                32 => line[..w * 4].chunks_exact_mut(4).for_each(|p| p.reverse()),
                _ => {}
            }
        }
        // Skip padding bytes, ugh
        for _ in 0..pad {
            src.read_u8()?;
        }
    }
    if correct_alpha {
        correct_alpha_channel(pixels, end);
    }
    Ok(surface)
}

/// `BMPSaveState`: how a surface is saved, and the converted surface when
/// it can't be saved as it is.
struct BmpSaveState {
    converted: Option<Surface<'static>>,
    save32bit: bool,
    save_legacy_bmp: bool,
}

/// Translation of `InitBMPSaveState()`.
fn init_bmp_save_state(surface: &Surface<'_>) -> Result<BmpSaveState> {
    let format = surface.format();
    let mut state = BmpSaveState {
        converted: None,
        save32bit: false,
        save_legacy_bmp: false,
    };

    // We can save alpha information in a 32-bit BMP
    if SAVE_32BIT_BMP
        && format.bits_per_pixel() >= 8
        && (format.has_alpha() || (surface.map.flags & COPY_COLORKEY) != 0)
    {
        state.save32bit = true;
    }

    if surface.palette().is_some() && !state.save32bit {
        if format.bits_per_pixel() != 8 {
            return Err(Error::new(format!(
                "{} bpp BMP files not supported",
                format.bits_per_pixel()
            )));
        }
    } else if (format == PixelFormat::BGR24 && !state.save32bit)
        || (format == PixelFormat::BGRA32 && state.save32bit)
    {
        // saved as it is
    } else {
        /* If the surface has a colorkey or alpha channel we'll save a
        32-bit BMP with alpha channel, otherwise save a 24-bit BMP. */
        let pixel_format = if state.save32bit {
            PixelFormat::BGRA32
        } else {
            PixelFormat::BGR24
        };
        match surface.convert(pixel_format) {
            Ok(s) => state.converted = Some(s),
            Err(_) => {
                return Err(Error::new(format!(
                    "Couldn't convert image to {} bpp",
                    pixel_format.bits_per_pixel()
                )));
            }
        }
    }

    if state.save32bit {
        state.save_legacy_bmp = hints::get_bool(hints::BMP_SAVE_LEGACY_FORMAT, false);
    }
    Ok(state)
}

/// Translation of `SDL_SaveBMP_IO_Internal()`.
fn save_bmp_io_internal(
    surface: &mut Surface<'_>,
    save32bit: bool,
    save_legacy_bmp: bool,
    dst: &mut IoStream<'_>,
) -> Result<()> {
    let colors: Option<Vec<Color>> = surface
        .palette()
        .map(|p| crate::video::surface::read_palette(p).colors().to_vec());
    let (w, h, format) = (surface.width(), surface.height(), surface.format());
    let lock = surface.lock()?;
    let pitch = lock.pitch() as usize;
    let pixels = lock.pixels().unwrap_or(&[]);

    let bw = w as usize * format.bytes_per_pixel() as usize;

    // The Win32 BMP file header (14 bytes)
    let magic = *b"BM";
    let mut bf_size: u32 = 0; // We'll write this when we're done
    let bf_reserved1: u16 = 0;
    let bf_reserved2: u16 = 0;
    let mut bf_off_bits: u32 = 0; // We'll write this when we're done

    // Write the BMP file header values
    let fp_offset = dst.tell()?;
    if dst.write(&magic) != 2 {
        return Err(write_error(dst));
    }
    dst.write_u32_le(bf_size)?;
    dst.write_u16_le(bf_reserved1)?;
    dst.write_u16_le(bf_reserved2)?;
    dst.write_u32_le(bf_off_bits)?;

    // Set the BMP info values
    let mut bi_size: u32 = 40;
    let bi_width: i32 = w;
    let bi_height: i32 = h;
    let bi_planes: u16 = 1;
    let bi_bit_count = format.bits_per_pixel() as u16;
    let mut bi_compression = BI_RGB;
    let bi_size_image = (h as usize * pitch) as u32;
    let bi_x_pels_per_meter: i32 = 0;
    let bi_y_pels_per_meter: i32 = 0;
    let bi_clr_used = colors.as_ref().map_or(0, |c| c.len() as u32);
    let bi_clr_important: u32 = 0;

    // The additional header members from the Win32 BITMAPV4HEADER struct (108 bytes in total)
    let mut b_v4_red_mask: u32 = 0;
    let mut b_v4_green_mask: u32 = 0;
    let mut b_v4_blue_mask: u32 = 0;
    let mut b_v4_alpha_mask: u32 = 0;
    let mut b_v4_cs_type: u32 = 0;
    let b_v4_endpoints = [0i32; 3 * 3];
    let b_v4_gamma_red: u32 = 0;
    let b_v4_gamma_green: u32 = 0;
    let b_v4_gamma_blue: u32 = 0;

    // The additional header members from the Win32 BITMAPV5HEADER struct (124 bytes in total)
    let mut b_v5_intent: u32 = 0;
    let b_v5_profile_data: u32 = 0;
    let b_v5_profile_size: u32 = 0;
    let b_v5_reserved: u32 = 0;

    // Set the BMP info values
    if save32bit && !save_legacy_bmp {
        bi_size = 124;
        // Version 4 values
        bi_compression = BI_BITFIELDS;
        // The BMP format is always little endian, these masks stay the same
        b_v4_red_mask = 0x00ff0000;
        b_v4_green_mask = 0x0000ff00;
        b_v4_blue_mask = 0x000000ff;
        b_v4_alpha_mask = 0xff000000;
        b_v4_cs_type = LCS_SRGB;
        // Version 5 values
        b_v5_intent = LCS_GM_GRAPHICS;
    }

    // Write the BMP info values
    dst.write_u32_le(bi_size)?;
    dst.write_s32_le(bi_width)?;
    dst.write_s32_le(bi_height)?;
    dst.write_u16_le(bi_planes)?;
    dst.write_u16_le(bi_bit_count)?;
    dst.write_u32_le(bi_compression)?;
    dst.write_u32_le(bi_size_image)?;
    dst.write_u32_le(bi_x_pels_per_meter as u32)?;
    dst.write_u32_le(bi_y_pels_per_meter as u32)?;
    dst.write_u32_le(bi_clr_used)?;
    dst.write_u32_le(bi_clr_important)?;

    // Write the BMP info values
    if save32bit && !save_legacy_bmp {
        // Version 4 values
        dst.write_u32_le(b_v4_red_mask)?;
        dst.write_u32_le(b_v4_green_mask)?;
        dst.write_u32_le(b_v4_blue_mask)?;
        dst.write_u32_le(b_v4_alpha_mask)?;
        dst.write_u32_le(b_v4_cs_type)?;
        for e in b_v4_endpoints {
            dst.write_u32_le(e as u32)?;
        }
        dst.write_u32_le(b_v4_gamma_red)?;
        dst.write_u32_le(b_v4_gamma_green)?;
        dst.write_u32_le(b_v4_gamma_blue)?;
        // Version 5 values
        dst.write_u32_le(b_v5_intent)?;
        dst.write_u32_le(b_v5_profile_data)?;
        dst.write_u32_le(b_v5_profile_size)?;
        dst.write_u32_le(b_v5_reserved)?;
    }

    // Write the palette (in BGR color order)
    for c in colors.iter().flatten() {
        dst.write_u8(c.b)?;
        dst.write_u8(c.g)?;
        dst.write_u8(c.r)?;
        dst.write_u8(c.a)?;
    }

    // Write the bitmap offset
    bf_off_bits = (dst.tell()? - fp_offset) as u32;
    dst.seek(fp_offset + 10, IoWhence::Set)?;
    dst.write_u32_le(bf_off_bits)?;
    dst.seek(fp_offset + bf_off_bits as i64, IoWhence::Set)?;

    // Write the bitmap image upside down
    let pad = if !bw.is_multiple_of(4) {
        4 - (bw % 4)
    } else {
        0
    };
    for row in (0..h as usize).rev() {
        let bits = &pixels[row * pitch..row * pitch + bw];
        if dst.write(bits) != bw {
            return Err(write_error(dst));
        }
        for _ in 0..pad {
            dst.write_u8(0)?;
        }
    }

    // Write the BMP file size
    let new_offset = dst.tell()?;
    bf_size = (new_offset - fp_offset) as u32;
    dst.seek(fp_offset + 2, IoWhence::Set)?;
    dst.write_u32_le(bf_size)?;
    dst.seek(fp_offset + bf_size as i64, IoWhence::Set)?;

    // Close it up..
    drop(lock);
    Ok(())
}

/// The error for a short write.
fn write_error(dst: &IoStream<'_>) -> Error {
    dst.last_error()
        .cloned()
        .unwrap_or_else(|| Error::new("Error writing to datastream"))
}
