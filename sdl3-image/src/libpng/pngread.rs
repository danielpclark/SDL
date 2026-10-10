// Rust translation of pngread.c from libpng 1.6.59 (the sequential reader
// SDL_image's APNG frame decoding calls).
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngread.c - read a PNG file
//!
//! This file contains routines that an application calls directly to
//! read a PNG file or stream.
//!
//! (The simplified API, png_read_png(), png_read_rows() and png_read_end()
//! are not translated: SDL_image's APNG code calls none of them. Neither
//! is the deinterlacing, as its frame streams are never interlaced.)

use super::png::*;
use super::pngerror::{png_app_error, png_error, png_warning};
use super::pnginfo::PngInfo;
use super::pngpriv::*;
use super::pngrio::png_set_read_fn;
use super::pngrtran::{png_do_read_transformations, png_read_transform_info};
use super::pngrutil::*;
use super::pngstruct::PngStruct;
use super::pngtrans::png_set_interlace_handling;

/// Create a PNG structure for reading, and allocate any memory needed.
/// (`png_create_read_struct`, with no error or warning functions)
pub(crate) fn png_create_read_struct<'s, 'b>(
    _user_png_ver: &str,
) -> Option<Box<PngStruct<'s, 'b>>> {
    let mut png_ptr = png_create_png_struct()?;

    png_ptr.mode = PNG_IS_READ_STRUCT;

    /* Added in libpng-1.6.0; this can be used to detect a read structure if
     * required (it will be zero in a write structure.)
     */
    png_ptr.IDAT_read_size = PNG_IDAT_READ_SIZE;

    png_ptr.flags |= PNG_FLAG_BENIGN_ERRORS_WARN;

    /* In stable builds only warn if an application error can be completely
     * handled.
     */
    png_ptr.flags |= PNG_FLAG_APP_WARNINGS_WARN;

    /* TODO: delay this, it can be done in png_init_io (if the app doesn't
     * do it itself) avoiding setting the default function if it is not
     * required.
     */
    png_set_read_fn(&mut png_ptr, None, None);

    Some(png_ptr)
}

/// Read the information before the actual image data.  This has been
/// changed in v0.90 to allow reading a file that already has the magic
/// bytes read from the stream.  You can tell libpng how many bytes have
/// been read from the beginning of the stream (up to the maximum of 8)
/// via png_set_sig_bytes(), and we will only check the remaining bytes
/// here.  The application can then have access to the signature bytes we
/// read if it is determined that this isn't a valid PNG file.
/// (`png_read_info`)
pub(crate) fn png_read_info(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
) -> PngResult<()> {
    /* Read and check the PNG file signature. */
    png_read_sig(png_ptr, info_ptr)?;

    loop {
        let length = png_read_chunk_header(png_ptr)?;
        let chunk_name = png_ptr.chunk_name;
        let keep: i32;

        /* IDAT logic needs to happen here to simplify getting the two flags
         * right.
         */
        if chunk_name == png_IDAT {
            if (png_ptr.mode & PNG_HAVE_IHDR) == 0 {
                return Err(super::pngerror::png_chunk_error(
                    png_ptr,
                    "Missing IHDR before IDAT",
                ));
            } else if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE
                && (png_ptr.mode & PNG_HAVE_PLTE) == 0
            {
                return Err(super::pngerror::png_chunk_error(
                    png_ptr,
                    "Missing PLTE before IDAT",
                ));
            } else if (png_ptr.mode & PNG_AFTER_IDAT) != 0 {
                super::pngerror::png_chunk_benign_error(png_ptr, "Too many IDATs found")?;
            }

            png_ptr.mode |= PNG_HAVE_IDAT;
        } else if (png_ptr.mode & PNG_HAVE_IDAT) != 0 {
            png_ptr.mode |= PNG_HAVE_CHUNK_AFTER_IDAT;
            png_ptr.mode |= PNG_AFTER_IDAT;
        }

        if chunk_name == png_IHDR || chunk_name == png_IEND {
            png_handle_chunk(png_ptr, info_ptr, length)?;
        } else if {
            keep = png_chunk_unknown_handling(png_ptr, chunk_name);
            keep != 0
        } {
            png_handle_unknown(png_ptr, info_ptr, length, keep)?;

            if chunk_name == png_PLTE {
                png_ptr.mode |= PNG_HAVE_PLTE;
            } else if chunk_name == png_IDAT {
                png_ptr.idat_size = 0; /* It has been consumed */
                break;
            }
        } else if chunk_name == png_IDAT {
            png_ptr.idat_size = length;
            break;
        } else {
            png_handle_chunk(png_ptr, info_ptr, length)?;
        }
    }
    Ok(())
}

