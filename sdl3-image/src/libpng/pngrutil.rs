// Rust translation of pngrutil.c from libpng 1.6.59 (the parts the
// sequential reader of SDL_image's APNG frames runs).
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! pngrutil.c - utilities to read a PNG file
//!
//! This file contains routines that are only called from within
//! libpng itself during the course of reading an image.
//!
//! The PNG streams SDL_image hands libpng (one per APNG frame, which it
//! assembles itself) only ever hold IHDR (never interlaced), PLTE, tRNS,
//! IDAT and IEND chunks, so only those chunks' handlers are translated;
//! the table that names the others keeps their positions and lengths, and
//! they are handled as unknown chunks (as in a libpng built without their
//! support). The deinterlacing (`png_do_read_interlace()` and the
//! interlaced paths of `png_combine_row()`), the decompression of
//! compressed text and profiles and the unknown chunk cache are not
//! translated either, for the same reason.

use super::png::*;
use super::pngerror::*;
use super::pnginfo::PngInfo;
use super::pngpriv::*;
use super::pngrio::png_read_data;
use super::pngset::{png_set_IHDR, png_set_PLTE, png_set_tRNS};
use super::pngstruct::PngStruct;
use crate::zlib::{
    inflate, inflate_init2, inflate_reset2, Z_DATA_ERROR, Z_NO_FLUSH, Z_OK, Z_STREAM_END,
};

/// `png_get_uint_31`
pub(crate) fn png_get_uint_31(png_ptr: &PngStruct<'_, '_>, buf: &[u8]) -> PngResult<u32> {
    let uval = png_get_uint_32(buf);

    if uval > PNG_UINT_31_MAX {
        return Err(png_error(png_ptr, "PNG unsigned integer out of range"));
    }

    Ok(uval)
}

/// Grab an unsigned 32-bit integer from a buffer in big-endian format.
/// (`png_get_uint_32`)
pub(crate) fn png_get_uint_32(buf: &[u8]) -> u32 {
    ((buf[0] as u32) << 24) + ((buf[1] as u32) << 16) + ((buf[2] as u32) << 8) + (buf[3] as u32)
}

/// Grab an unsigned 16-bit integer from a buffer in big-endian format.
/// (`png_get_uint_16`)
pub(crate) fn png_get_uint_16(buf: &[u8]) -> u16 {
    /* ANSI-C requires an int value to accommodate at least 16 bits so this
     * works and allows the compiler not to worry about possible narrowing
     * on 32-bit systems.  (Pre-ANSI systems did not make integers smaller
     * than 16 bits either.)
     */
    let val: u32 = ((buf[0] as u32) << 8) + (buf[1] as u32);

    val as u16
}

/// Read and check the PNG file signature (`png_read_sig`)
pub(crate) fn png_read_sig(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
) -> PngResult<()> {
    /* Exit if the user application does not expect a signature. */
    if png_ptr.sig_bytes >= 8 {
        return Ok(());
    }

    let num_checked = png_ptr.sig_bytes as usize;
    let num_to_check = 8 - num_checked;

    /* The signature must be serialized in a single I/O call. */
    png_read_data(png_ptr, &mut info_ptr.signature[num_checked..])?;
    png_ptr.sig_bytes = 8;

    if png_sig_cmp(&info_ptr.signature, num_checked, num_to_check) != 0 {
        if num_checked < 4 && png_sig_cmp(&info_ptr.signature, num_checked, num_to_check - 4) != 0 {
            return Err(png_error(png_ptr, "Not a PNG file"));
        } else {
            return Err(png_error(png_ptr, "PNG file corrupted by ASCII conversion"));
        }
    }
    if num_checked < 3 {
        png_ptr.mode |= PNG_HAVE_PNG_SIGNATURE;
    }
    Ok(())
}

/// This function is called to verify that a chunk name is valid.
/// Do this using the bit-whacking approach from contrib/tools/pngfix.c
///
/// Copied from libpng 1.7. (`check_chunk_name`)
fn check_chunk_name(mut name: u32) -> bool {
    let mut t: u32;

    /* Remove bit 5 from all but the reserved byte; this means
     * every 8-bit unit must be in the range 65-90 to be valid.
     * So bit 5 must be zero, bit 6 must be set and bit 7 zero.
     */
    name &= !png_u32(32, 32, 0, 32);
    t = (name & !0x1f1f1f1f) ^ 0x40404040;

    /* Subtract 65 for each 8-bit quantity, this must not
     * overflow and each byte must then be in the range 0-25.
     */
    name = name.wrapping_sub(png_u32(65, 65, 65, 65));
    t |= name;

    /* Subtract 26, handling the overflow which should set the
     * top three bits of each byte.
     */
    name = name.wrapping_sub(png_u32(25, 25, 25, 26));
    t |= !name;

    (t & 0xe0e0e0e0) == 0
}

