// Rust translation of pngwrite.c from libpng 1.6.59 (the sequential writer
// SDL_image's APNG frame compression calls).
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngwrite.c - general routines to write a PNG file
//!
//! (Only the chunks SDL_image's APNG encoder puts in its temporary PNGs are
//! written: IHDR, PLTE, tRNS, IDAT and IEND. The unknown-chunk, colour
//! space, text and time chunk writers, the write transformations, the
//! interlacing and the simplified API are not translated; SDL_image sets
//! none of them on a write struct.)

use super::png::*;
use super::pngerror::{png_app_error, png_app_warning, png_benign_error, png_error};
use super::pnginfo::PngInfo;
use super::pngmem::png_malloc;
use super::pngpriv::*;
use super::pngstruct::PngStruct;
use super::pngtrans::{png_do_check_palette_indexes, png_set_interlace_handling};
use super::pngwio::{png_flush, png_set_write_fn};
use super::pngwutil::*;
use crate::zlib::Z_SYNC_FLUSH;

/// Writes all the PNG information.  This is the suggested way to use the
/// library.  If you have a new chunk to add, make a function to write it,
/// and put it in the correct location here.  If you want the chunk written
/// after the image data, put it in png_write_end().  I strongly encourage
/// you to supply a PNG_INFO_<chunk> flag, and check info_ptr->valid before
/// writing the chunk, as that will keep the code from breaking if you want
/// to just write a plain PNG file.  If you have long comments, I suggest
/// writing them in png_write_end(), and compressing them.
/// (`png_write_info_before_PLTE`)
#[allow(non_snake_case)]
pub(crate) fn png_write_info_before_PLTE(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &PngInfo,
) -> PngResult<()> {
    if (png_ptr.mode & PNG_WROTE_INFO_BEFORE_PLTE) == 0 {
        /* Write PNG signature */
        png_write_sig(png_ptr)?;

        if (png_ptr.mode & PNG_HAVE_PNG_SIGNATURE) != 0 && png_ptr.mng_features_permitted != 0 {
            super::pngerror::png_warning(
                png_ptr,
                "MNG features are not allowed in a PNG datastream",
            );
            png_ptr.mng_features_permitted = 0;
        }

        /* Write IHDR information. */
        png_write_IHDR(
            png_ptr,
            info_ptr.width,
            info_ptr.height,
            info_ptr.bit_depth as i32,
            info_ptr.color_type as i32,
            info_ptr.compression_type as i32,
            info_ptr.filter_type as i32,
            info_ptr.interlace_type as i32,
        )?;

        /* The rest of these check to see if the valid field has the appropriate
         * flag set, and if it does, writes the chunk.
         *
         * (The unknown, sBIT, cLLI, mDCV, cICP, iCCP, sRGB, gAMA and cHRM
         * chunks: SDL_image never sets them on a write struct.)
         */

        png_ptr.mode |= PNG_WROTE_INFO_BEFORE_PLTE;
    }
    Ok(())
}

/// `png_write_info`
pub(crate) fn png_write_info(png_ptr: &mut PngStruct<'_, '_>, info_ptr: &PngInfo) -> PngResult<()> {
    png_write_info_before_PLTE(png_ptr, info_ptr)?;

    if (info_ptr.valid & PNG_INFO_PLTE) != 0 {
        let palette = info_ptr.palette.as_deref().unwrap_or(&[]);
        png_write_PLTE(png_ptr, palette, info_ptr.num_palette as u32)?;
    } else if info_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
        return Err(png_error(
            png_ptr,
            "Valid palette required for paletted images",
        ));
    }

    if (info_ptr.valid & PNG_INFO_tRNS) != 0 {
        /* (PNG_INVERT_ALPHA is never set) */
        png_write_tRNS(
            png_ptr,
            info_ptr.trans_alpha.as_deref().unwrap_or(&[]),
            &info_ptr.trans_color,
            info_ptr.num_trans as i32,
            info_ptr.color_type as i32,
        )?;
    }

    /* (bKGD, eXIf, hIST, oFFs, pCAL, sCAL, pHYs, tIME, sPLT, text and unknown
     * chunks: SDL_image never sets them on a write struct.)
     */
    Ok(())
}