/// Optional call to update the users info_ptr structure
/// (`png_read_update_info`)
pub(crate) fn png_read_update_info(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
) -> PngResult<()> {
    if (png_ptr.flags & PNG_FLAG_ROW_INIT) == 0 {
        png_read_start_row(png_ptr)?;

        png_read_transform_info(png_ptr, info_ptr)?;
    }
    /* New in 1.6.0 this avoids the bug of doing the initializations twice */
    else {
        png_app_error(
            png_ptr,
            "png_read_update_info/png_start_read_image: duplicate call",
        )?;
    }
    Ok(())
}

/// Initialize palette, background, etc, after transformations
/// are set, but before any reading takes place.  This allows
/// the user to obtain a gamma-corrected palette, for example.
/// If the user doesn't call this, we will do it ourselves.
/// (`png_start_read_image`)
pub(crate) fn png_start_read_image(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    if (png_ptr.flags & PNG_FLAG_ROW_INIT) == 0 {
        png_read_start_row(png_ptr)?;
    }
    /* New in 1.6.0 this avoids the bug of doing the initializations twice */
    else {
        png_app_error(
            png_ptr,
            "png_start_read_image/png_read_update_info: duplicate call",
        )?;
    }
    Ok(())
}

/// `png_read_row` (with no display row, for a non-interlaced image)
pub(crate) fn png_read_row(
    png_ptr: &mut PngStruct<'_, '_>,
    row: Option<&mut [u8]>,
) -> PngResult<()> {
    /* png_read_start_row sets the information (in particular iwidth) for this
     * interlace pass.
     */
    if (png_ptr.flags & PNG_FLAG_ROW_INIT) == 0 {
        png_read_start_row(png_ptr)?;
    }

    /* 1.5.6: row_info moved out of png_struct to a local here. */
    let mut row_info = PngRowInfo {
        width: png_ptr.iwidth, /* NOTE: width of current interlaced row */
        color_type: png_ptr.color_type,
        bit_depth: png_ptr.bit_depth,
        channels: png_ptr.channels,
        pixel_depth: png_ptr.pixel_depth,
        rowbytes: 0,
    };
    row_info.rowbytes = png_rowbytes(row_info.pixel_depth as u32, row_info.width as usize);

    /* (the warnings for transforms that have been set but were defined out:
     * none are defined out) */

    /* (png_ptr->interlaced with PNG_INTERLACE: never the case here) */

    if (png_ptr.mode & PNG_HAVE_IDAT) == 0 {
        return Err(png_error(png_ptr, "Invalid attempt to read row data"));
    }

    /* Fill the row with IDAT data: */
    png_ptr.row_buf[0] = 255; /* to force error if no data was found */
    let mut row_buf = std::mem::take(&mut png_ptr.row_buf);
    let r = png_read_IDAT_data(
        png_ptr,
        Some(&mut row_buf[..row_info.rowbytes + 1]),
        row_info.rowbytes + 1,
    );
    png_ptr.row_buf = row_buf;
    r?;

    if png_ptr.row_buf[0] > PNG_FILTER_VALUE_NONE {
        if png_ptr.row_buf[0] < PNG_FILTER_VALUE_LAST {
            let filter = png_ptr.row_buf[0];
            let mut row_buf = std::mem::take(&mut png_ptr.row_buf);
            let prev_row = png_ptr.prev_row.take().unwrap_or_default();
            png_read_filter_row(
                png_ptr,
                &row_info,
                &mut row_buf[1..],
                &prev_row[1..],
                filter,
            );
            png_ptr.row_buf = row_buf;
            png_ptr.prev_row = Some(prev_row);
        } else {
            return Err(png_error(png_ptr, "bad adaptive filter value"));
        }
    }

    /* libpng 1.5.6: the following line was copying png_ptr->rowbytes before
     * 1.5.6, while the buffer really is this big in current versions of libpng
     * it may not be in the future, so this was changed just to copy the
     * interlaced count:
     */
    let n = row_info.rowbytes + 1;
    if let Some(prev_row) = png_ptr.prev_row.as_mut() {
        prev_row[..n].copy_from_slice(&png_ptr.row_buf[..n]);
    }

    /* (PNG_MNG_FEATURES: the intrapixel differencing is never permitted) */

    png_do_read_transformations(png_ptr, &mut row_info)?;

    /* The transformed pixel depth should match the depth now in row_info. */
    if png_ptr.transformed_pixel_depth == 0 {
        png_ptr.transformed_pixel_depth = row_info.pixel_depth;
        if row_info.pixel_depth > png_ptr.maximum_pixel_depth {
            return Err(png_error(png_ptr, "sequential row overflow"));
        }
    } else if png_ptr.transformed_pixel_depth != row_info.pixel_depth {
        return Err(png_error(
            png_ptr,
            "internal sequential row size calculation error",
        ));
    }

    if let Some(row) = row {
        png_combine_row(png_ptr, row, -1 /*ignored*/)?;
    }

    png_read_finish_row(png_ptr)?;

    /* (no read_row_fn) */
    Ok(())
}