/// Read the chunk header (length + type name).
/// Put the type name into png_ptr->chunk_name, and return the length.
/// (`png_read_chunk_header`)
pub(crate) fn png_read_chunk_header(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<u32> {
    let mut buf = [0u8; 8];

    /* Read the length and the chunk name.  png_struct::chunk_name is immediately
     * updated even if they are detectably wrong.  This aids error message
     * handling by allowing png_chunk_error to be used.
     */
    png_read_data(png_ptr, &mut buf)?;
    let length = png_get_uint_31(png_ptr, &buf)?;
    let chunk_name = png_chunk_from_string(&buf[4..]);
    png_ptr.chunk_name = chunk_name;

    /* Reset the crc and run it over the chunk name. */
    png_reset_crc(png_ptr);
    png_calculate_crc(png_ptr, &buf[4..8]);

    /* Sanity check the length (first by <= 0x80) and the chunk name.  An error
     * here indicates a broken stream and libpng has no recovery from this.
     */
    if buf[0] >= 0x80 {
        return Err(png_chunk_error(png_ptr, "bad header (invalid length)"));
    }

    /* Check to see if chunk name is valid. */
    if !check_chunk_name(chunk_name) {
        return Err(png_chunk_error(png_ptr, "bad header (invalid type)"));
    }

    Ok(length)
}

/// Read data, and (optionally) run it through the CRC. (`png_crc_read`)
pub(crate) fn png_crc_read(png_ptr: &mut PngStruct<'_, '_>, buf: &mut [u8]) -> PngResult<()> {
    png_read_data(png_ptr, buf)?;
    png_calculate_crc(png_ptr, buf);
    Ok(())
}

/// Compare the CRC stored in the PNG file with that calculated by libpng from
/// the data it has read thus far. (`png_crc_error`)
fn png_crc_error(png_ptr: &mut PngStruct<'_, '_>, handle_as_ancillary: bool) -> PngResult<bool> {
    let mut crc_bytes = [0u8; 4];
    let mut need_crc = 1;

    /* There are four flags two for ancillary and two for critical chunks.  The
     * default setting of these flags is all zero.
     *
     * PNG_FLAG_CRC_ANCILLARY_USE
     * PNG_FLAG_CRC_ANCILLARY_NOWARN
     *  USE+NOWARN: no CRC calculation (implemented here), else;
     *  NOWARN:     png_chunk_error on error (implemented in png_crc_finish)
     *  else:       png_chunk_warning on error (implemented in png_crc_finish)
     *              This is the default.
     *
     *    I.e. NOWARN without USE produces png_chunk_error.  The default setting
     *    where neither are set does the same thing.
     *
     * PNG_FLAG_CRC_CRITICAL_USE
     * PNG_FLAG_CRC_CRITICAL_IGNORE
     *  IGNORE: no CRC calculation (implemented here), else;
     *  USE:    png_chunk_warning on error (implemented in png_crc_finish)
     *  else:   png_chunk_error on error (implemented in png_crc_finish)
     *          This is the default.
     *
     * This arose because of original mis-implementation and has persisted for
     * compatibility reasons.
     *
     * TODO: the flag names are internal so maybe this can be changed to
     * something comprehensible.
     */
    if handle_as_ancillary || png_chunk_ancillary(png_ptr.chunk_name) != 0 {
        if (png_ptr.flags & PNG_FLAG_CRC_ANCILLARY_MASK)
            == (PNG_FLAG_CRC_ANCILLARY_USE | PNG_FLAG_CRC_ANCILLARY_NOWARN)
        {
            need_crc = 0;
        }
    } else {
        /* critical */
        if (png_ptr.flags & PNG_FLAG_CRC_CRITICAL_IGNORE) != 0 {
            need_crc = 0;
        }
    }

    /* The chunk CRC must be serialized in a single I/O call. */
    png_read_data(png_ptr, &mut crc_bytes)?;

    if need_crc != 0 {
        let crc = png_get_uint_32(&crc_bytes);
        Ok(crc != png_ptr.crc)
    } else {
        Ok(false)
    }
}

/// Optionally skip data and then check the CRC.  Depending on whether we
/// are reading an ancillary or critical chunk, and how the program has set
/// things up, we may calculate the CRC on the data and print a message.
/// Returns '1' if there was a CRC error, '0' otherwise.
///
/// There is one public version which is used in most places and another which
/// takes the value for the 'critical' flag to check.  This allows PLTE and IEND
/// handling code to ignore the CRC error and removes some confusing code
/// duplication. (`png_crc_finish_critical`)
fn png_crc_finish_critical(
    png_ptr: &mut PngStruct<'_, '_>,
    mut skip: u32,
    mut handle_as_ancillary: bool,
) -> PngResult<bool> {
    /* The size of the local buffer for inflate is a good guess as to a
     * reasonable size to use for buffering reads from the application.
     */
    while skip > 0 {
        let mut tmpbuf = [0u8; PNG_INFLATE_BUF_SIZE];

        let mut len = tmpbuf.len() as u32;
        if len > skip {
            len = skip;
        }
        skip -= len;

        png_crc_read(png_ptr, &mut tmpbuf[..len as usize])?;
    }

    /* If 'handle_as_ancillary' has been requested and this is a critical chunk
     * but PNG_FLAG_CRC_CRITICAL_IGNORE was set then png_read_crc did not, in
     * fact, calculate the CRC so the ANCILLARY settings should not be used
     * instead.
     */
    if handle_as_ancillary && (png_ptr.flags & PNG_FLAG_CRC_CRITICAL_IGNORE) != 0 {
        handle_as_ancillary = false;
    }

    /* TODO: this might be more comprehensible if png_crc_error was inlined here.
     */
    if png_crc_error(png_ptr, handle_as_ancillary)? {
        /* See above for the explanation of how the flags work. */
        if if handle_as_ancillary || png_chunk_ancillary(png_ptr.chunk_name) != 0 {
            (png_ptr.flags & PNG_FLAG_CRC_ANCILLARY_NOWARN) == 0
        } else {
            (png_ptr.flags & PNG_FLAG_CRC_CRITICAL_USE) != 0
        } {
            png_chunk_warning(png_ptr, "CRC error");
        } else {
            return Err(png_chunk_error(png_ptr, "CRC error"));
        }

        return Ok(true);
    }

    Ok(false)
}

/// `png_crc_finish`
pub(crate) fn png_crc_finish(png_ptr: &mut PngStruct<'_, '_>, skip: u32) -> PngResult<bool> {
    png_crc_finish_critical(png_ptr, skip, false /*critical handling*/)
}

/// Manage the read buffer; this simply reallocates the buffer if it is not small
/// enough (or if it is not allocated).  The routine returns a pointer to the
/// buffer; if an error occurs and 'warn' is set the routine returns NULL, else
/// it will call png_error on failure. (`png_read_buffer`: whether the
/// buffer, `png_ptr->read_buffer`, is there)
fn png_read_buffer(png_ptr: &mut PngStruct<'_, '_>, new_size: usize) -> bool {
    if new_size > png_chunk_max(png_ptr) {
        return false;
    }

    if png_ptr.read_buffer.is_some() && new_size > png_ptr.read_buffer_size {
        png_ptr.read_buffer = None;
        png_ptr.read_buffer_size = 0;
    }

    if png_ptr.read_buffer.is_none() {
        if let Some(buffer) = super::pngmem::png_malloc_base(png_ptr, new_size) {
            /* (zeroed: memset(buffer, 0, new_size); just in case) */
            png_ptr.read_buffer = Some(buffer);
            png_ptr.read_buffer_size = new_size;
        }
    }

    png_ptr.read_buffer.is_some()
}

/// Detach the zstream from the input and output buffers left by
/// the current or a previous owner, and possibly deallocated since.
/// (`png_inflate_detach_buffers`)
fn png_inflate_detach_buffers(png_ptr: &mut PngStruct<'_, '_>) {
    png_ptr.zstream.next_in = 0;
    png_ptr.zstream.avail_in = 0;
    png_ptr.zstream.next_out = 0;
    png_ptr.zstream.avail_out = 0;
}

/// png_inflate_claim: claim the zstream for some nefarious purpose that involves
/// decompression.  Returns Z_OK on success, else a zlib error code.  It checks
/// the owner but, in final release builds, just issues a warning if some other
/// chunk apparently owns the stream.  Prior to release it does a png_error.
fn png_inflate_claim(png_ptr: &mut PngStruct<'_, '_>, owner: u32) -> i32 {
    if png_ptr.zowner != 0 {
        /* So the message that results is "<chunk> using zstream"; this is an
         * internal error, but is very useful for debugging.  i18n requirements
         * are minimal.
         */
        let name = png_string_from_chunk(png_ptr.zowner);
        let msg = format!("{} using zstream", String::from_utf8_lossy(&name));
        png_chunk_warning(png_ptr, &msg);
        png_ptr.zowner = 0;
    }

    /* Implementation note: unlike 'png_deflate_claim' this internal function
     * does not take the size of the data as an argument.  Some efficiency could
     * be gained by using this when it is known *if* the zlib stream itself does
     * not record the number; however, this is an illusion: the original writer
     * of the PNG may have selected a lower window size, and we really must
     * follow that because, for systems with limited capabilities, we
     * would otherwise reject the application's attempts to use a smaller window
     * size (zlib doesn't have an interface to say "this or lower"!).
     *
     * inflateReset2 was added to zlib 1.2.4; before this the window could not be
     * reset, therefore it is necessary to always allocate the maximum window
     * size with earlier zlibs just in case later compressed chunks need it.
     */
    let ret: i32; /* zlib return code */
    let window_bits = 0;

    /* (PNG_MAXIMUM_INFLATE_WINDOW is an option SDL_image doesn't set) */
    png_ptr.zstream_start = 1;

    png_inflate_detach_buffers(png_ptr);

    if (png_ptr.flags & PNG_FLAG_ZSTREAM_INITIALIZED) != 0 {
        ret = inflate_reset2(&mut png_ptr.zstream, window_bits);
    } else {
        let r = inflate_init2(&mut png_ptr.zstream, window_bits);

        if r == Z_OK {
            png_ptr.flags |= PNG_FLAG_ZSTREAM_INITIALIZED;
        }
        ret = r;
    }

    if ret == Z_OK {
        png_ptr.zowner = owner;
    } else {
        png_zstream_error(png_ptr, ret);
    }

    ret
}

/// Handle the start of the inflate stream if we called inflateInit2(strm,0);
/// in this case some zlib versions skip validation of the CINFO field and, in
/// certain circumstances, libpng may end up displaying an invalid image, in
/// contrast to implementations that call zlib in the normal way (e.g. libpng
/// 1.5). (`png_zlib_inflate`, as the `PNG_INFLATE` macro calls it: the
/// input is the read buffer)
fn png_zlib_inflate(png_ptr: &mut PngStruct<'_, '_>, output: &mut [u8], flush: i32) -> i32 {
    let input: &[u8] = png_ptr.read_buffer.as_deref().unwrap_or(&[]);
    if png_ptr.zstream_start != 0 && png_ptr.zstream.avail_in > 0 {
        if (input[png_ptr.zstream.next_in] >> 4) > 7 {
            png_ptr.zstream.msg = Some("invalid window size (libpng)");
            return Z_DATA_ERROR;
        }

        png_ptr.zstream_start = 0;
    }

    inflate(&mut png_ptr.zstream, input, output, flush)
}

/// The result of a chunk handler (`png_handle_result_code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PngHandleResult {
    HandledError = 0,     /* bad crc or known and bad format or too long */
    HandledDiscarded = 1, /* not saved in the unknown chunk list */
    HandledSaved = 2,     /* saved in the unknown chunk list */
    HandledOk = 3,        /* known, supported and handled without error */
}

