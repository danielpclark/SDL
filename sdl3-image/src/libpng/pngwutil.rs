// Rust translation of pngwutil.c from libpng 1.6.59 (the chunks, row
// filtering and IDAT compression of SDL_image's APNG frame writer).
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngwutil.c - utilities to write a PNG file
//!
//! This file contains routines that are only called from within
//! libpng itself during the course of writing an image.
//!
//! (The ancillary chunk writers other than tRNS's, the text compression
//! and the interlacing are not translated: SDL_image's writes set none of
//! them, and are never interlaced.)

use super::png::*;
use super::pngerror::*;
use super::pngpriv::*;
use super::pngstruct::PngStruct;
use super::pngwio::png_write_data;
use crate::zlib::{
    deflate, deflate_end, deflate_init2, deflate_reset, Z_DEFAULT_STRATEGY, Z_FINISH, Z_NO_FLUSH,
    Z_OK, Z_STREAM_END, Z_STREAM_ERROR,
};

/// Place a 32-bit number into a buffer in PNG byte order.  We work
/// with unsigned numbers for convenience, although one supported
/// ancillary chunk uses signed (two's complement) numbers.
/// (`png_save_uint_32`)
pub(crate) fn png_save_uint_32(buf: &mut [u8], i: u32) {
    buf[0] = ((i >> 24) & 0xff) as u8;
    buf[1] = ((i >> 16) & 0xff) as u8;
    buf[2] = ((i >> 8) & 0xff) as u8;
    buf[3] = (i & 0xff) as u8;
}

/// Place a 16-bit number into a buffer in PNG byte order.
/// The parameter is declared unsigned int, not png_uint_16,
/// just to avoid potential problems on pre-ANSI C compilers.
/// (`png_save_uint_16`)
pub(crate) fn png_save_uint_16(buf: &mut [u8], i: u32) {
    buf[0] = ((i >> 8) & 0xff) as u8;
    buf[1] = (i & 0xff) as u8;
}

/// Simple function to write the signature.  If we have already written
/// the magic bytes of the signature, or more likely, the PNG stream is
/// being embedded into another stream and doesn't need its own signature,
/// we should call png_set_sig_bytes() to tell libpng how many of the
/// bytes have already been written. (`png_write_sig`)
pub(crate) fn png_write_sig(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    let png_signature: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

    /* Write the rest of the 8 byte signature */
    let sig_bytes = png_ptr.sig_bytes as usize;
    png_write_data(png_ptr, &png_signature[sig_bytes..])?;

    if png_ptr.sig_bytes < 3 {
        png_ptr.mode |= PNG_HAVE_PNG_SIGNATURE;
    }
    Ok(())
}

/// Write the start of a PNG chunk.  The type is the chunk type.
/// The total_length is the sum of the lengths of all the data you will be
/// passing in png_write_chunk_data(). (`png_write_chunk_header`)
fn png_write_chunk_header(
    png_ptr: &mut PngStruct<'_, '_>,
    chunk_name: u32,
    length: u32,
) -> PngResult<()> {
    let mut buf = [0u8; 8];

    /* Write the length and the chunk name */
    png_save_uint_32(&mut buf, length);
    png_save_uint_32(&mut buf[4..], chunk_name);
    png_write_data(png_ptr, &buf)?;

    /* Put the chunk name into png_ptr->chunk_name */
    png_ptr.chunk_name = chunk_name;

    /* Reset the crc and run it over the chunk name */
    png_reset_crc(png_ptr);

    png_calculate_crc(png_ptr, &buf[4..8]);
    Ok(())
}

/// Write the data of a PNG chunk started with png_write_chunk_header().
/// Note that multiple calls to this function are allowed, and that the
/// sum of the lengths from these calls *must* add up to the total_length
/// given to png_write_chunk_header(). (`png_write_chunk_data`)
pub(crate) fn png_write_chunk_data(png_ptr: &mut PngStruct<'_, '_>, data: &[u8]) -> PngResult<()> {
    /* Write the data, and run the CRC over it */
    if !data.is_empty() {
        png_write_data(png_ptr, data)?;

        /* Update the CRC after writing the data,
         * in case the user I/O routine alters it.
         */
        png_calculate_crc(png_ptr, data);
    }
    Ok(())
}

/// Finish a chunk started with png_write_chunk_header().
/// (`png_write_chunk_end`)
pub(crate) fn png_write_chunk_end(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    let mut buf = [0u8; 4];

    /* Write the crc in a single operation */
    png_save_uint_32(&mut buf, png_ptr.crc);

    png_write_data(png_ptr, &buf)
}

/// Write a PNG chunk all at once.  The type is an array of ASCII characters
/// representing the chunk name.  The array must be at least 4 bytes in
/// length, and does not need to be null terminated.  To be safe, pass the
/// pre-defined chunk names here, and if you need a new one, define it
/// where the others are defined.  The length is the length of the data.
/// All the data must be present.  If that is not possible, use the
/// png_write_chunk_start(), png_write_chunk_data(), and png_write_chunk_end()
/// functions instead. (`png_write_complete_chunk`)
fn png_write_complete_chunk(
    png_ptr: &mut PngStruct<'_, '_>,
    chunk_name: u32,
    data: &[u8],
) -> PngResult<()> {
    /* On 64-bit architectures 'length' may not fit in a png_uint_32. */
    if data.len() > PNG_UINT_31_MAX as usize {
        return Err(png_error(png_ptr, "length exceeds PNG maximum"));
    }

    png_write_chunk_header(png_ptr, chunk_name, data.len() as u32)?;
    png_write_chunk_data(png_ptr, data)?;
    png_write_chunk_end(png_ptr)
}

