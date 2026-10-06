// Rust translation of src/IMG_bmp.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a BMP image file loading framework
//!
//! ICO/CUR file support is here as well since it uses similar internal
//! representation
//!
//! A good test suite of BMP images is available at:
//! <http://entropymine.com/jason/bmpsuite/bmpsuite/html/bmpsuite.html>

use std::path::Path;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{
    PixelFormat, Surface, PROP_SURFACE_HOTSPOT_X_NUMBER, PROP_SURFACE_HOTSPOT_Y_NUMBER,
};

use crate::img::verify_can_save_surface;
use crate::util::{read_byte, read_error, read_ok, write_error};

/// Translation of `RIFF_FOURCC()`.
const fn riff_fourcc(c0: u8, c1: u8, c2: u8, c3: u8) -> u32 {
    c0 as u32 | (c1 as u32) << 8 | (c2 as u32) << 16 | (c3 as u32) << 24
}

const ICON_TYPE_ICO: u16 = 1;
const ICON_TYPE_CUR: u16 = 2;

/// The size of `CURSORICONFILEDIRENTRY` (packed): bWidth, bHeight,
/// bColorCount, bReserved, xHotspot, yHotspot, dwImageSize, dwImageOffset.
const CURSORICONFILEDIRENTRY_SIZE: usize = 16;
/// The size of `CURSORICONFILEDIR` (packed): idReserved, idType, idCount.
const CURSORICONFILEDIR_SIZE: usize = 6;

/// Translation of `IconEntry`.
struct IconEntry {
    offset: i64,
    width: i32,
    height: i32,
    ncolors: i32,
    hot_x: i32,
    hot_y: i32,
    surface: Option<Surface<'static>>,
}

/* See if an image is contained in a data source */