use PngHandleResult::*;

/* CHUNK HANDLING */
/// Read and check the IDHR chunk (`png_handle_IHDR`)
#[allow(non_snake_case)]
fn png_handle_IHDR(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
    _length: u32,
) -> PngResult<PngHandleResult> {
    let mut buf = [0u8; 13];

    /* Length and position are checked by the caller. */

    png_ptr.mode |= PNG_HAVE_IHDR;

    png_crc_read(png_ptr, &mut buf)?;
    png_crc_finish(png_ptr, 0)?;

    let width = png_get_uint_31(png_ptr, &buf)?;
    let height = png_get_uint_31(png_ptr, &buf[4..])?;
    let bit_depth = buf[8] as i32;
    let color_type = buf[9] as i32;
    let compression_type = buf[10] as i32;
    let filter_type = buf[11] as i32;
    let interlace_type = buf[12] as i32;

    /* Set internal variables */
    png_ptr.width = width;
    png_ptr.height = height;
    png_ptr.bit_depth = bit_depth as u8;
    png_ptr.interlaced = interlace_type as u8;
    png_ptr.color_type = color_type as u8;
    png_ptr.filter_type = filter_type as u8;
    png_ptr.compression = compression_type as u8;

    /* Find number of channels */
    png_ptr.channels = match png_ptr.color_type {
        PNG_COLOR_TYPE_RGB => 3,
        PNG_COLOR_TYPE_GRAY_ALPHA => 2,
        PNG_COLOR_TYPE_RGB_ALPHA => 4,
        /* invalid, png_set_IHDR calls png_error */
        _ /* PNG_COLOR_TYPE_GRAY, PNG_COLOR_TYPE_PALETTE */ => 1,
    };

    /* Set up other useful info */
    png_ptr.pixel_depth = png_ptr.bit_depth.wrapping_mul(png_ptr.channels);
    png_ptr.rowbytes = png_rowbytes(png_ptr.pixel_depth as u32, png_ptr.width as usize);

    /* Rely on png_set_IHDR to completely validate the data and call png_error if
     * it's wrong.
     */
    png_set_IHDR(
        png_ptr,
        info_ptr,
        width,
        height,
        bit_depth,
        color_type,
        interlace_type,
        compression_type,
        filter_type,
    )?;

    Ok(HandledOk)
}

/// Read and check the palette (`png_handle_PLTE`)
/// TODO: there are several obvious errors in this code when handling
/// out-of-place chunks and there is much over-complexity caused by trying to
/// patch up the problems.
#[allow(non_snake_case)]
fn png_handle_PLTE(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
    length: u32,
) -> PngResult<PngHandleResult> {
    /* 1.6.47: consistency.  This used to be especially treated as a critical
     * error even in an image which is not colour mapped, there isn't a good
     * justification for treating some errors here one way and others another so
     * everything uses the same logic.
     */
    let errmsg: &str = if (png_ptr.mode & PNG_HAVE_PLTE) != 0 {
        "duplicate"
    } else if (png_ptr.mode & PNG_HAVE_IDAT) != 0 {
        "out of place"
    } else if (png_ptr.color_type & PNG_COLOR_MASK_COLOR) == 0 {
        "ignored in grayscale PNG"
    } else if length > 3 * PNG_MAX_PALETTE_LENGTH as u32 || (length % 3) != 0 {
        "invalid"
    }
    /* This drops PLTE in favour of tRNS or bKGD because both of those chunks
     * can have an effect on the rendering of the image whereas PLTE only matters
     * in the case of an 8-bit display with a decoder which controls the palette.
     *
     * The alternative here is to ignore the error and store the palette anyway;
     * destroying the tRNS will definitely cause problems.
     *
     * NOTE: the case of PNG_COLOR_TYPE_PALETTE need not be considered because
     * the png_handle_ routines for the three 'after PLTE' chunks tRNS, bKGD and
     * hIST all check for a preceding PLTE in these cases.
     */
    else if png_ptr.color_type != PNG_COLOR_TYPE_PALETTE
        && (png_has_chunk(png_ptr, PngIndex::tRNS) || png_has_chunk(png_ptr, PngIndex::bKGD))
    {
        "out of place"
    } else {
        /* If the palette has 256 or fewer entries but is too large for the bit
         * depth we don't issue an error to preserve the behavior of previous
         * libpng versions. We silently truncate the unused extra palette entries
         * here.
         */
        let max_palette_length: u32 = if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
            1u32 << png_ptr.bit_depth
        } else {
            PNG_MAX_PALETTE_LENGTH as u32
        };

        /* The cast is safe because 'length' is less than
         * 3*PNG_MAX_PALETTE_LENGTH
         */
        let num: u32 = if length > 3 * max_palette_length {
            max_palette_length
        } else {
            length / 3
        };

        let mut buf = [0u8; 3 * PNG_MAX_PALETTE_LENGTH];
        let mut palette = [PngColor::default(); PNG_MAX_PALETTE_LENGTH];

        /* Read the chunk into the buffer then read to the end of the chunk. */
        png_crc_read(png_ptr, &mut buf[..(num * 3) as usize])?;
        png_crc_finish_critical(
            png_ptr,
            length - 3 * num,
            /* Handle as ancillary if PLTE is optional: */
            png_ptr.color_type != PNG_COLOR_TYPE_PALETTE,
        )?;

        let mut j = 0usize;
        for p in palette.iter_mut().take(num as usize) {
            p.red = buf[j];
            p.green = buf[j + 1];
            p.blue = buf[j + 2];
            j += 3;
        }

        /* A valid PLTE chunk has been read */
        png_ptr.mode |= PNG_HAVE_PLTE;

        png_set_PLTE(png_ptr, info_ptr, &palette, num as i32)?;
        return Ok(HandledOk);
    };

    /* Here on error: errmsg is non NULL. */
    if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
        png_crc_finish(png_ptr, length)?;
        return Err(png_chunk_error(png_ptr, errmsg));
    } else {
        /* not critical to this image */
        png_crc_finish_critical(png_ptr, length, true /*handle as ancillary*/)?;
        png_chunk_benign_error(png_ptr, errmsg)?;
    }

    Ok(HandledError)
}

/// `png_handle_IEND`
#[allow(non_snake_case)]
fn png_handle_IEND(
    png_ptr: &mut PngStruct<'_, '_>,
    _info_ptr: &mut PngInfo,
    length: u32,
) -> PngResult<PngHandleResult> {
    png_ptr.mode |= PNG_AFTER_IDAT | PNG_HAVE_IEND;

    if length != 0 {
        png_chunk_benign_error(png_ptr, "invalid")?;
    }

    png_crc_finish_critical(png_ptr, length, true /*handle as ancillary*/)?;

    Ok(HandledOk)
}