/// This is used below to find the size of an image to pass to png_deflate_claim,
/// so it only needs to be accurate if the size is less than 16384 bytes (the
/// point at which a lower LZ window size can be used.) (`png_image_size`)
fn png_image_size(png_ptr: &PngStruct<'_, '_>) -> usize {
    /* Only return sizes up to the maximum of a png_uint_32; do this by limiting
     * the width and height used to 15 bits.
     */
    let h = png_ptr.height;

    if png_ptr.rowbytes < 32768 && h < 32768 {
        /* (png_ptr->interlaced: never the case here) */
        (png_ptr.rowbytes + 1) * h as usize
    } else {
        0xffffffff
    }
}

/// This is the code to hack the first two bytes of the deflate stream (the
/// deflate header) to correct the windowBits value to match the actual data
/// size.  Note that the second argument is the *uncompressed* size but the
/// first argument is the *compressed* data (and it must be deflate
/// compressed.) (`optimize_cmf`)
fn optimize_cmf(data: &mut [u8], data_size: usize) {
    /* Optimize the CMF field in the zlib stream.  The resultant zlib stream is
     * still compliant to the stream specification.
     */
    if data_size <= 16384 {
        /* else windowBits must be 15 */
        let mut z_cmf = data[0] as u32; /* zlib compression method and flags */

        if (z_cmf & 0x0f) == 8 && (z_cmf & 0xf0) <= 0x70 {
            let mut z_cinfo: u32;
            let mut half_z_window_size: u32;

            z_cinfo = z_cmf >> 4;
            half_z_window_size = 1u32 << (z_cinfo + 7);

            if data_size <= half_z_window_size as usize {
                /* else no change */
                let mut tmp: u32;

                loop {
                    half_z_window_size >>= 1;
                    z_cinfo -= 1;
                    if !(z_cinfo > 0 && data_size <= half_z_window_size as usize) {
                        break;
                    }
                }

                z_cmf = (z_cmf & 0x0f) | (z_cinfo << 4);

                data[0] = z_cmf as u8;
                tmp = data[1] as u32 & 0xe0;
                tmp += 0x1f - ((z_cmf << 8) + tmp) % 0x1f;
                data[1] = tmp as u8;
            }
        }
    }
}

/// Initialize the compressor for the appropriate type of compression.
/// (`png_deflate_claim`)
fn png_deflate_claim(
    png_ptr: &mut PngStruct<'_, '_>,
    owner: u32,
    data_size: usize,
) -> PngResult<i32> {
    if png_ptr.zowner != 0 {
        /* So the message that results is "<chunk> using zstream"; this is an
         * internal error, but is very useful for debugging.  i18n requirements
         * are minimal.
         */
        let msg = format!(
            "{}: {} using zstream",
            String::from_utf8_lossy(&png_string_from_chunk(owner)),
            String::from_utf8_lossy(&png_string_from_chunk(png_ptr.zowner))
        );
        png_warning(png_ptr, &msg);

        /* Attempt sane error recovery */
        if png_ptr.zowner == png_IDAT {
            /* don't steal from IDAT */
            png_ptr.zstream.msg = Some("in use by IDAT");
            return Ok(Z_STREAM_ERROR);
        }

        png_ptr.zowner = 0;
    }

    let level = png_ptr.zlib_level;
    let method = png_ptr.zlib_method;
    let mut window_bits = png_ptr.zlib_window_bits;
    let mem_level = png_ptr.zlib_mem_level;
    let strategy: i32; /* set below */
    let ret: i32; /* zlib return code */

    if owner == png_IDAT {
        if (png_ptr.flags & PNG_FLAG_ZLIB_CUSTOM_STRATEGY) != 0 {
            strategy = png_ptr.zlib_strategy;
        } else if png_ptr.do_filter != PNG_FILTER_NONE {
            strategy = PNG_Z_DEFAULT_STRATEGY;
        } else {
            strategy = PNG_Z_DEFAULT_NOFILTER_STRATEGY;
        }
    } else {
        /* (PNG_WRITE_CUSTOMIZE_ZTXT_COMPRESSION: the text settings; only the
         * IDAT claims the stream here) */
        strategy = Z_DEFAULT_STRATEGY;
    }

    /* Adjust 'windowBits' down if larger than 'data_size'; to stop this
     * happening just pass 32768 as the data_size parameter.  Notice that zlib
     * requires an extra 262 bytes in the window in addition to the data to be
     * able to see the whole of the data, so if data_size+262 takes us to the
     * next windowBits size we need to fix up the value later.  (Because even
     * though deflate needs the extra window, inflate does not!)
     */
    if data_size <= 16384 {
        /* IMPLEMENTATION NOTE: this 'half_window_size' stuff is only here to
         * work round a Microsoft Visual C misbehavior which, contrary to C-90,
         * widens the result of the following shift to 64-bits if (and,
         * apparently, only if) it is used in a test.
         */
        let mut half_window_size: u32 = 1u32 << (window_bits - 1);

        while data_size + 262 <= half_window_size as usize {
            half_window_size >>= 1;
            window_bits -= 1;
        }
    }

    /* Check against the previous initialized values, if any. */
    // FIXME (upstream): libpng never assigns zlib_set_*, so a second claim
    // with any non-zero setting always ends and reinitializes the stream
    // (each struct here claims it once).
    if (png_ptr.flags & PNG_FLAG_ZSTREAM_INITIALIZED) != 0
        && (png_ptr.zlib_set_level != level
            || png_ptr.zlib_set_method != method
            || png_ptr.zlib_set_window_bits != window_bits
            || png_ptr.zlib_set_mem_level != mem_level
            || png_ptr.zlib_set_strategy != strategy)
    {
        if deflate_end(&mut png_ptr.zstream) != Z_OK {
            png_warning(png_ptr, "deflateEnd failed (ignored)");
        }

        png_ptr.flags &= !PNG_FLAG_ZSTREAM_INITIALIZED;
    }

    /* For safety clear out the input and output pointers (currently zlib
     * doesn't use them on Init, but it might in the future).
     */
    png_ptr.zstream.next_in = 0;
    png_ptr.zstream.avail_in = 0;
    png_ptr.zstream.next_out = 0;
    png_ptr.zstream.avail_out = 0;

    /* Now initialize if required, setting the new parameters, otherwise just
     * do a simple reset to the previous parameters.
     */
    if (png_ptr.flags & PNG_FLAG_ZSTREAM_INITIALIZED) != 0 {
        ret = deflate_reset(&mut png_ptr.zstream);
    } else {
        let r = deflate_init2(
            &mut png_ptr.zstream,
            level,
            method,
            window_bits,
            mem_level,
            strategy,
        );

        if r == Z_OK {
            png_ptr.flags |= PNG_FLAG_ZSTREAM_INITIALIZED;
        }
        ret = r;
    }

    /* The return code is from either deflateReset or deflateInit2; they have
     * pretty much the same set of error codes.
     */
    if ret == Z_OK {
        png_ptr.zowner = owner;
    } else {
        png_zstream_error(png_ptr, ret);
    }

    Ok(ret)
}