/// Whether `src` holds a BMP image; the stream position is unchanged.
/// Translation of `IMG_isBMP()`.
pub fn is_bmp(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_bmp = false;
    let mut magic = [0u8; 2];
    if read_ok(src, &mut magic) && &magic == b"BM" {
        is_bmp = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_bmp
}

/// Translation of `IMG_isICOCUR()`.
fn is_icocur(src: &mut IoStream<'_>, type_: u16) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_icocur = false;

    /* The Win32 ICO file header (12 bytes) */
    if let (Ok(bf_reserved), Ok(bf_type), Ok(bf_count)) =
        (src.read_u16_le(), src.read_u16_le(), src.read_u16_le())
    {
        if bf_reserved == 0 && bf_type == type_ && bf_count != 0 {
            is_icocur = true;
        }
    }
    let _ = src.seek(start, IoWhence::Set);

    is_icocur
}

/// Whether `src` holds a Windows icon; the stream position is unchanged.
/// Translation of `IMG_isICO()`.
pub fn is_ico(src: &mut IoStream<'_>) -> bool {
    is_icocur(src, ICON_TYPE_ICO)
}

/// Whether `src` holds a Windows cursor; the stream position is unchanged.
/// Translation of `IMG_isCUR()`.
pub fn is_cur(src: &mut IoStream<'_>) -> bool {
    is_icocur(src, ICON_TYPE_CUR)
}

/* Compression encodings for BMP files */
const BI_RGB: u32 = 0;
#[allow(dead_code)]
const BI_RLE8: u32 = 1;
#[allow(dead_code)]
const BI_RLE4: u32 = 2;
#[allow(dead_code)]
const BI_BITFIELDS: u32 = 3;

/// Translation of `LoadBMP_IO()`.
fn load_bmp(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    Surface::load_bmp_io(src)
}

/// The fields of the Win32 `BITMAPINFOHEADER` struct (40 bytes) the icon
/// code reads.
struct BitmapInfoHeader {
    width: i32,
    height: i32,
    bit_count: u16,
    compression: u32,
    clr_used: u32,
}

/// Read a `BITMAPINFOHEADER`, skipping the fields that aren't used.
fn read_bitmap_info_header(src: &mut IoStream<'_>) -> Result<BitmapInfoHeader> {
    let _bi_size = src.read_u32_le()?;
    let bi_width = src.read_s32_le()?;
    let bi_height = src.read_s32_le()?;
    let _bi_planes = src.read_u16_le()?;
    let bi_bit_count = src.read_u16_le()?;
    let bi_compression = src.read_u32_le()?;
    let _bi_size_image = src.read_u32_le()?;
    let _bi_x_pels_per_meter = src.read_u32_le()?;
    let _bi_y_pels_per_meter = src.read_u32_le()?;
    let bi_clr_used = src.read_u32_le()?;
    let _bi_clr_important = src.read_u32_le()?;
    Ok(BitmapInfoHeader {
        width: bi_width,
        height: bi_height,
        bit_count: bi_bit_count,
        compression: bi_compression,
        clr_used: bi_clr_used,
    })
}

/// The size and color count of a BMP icon image: `(width, height,
/// ncolors)`. Translation of `GetBMPIconInfo()`.
fn get_bmp_icon_info(src: &mut IoStream<'_>) -> Result<(i32, i32, i32)> {
    let mut h = read_bitmap_info_header(src)?;

    h.height >>= 1;

    if h.bit_count <= 8 {
        if h.clr_used == 0 {
            h.clr_used = 1u32.wrapping_shl(h.bit_count as u32);
        }
    } else {
        h.clr_used = 256;
    }

    Ok((h.width, h.height, h.clr_used as i32))
}

/// Decode a BMP icon image (an XOR bitmap and an AND mask, twice the
/// image's height together) to ARGB8888. Translation of `GetBMPSurface()`.
fn get_bmp_surface(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let mut palette = [0u32; 256];
    // Note (upstream): the entries past biClrUsed are uninitialized there.

    let h = read_bitmap_info_header(src)?;
    let (bi_width, mut bi_height, bi_bit_count) = (h.width, h.height, h.bit_count);
    let mut bi_clr_used = h.clr_used;

    /* We don't support any BMP compression right now */
    let mut expand_bmp: u32 = match h.compression {
        BI_RGB => {
            /* Default values for the BMP format */
            match bi_bit_count {
                1 | 4 => bi_bit_count as u32,
                8 => 8,
                24 => 24,
                32 => {
                    /*
                    Rmask = 0x00FF0000;
                    Gmask = 0x0000FF00;
                    Bmask = 0x000000FF;
                    */
                    0
                }
                _ => return Err(Error::new("ICO file with unsupported bit count")),
            }
        }
        _ => return Err(Error::new("Compressed ICO files not supported")),
    };

    /* sanity check image size, so we don't overflow integers, etc. */
    if !(0..=0xFFFFFF).contains(&bi_width) || !(0..=0xFFFFFF).contains(&bi_height) {
        return Err(Error::new("Unsupported or invalid ICO dimensions"));
    }

    /* Create a RGBA surface */
    bi_height >>= 1;
    let mut surface = Surface::new(bi_width, bi_height, PixelFormat::ARGB8888)?;

    /* Load the palette, if any */
    if bi_bit_count <= 8 {
        if bi_clr_used == 0 {
            bi_clr_used = 1 << bi_bit_count;
        }
        if bi_clr_used as usize > palette.len() {
            return Err(Error::new("Unsupported or incorrect biClrUsed field"));
        }
        for entry in palette.iter_mut().take(bi_clr_used as usize) {
            let mut bytes = [0u8; 4];
            if !read_ok(src, &mut bytes) {
                return Err(read_error(src));
            }
            *entry = u32::from_le_bytes(bytes);

            /* Since biSize == 40, we know alpha is reserved and should be zero, meaning opaque */
            if (*entry & 0xFF000000) == 0 {
                *entry |= 0xFF000000;
            }
        }
    }

    let w = surface.width() as usize;
    let h = surface.height() as usize;
    let pitch = surface.pitch() as usize;
    let Some(pixels) = surface.pixels_mut() else {
        // (a surface of zero width or height has no pixels to read)
        return Ok(surface);
    };

    /* Read the surface pixels.  Note that the bmp image is upside down */
    let (_bmp_pitch, pad) = pitch_and_pad(expand_bmp, bi_width);
    for row in (0..h).rev() {
        let bits = &mut pixels[row * pitch..row * pitch + w * 4];
        match expand_bmp {
            1 | 4 | 8 => {
                let mut pixelvalue: u8 = 0;
                let shift = 8 - expand_bmp;
                for i in 0..w {
                    if i % (8 / expand_bmp as usize) == 0 {
                        pixelvalue = read_byte(src).ok_or_else(|| read_error(src))?;
                    }
                    let p = palette[(pixelvalue >> shift) as usize];
                    bits[i * 4..i * 4 + 4].copy_from_slice(&p.to_ne_bytes());
                    pixelvalue = pixelvalue.wrapping_shl(expand_bmp);
                }
            }
            24 => {
                for i in 0..w {
                    let mut pixelvalue: u32 = 0xFF000000;
                    for j in 0..3 {
                        /* Load each color channel into pixel */
                        let channel = read_byte(src).ok_or_else(|| read_error(src))?;
                        pixelvalue |= (channel as u32) << (j * 8);
                    }
                    bits[i * 4..i * 4 + 4].copy_from_slice(&pixelvalue.to_ne_bytes());
                }
            }
            _ => {
                // (upstream reads `surface->pitch` bytes, the row's w * 4)
                let row_bytes = &mut pixels[row * pitch..row * pitch + pitch];
                if !read_ok(src, row_bytes) {
                    return Err(read_error(src));
                }
                // (the file's B, G, R, A bytes are a little-endian ARGB8888
                // pixel; upstream copies them as they are)
                if cfg!(target_endian = "big") {
                    for p in row_bytes.chunks_exact_mut(4) {
                        p.reverse();
                    }
                }
            }
        }
        /* Skip padding bytes, ugh */
        for _ in 0..pad {
            read_byte(src).ok_or_else(|| read_error(src))?;
        }
    }

    /* Read the mask pixels.  Note that the bmp image is upside down */
    expand_bmp = 1;
    let (_bmp_pitch, pad) = pitch_and_pad(expand_bmp, bi_width);
    for row in (0..h).rev() {
        let mut pixelvalue: u8 = 0;
        let shift = 8 - expand_bmp;

        let bits = &mut pixels[row * pitch..row * pitch + w * 4];
        for i in 0..w {
            if i % (8 / expand_bmp as usize) == 0 {
                pixelvalue = read_byte(src).ok_or_else(|| read_error(src))?;
            }
            let p = &mut bits[i * 4..i * 4 + 4];
            let value = u32::from_ne_bytes([p[0], p[1], p[2], p[3]])
                & if (pixelvalue >> shift) != 0 {
                    0
                } else {
                    0xFFFFFFFF
                };
            p.copy_from_slice(&value.to_ne_bytes());
            pixelvalue = pixelvalue.wrapping_shl(expand_bmp);
        }
        /* Skip padding bytes, ugh */
        for _ in 0..pad {
            read_byte(src).ok_or_else(|| read_error(src))?;
        }
    }

    Ok(surface)
}

/// The bytes per row of a BMP image and the padding to the next 4-byte
/// boundary, for `ExpandBMP` bits per pixel (0 for 32).
fn pitch_and_pad(expand_bmp: u32, bi_width: i32) -> (i32, i32) {
    let pad4 = |bmp_pitch: i32| {
        if bmp_pitch % 4 != 0 {
            4 - (bmp_pitch % 4)
        } else {
            0
        }
    };
    match expand_bmp {
        1 => {
            let bmp_pitch = (bi_width + 7) >> 3;
            (bmp_pitch, pad4(bmp_pitch))
        }
        4 => {
            let bmp_pitch = (bi_width + 1) >> 1;
            (bmp_pitch, pad4(bmp_pitch))
        }
        8 => (bi_width, pad4(bi_width)),
        24 => {
            let bmp_pitch = bi_width * 3;
            (bmp_pitch, pad4(bmp_pitch))
        }
        _ => (bi_width * 4, 0),
    }
}

/// The size of a PNG icon image, from its `IHDR` chunk: `(width, height,
/// ncolors)`. Translation of `GetPNGIconInfo()`.
fn get_png_icon_info(src: &mut IoStream<'_>) -> Option<(i32, i32, i32)> {
    let mut magic = [0u8; 16];
    if !read_ok(src, &mut magic) {
        return None;
    }
    let un_width = src.read_u32_be().ok().filter(|&w| w <= i32::MAX as u32)?;
    let un_height = src.read_u32_be().ok().filter(|&h| h <= i32::MAX as u32)?;

    Some((un_width as i32, un_height as i32, 256))
}

/// The size and color count of the icon image at `offset`, which is BMP
/// or PNG data; the stream position is unchanged (unless the seek to
/// `offset` failed). Translation of `GetIconInfo()`.
fn get_icon_info(src: &mut IoStream<'_>, offset: i64) -> Option<(i32, i32, i32)> {
    let start = src.tell().unwrap_or(-1);

    if src.seek(offset, IoWhence::Set).is_err() {
        return None;
    }

    let result = (|| {
        let bi_size = src.read_u32_le().ok()?;
        src.seek(-4, IoWhence::Cur).ok()?;
        if bi_size == 40 {
            get_bmp_icon_info(src).ok()
        } else if bi_size == riff_fourcc(0x89, b'P', b'N', b'G') {
            get_png_icon_info(src)
        } else {
            // ("Unsupported ICO bitmap format": the entry is skipped)
            None
        }
    })();

    let _ = src.seek(start, IoWhence::Set);
    result
}

/// Add an icon to the list, or for a size already in it keep the one with
/// more colors. Translation of `AddIconEntry()`.
fn add_icon_entry(
    entries: &mut Vec<IconEntry>,
    offset: i64,
    width: i32,
    height: i32,
    ncolors: i32,
    hot_x: i32,
    hot_y: i32,
) {
    for entry in entries.iter_mut() {
        if width == entry.width && height == entry.height {
            if ncolors > entry.ncolors {
                // Replace the existing entry
                entry.offset = offset;
                entry.ncolors = ncolors;
                entry.hot_x = hot_x;
                entry.hot_y = hot_y;
            } else {
                // The existing entry is better
            }
            return;
        }
    }

    entries.push(IconEntry {
        offset,
        width,
        height,
        ncolors,
        hot_x,
        hot_y,
        surface: None,
    });
}

/// Decode the icon image at `offset`; a cursor's gets its hotspot as
/// surface properties. Translation of `GetIconSurface()`.
fn get_icon_surface(
    src: &mut IoStream<'_>,
    offset: i64,
    type_: u16,
    hot_x: i32,
    hot_y: i32,
) -> Result<Surface<'static>> {
    src.seek(offset, IoWhence::Set)?;

    let bi_size = src.read_u32_le()?;
    src.seek(-4, IoWhence::Cur)?;
    let mut surface = if bi_size == 40 {
        get_bmp_surface(src)?
    } else if bi_size == riff_fourcc(0x89, b'P', b'N', b'G') {
        crate::png::load_png_io(src)?
    } else {
        return Err(Error::new("Unsupported ICO bitmap format"));
    };

    if type_ == ICON_TYPE_CUR {
        let props = surface.properties();
        props.set(PROP_SURFACE_HOTSPOT_X_NUMBER, hot_x as i64)?;
        props.set(PROP_SURFACE_HOTSPOT_Y_NUMBER, hot_y as i64)?;
    }

    Ok(surface)
}