/// `png_handle_tRNS`
#[allow(non_snake_case)]
fn png_handle_tRNS(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
    length: u32,
) -> PngResult<PngHandleResult> {
    let mut readbuf = [0u8; PNG_MAX_PALETTE_LENGTH];

    if png_ptr.color_type == PNG_COLOR_TYPE_GRAY {
        let mut buf = [0u8; 2];

        if length != 2 {
            png_crc_finish(png_ptr, length)?;
            png_chunk_benign_error(png_ptr, "invalid")?;
            return Ok(HandledError);
        }

        png_crc_read(png_ptr, &mut buf)?;
        png_ptr.num_trans = 1;
        png_ptr.trans_color.gray = png_get_uint_16(&buf);
    } else if png_ptr.color_type == PNG_COLOR_TYPE_RGB {
        let mut buf = [0u8; 6];

        if length != 6 {
            png_crc_finish(png_ptr, length)?;
            png_chunk_benign_error(png_ptr, "invalid")?;
            return Ok(HandledError);
        }

        png_crc_read(png_ptr, &mut buf)?;
        png_ptr.num_trans = 1;
        png_ptr.trans_color.red = png_get_uint_16(&buf);
        png_ptr.trans_color.green = png_get_uint_16(&buf[2..]);
        png_ptr.trans_color.blue = png_get_uint_16(&buf[4..]);
    } else if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
        if (png_ptr.mode & PNG_HAVE_PLTE) == 0 {
            png_crc_finish(png_ptr, length)?;
            png_chunk_benign_error(png_ptr, "out of place")?;
            return Ok(HandledError);
        }

        if length > png_ptr.num_palette as u32
            || length > PNG_MAX_PALETTE_LENGTH as u32
            || length == 0
        {
            png_crc_finish(png_ptr, length)?;
            png_chunk_benign_error(png_ptr, "invalid")?;
            return Ok(HandledError);
        }

        png_crc_read(png_ptr, &mut readbuf[..length as usize])?;
        png_ptr.num_trans = length as u16;
    } else {
        png_crc_finish(png_ptr, length)?;
        png_chunk_benign_error(png_ptr, "invalid with alpha channel")?;
        return Ok(HandledError);
    }

    if png_crc_finish(png_ptr, 0)? {
        png_ptr.num_trans = 0;
        return Ok(HandledError);
    }

    let trans_color = png_ptr.trans_color;
    let num_trans = png_ptr.num_trans as i32;
    png_set_tRNS(
        png_ptr,
        info_ptr,
        Some(&readbuf),
        num_trans,
        Some(&trans_color),
    )?;
    Ok(HandledOk)
}

/// Handle an unknown, or known but disabled, chunk (`png_handle_unknown`,
/// as libpng is built, with no user chunk function, no keep setting and so
/// nothing saved: the chunk is skipped)
pub(crate) fn png_handle_unknown(
    png_ptr: &mut PngStruct<'_, '_>,
    _info_ptr: &mut PngInfo,
    length: u32,
    mut keep: i32,
) -> PngResult<PngHandleResult> {
    let handled = HandledDiscarded; /* the default */

    /* (PNG_READ_USER_CHUNKS: no read_user_chunk_fn is set) */

    /* keep is currently just the per-chunk setting, if there was no
     * setting change it to the global default now (not that this may
     * still be AS_DEFAULT) then obtain the cache of the chunk if required,
     * if not simply skip the chunk.
     */
    if keep == PNG_HANDLE_CHUNK_AS_DEFAULT {
        keep = png_ptr.unknown_default;
    }

    /* (keep is PNG_HANDLE_CHUNK_AS_DEFAULT: SDL_image never asks for chunks
     * to be kept, so png_cache_unknown_chunk() and png_set_unknown_chunks()
     * have nothing to do) */
    let _ = (
        keep,
        PNG_HANDLE_CHUNK_ALWAYS,
        PNG_HANDLE_CHUNK_IF_SAFE,
        PNG_HANDLE_CHUNK_NEVER,
    );
    png_crc_finish(png_ptr, length)?;

    /* Check for unhandled critical chunks */
    if handled < HandledSaved && png_chunk_critical(png_ptr.chunk_name) {
        return Err(png_chunk_error(png_ptr, "unhandled critical chunk"));
    }

    Ok(handled)
}

/// `png_has_chunk`
fn png_has_chunk(png_ptr: &PngStruct<'_, '_>, i: PngIndex) -> bool {
    png_file_has_chunk(png_ptr, i)
}

/// `png_file_has_chunk`: the chunk has been recorded in png_struct
fn png_file_has_chunk(png_ptr: &PngStruct<'_, '_>, i: PngIndex) -> bool {
    (png_ptr.chunks & png_chunk_flag_from_index(i)) != 0
}

/// `png_file_add_chunk`: record the chunk in the png_struct
fn png_file_add_chunk(png_ptr: &mut PngStruct<'_, '_>, i: PngIndex) {
    png_ptr.chunks |= png_chunk_flag_from_index(i);
}

/// A chunk handler (`png_handle_cHNK`).
type PngHandler = fn(&mut PngStruct<'_, '_>, &mut PngInfo, u32) -> PngResult<PngHandleResult>;

/// An entry of `read_chunks`: the PNG standard rules for **reading** known
/// chunks.
struct ReadChunk {
    handler: Option<PngHandler>,
    /* A chunk-specific 'handler', NULL if the chunk is not supported in this
     * build.
     */
    max_length: u32, /* Length min, max in bytes */
    min_length: u32,
    /* Length errors on critical chunks have special handling to preserve the
     * existing behaviour in libpng 1.6.  Ancillary chunks are checked below
     * and produce a 'benign' error.
     */
    pos_before: u32, /* PNG_HAVE_ values chunk must precede */
    pos_after: u32,  /* PNG_HAVE_ values chunk must follow */
    /* NOTE: PLTE, tRNS and bKGD require special handling which depends on
     * the colour type of the base image.
     */
    multiple: u32, /* Multiple occurrences permitted */
                   /* This is enabled for PLTE because PLTE may, in practice, be optional */
}

const NO_CHECK: u32 = 0x801; /* Do not check the maximum length */
const LIMIT: u32 = 0x802; /* Limit to png_chunk_max bytes */
const LZ77_MIN: u32 = 2 + 5 + 4;
const LK_MIN: u32 = 3 + LZ77_MIN; /* Minimum length of keyword+LZ77 */

const H_IHDR: u32 = PNG_HAVE_IHDR;
const H_PLTE: u32 = PNG_HAVE_PLTE;
const H_IDAT: u32 = PNG_HAVE_IDAT;
/* For the two chunks, tRNS and bKGD which can occur in PNGs without a PLTE
 * but must occur after the PLTE use this and put the check in the handler
 * routine for colour mapped images were PLTE is required.  Also put a check
 * in PLTE for other image types to drop the PLTE if tRNS or bKGD have been
 * seen.
 */
const H_COL: u32 = PNG_HAVE_PLTE | PNG_HAVE_IDAT;
/* Used for the decoding chunks which must be before PLTE. */
const A_IDAT: u32 = PNG_AFTER_IDAT;

const fn rc(
    handler: Option<PngHandler>,
    max_length: u32,
    min_length: u32,
    pos_before: u32,
    pos_after: u32,
    multiple: u32,
) -> ReadChunk {
    ReadChunk {
        handler,
        max_length,
        min_length,
        pos_before,
        pos_after,
        multiple,
    }
}