/* (png_free_buffer_list(): the buffer is dropped) */

/// Write the IHDR chunk, and update the png_struct with the necessary
/// information.  Note that the rest of this code depends upon this
/// information being correct. (`png_write_IHDR`)
#[allow(non_snake_case, clippy::too_many_arguments)]
pub(crate) fn png_write_IHDR(
    png_ptr: &mut PngStruct<'_, '_>,
    width: u32,
    height: u32,
    bit_depth: i32,
    color_type: i32,
    mut compression_type: i32,
    mut filter_type: i32,
    mut interlace_type: i32,
) -> PngResult<()> {
    let mut buf = [0u8; 13]; /* Buffer to store the IHDR info */
    let is_invalid_depth: bool;

    /* Check that we have valid input data from the application info */
    match color_type as u8 {
        PNG_COLOR_TYPE_GRAY => match bit_depth {
            1 | 2 | 4 | 8 | 16 => png_ptr.channels = 1,

            _ => return Err(png_error(png_ptr, "Invalid bit depth for grayscale image")),
        },

        PNG_COLOR_TYPE_RGB => {
            is_invalid_depth = bit_depth != 8 && bit_depth != 16;
            if is_invalid_depth {
                return Err(png_error(png_ptr, "Invalid bit depth for RGB image"));
            }

            png_ptr.channels = 3;
        }

        PNG_COLOR_TYPE_PALETTE => match bit_depth {
            1 | 2 | 4 | 8 => png_ptr.channels = 1,

            _ => return Err(png_error(png_ptr, "Invalid bit depth for paletted image")),
        },

        PNG_COLOR_TYPE_GRAY_ALPHA => {
            is_invalid_depth = bit_depth != 8 && bit_depth != 16;
            if is_invalid_depth {
                return Err(png_error(
                    png_ptr,
                    "Invalid bit depth for grayscale+alpha image",
                ));
            }

            png_ptr.channels = 2;
        }

        PNG_COLOR_TYPE_RGB_ALPHA => {
            is_invalid_depth = bit_depth != 8 && bit_depth != 16;
            if is_invalid_depth {
                return Err(png_error(png_ptr, "Invalid bit depth for RGBA image"));
            }

            png_ptr.channels = 4;
        }

        _ => return Err(png_error(png_ptr, "Invalid image color type specified")),
    }

    if compression_type != PNG_COMPRESSION_TYPE_BASE {
        png_warning(png_ptr, "Invalid compression type specified");
        compression_type = PNG_COMPRESSION_TYPE_BASE;
    }

    /* Write filter_method 64 (intrapixel differencing) only if
     * 1. Libpng was compiled with PNG_MNG_FEATURES_SUPPORTED and
     * 2. Libpng did not write a PNG signature (this filter_method is only
     *    used in PNG datastreams that are embedded in MNG datastreams) and
     * 3. The application called png_permit_mng_features with a mask that
     *    included PNG_FLAG_MNG_FILTER_64 and
     * 4. The filter_method is 64 and
     * 5. The color_type is RGB or RGBA
     */
    if !((png_ptr.mng_features_permitted & PNG_FLAG_MNG_FILTER_64) != 0
        && ((png_ptr.mode & PNG_HAVE_PNG_SIGNATURE) == 0)
        && (color_type == PNG_COLOR_TYPE_RGB as i32
            || color_type == PNG_COLOR_TYPE_RGB_ALPHA as i32)
        && (filter_type == PNG_INTRAPIXEL_DIFFERENCING))
        && filter_type != PNG_FILTER_TYPE_BASE
    {
        png_warning(png_ptr, "Invalid filter type specified");
        filter_type = PNG_FILTER_TYPE_BASE;
    }

    if interlace_type != PNG_INTERLACE_NONE && interlace_type != 1
    /* PNG_INTERLACE_ADAM7 */
    {
        png_warning(png_ptr, "Invalid interlace type specified");
        interlace_type = 1;
    }

    /* Save the relevant information */
    png_ptr.bit_depth = bit_depth as u8;
    png_ptr.color_type = color_type as u8;
    png_ptr.interlaced = interlace_type as u8;
    png_ptr.filter_type = filter_type as u8;
    png_ptr.compression = compression_type as u8;
    png_ptr.width = width;
    png_ptr.height = height;

    png_ptr.pixel_depth = (bit_depth as u8).wrapping_mul(png_ptr.channels);
    png_ptr.rowbytes = png_rowbytes(png_ptr.pixel_depth as u32, width as usize);
    /* Set the usr info, so any transformations can modify it */
    png_ptr.usr_width = png_ptr.width;
    png_ptr.usr_bit_depth = png_ptr.bit_depth;
    png_ptr.usr_channels = png_ptr.channels;

    /* Pack the header information into the buffer */
    png_save_uint_32(&mut buf, width);
    png_save_uint_32(&mut buf[4..], height);
    buf[8] = bit_depth as u8;
    buf[9] = color_type as u8;
    buf[10] = compression_type as u8;
    buf[11] = filter_type as u8;
    buf[12] = interlace_type as u8;

    /* Write the chunk */
    png_write_complete_chunk(png_ptr, png_IHDR, &buf)?;

    if png_ptr.do_filter == PNG_NO_FILTERS {
        if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE || png_ptr.bit_depth < 8 {
            png_ptr.do_filter = PNG_FILTER_NONE;
        } else {
            png_ptr.do_filter = PNG_ALL_FILTERS;
        }
    }

    png_ptr.mode = PNG_HAVE_IHDR; /* not READY_FOR_ZTXT */
    Ok(())
}