/// Load an icon or cursor: the best image of each size, the first (or a
/// cursor's last 32x32 one) as the surface and the others as its
/// alternate images. Translation of `LoadICOCUR_IO()`.
fn load_icocur(src: &mut IoStream<'_>, type_: u16) -> Result<Surface<'static>> {
    /* Read in the ICO file header */
    let start = src.tell().unwrap_or(0);

    let result = load_icocur_entries(src, type_, start);
    if result.is_err() {
        let _ = src.seek(start, IoWhence::Set);
    }
    result
}

#[allow(clippy::needless_range_loop)] // the entries are indexed as upstream's
fn load_icocur_entries(src: &mut IoStream<'_>, type_: u16, start: i64) -> Result<Surface<'static>> {
    let mut entries: Vec<IconEntry> = Vec::new();

    /* The Win32 ICO file header (14 bytes) */
    let header = (|| {
        Some((
            src.read_u16_le().ok()?,
            src.read_u16_le().ok()?,
            src.read_u16_le().ok()?,
        ))
    })();
    let bf_count = match header {
        Some((0, bf_type, bf_count)) if bf_type == type_ && bf_count != 0 => bf_count,
        _ => {
            return Err(Error::new(format!(
                "File is not a Windows {} file",
                if type_ == 1 { "ICO" } else { "CUR" }
            )))
        }
    };

    /* Read the Win32 Icon Directory */
    for _ in 0..bf_count {
        /* Icon Directory Entries */
        let _b_width = src.read_u8()?; /* Uint8, but 0 = 256 ! */
        let _b_height = src.read_u8()?; /* Uint8, but 0 = 256 ! */
        let _b_color_count = src.read_u8()?; /* Uint8, but 0 = 256 ! */
        let _b_reserved = src.read_u8()?;
        let w_planes = src.read_u16_le()?;
        let w_bit_count = src.read_u16_le()?;
        let _dw_bytes_in_res = src.read_u32_le()?;
        let dw_image_offset = src.read_u32_le()?;

        let (n_hot_x, n_hot_y) = if type_ == ICON_TYPE_CUR {
            (w_planes as i32, w_bit_count as i32)
        } else {
            (0, 0)
        };

        let Some((n_width, n_height, n_color_count)) =
            get_icon_info(src, start + dw_image_offset as i64)
        else {
            continue;
        };

        add_icon_entry(
            &mut entries,
            start + dw_image_offset as i64,
            n_width,
            n_height,
            n_color_count,
            n_hot_x,
            n_hot_y,
        );
    }

    if entries.is_empty() {
        return Err(Error::new("Couldn't find any valid icons"));
    }

    /* Load the icon surfaces */
    let mut primary = 0;
    for i in 0..entries.len() {
        let entry = &entries[i];
        let surface = get_icon_surface(src, entry.offset, type_, entry.hot_x, entry.hot_y)?;

        if i == 0 {
            primary = 0;
        } else if type_ == ICON_TYPE_CUR && surface.width() == 32 && surface.height() == 32 {
            // Windows defaults to 32x32 pixel cursors, adjusting for DPI scale, so use that as the primary surface
            primary = i;
        }
        entries[i].surface = Some(surface);
    }
    let mut surfaces: Vec<Surface<'static>> =
        entries.into_iter().filter_map(|e| e.surface).collect();
    let mut surface = surfaces.remove(primary);
    for alternate in surfaces {
        surface.add_alternate_image(alternate);
    }

    /* All done! */
    Ok(surface)
}

/// Load a BMP image (through `sdl3`'s BMP loader, as upstream uses
/// `SDL_LoadBMP_IO()`). Translation of `IMG_LoadBMP_IO()`.
pub fn load_bmp_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    load_bmp(src)
}

/// Load a Windows icon. The best image of each size in the file is
/// loaded; the first is the surface and the others its alternate images.
/// Translation of `IMG_LoadICO_IO()`.
pub fn load_ico_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    load_icocur(src, ICON_TYPE_ICO)
}