/// 1.6.47: This is the new table driven interface to all the chunk handling.
///
/// The table describes the PNG standard rules for **reading** known chunks -
/// every chunk which has an entry in PNG_KNOWN_CHUNKS.  The table contains an
/// entry for each PNG_INDEX_cHNK describing the rules.
///
/// In this initial version the only information in the entry is the
/// png_handle_cHNK function for the chunk in question.  When chunk support is
/// compiled out the entry will be NULL. (The handlers of the chunks
/// SDL_image's frame streams never hold are not translated: NULL.)
static READ_CHUNKS: [ReadChunk; PngIndex::unknown as usize] = [
    /* Chunks from W3C PNG v3: */
    /*       cHNK  max_len,   min, before, after, multiple */
    /* IHDR */
    rc(Some(png_handle_IHDR), 13, 13, H_IHDR, 0, 0),
    /* PLTE */ rc(Some(png_handle_PLTE), NO_CHECK, 0, 0, H_IHDR, 1),
    /* PLTE errors are only critical for colour-map images, consequently the
     * handler does all the checks.
     */
    /* IDAT */
    rc(None, NO_CHECK, 0, A_IDAT, H_IHDR, 1),
    /* IEND */ rc(Some(png_handle_IEND), NO_CHECK, 0, 0, A_IDAT, 0),
    /* Historically data was allowed in IEND */
    /* APNG handling: the minimal implementation of APNG handling in libpng 1.6
     * requires that those significant applications which already handle APNG not
     * get hosed.  To do this ensure the code here will have to ensure than APNG
     * data by default (at least in 1.6) gets stored in the unknown chunk list.
     * Maybe this can be relaxed in a few years but at present it's just the only
     * safe way.
     *
     * ATM just cause unknown handling for all three chunks:
     */
    /* acTL */
    rc(None, 8, 8, H_IDAT, H_IHDR, 0),
    /* bKGD */ rc(None, 6, 1, H_IDAT, H_IHDR, 0),
    /* cHRM */ rc(None, 32, 32, H_COL, H_IHDR, 0),
    /* cICP */ rc(None, 4, 4, H_COL, H_IHDR, 0),
    /* cLLI */ rc(None, 8, 8, H_COL, H_IHDR, 0),
    /* eXIf */ rc(None, LIMIT, 4, 0, H_IHDR, 0),
    /* fcTL */ rc(None, 25, 26, 0, H_IHDR, 1),
    /* fdAT */ rc(None, LIMIT, 4, H_IDAT, H_IHDR, 1),
    /* gAMA */ rc(None, 4, 4, H_COL, H_IHDR, 0),
    /* hIST */ rc(None, 1024, 0, H_IDAT, H_PLTE, 0),
    /* iCCP */ rc(None, NO_CHECK, LK_MIN, H_COL, H_IHDR, 0),
    /* iTXt */ rc(None, NO_CHECK, 6, 0, H_IHDR, 1),
    /* Allocates 'length+1'; checked in the handler */
    /* mDCV */
    rc(None, 24, 24, H_COL, H_IHDR, 0),
    /* oFFs */ rc(None, 9, 9, H_IDAT, H_IHDR, 0),
    /* pCAL */ rc(None, NO_CHECK, 14, H_IDAT, H_IHDR, 0),
    /* Allocates 'length+1'; checked in the handler */
    /* pHYs */
    rc(None, 9, 9, H_IDAT, H_IHDR, 0),
    /* sBIT */ rc(None, 4, 1, H_COL, H_IHDR, 0),
    /* sCAL */ rc(None, LIMIT, 4, H_IDAT, H_IHDR, 0),
    /* Allocates 'length+1'; checked in the handler */
    /* sPLT */
    rc(None, NO_CHECK, 3, H_IDAT, H_IHDR, 1),
    /* Allocates 'length+1'; checked in the handler */
    /* sRGB */
    rc(None, 1, 1, H_COL, H_IHDR, 0),
    /* tEXt */ rc(None, NO_CHECK, 2, 0, H_IHDR, 1),
    /* Allocates 'length+1'; checked in the handler */
    /* tIME */
    rc(None, 7, 7, 0, H_IHDR, 0),
    /* tRNS */ rc(Some(png_handle_tRNS), 256, 0, H_IDAT, H_IHDR, 0),
    /* zTXt */ rc(None, LIMIT, LK_MIN, 0, H_IHDR, 1),
];

/// `png_chunk_index_from_name`
fn png_chunk_index_from_name(chunk_name: u32) -> PngIndex {
    /* For chunk png_cHNK return PNG_INDEX_cHNK.  Return PNG_INDEX_unknown if
     * chunk_name is not known.  Notice that in a particular build "known" does
     * not necessarily mean "supported", although the inverse applies.
     */
    match chunk_name {
        png_IHDR => PngIndex::IHDR,
        png_PLTE => PngIndex::PLTE,
        png_IDAT => PngIndex::IDAT,
        png_IEND => PngIndex::IEND,
        png_acTL => PngIndex::acTL,
        png_bKGD => PngIndex::bKGD,
        png_cHRM => PngIndex::cHRM,
        png_cICP => PngIndex::cICP,
        png_cLLI => PngIndex::cLLI,
        png_eXIf => PngIndex::eXIf,
        png_fcTL => PngIndex::fcTL,
        png_fdAT => PngIndex::fdAT,
        png_gAMA => PngIndex::gAMA,
        png_hIST => PngIndex::hIST,
        png_iCCP => PngIndex::iCCP,
        png_iTXt => PngIndex::iTXt,
        png_mDCV => PngIndex::mDCV,
        png_oFFs => PngIndex::oFFs,
        png_pCAL => PngIndex::pCAL,
        png_pHYs => PngIndex::pHYs,
        png_sBIT => PngIndex::sBIT,
        png_sCAL => PngIndex::sCAL,
        png_sPLT => PngIndex::sPLT,
        png_sRGB => PngIndex::sRGB,
        png_tEXt => PngIndex::tEXt,
        png_tIME => PngIndex::tIME,
        png_tRNS => PngIndex::tRNS,
        png_zTXt => PngIndex::zTXt,
        _ => PngIndex::unknown,
    }
}

/// `png_handle_chunk`
pub(crate) fn png_handle_chunk(
    png_ptr: &mut PngStruct<'_, '_>,
    info_ptr: &mut PngInfo,
    length: u32,
) -> PngResult<PngHandleResult> {
    /* CSE: these things don't change, these autos are just to save typing and
     * make the code more clear.
     */
    let chunk_name = png_ptr.chunk_name;
    let chunk_index = png_chunk_index_from_name(chunk_name);

    let mut handled = HandledError;
    let mut errmsg: Option<&str> = None;

    /* Is this a known chunk?  If not there are no checks performed here;
     * png_handle_unknown does the correct checks.  This means that the values
     * for known but unsupported chunks in the above table are not used here
     * however the chunks_seen fields in png_struct are still set.
     */
    if chunk_index == PngIndex::unknown || READ_CHUNKS[chunk_index as usize].handler.is_none() {
        handled = png_handle_unknown(png_ptr, info_ptr, length, PNG_HANDLE_CHUNK_AS_DEFAULT)?;
    }
    /* First check the position.   The first check is historical; the stream must
     * start with IHDR and anything else causes libpng to give up immediately.
     */
    else if chunk_index != PngIndex::IHDR && (png_ptr.mode & PNG_HAVE_IHDR) == 0 {
        return Err(png_chunk_error(png_ptr, "missing IHDR")); /* NORETURN */
    } else {
        let entry = &READ_CHUNKS[chunk_index as usize];

        /* Before all the pos_before chunks, after all the pos_after chunks. */
        if ((png_ptr.mode & entry.pos_before) != 0)
            || ((png_ptr.mode & entry.pos_after) != entry.pos_after)
        {
            errmsg = Some("out of place");
        }
        /* Now check for duplicates: duplicated critical chunks also produce a
         * full error.
         */
        else if entry.multiple == 0 && png_file_has_chunk(png_ptr, chunk_index) {
            errmsg = Some("duplicate");
        } else if length < entry.min_length {
            errmsg = Some("too short");
        } else {
            /* NOTE: apart from IHDR the critical chunks (PLTE, IDAT and IEND) are set
             * up above not to do any length checks.
             *
             * The png_chunk_max check ensures that the variable length chunks are
             * always checked at this point for being within the system allocation
             * limits.
             */
            let max_length = entry.max_length;

            let meets_limit = match max_length {
                LIMIT => {
                    /* png_read_chunk_header has already png_error'ed chunks with a
                     * length exceeding the 31-bit PNG limit, so just check the memory
                     * limit:
                     */
                    if length as usize <= png_chunk_max(png_ptr) {
                        true
                    } else {
                        errmsg = Some("length exceeds libpng limit");
                        false
                    }
                }

                NO_CHECK => true,

                _ => {
                    if length <= max_length {
                        true
                    } else {
                        errmsg = Some("too long");
                        false
                    }
                }
            };
            if meets_limit {
                if let Some(handler) = entry.handler {
                    handled = handler(png_ptr, info_ptr, length)?;
                }
            }
        }
    }

    /* If there was an error or the chunk was simply skipped it is not counted as
     * 'seen'.
     */
    if let Some(errmsg) = errmsg {
        if png_chunk_critical(chunk_name) {
            /* stop immediately */
            return Err(png_chunk_error(png_ptr, errmsg));
        } else {
            /* ancillary chunk */
            /* The chunk data is skipped: */
            png_crc_finish(png_ptr, length)?;
            png_chunk_benign_error(png_ptr, errmsg)?;
        }
    } else if handled >= HandledSaved && chunk_index != PngIndex::unknown {
        png_file_add_chunk(png_ptr, chunk_index);
    }

    Ok(handled)
}