/// Write the palette.  We are careful not to trust png_color to be in the
/// correct order for PNG, so people can redefine it to any convenient
/// structure. (`png_write_PLTE`)
#[allow(non_snake_case)]
pub(crate) fn png_write_PLTE(
    png_ptr: &mut PngStruct<'_, '_>,
    palette: &[PngColor],
    num_pal: u32,
) -> PngResult<()> {
    let mut buf = [0u8; 3];

    let max_palette_length: u32 = if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
        1 << png_ptr.bit_depth
    } else {
        PNG_MAX_PALETTE_LENGTH as u32
    };

    if ((png_ptr.mng_features_permitted & PNG_FLAG_MNG_EMPTY_PLTE) == 0 && num_pal == 0)
        || num_pal > max_palette_length
    {
        if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
            return Err(png_error(png_ptr, "Invalid number of colors in palette"));
        } else {
            png_warning(png_ptr, "Invalid number of colors in palette");
            return Ok(());
        }
    }

    if (png_ptr.color_type & PNG_COLOR_MASK_COLOR) == 0 {
        png_warning(
            png_ptr,
            "Ignoring request to write a PLTE chunk in grayscale PNG",
        );

        return Ok(());
    }

    png_ptr.num_palette = num_pal as u16;

    png_write_chunk_header(png_ptr, png_PLTE, num_pal * 3)?;

    for pal_ptr in palette.iter().take(num_pal as usize) {
        buf[0] = pal_ptr.red;
        buf[1] = pal_ptr.green;
        buf[2] = pal_ptr.blue;
        png_write_chunk_data(png_ptr, &buf)?;
    }

    png_write_chunk_end(png_ptr)?;
    png_ptr.mode |= PNG_HAVE_PLTE;
    Ok(())
}

/// This is similar to png_text_compress, above, except that it does not require
/// all of the data at once and, instead of buffering the compressed result,
/// writes it as IDAT chunks.  Unlike png_text_compress it *can* png_error out
/// because it calls the write interface.  As a result it does its own error
/// reporting and does not return an error code.  In the event of error it will
/// just call png_error.  The input data length may exceed 32-bits.  The 'flush'
/// parameter is exactly the same as that to deflate, with the following
/// meanings:
///
/// Z_NO_FLUSH: normal incremental output of compressed data
/// Z_SYNC_FLUSH: do a SYNC_FLUSH, used by png_write_flush
/// Z_FINISH: this is the end of the input, do a Z_FINISH and clean up
///
/// The routine manages the acquire and release of the png_ptr->zstream by
/// checking and (at the end) clearing png_ptr->zowner; it does some sanity
/// checks on the 'mode' flags while doing this. (`png_compress_IDAT`)
#[allow(non_snake_case)]
pub(crate) fn png_compress_IDAT(
    png_ptr: &mut PngStruct<'_, '_>,
    input: &[u8],
    flush: i32,
) -> PngResult<()> {
    let mut input_len = input.len();

    if png_ptr.zowner != png_IDAT {
        /* First time.   Ensure we have a temporary buffer for compression and
         * trim the buffer list if it has more than one entry to free memory.
         * If 'WRITE_COMPRESSED_TEXT' is not set the list will never have been
         * created at this point, but the check here is quick and safe.
         */
        if png_ptr.zbuffer_list.is_none() {
            let size = png_ptr.zbuffer_size as usize;
            png_ptr.zbuffer_list = Some(super::pngmem::png_malloc(png_ptr, size)?);
        }

        /* It is a terminal error if we can't claim the zstream. */
        let image_size = png_image_size(png_ptr);
        if png_deflate_claim(png_ptr, png_IDAT, image_size)? != Z_OK {
            let msg = png_ptr.zstream.msg.unwrap_or("");
            return Err(png_error(png_ptr, msg));
        }

        /* The output state is maintained in png_ptr->zstream, so it must be
         * initialized here after the claim.
         */
        png_ptr.zstream.next_out = 0;
        png_ptr.zstream.avail_out = png_ptr.zbuffer_size;
    }

    /* Now loop reading and writing until all the input is consumed or an error
     * terminates the operation.  The _out values are maintained across calls to
     * this function, but the input must be reset each time.
     */
    png_ptr.zstream.next_in = 0;
    png_ptr.zstream.avail_in = 0; /* set below */
    loop {
        let ret: i32;

        /* INPUT: from the row data */
        let mut avail: u32 = ZLIB_IO_MAX;

        if avail as usize > input_len {
            avail = input_len as u32; /* safe because of the check */
        }

        png_ptr.zstream.avail_in = avail;
        input_len -= avail as usize;

        let mut zbuf = png_ptr.zbuffer_list.take().unwrap_or_default();
        ret = deflate(
            &mut png_ptr.zstream,
            input,
            &mut zbuf,
            if input_len > 0 { Z_NO_FLUSH } else { flush },
        );

        /* Include as-yet unconsumed input */
        input_len += png_ptr.zstream.avail_in as usize;
        png_ptr.zstream.avail_in = 0;

        /* OUTPUT: write complete IDAT chunks when avail_out drops to zero. Note
         * that these two zstream fields are preserved across the calls, therefore
         * there is no need to set these up on entry to the loop.
         */
        if png_ptr.zstream.avail_out == 0 {
            let size = png_ptr.zbuffer_size as usize;

            /* Write an IDAT containing the data then reset the buffer.  The
             * first IDAT may need deflate header optimization.
             */
            if (png_ptr.mode & PNG_HAVE_IDAT) == 0
                && png_ptr.compression as i32 == PNG_COMPRESSION_TYPE_BASE
            {
                optimize_cmf(&mut zbuf, png_image_size(png_ptr));
            }

            if size > 0 {
                let r = png_write_complete_chunk(png_ptr, png_IDAT, &zbuf[..size]);
                if r.is_err() {
                    png_ptr.zbuffer_list = Some(zbuf);
                    return r;
                }
            }
            png_ptr.mode |= PNG_HAVE_IDAT;

            png_ptr.zstream.next_out = 0;
            png_ptr.zstream.avail_out = size as u32;
            png_ptr.zbuffer_list = Some(zbuf);

            /* For SYNC_FLUSH or FINISH it is essential to keep calling zlib with
             * the same flush parameter until it has finished output, for NO_FLUSH
             * it doesn't matter.
             */
            if ret == Z_OK && flush != Z_NO_FLUSH {
                continue;
            }
        } else {
            png_ptr.zbuffer_list = Some(zbuf);
        }

        /* The order of these checks doesn't matter much; it just affects which
         * possible error might be detected if multiple things go wrong at once.
         */
        if ret == Z_OK {
            /* most likely return code! */
            /* If all the input has been consumed then just return.  If Z_FINISH
             * was used as the flush parameter something has gone wrong if we get
             * here.
             */
            if input_len == 0 {
                if flush == Z_FINISH {
                    return Err(png_error(png_ptr, "Z_OK on Z_FINISH with output space"));
                }

                return Ok(());
            }
        } else if ret == Z_STREAM_END && flush == Z_FINISH {
            /* This is the end of the IDAT data; any pending output must be
             * flushed.  For small PNG files we may still be at the beginning.
             */
            let mut zbuf = png_ptr.zbuffer_list.take().unwrap_or_default();
            let size = (png_ptr.zbuffer_size - png_ptr.zstream.avail_out) as usize;

            if (png_ptr.mode & PNG_HAVE_IDAT) == 0
                && png_ptr.compression as i32 == PNG_COMPRESSION_TYPE_BASE
            {
                optimize_cmf(&mut zbuf, png_image_size(png_ptr));
            }

            if size > 0 {
                let r = png_write_complete_chunk(png_ptr, png_IDAT, &zbuf[..size]);
                if r.is_err() {
                    png_ptr.zbuffer_list = Some(zbuf);
                    return r;
                }
            }
            png_ptr.zbuffer_list = Some(zbuf);
            png_ptr.zstream.avail_out = 0;
            png_ptr.zstream.next_out = 0;
            png_ptr.mode |= PNG_HAVE_IDAT | PNG_AFTER_IDAT;

            png_ptr.zowner = 0; /* Release the stream */
            return Ok(());
        } else {
            /* This is an error condition. */
            png_zstream_error(png_ptr, ret);
            let msg = png_ptr.zstream.msg.unwrap_or("");
            return Err(png_error(png_ptr, msg));
        }
    }
}