/// Writes the end of the PNG file.  If you don't want to write comments or
/// time information, you can pass NULL for info.  If you already wrote these
/// in png_write_info(), do not write them again here.  If you have long
/// comments, I suggest writing them here, and compressing them.
/// (`png_write_end`)
pub(crate) fn png_write_end(
    png_ptr: &mut PngStruct<'_, '_>,
    _info_ptr: Option<&mut PngInfo>,
) -> PngResult<()> {
    if (png_ptr.mode & PNG_HAVE_IDAT) == 0 {
        return Err(png_error(png_ptr, "No IDATs written into file"));
    }

    if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE
        && png_ptr.num_palette_max >= png_ptr.num_palette as i32
    {
        png_benign_error(png_ptr, "Wrote palette index exceeding num_palette")?;
    }

    /* See if user wants us to write information chunks */
    /* (tIME, text, eXIf and unknown chunks: SDL_image never sets them) */

    png_ptr.mode |= PNG_AFTER_IDAT;

    /* Write end of PNG file */
    png_write_IEND(png_ptr)?;

    /* This flush, added in libpng-1.0.8, removed from libpng-1.0.9beta03,
     * and restored again in libpng-1.2.30, may cause some applications that
     * do not set png_ptr->output_flush_fn to crash.  If your application
     * experiences a problem, please try building libpng with
     * PNG_WRITE_FLUSH_AFTER_IEND_SUPPORTED defined, and report the event to
     * png-mng-implement at lists.sf.net .
     */
    /* (PNG_WRITE_FLUSH_AFTER_IEND_SUPPORTED is not defined) */
    Ok(())
}

/// Initialize png_ptr structure, and allocate any memory needed
/// (`png_create_write_struct`, with no error or warning functions)
pub(crate) fn png_create_write_struct<'s, 'b>(
    _user_png_ver: &str,
) -> Option<Box<PngStruct<'s, 'b>>> {
    let mut png_ptr = png_create_png_struct()?;

    /* Set the zlib control values to defaults; they can be overridden by the
     * application after the struct has been created.
     */
    png_ptr.zbuffer_size = PNG_ZBUF_SIZE;

    /* The 'zlib_strategy' setting is irrelevant because png_default_claim in
     * pngwutil.c defaults it according to whether or not filters will be
     * used, and ignores this setting.
     */
    png_ptr.zlib_strategy = PNG_Z_DEFAULT_STRATEGY;
    png_ptr.zlib_level = PNG_Z_DEFAULT_COMPRESSION;
    png_ptr.zlib_mem_level = 8;
    png_ptr.zlib_window_bits = 15;
    png_ptr.zlib_method = 8;

    png_ptr.zlib_text_strategy = PNG_TEXT_Z_DEFAULT_STRATEGY;
    png_ptr.zlib_text_level = PNG_TEXT_Z_DEFAULT_COMPRESSION;
    png_ptr.zlib_text_mem_level = 8;
    png_ptr.zlib_text_window_bits = 15;
    png_ptr.zlib_text_method = 8;

    /* This is a highly dubious configuration option; by default it is off,
     * but it may be appropriate for private builds that are testing
     * extensions not conformant to the current specification, or of
     * applications that must not fail to write at all costs!
     */
    /* (PNG_BENIGN_WRITE_ERRORS_SUPPORTED is not defined) */

    /* App warnings are warnings in release (or release candidate) builds but
     * are errors during development.
     */
    png_ptr.flags |= PNG_FLAG_APP_WARNINGS_WARN;

    /* TODO: delay this, it can be done in png_init_io() (if the app doesn't
     * do it itself) avoiding setting the default function if it is not
     * required.
     */
    png_set_write_fn(&mut png_ptr, None, None, None);

    Some(png_ptr)
}