/// Combines the row recently read in with the existing pixels in the row.  This
/// routine takes care of alpha and transparency if requested.  This routine also
/// handles the two methods of progressive display of interlaced images,
/// depending on the 'display' value; if 'display' is true then the whole row
/// (dp) is filled from the start by replicating the available pixels.  If
/// 'display' is false only those pixels present in the pass are filled in.
/// (`png_combine_row`, for a non-interlaced read)
pub(crate) fn png_combine_row(
    png_ptr: &PngStruct<'_, '_>,
    dp: &mut [u8],
    _display: i32,
) -> PngResult<()> {
    let pixel_depth = png_ptr.transformed_pixel_depth as u32;
    let sp = &png_ptr.row_buf[1..];
    let row_width = png_ptr.width as usize;
    let mut end_ptr: Option<usize> = None;
    let mut end_byte: u8 = 0;
    let mut end_mask: u32;

    /* Added in 1.5.6: it should not be possible to enter this routine until at
     * least one row has been read from the PNG data and transformed.
     */
    if pixel_depth == 0 {
        return Err(png_error(png_ptr, "internal row logic error"));
    }

    /* Added in 1.5.4: the pixel depth should match the information returned by
     * any call to png_read_update_info at this point.  Do not continue if we got
     * this wrong.
     */
    if png_ptr.info_rowbytes != 0 && png_ptr.info_rowbytes != png_rowbytes(pixel_depth, row_width) {
        return Err(png_error(png_ptr, "internal row size calculation error"));
    }

    /* Don't expect this to ever happen: */
    if row_width == 0 {
        return Err(png_error(png_ptr, "internal row width error"));
    }

    /* Preserve the last byte in cases where only part of it will be overwritten,
     * the multiply below may overflow, we don't care because ANSI-C guarantees
     * we get the low bits.
     */
    end_mask = (pixel_depth.wrapping_mul(row_width as u32)) & 7;
    if end_mask != 0 {
        /* end_ptr == NULL is a flag to say do nothing */
        let e = png_rowbytes(pixel_depth, row_width) - 1;
        end_ptr = Some(e);
        end_byte = dp[e];
        /* (PNG_PACKSWAP: not set) big-endian byte */
        end_mask = 0xff >> end_mask;
        /* end_mask is now the bits to *keep* from the destination row */
    }

    /* For non-interlaced images this reduces to a memcpy(). A memcpy()
     * will also happen if interlacing isn't supported or if the application
     * does not call png_set_interlace_handling().  In the latter cases the
     * caller just gets a sequence of the unexpanded rows from each interlace
     * pass.
     * (The interlaced combination isn't translated: SDL_image's frame
     * streams are never interlaced.)
     */

    /* If here then the switch above wasn't used so just memcpy the whole row
     * from the temporary row buffer (notice that this overwrites the end of the
     * destination row if it is a partial byte.)
     */
    let n = png_rowbytes(pixel_depth, row_width);
    dp[..n].copy_from_slice(&sp[..n]);

    /* Restore the overwritten bits from the last byte if necessary. */
    if let Some(e) = end_ptr {
        dp[e] = ((end_byte as u32 & end_mask) | (dp[e] as u32 & !end_mask)) as u8;
    }
    Ok(())
}

/// `png_read_filter_row_sub`
fn png_read_filter_row_sub(row_info: &PngRowInfo, row: &mut [u8], _prev_row: &[u8]) {
    let istop = row_info.rowbytes;
    let bpp = ((row_info.pixel_depth as usize) + 7) >> 3;

    for i in bpp..istop {
        row[i] = ((row[i] as i32 + row[i - bpp] as i32) & 0xff) as u8;
    }
}

/// `png_read_filter_row_up`
fn png_read_filter_row_up(row_info: &PngRowInfo, row: &mut [u8], prev_row: &[u8]) {
    let istop = row_info.rowbytes;

    for i in 0..istop {
        row[i] = ((row[i] as i32 + prev_row[i] as i32) & 0xff) as u8;
    }
}

/// `png_read_filter_row_avg`
fn png_read_filter_row_avg(row_info: &PngRowInfo, row: &mut [u8], prev_row: &[u8]) {
    let bpp = ((row_info.pixel_depth as usize) + 7) >> 3;
    let istop = row_info.rowbytes - bpp;

    for i in 0..bpp {
        row[i] = ((row[i] as i32 + (prev_row[i] as i32 / 2)) & 0xff) as u8;
    }

    for i in bpp..bpp + istop {
        row[i] = ((row[i] as i32 + (prev_row[i] as i32 + row[i - bpp] as i32) / 2) & 0xff) as u8;
    }
}

/// `png_read_filter_row_paeth_1byte_pixel`
fn png_read_filter_row_paeth_1byte_pixel(row_info: &PngRowInfo, row: &mut [u8], prev_row: &[u8]) {
    let rp_end = row_info.rowbytes;
    let mut a: i32;
    let mut c: i32;

    /* First pixel/byte */
    c = prev_row[0] as i32;
    a = row[0] as i32 + c;
    row[0] = a as u8;

    /* Remainder */
    let mut i = 1;
    while i < rp_end {
        let mut pa: i32;
        let p: i32;
        let mut pc: i32;

        a &= 0xff; /* From previous iteration or start */
        let b = prev_row[i] as i32;

        p = b - c;
        pc = a - c;

        pa = if p < 0 { -p } else { p };
        let pb = if pc < 0 { -pc } else { pc };
        pc = if (p + pc) < 0 { -(p + pc) } else { p + pc };

        /* Find the best predictor, the least of pa, pb, pc favoring the earlier
         * ones in the case of a tie.
         */
        if pb < pa {
            pa = pb;
            a = b;
        }
        if pc < pa {
            a = c;
        }

        /* Calculate the current pixel in a, and move the previous row pixel to c
         * for the next time round the loop
         */
        c = b;
        a += row[i] as i32;
        row[i] = a as u8;
        i += 1;
    }
}

/// `png_read_filter_row_paeth_multibyte_pixel`
fn png_read_filter_row_paeth_multibyte_pixel(
    row_info: &PngRowInfo,
    row: &mut [u8],
    prev_row: &[u8],
) {
    let bpp = ((row_info.pixel_depth as usize) + 7) >> 3;
    let mut rp_end = bpp;

    /* Process the first pixel in the row completely (this is the same as 'up'
     * because there is only one candidate predictor for the first row).
     */
    let mut i = 0;
    while i < rp_end {
        let a = row[i] as i32 + prev_row[i] as i32;
        row[i] = a as u8;
        i += 1;
    }

    /* Remainder */
    rp_end += row_info.rowbytes - bpp;

    while i < rp_end {
        let mut pa: i32;
        let p: i32;
        let mut pc: i32;

        let c = prev_row[i - bpp] as i32;
        let mut a = row[i - bpp] as i32;
        let b = prev_row[i] as i32;

        p = b - c;
        pc = a - c;

        pa = if p < 0 { -p } else { p };
        let pb = if pc < 0 { -pc } else { pc };
        pc = if (p + pc) < 0 { -(p + pc) } else { p + pc };

        if pb < pa {
            pa = pb;
            a = b;
        }
        if pc < pa {
            a = c;
        }

        a += row[i] as i32;
        row[i] = a as u8;
        i += 1;
    }
}