/// Write an IEND chunk (`png_write_IEND`)
#[allow(non_snake_case)]
pub(crate) fn png_write_IEND(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    png_write_complete_chunk(png_ptr, png_IEND, &[])?;
    png_ptr.mode |= PNG_HAVE_IEND;
    Ok(())
}

/// Write the tRNS chunk (`png_write_tRNS`)
#[allow(non_snake_case)]
pub(crate) fn png_write_tRNS(
    png_ptr: &mut PngStruct<'_, '_>,
    trans_alpha: &[u8],
    tran: &PngColor16,
    num_trans: i32,
    color_type: i32,
) -> PngResult<()> {
    let mut buf = [0u8; 6];

    if color_type == PNG_COLOR_TYPE_PALETTE as i32 {
        if num_trans <= 0 || num_trans > png_ptr.num_palette as i32 {
            return png_app_warning(png_ptr, "Invalid number of transparent colors specified");
        }

        /* Write the chunk out as it is */
        png_write_complete_chunk(png_ptr, png_tRNS, &trans_alpha[..num_trans as usize])?;
    } else if color_type == PNG_COLOR_TYPE_GRAY as i32 {
        /* One 16-bit value */
        if tran.gray as u32 >= (1u32 << png_ptr.bit_depth) {
            return png_app_warning(
                png_ptr,
                "Ignoring attempt to write tRNS chunk out-of-range for bit_depth",
            );
        }

        png_save_uint_16(&mut buf, tran.gray as u32);
        png_write_complete_chunk(png_ptr, png_tRNS, &buf[..2])?;
    } else if color_type == PNG_COLOR_TYPE_RGB as i32 {
        /* Three 16-bit values */
        png_save_uint_16(&mut buf, tran.red as u32);
        png_save_uint_16(&mut buf[2..], tran.green as u32);
        png_save_uint_16(&mut buf[4..], tran.blue as u32);
        if png_ptr.bit_depth == 8 && (buf[0] | buf[2] | buf[4]) != 0 {
            return png_app_warning(
                png_ptr,
                "Ignoring attempt to write 16-bit tRNS chunk when bit_depth is 8",
            );
        }

        png_write_complete_chunk(png_ptr, png_tRNS, &buf)?;
    } else {
        png_app_warning(png_ptr, "Can't write tRNS with an alpha channel")?;
    }
    Ok(())
}