/// Load a Windows cursor: like [`load_ico_io`], with the 32x32 image (if
/// any) as the surface, and each image's hotspot in its
/// `PROP_SURFACE_HOTSPOT_X_NUMBER`/`_Y_NUMBER` properties.
/// Translation of `IMG_LoadCUR_IO()`.
pub fn load_cur_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    load_icocur(src, ICON_TYPE_CUR)
}

/// Save a surface in BMP format (through `sdl3`'s BMP saver).
/// Translation of `IMG_SaveBMP_IO()`.
pub fn save_bmp_io(surface: &mut Surface<'_>, dst: &mut IoStream<'_>) -> Result<()> {
    verify_can_save_surface(surface)?;
    surface.save_bmp_io(dst)
}

/// Save a surface to a BMP file. Translation of `IMG_SaveBMP()`.
pub fn save_bmp(surface: &mut Surface<'_>, file: impl AsRef<Path>) -> Result<()> {
    verify_can_save_surface(surface)?;
    let mut dst = IoStream::from_file(file, "wb")?;
    let result = save_bmp_io(surface, &mut dst);
    let closed = dst.close();
    result.and(closed)
}

/// The directory entry of an icon image. Translation of `FillIconEntry()`.
fn fill_icon_entry(
    entry: &mut [u8],
    surface: &mut Surface<'_>,
    type_: u16,
    dw_image_size: u32,
    dw_image_offset: u32,
) {
    let (mut hot_x, mut hot_y) = (0i32, 0i32);

    if type_ == ICON_TYPE_CUR {
        let props = surface.properties();
        hot_x = props
            .get_number(PROP_SURFACE_HOTSPOT_X_NUMBER)
            .unwrap_or(hot_x as i64) as i32;
        hot_y = props
            .get_number(PROP_SURFACE_HOTSPOT_Y_NUMBER)
            .unwrap_or(hot_y as i64) as i32;
    }

    entry.fill(0);
    entry[0] = if surface.width() < 256 {
        surface.width() as u8
    } else {
        0
    }; // 0 means a width of 256
    entry[1] = if surface.height() < 256 {
        surface.height() as u8
    } else {
        0
    }; // 0 means a height of 256
    entry[4..6].copy_from_slice(&(hot_x as u16).to_le_bytes());
    entry[6..8].copy_from_slice(&(hot_y as u16).to_le_bytes());
    entry[8..12].copy_from_slice(&dw_image_size.to_le_bytes());
    entry[12..16].copy_from_slice(&dw_image_offset.to_le_bytes());
}