/// This function is called once for every PNG image (except for PNG images
/// that only use PNG_FILTER_VALUE_NONE for all rows) to set the
/// implementations required to reverse the filtering of PNG rows.  Reversing
/// the filter is the first transformation performed on the row data.  It is
/// performed in place, therefore an implementation can be selected based on
/// the image pixel format.  If the implementation depends on image width then
/// take care to ensure that it works correctly if the image is interlaced -
/// interlacing causes the actual row width to vary.
/// (`png_init_filter_functions`: the functions are chosen by
/// [`png_read_filter_row`] from the pixel depth; libpng's SIMD versions of
/// them compute the same rows)
fn png_init_filter_functions(pp: &mut PngStruct<'_, '_>) {
    pp.read_filter_init = true;
}

/// `png_read_filter_row`
pub(crate) fn png_read_filter_row(
    pp: &mut PngStruct<'_, '_>,
    row_info: &PngRowInfo,
    row: &mut [u8],
    prev_row: &[u8],
    filter: u8,
) {
    /* OPTIMIZATION: DO NOT MODIFY THIS FUNCTION, instead #define
     * PNG_FILTER_OPTIMIZATIONS to a function that overrides the generic
     * implementations.  See png_init_filter_functions above.
     */
    if filter > PNG_FILTER_VALUE_NONE && filter < PNG_FILTER_VALUE_LAST {
        if !pp.read_filter_init {
            png_init_filter_functions(pp);
        }

        let bpp = ((pp.pixel_depth as u32) + 7) >> 3;
        match filter {
            PNG_FILTER_VALUE_SUB => png_read_filter_row_sub(row_info, row, prev_row),
            PNG_FILTER_VALUE_UP => png_read_filter_row_up(row_info, row, prev_row),
            PNG_FILTER_VALUE_AVG => png_read_filter_row_avg(row_info, row, prev_row),
            _ /* PNG_FILTER_VALUE_PAETH */ => {
                if bpp == 1 {
                    png_read_filter_row_paeth_1byte_pixel(row_info, row, prev_row)
                } else {
                    png_read_filter_row_paeth_multibyte_pixel(row_info, row, prev_row)
                }
            }
        }
    }
}

/// `png_read_IDAT_data`: `output` is `None` to check for the end of the
/// stream.
pub(crate) fn png_read_IDAT_data(
    png_ptr: &mut PngStruct<'_, '_>,
    mut output: Option<&mut [u8]>,
    mut avail_out: usize,
) -> PngResult<()> {
    /* Loop reading IDATs and decompressing the result into output[avail_out] */
    png_ptr.zstream.next_out = 0;
    png_ptr.zstream.avail_out = 0; /* safety: set below */

    if output.is_none() {
        avail_out = 0;
    }

    loop {
        let ret: i32;
        let mut tmpbuf = [0u8; PNG_INFLATE_BUF_SIZE];

        if png_ptr.zstream.avail_in == 0 {
            let mut avail_in: u32;

            while png_ptr.idat_size == 0 {
                png_crc_finish(png_ptr, 0)?;

                png_ptr.idat_size = png_read_chunk_header(png_ptr)?;
                /* This is an error even in the 'check' case because the code just
                 * consumed a non-IDAT header.
                 */
                if png_ptr.chunk_name != png_IDAT {
                    return Err(png_error(png_ptr, "Not enough image data"));
                }
            }

            avail_in = png_ptr.IDAT_read_size;

            if avail_in as usize > png_chunk_max(png_ptr) {
                avail_in = png_chunk_max(png_ptr) as u32;
            }

            if avail_in > png_ptr.idat_size {
                avail_in = png_ptr.idat_size;
            }

            /* A PNG with a gradually increasing IDAT size will defeat this attempt
             * to minimize memory usage by causing lots of re-allocs, but
             * realistically doing IDAT_read_size re-allocs is not likely to be a
             * big problem.
             *
             * An error here corresponds to the system being out of memory.
             */
            if !png_read_buffer(png_ptr, avail_in as usize) {
                return Err(png_chunk_error(png_ptr, "out of memory"));
            }

            let mut buffer = png_ptr.read_buffer.take().unwrap_or_default();
            let r = png_crc_read(png_ptr, &mut buffer[..avail_in as usize]);
            png_ptr.read_buffer = Some(buffer);
            r?;
            png_ptr.idat_size -= avail_in;

            png_ptr.zstream.next_in = 0;
            png_ptr.zstream.avail_in = avail_in;
        }

        /* And set up the output side. */
        let out_buf: &mut [u8] = match output.as_deref_mut() {
            Some(out) => {
                /* standard read */
                let mut o = ZLIB_IO_MAX;

                if o as usize > avail_out {
                    o = avail_out as u32;
                }

                avail_out -= o as usize;
                png_ptr.zstream.avail_out = o;
                out
            }
            None => {
                /* after last row, checking for end */
                png_ptr.zstream.next_out = 0;
                png_ptr.zstream.avail_out = tmpbuf.len() as u32;
                &mut tmpbuf
            }
        };

        /* Use NO_FLUSH; this gives zlib the maximum opportunity to optimize the
         * process.  If the LZ stream is truncated the sequential reader will
         * terminally damage the stream, above, by reading the chunk header of the
         * following chunk (it then exits with png_error).
         *
         * TODO: deal more elegantly with truncated IDAT lists.
         */
        ret = png_zlib_inflate(png_ptr, out_buf, Z_NO_FLUSH);

        /* Take the unconsumed output back. */
        if output.is_some() {
            avail_out += png_ptr.zstream.avail_out as usize;
        } else {
            /* avail_out counts the extra bytes */
            avail_out += tmpbuf.len() - png_ptr.zstream.avail_out as usize;
        }

        png_ptr.zstream.avail_out = 0;

        if ret == Z_STREAM_END {
            /* Do this for safety; we won't read any more into this row. */
            png_ptr.zstream.next_out = 0;

            png_ptr.mode |= PNG_AFTER_IDAT;
            png_ptr.flags |= PNG_FLAG_ZSTREAM_ENDED;

            if png_ptr.zstream.avail_in > 0 || png_ptr.idat_size > 0 {
                png_chunk_benign_error(png_ptr, "Extra compressed data")?;
            }
            break;
        }

        if ret != Z_OK {
            png_zstream_error(png_ptr, ret);
            let msg = png_ptr.zstream.msg.unwrap_or("");

            if output.is_some() {
                return Err(png_chunk_error(png_ptr, msg));
            } else {
                /* checking */
                png_chunk_benign_error(png_ptr, msg)?;
                return Ok(());
            }
        }

        if avail_out == 0 {
            break;
        }
    }

    if avail_out > 0 {
        /* The stream ended before the image; this is the same as too few IDATs so
         * should be handled the same way.
         */
        if output.is_some() {
            return Err(png_error(png_ptr, "Not enough image data"));
        } else {
            /* the deflate stream contained extra data */
            png_chunk_benign_error(png_ptr, "Too much image data")?;
        }
    }
    Ok(())
}