/// Initializes the row writing capability of libpng (`png_write_start_row`)
pub(crate) fn png_write_start_row(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    let usr_pixel_depth = png_ptr.usr_channels as u32 * png_ptr.usr_bit_depth as u32;
    let buf_size = png_rowbytes(usr_pixel_depth, png_ptr.width as usize) + 1;

    /* 1.5.6: added to allow checking in the row write code. */
    png_ptr.transformed_pixel_depth = png_ptr.pixel_depth;
    png_ptr.maximum_pixel_depth = usr_pixel_depth as u8;

    /* Set up row buffer */
    png_ptr.row_buf = super::pngmem::png_malloc(png_ptr, buf_size)?;

    png_ptr.row_buf[0] = PNG_FILTER_VALUE_NONE;

    let mut filters = png_ptr.do_filter;

    if png_ptr.height == 1 {
        filters &= !(PNG_FILTER_UP | PNG_FILTER_AVG | PNG_FILTER_PAETH);
    }

    if png_ptr.width == 1 {
        filters &= !(PNG_FILTER_SUB | PNG_FILTER_AVG | PNG_FILTER_PAETH);
    }

    if filters == 0 {
        filters = PNG_FILTER_NONE;
    }

    png_ptr.do_filter = filters;

    if (filters & (PNG_FILTER_SUB | PNG_FILTER_UP | PNG_FILTER_AVG | PNG_FILTER_PAETH)) != 0
        && png_ptr.try_row.is_none()
    {
        let mut num_filters = 0;

        png_ptr.try_row = Some(super::pngmem::png_malloc(png_ptr, buf_size)?);

        if (filters & PNG_FILTER_SUB) != 0 {
            num_filters += 1;
        }

        if (filters & PNG_FILTER_UP) != 0 {
            num_filters += 1;
        }

        if (filters & PNG_FILTER_AVG) != 0 {
            num_filters += 1;
        }

        if (filters & PNG_FILTER_PAETH) != 0 {
            num_filters += 1;
        }

        if num_filters > 1 {
            png_ptr.tst_row = Some(super::pngmem::png_malloc(png_ptr, buf_size)?);
        }
    }

    /* We only need to keep the previous row if we are using one of the following
     * filters.
     */
    if (filters & (PNG_FILTER_AVG | PNG_FILTER_UP | PNG_FILTER_PAETH)) != 0 {
        png_ptr.prev_row = Some(super::pngmem::png_calloc(png_ptr, buf_size)?);
    }

    /* (png_ptr->interlaced: never the case here) */
    png_ptr.num_rows = png_ptr.height;
    png_ptr.usr_width = png_ptr.width;
    Ok(())
}

/// Internal use only.  Called when finished processing a row of data.
/// (`png_write_finish_row`)
pub(crate) fn png_write_finish_row(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    /* Next row */
    png_ptr.row_number += 1;

    /* See if we are done */
    if png_ptr.row_number < png_ptr.num_rows {
        return Ok(());
    }

    /* (png_ptr->interlaced: the next pass; never the case here) */

    /* If we get here, we've just written the last row, so we need
    to flush the compressor */
    png_compress_IDAT(png_ptr, &[], Z_FINISH)
}

/// The row a filter search picked: one of the struct's row buffers
/// (C's `best_row` pointer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BestRow {
    RowBuf,
    TryRow,
    TstRow,
}

/// `png_setup_sub_row`
fn png_setup_sub_row(
    png_ptr: &mut PngStruct<'_, '_>,
    bpp: usize,
    row_bytes: usize,
    lmins: usize,
) -> usize {
    let rb = &png_ptr.row_buf;
    let tr = png_ptr.try_row.as_mut().unwrap();
    let mut sum: usize = 0;
    let mut v: u32;

    tr[0] = PNG_FILTER_VALUE_SUB;

    let mut i = 0;
    while i < bpp {
        tr[i + 1] = rb[i + 1];
        v = tr[i + 1] as u32;
        sum += if v < 128 { v } else { 256 - v } as usize;
        i += 1;
    }

    while i < row_bytes {
        tr[i + 1] = ((rb[i + 1] as i32 - rb[i + 1 - bpp] as i32) & 0xff) as u8;
        v = tr[i + 1] as u32;
        sum += if v < 128 { v } else { 256 - v } as usize;

        if sum > lmins {
            /* We are already worse, don't continue. */
            break;
        }
        i += 1;
    }

    sum
}

/// `png_setup_sub_row_only`
fn png_setup_sub_row_only(png_ptr: &mut PngStruct<'_, '_>, bpp: usize, row_bytes: usize) {
    let rb = &png_ptr.row_buf;
    let tr = png_ptr.try_row.as_mut().unwrap();

    tr[0] = PNG_FILTER_VALUE_SUB;

    let mut i = 0;
    while i < bpp {
        tr[i + 1] = rb[i + 1];
        i += 1;
    }

    while i < row_bytes {
        tr[i + 1] = ((rb[i + 1] as i32 - rb[i + 1 - bpp] as i32) & 0xff) as u8;
        i += 1;
    }
}

/// `png_setup_up_row`
fn png_setup_up_row(png_ptr: &mut PngStruct<'_, '_>, row_bytes: usize, lmins: usize) -> usize {
    let rb = &png_ptr.row_buf;
    let pp = png_ptr.prev_row.as_ref().unwrap();
    let tr = png_ptr.try_row.as_mut().unwrap();
    let mut sum: usize = 0;
    let mut v: u32;

    tr[0] = PNG_FILTER_VALUE_UP;

    for i in 0..row_bytes {
        tr[i + 1] = ((rb[i + 1] as i32 - pp[i + 1] as i32) & 0xff) as u8;
        v = tr[i + 1] as u32;
        sum += if v < 128 { v } else { 256 - v } as usize;

        if sum > lmins {
            /* We are already worse, don't continue. */
            break;
        }
    }

    sum
}

/// `png_setup_up_row_only`
fn png_setup_up_row_only(png_ptr: &mut PngStruct<'_, '_>, row_bytes: usize) {
    let rb = &png_ptr.row_buf;
    let pp = png_ptr.prev_row.as_ref().unwrap();
    let tr = png_ptr.try_row.as_mut().unwrap();

    tr[0] = PNG_FILTER_VALUE_UP;

    for i in 0..row_bytes {
        tr[i + 1] = ((rb[i + 1] as i32 - pp[i + 1] as i32) & 0xff) as u8;
    }
}