/// Write an icon image, as PNG data padded to an even length.
/// Translation of `WriteIconSurface()`.
fn write_icon_surface(surface: &mut Surface<'_>, dst: &mut IoStream<'_>) -> Result<()> {
    // We'll use PNG format, for simplicity
    crate::png::save_png_io(surface, dst)?;

    // Image data offsets must be WORD aligned
    let offset = dst.tell()?;
    if offset & 1 != 0 {
        dst.write_u8(0)?;
    }
    Ok(())
}

/// Keep the first error of a sequence of steps that all run (upstream's
/// `result &= ...`).
fn keep_first_error(result: &mut Result<()>, step: Result<()>) {
    if result.is_ok() {
        *result = step;
    }
}

/// One image of `SaveICOCUR()`'s loop: write it, fill its directory entry,
/// and return the offset after it.
fn write_icon_image(
    image: &mut Surface<'_>,
    dst: &mut IoStream<'_>,
    type_: u16,
    entry: &mut [u8],
    image_offset: i64,
    start: i64,
    result: &mut Result<()>,
) -> i64 {
    keep_first_error(result, write_icon_surface(image, dst));

    let next_offset = dst.tell().unwrap_or(-1);
    let dw_image_size = (next_offset - image_offset) as u32;
    let dw_image_offset = (image_offset - start) as u32;

    fill_icon_entry(entry, image, type_, dw_image_size, dw_image_offset);

    next_offset
}