/// Write the image.  You only need to call this function once, even
/// if you are writing an interlaced image. (`png_write_image`: `image`
/// holds the row pointers)
pub(crate) fn png_write_image(png_ptr: &mut PngStruct<'_, '_>, image: &[&[u8]]) -> PngResult<()> {
    /* Initialize interlace handling.  If image is not interlaced,
     * this will set pass to 1
     */
    let num_pass = png_set_interlace_handling(png_ptr);
    /* Loop through passes */
    for _pass in 0..num_pass {
        /* Loop through image */
        for rp in image.iter().take(png_ptr.height as usize) {
            png_write_row(png_ptr, rp)?;
        }
    }
    Ok(())
}

/// Called by user to write a row of image data (`png_write_row`)
pub(crate) fn png_write_row(png_ptr: &mut PngStruct<'_, '_>, row: &[u8]) -> PngResult<()> {
    /* Initialize transformations and other stuff if first time */
    if png_ptr.row_number == 0 && png_ptr.pass == 0 {
        /* Make sure we wrote the header info */
        if (png_ptr.mode & PNG_WROTE_INFO_BEFORE_PLTE) == 0 {
            return Err(png_error(
                png_ptr,
                "png_write_info was never called before png_write_row",
            ));
        }

        /* (Check for transforms that have been set but were defined out:
         * none are defined out) */

        png_write_start_row(png_ptr)?;
    }

    /* (If interlaced and not interested in row, return: SDL_image never
     * writes an interlaced image) */

    /* Set up row info for transformations */
    let mut row_info = PngRowInfo {
        color_type: png_ptr.color_type,
        width: png_ptr.usr_width,
        channels: png_ptr.usr_channels,
        bit_depth: png_ptr.usr_bit_depth,
        pixel_depth: 0,
        rowbytes: 0,
    };
    row_info.pixel_depth = row_info.bit_depth.wrapping_mul(row_info.channels);
    row_info.rowbytes = png_rowbytes(row_info.pixel_depth as u32, row_info.width as usize);

    /* Copy user's row into buffer, leaving room for filter byte. */
    let n = row_info.rowbytes;
    png_ptr.row_buf[1..1 + n].copy_from_slice(&row[..n]);

    /* (Handle other transformations: SDL_image sets none on write) */

    /* At this point the row_info pixel depth must match the 'transformed' depth,
     * which is also the output depth.
     */
    if row_info.pixel_depth != png_ptr.pixel_depth
        || row_info.pixel_depth != png_ptr.transformed_pixel_depth
    {
        return Err(png_error(png_ptr, "internal write transform logic error"));
    }

    /* (MNG intrapixel differencing: never permitted) */

    /* Added at libpng-1.5.10 */
    /* Check for out-of-range palette index */
    if row_info.color_type == PNG_COLOR_TYPE_PALETTE && png_ptr.num_palette_max >= 0 {
        png_do_check_palette_indexes(png_ptr, &row_info);
    }

    /* Find a filter if necessary, filter the row and write it out. */
    png_write_find_filter(png_ptr, &row_info)?;

    /* (no write_row_fn) */
    Ok(())
}

/// Flush the current output buffers now (`png_write_flush`)
pub(crate) fn png_write_flush(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    /* We have already written out all of the data */
    if png_ptr.row_number >= png_ptr.num_rows {
        return Ok(());
    }

    png_compress_IDAT(png_ptr, &[], Z_SYNC_FLUSH)?;
    png_ptr.flush_rows = 0;
    png_flush(png_ptr)
}

/* (png_write_destroy() and png_destroy_write_struct(): dropping the
 * structs; zlib's deflate state goes with them, as deflateEnd() would free
 * it) */