/// `png_read_finish_IDAT`
#[allow(non_snake_case)]
pub(crate) fn png_read_finish_IDAT(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    /* We don't need any more data and the stream should have ended, however the
     * LZ end code may actually not have been processed.  In this case we must
     * read it otherwise stray unread IDAT data or, more likely, an IDAT chunk
     * may still remain to be consumed.
     */
    if (png_ptr.flags & PNG_FLAG_ZSTREAM_ENDED) == 0 {
        /* The NULL causes png_read_IDAT_data to swallow any remaining bytes in
         * the compressed stream, but the stream may be damaged too, so even after
         * this call we may need to terminate the zstream ownership.
         */
        png_read_IDAT_data(png_ptr, None, 0)?;
        png_ptr.zstream.next_out = 0; /* safety */

        /* Now clear everything out for safety; the following may not have been
         * done.
         */
        if (png_ptr.flags & PNG_FLAG_ZSTREAM_ENDED) == 0 {
            png_ptr.mode |= PNG_AFTER_IDAT;
            png_ptr.flags |= PNG_FLAG_ZSTREAM_ENDED;
        }
    }

    /* If the zstream has not been released do it now *and* terminate the reading
     * of the final IDAT chunk.
     */
    if png_ptr.zowner == png_IDAT {
        png_inflate_detach_buffers(png_ptr);
        png_ptr.zowner = 0;

        /* The slightly weird semantics of the sequential IDAT reading is that we
         * are always in or at the end of an IDAT chunk, so we always need to do a
         * crc_finish here.  If idat_size is non-zero we also need to read the
         * spurious bytes at the end of the chunk now.
         */
        let idat_size = png_ptr.idat_size;
        let _ = png_crc_finish(png_ptr, idat_size)?;
    }
    Ok(())
}

/// `png_read_finish_row`
pub(crate) fn png_read_finish_row(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    png_ptr.row_number += 1;
    if png_ptr.row_number < png_ptr.num_rows {
        return Ok(());
    }

    /* (png_ptr->interlaced != 0: the next pass; never the case here) */

    /* Here after at the end of the last row of the last pass. */
    png_read_finish_IDAT(png_ptr)
}

/// `png_read_start_row`
pub(crate) fn png_read_start_row(png_ptr: &mut PngStruct<'_, '_>) -> PngResult<()> {
    let mut max_pixel_depth: u32;
    let mut row_bytes: usize;

    super::pngrtran::png_init_read_transformations(png_ptr);

    /* (png_ptr->interlaced != 0: never the case here) */
    png_ptr.num_rows = png_ptr.height;
    png_ptr.iwidth = png_ptr.width;

    max_pixel_depth = png_ptr.pixel_depth as u32;

    /* WARNING: * png_read_transform_info (pngrtran.c) performs a simpler set of
     * calculations to calculate the final pixel depth, then
     * png_do_read_transforms actually does the transforms.  This means that the
     * code which effectively calculates this value is actually repeated in three
     * separate places.  They must all match.  Innocent changes to the order of
     * transformations can and will break libpng in a way that causes memory
     * overwrites.
     *
     * TODO: fix this.
     */
    if (png_ptr.transformations & PNG_PACK) != 0 && png_ptr.bit_depth < 8 {
        max_pixel_depth = 8;
    }

    if (png_ptr.transformations & PNG_EXPAND) != 0 {
        if png_ptr.color_type == PNG_COLOR_TYPE_PALETTE {
            if png_ptr.num_trans != 0 {
                max_pixel_depth = 32;
            } else {
                max_pixel_depth = 24;
            }
        } else if png_ptr.color_type == PNG_COLOR_TYPE_GRAY {
            if max_pixel_depth < 8 {
                max_pixel_depth = 8;
            }

            if png_ptr.num_trans != 0 {
                max_pixel_depth *= 2;
            }
        } else if png_ptr.color_type == PNG_COLOR_TYPE_RGB && png_ptr.num_trans != 0 {
            max_pixel_depth *= 4;
            max_pixel_depth /= 3;
        }
    }

    if (png_ptr.transformations & PNG_EXPAND_16) != 0 {
        /* In fact it is an error if it isn't supported, but checking is
         * the safe way.
         */
        if (png_ptr.transformations & PNG_EXPAND) != 0 {
            if png_ptr.bit_depth < 16 {
                max_pixel_depth *= 2;
            }
        } else {
            png_ptr.transformations &= !PNG_EXPAND_16;
        }
    }

    if (png_ptr.transformations & PNG_FILLER) != 0 {
        if png_ptr.color_type == PNG_COLOR_TYPE_GRAY {
            if max_pixel_depth <= 8 {
                max_pixel_depth = 16;
            } else {
                max_pixel_depth = 32;
            }
        } else if png_ptr.color_type == PNG_COLOR_TYPE_RGB
            || png_ptr.color_type == PNG_COLOR_TYPE_PALETTE
        {
            if max_pixel_depth <= 32 {
                max_pixel_depth = 32;
            } else {
                max_pixel_depth = 64;
            }
        }
    }

    if (png_ptr.transformations & PNG_GRAY_TO_RGB) != 0 {
        if (png_ptr.num_trans != 0 && (png_ptr.transformations & PNG_EXPAND) != 0)
            || (png_ptr.transformations & PNG_FILLER) != 0
            || png_ptr.color_type == PNG_COLOR_TYPE_GRAY_ALPHA
        {
            if max_pixel_depth <= 16 {
                max_pixel_depth = 32;
            } else {
                max_pixel_depth = 64;
            }
        } else if max_pixel_depth <= 8 {
            if png_ptr.color_type == PNG_COLOR_TYPE_RGB_ALPHA {
                max_pixel_depth = 32;
            } else {
                max_pixel_depth = 24;
            }
        } else if png_ptr.color_type == PNG_COLOR_TYPE_RGB_ALPHA {
            max_pixel_depth = 64;
        } else {
            max_pixel_depth = 48;
        }
    }

    /* (PNG_USER_TRANSFORM: not set) */

    /* This value is stored in png_struct and double checked in the row read
     * code.
     */
    png_ptr.maximum_pixel_depth = max_pixel_depth as u8;
    png_ptr.transformed_pixel_depth = 0; /* calculated on demand */

    /* Align the width on the next larger 8 pixels.  Mainly used
     * for interlacing
     */
    row_bytes = (png_ptr.width as usize + 7) & !7usize;
    /* Calculate the maximum bytes needed, adding a byte and a pixel
     * for safety's sake
     */
    row_bytes =
        png_rowbytes(max_pixel_depth, row_bytes) + 1 + ((max_pixel_depth as usize + 7) >> 3);

    if row_bytes + 48 > png_ptr.old_big_row_buf_size {
        png_ptr.row_buf = Vec::new();
        png_ptr.prev_row = None;

        /* (malloc'ed for a non-interlaced image; zeroed here) */
        png_ptr.row_buf = super::pngmem::png_malloc(png_ptr, row_bytes + 48)?;

        png_ptr.prev_row = Some(super::pngmem::png_malloc(png_ptr, row_bytes + 48)?);

        /* (Use 31 bytes of padding before and 17 bytes after row_buf: the
         * buffers start at row_buf here) */
        png_ptr.old_big_row_buf_size = row_bytes + 48;
    }

    if png_ptr.rowbytes > (PNG_SIZE_MAX - 1) {
        return Err(png_error(
            png_ptr,
            "Row has too many bytes to allocate in memory",
        ));
    }

    let n = png_ptr.rowbytes + 1;
    if let Some(prev) = png_ptr.prev_row.as_mut() {
        prev[..n].fill(0);
    }

    /* The sequential reader needs a buffer for IDAT, but the progressive reader
     * does not, so free the read buffer now regardless; the sequential reader
     * reallocates it on demand.
     */
    if png_ptr.read_buffer.is_some() {
        png_ptr.read_buffer_size = 0;
        png_ptr.read_buffer = None;
    }

    /* Finally claim the zstream for the inflate of the IDAT data, use the bits
     * value from the stream (note that this will result in a fatal error if the
     * IDAT stream has a bogus deflate header window_bits value, but this should
     * not be happening any longer!)
     */
    if png_inflate_claim(png_ptr, png_IDAT) != Z_OK {
        let msg = png_ptr.zstream.msg.unwrap_or("");
        return Err(png_error(png_ptr, msg));
    }

    png_ptr.flags |= PNG_FLAG_ROW_INIT;
    Ok(())
}