/// Write an icon or cursor file of the surface and its alternate images,
/// each as PNG data. Translation of `SaveICOCUR()`.
fn save_icocur(surface: &mut Surface<'_>, dst: &mut IoStream<'_>, type_: u16) -> Result<()> {
    let start = dst.tell()?;
    // We need to be able to seek in the stream
    if start < 0 {
        return Err(Error::new("Can't seek in this data source"));
    }

    let count = 1 + surface.alternate_images().len();

    // Raymond Chen has more insight into this format at:
    // https://devblogs.microsoft.com/oldnewthing/20101018-00/?p=12513
    let mut dir = [0u8; CURSORICONFILEDIR_SIZE];
    dir[0..2].copy_from_slice(&0u16.to_le_bytes());
    dir[2..4].copy_from_slice(&type_.to_le_bytes());
    dir[4..6].copy_from_slice(&(count as u16).to_le_bytes());
    let mut result = if dst.write(&dir) == dir.len() {
        Ok(())
    } else {
        Err(write_error(dst))
    };

    let entries_size = count * CURSORICONFILEDIRENTRY_SIZE;
    // Note (upstream): the entries are written uninitialized at first,
    // then rewritten once they are known; here they start zeroed.
    let mut entries = vec![0u8; entries_size];
    if dst.write(&entries) != entries_size {
        keep_first_error(&mut result, Err(write_error(dst)));
    }

    let mut image_offset = dst.tell().unwrap_or(-1);
    for i in 0..count {
        let entry =
            &mut entries[i * CURSORICONFILEDIRENTRY_SIZE..(i + 1) * CURSORICONFILEDIRENTRY_SIZE];
        image_offset = if i == 0 {
            write_icon_image(surface, dst, type_, entry, image_offset, start, &mut result)
        } else {
            let image = &mut surface.alternate_images_mut()[i - 1];
            write_icon_image(image, dst, type_, entry, image_offset, start, &mut result)
        };
    }

    // Now that we have the icon entries filled out, rewrite them
    keep_first_error(
        &mut result,
        dst.seek(start + CURSORICONFILEDIR_SIZE as i64, IoWhence::Set)
            .map(|_| ()),
    );
    if dst.write(&entries) != entries_size {
        keep_first_error(&mut result, Err(write_error(dst)));
    }
    keep_first_error(
        &mut result,
        dst.seek(image_offset, IoWhence::Set).map(|_| ()),
    );

    if result.is_err() {
        let _ = dst.seek(start, IoWhence::Set);
    }

    result
}