/// Read the entire image.  If the image has an alpha channel or a tRNS
/// chunk, and you have called png_handle_alpha()[*], you will need to
/// initialize the image to the current image that PNG will be overlaying.
/// We set the num_rows again here, in case it was incorrectly set in
/// png_read_start_row() by a call to png_read_update_info() or
/// png_start_read_image() if png_set_interlace_handling() wasn't called
/// prior to either of these functions like it should have been.  You can
/// only call this function once.  If you desire to have an image for
/// each pass of a interlaced image, use png_read_rows() instead.
///
/// [*] png_handle_alpha() does not exist yet, as of this version of libpng
/// (`png_read_image`: `image` holds the row pointers)
pub(crate) fn png_read_image(
    png_ptr: &mut PngStruct<'_, '_>,
    image: &mut [&mut [u8]],
) -> PngResult<()> {
    let pass: i32;

    if (png_ptr.flags & PNG_FLAG_ROW_INIT) == 0 {
        pass = png_set_interlace_handling(png_ptr);
        /* And make sure transforms are initialized. */
        png_start_read_image(png_ptr)?;
    } else {
        if png_ptr.interlaced != 0 && (png_ptr.transformations & PNG_INTERLACE) == 0 {
            /* Caller called png_start_read_image or png_read_update_info without
             * first turning on the PNG_INTERLACE transform.  We can fix this here,
             * but the caller should do it!
             */
            png_warning(
                png_ptr,
                "Interlace handling should be turned on when using png_read_image",
            );
            /* Make sure this is set correctly */
            png_ptr.num_rows = png_ptr.height;
        }

        /* Obtain the pass number, which also turns on the PNG_INTERLACE flag in
         * the above error case.
         */
        pass = png_set_interlace_handling(png_ptr);
    }

    let image_height = png_ptr.height as usize;

    for _j in 0..pass {
        for rp in image.iter_mut().take(image_height) {
            png_read_row(png_ptr, Some(rp))?;
        }
    }
    Ok(())
}

/* (png_read_destroy() and png_destroy_read_struct(): dropping the structs;
 * zlib's inflate state goes with them, as inflateEnd() would free it) */