/// Allow the application to select one or more row filters to use.
/// (`png_set_filter`)
pub(crate) fn png_set_filter(
    png_ptr: &mut PngStruct<'_, '_>,
    method: i32,
    mut filters: i32,
) -> PngResult<()> {
    /* (MNG_FEATURES: PNG_FLAG_MNG_FILTER_64 is never permitted) */
    if method == PNG_FILTER_TYPE_BASE {
        match filters & (PNG_ALL_FILTERS as i32 | 0x07) {
            5..=7 => {
                png_app_error(png_ptr, "Unknown row filter for method 0")?;
                /* FALLTHROUGH */
                png_ptr.do_filter = PNG_FILTER_NONE;
            }
            f if f == PNG_FILTER_VALUE_NONE as i32 => {
                png_ptr.do_filter = PNG_FILTER_NONE;
            }

            f if f == PNG_FILTER_VALUE_SUB as i32 => {
                png_ptr.do_filter = PNG_FILTER_SUB;
            }

            f if f == PNG_FILTER_VALUE_UP as i32 => {
                png_ptr.do_filter = PNG_FILTER_UP;
            }

            f if f == PNG_FILTER_VALUE_AVG as i32 => {
                png_ptr.do_filter = PNG_FILTER_AVG;
            }

            f if f == PNG_FILTER_VALUE_PAETH as i32 => {
                png_ptr.do_filter = PNG_FILTER_PAETH;
            }

            _ => {
                png_ptr.do_filter = filters as u8;
            }
        }

        /* If we have allocated the row_buf, this means we have already started
         * with the image and we should have allocated all of the filter buffers
         * that have been selected.  If prev_row isn't already allocated, then
         * it is too late to start using the filters that need it, since we
         * will be missing the data in the previous row.  If an application
         * wants to start and stop using particular filters during compression,
         * it should start out with all of the filters, and then remove them
         * or add them back after the start of compression.
         *
         * NOTE: this is a nasty constraint on the code, because it means that the
         * prev_row buffer must be maintained even if there are currently no
         * 'prev_row' requiring filters active.
         */
        if !png_ptr.row_buf.is_empty() {
            let up_avg_paeth = (PNG_FILTER_UP | PNG_FILTER_AVG | PNG_FILTER_PAETH) as i32;
            let sub_avg_paeth = (PNG_FILTER_SUB | PNG_FILTER_AVG | PNG_FILTER_PAETH) as i32;

            /* Repeat the checks in png_write_start_row; 1 pixel high or wide
             * images cannot benefit from certain filters.  If this isn't done here
             * the check below will fire on 1 pixel high images.
             */
            if png_ptr.height == 1 {
                filters &= !up_avg_paeth;
            }

            if png_ptr.width == 1 {
                filters &= !sub_avg_paeth;
            }

            if (filters & up_avg_paeth) != 0 && png_ptr.prev_row.is_none() {
                /* This is the error case, however it is benign - the previous row
                 * is not available so the filter can't be used.  Just warn here.
                 */
                png_app_warning(
                    png_ptr,
                    "png_set_filter: UP/AVG/PAETH cannot be added after start",
                )?;
                filters &= !up_avg_paeth;
            }

            let mut num_filters = 0;

            for f in [
                PNG_FILTER_SUB,
                PNG_FILTER_UP,
                PNG_FILTER_AVG,
                PNG_FILTER_PAETH,
            ] {
                if (filters & f as i32) != 0 {
                    num_filters += 1;
                }
            }

            /* Allocate needed row buffers if they have not already been
             * allocated.
             */
            let buf_size = png_rowbytes(
                png_ptr.usr_channels as u32 * png_ptr.usr_bit_depth as u32,
                png_ptr.width as usize,
            ) + 1;

            if png_ptr.try_row.is_none() {
                png_ptr.try_row = Some(png_malloc(png_ptr, buf_size)?);
            }

            if num_filters > 1 && png_ptr.tst_row.is_none() {
                png_ptr.tst_row = Some(png_malloc(png_ptr, buf_size)?);
            }
        }
        png_ptr.do_filter = filters as u8;
        Ok(())
    } else {
        Err(png_error(png_ptr, "Unknown custom filter method"))
    }
}

/// `png_set_compression_level`
pub(crate) fn png_set_compression_level(png_ptr: &mut PngStruct<'_, '_>, level: i32) {
    png_ptr.zlib_level = level;
}