/// Save a surface (and its alternate images) as a Windows cursor, each
/// image in PNG format with its hotspot from the surface's
/// `PROP_SURFACE_HOTSPOT_X_NUMBER`/`_Y_NUMBER` properties.
/// Translation of `IMG_SaveCUR_IO()`.
pub fn save_cur_io(surface: &mut Surface<'_>, dst: &mut IoStream<'_>) -> Result<()> {
    verify_can_save_surface(surface)?;
    save_icocur(surface, dst, ICON_TYPE_CUR)
}

/// Save a surface (and its alternate images) to a Windows cursor file.
/// Translation of `IMG_SaveCUR()`.
pub fn save_cur(surface: &mut Surface<'_>, file: impl AsRef<Path>) -> Result<()> {
    verify_can_save_surface(surface)?;
    let mut dst = IoStream::from_file(file, "wb")?;
    let result = save_icocur(surface, &mut dst, ICON_TYPE_CUR);
    // (upstream ignores the result of closing the file here)
    let _ = dst.close();
    result
}

/// Save a surface (and its alternate images) as a Windows icon, each image
/// in PNG format. Translation of `IMG_SaveICO_IO()`.
pub fn save_ico_io(surface: &mut Surface<'_>, dst: &mut IoStream<'_>) -> Result<()> {
    verify_can_save_surface(surface)?;
    save_icocur(surface, dst, ICON_TYPE_ICO)
}

/// Save a surface (and its alternate images) to a Windows icon file.
/// Translation of `IMG_SaveICO()`.
pub fn save_ico(surface: &mut Surface<'_>, file: impl AsRef<Path>) -> Result<()> {
    verify_can_save_surface(surface)?;
    let mut dst = IoStream::from_file(file, "wb")?;
    let result = save_icocur(surface, &mut dst, ICON_TYPE_ICO);
    // (upstream ignores the result of closing the file here)
    let _ = dst.close();
    result
}