/// `png_setup_avg_row`
fn png_setup_avg_row(
    png_ptr: &mut PngStruct<'_, '_>,
    bpp: usize,
    row_bytes: usize,
    lmins: usize,
) -> usize {
    let rb = &png_ptr.row_buf;
    let pp = png_ptr.prev_row.as_ref().unwrap();
    let tr = png_ptr.try_row.as_mut().unwrap();
    let mut sum: usize = 0;
    let mut v: u32;

    tr[0] = PNG_FILTER_VALUE_AVG;

    let mut i = 0;
    while i < bpp {
        tr[i + 1] = ((rb[i + 1] as i32 - (pp[i + 1] as i32 / 2)) & 0xff) as u8;
        v = tr[i + 1] as u32;
        sum += if v < 128 { v } else { 256 - v } as usize;
        i += 1;
    }

    while i < row_bytes {
        tr[i + 1] =
            ((rb[i + 1] as i32 - ((pp[i + 1] as i32 + rb[i + 1 - bpp] as i32) / 2)) & 0xff) as u8;
        v = tr[i + 1] as u32;
        sum += if v < 128 { v } else { 256 - v } as usize;

        if sum > lmins {
            /* We are already worse, don't continue. */
            break;
        }
        i += 1;
    }

    sum
}

/// `png_setup_avg_row_only`
fn png_setup_avg_row_only(png_ptr: &mut PngStruct<'_, '_>, bpp: usize, row_bytes: usize) {
    let rb = &png_ptr.row_buf;
    let pp = png_ptr.prev_row.as_ref().unwrap();
    let tr = png_ptr.try_row.as_mut().unwrap();

    tr[0] = PNG_FILTER_VALUE_AVG;

    let mut i = 0;
    while i < bpp {
        tr[i + 1] = ((rb[i + 1] as i32 - (pp[i + 1] as i32 / 2)) & 0xff) as u8;
        i += 1;
    }

    while i < row_bytes {
        tr[i + 1] =
            ((rb[i + 1] as i32 - ((pp[i + 1] as i32 + rb[i + 1 - bpp] as i32) / 2)) & 0xff) as u8;
        i += 1;
    }
}

/// The Paeth predictor of the filter setups.
fn paeth_predictor(a: i32, b: i32, c: i32) -> i32 {
    let p = b - c;
    let mut pc = a - c;

    let pa = if p < 0 { -p } else { p };
    let pb = if pc < 0 { -pc } else { pc };
    pc = if (p + pc) < 0 { -(p + pc) } else { p + pc };

    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// `png_setup_paeth_row`
fn png_setup_paeth_row(
    png_ptr: &mut PngStruct<'_, '_>,
    bpp: usize,
    row_bytes: usize,
    lmins: usize,
) -> usize {
    let rb = &png_ptr.row_buf;
    let pp = png_ptr.prev_row.as_ref().unwrap();
    let tr = png_ptr.try_row.as_mut().unwrap();
    let mut sum: usize = 0;
    let mut v: u32;

    tr[0] = PNG_FILTER_VALUE_PAETH;

    let mut i = 0;
    while i < bpp {
        tr[i + 1] = ((rb[i + 1] as i32 - pp[i + 1] as i32) & 0xff) as u8;
        v = tr[i + 1] as u32;
        sum += if v < 128 { v } else { 256 - v } as usize;
        i += 1;
    }

    while i < row_bytes {
        let b = pp[i + 1] as i32;
        let c = pp[i + 1 - bpp] as i32;
        let a = rb[i + 1 - bpp] as i32;

        let p = paeth_predictor(a, b, c);

        tr[i + 1] = ((rb[i + 1] as i32 - p) & 0xff) as u8;
        v = tr[i + 1] as u32;
        sum += if v < 128 { v } else { 256 - v } as usize;

        if sum > lmins {
            /* We are already worse, don't continue. */
            break;
        }
        i += 1;
    }

    sum
}

/// `png_setup_paeth_row_only`
fn png_setup_paeth_row_only(png_ptr: &mut PngStruct<'_, '_>, bpp: usize, row_bytes: usize) {
    let rb = &png_ptr.row_buf;
    let pp = png_ptr.prev_row.as_ref().unwrap();
    let tr = png_ptr.try_row.as_mut().unwrap();

    tr[0] = PNG_FILTER_VALUE_PAETH;

    let mut i = 0;
    while i < bpp {
        tr[i + 1] = ((rb[i + 1] as i32 - pp[i + 1] as i32) & 0xff) as u8;
        i += 1;
    }

    while i < row_bytes {
        let b = pp[i + 1] as i32;
        let c = pp[i + 1 - bpp] as i32;
        let a = rb[i + 1 - bpp] as i32;

        let p = paeth_predictor(a, b, c);

        tr[i + 1] = ((rb[i + 1] as i32 - p) & 0xff) as u8;
        i += 1;
    }
}

/// The trial row was better: it is the best row, and the search goes on in
/// the other trial buffer (C swaps `try_row` and `tst_row`).
fn take_try_row(png_ptr: &mut PngStruct<'_, '_>) -> BestRow {
    if png_ptr.tst_row.is_some() {
        std::mem::swap(&mut png_ptr.try_row, &mut png_ptr.tst_row);
        BestRow::TstRow
    } else {
        BestRow::TryRow
    }
}

/// `png_write_find_filter`
pub(crate) fn png_write_find_filter(
    png_ptr: &mut PngStruct<'_, '_>,
    row_info: &PngRowInfo,
) -> PngResult<()> {
    let mut filter_to_do = png_ptr.do_filter as u32;
    let mut best_row: BestRow;
    let bpp: usize;
    let mut mins: usize;
    let row_bytes = row_info.rowbytes;

    /* Find out how many bytes offset each pixel is */
    bpp = ((row_info.pixel_depth as usize) + 7) >> 3;

    mins = PNG_SIZE_MAX - 256; /* so we can detect potential overflow of the
                               running sum */

    /* The prediction method we use is to find which method provides the
     * smallest value when summing the absolute values of the distances
     * from zero, using anything >= 128 as negative numbers.  This is known
     * as the "minimum sum of absolute differences" heuristic.  Other
     * heuristics are the "weighted minimum sum of absolute differences"
     * (experimental and can in theory improve compression), and the "zlib
     * predictive" method (not implemented yet), which does test compressions
     * of lines using different filter methods, and then chooses the
     * (series of) filter(s) that give minimum compressed data size (VERY
     * computationally expensive).
     *
     * GRR 980525:  consider also
     *
     *   (1) minimum sum of absolute differences from running average (i.e.,
     *       keep running sum of non-absolute differences & count of bytes)
     *       [track dispersion, too?  restart average if dispersion too large?]
     *
     *  (1b) minimum sum of absolute differences from sliding average, probably
     *       with window size <= deflate window (usually 32K)
     *
     *   (2) minimum sum of squared differences from zero or running average
     *       (i.e., ~ root-mean-square approach)
     */

    /* We don't need to test the 'no filter' case if this is the only filter
     * that has been chosen, as it doesn't actually do anything to the data.
     */
    best_row = BestRow::RowBuf;

    if PNG_SIZE_MAX / 128 <= row_bytes {
        /* Overflow can occur in the calculation, just select the lowest set
         * filter.
         */
        filter_to_do &= 0u32.wrapping_sub(filter_to_do);
    } else if (filter_to_do & PNG_FILTER_NONE as u32) != 0 && filter_to_do != PNG_FILTER_NONE as u32
    {
        /* Overflow not possible and multiple filters in the list, including the
         * 'none' filter.
         */
        let mut sum: usize = 0;

        for &v in &png_ptr.row_buf[1..1 + row_bytes] {
            let v = v as u32;
            sum += if v < 128 { v } else { 256 - v } as usize;
        }

        mins = sum;
    }

    /* Sub filter */
    if filter_to_do == PNG_FILTER_SUB as u32 {
        /* It's the only filter so no testing is needed */
        png_setup_sub_row_only(png_ptr, bpp, row_bytes);
        best_row = BestRow::TryRow;
    } else if (filter_to_do & PNG_FILTER_SUB as u32) != 0 {
        let lmins = mins;

        let sum = png_setup_sub_row(png_ptr, bpp, row_bytes, lmins);

        if sum < mins {
            mins = sum;
            best_row = take_try_row(png_ptr);
        }
    }

    /* Up filter */
    if filter_to_do == PNG_FILTER_UP as u32 {
        png_setup_up_row_only(png_ptr, row_bytes);
        best_row = BestRow::TryRow;
    } else if (filter_to_do & PNG_FILTER_UP as u32) != 0 {
        let lmins = mins;

        let sum = png_setup_up_row(png_ptr, row_bytes, lmins);

        if sum < mins {
            mins = sum;
            best_row = take_try_row(png_ptr);
        }
    }

    /* Avg filter */
    if filter_to_do == PNG_FILTER_AVG as u32 {
        png_setup_avg_row_only(png_ptr, bpp, row_bytes);
        best_row = BestRow::TryRow;
    } else if (filter_to_do & PNG_FILTER_AVG as u32) != 0 {
        let lmins = mins;

        let sum = png_setup_avg_row(png_ptr, bpp, row_bytes, lmins);

        if sum < mins {
            mins = sum;
            best_row = take_try_row(png_ptr);
        }
    }

    /* Paeth filter */
    if filter_to_do == PNG_FILTER_PAETH as u32 {
        png_setup_paeth_row_only(png_ptr, bpp, row_bytes);
        best_row = BestRow::TryRow;
    } else if (filter_to_do & PNG_FILTER_PAETH as u32) != 0 {
        let lmins = mins;

        let sum = png_setup_paeth_row(png_ptr, bpp, row_bytes, lmins);

        if sum < mins {
            best_row = take_try_row(png_ptr);
        }
    }

    /* Do the actual writing of the filtered row data from the chosen filter. */
    png_write_filtered_row(png_ptr, best_row, row_info.rowbytes + 1)
}

/// Do the actual writing of a previously filtered row.
/// (`png_write_filtered_row`)
fn png_write_filtered_row(
    png_ptr: &mut PngStruct<'_, '_>,
    filtered_row: BestRow,
    full_row_length: usize, /*includes filter byte*/
) -> PngResult<()> {
    let buf = match filtered_row {
        BestRow::RowBuf => std::mem::take(&mut png_ptr.row_buf),
        BestRow::TryRow => png_ptr.try_row.take().unwrap_or_default(),
        BestRow::TstRow => png_ptr.tst_row.take().unwrap_or_default(),
    };
    let r = png_compress_IDAT(png_ptr, &buf[..full_row_length], Z_NO_FLUSH);
    match filtered_row {
        BestRow::RowBuf => png_ptr.row_buf = buf,
        BestRow::TryRow => png_ptr.try_row = Some(buf),
        BestRow::TstRow => png_ptr.tst_row = Some(buf),
    }
    r?;

    /* Swap the current and previous rows */
    if let Some(prev_row) = png_ptr.prev_row.as_mut() {
        std::mem::swap(prev_row, &mut png_ptr.row_buf);
    }

    /* Finish row - updates counters and flushes zlib if last row */
    png_write_finish_row(png_ptr)?;

    png_ptr.flush_rows += 1;

    if png_ptr.flush_dist > 0 && png_ptr.flush_rows >= png_ptr.flush_dist {
        super::pngwrite::png_write_flush(png_ptr)?;
    }
    Ok(())
}
