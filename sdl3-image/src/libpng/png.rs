// Rust translation of png.h and png.c from libpng 1.6.59 (as SDL_image's
// external/libpng pins it).
// Copyright (c) 2018-2026 Cosmin Truta
// Copyright (c) 1998-2002,2004,2006-2018 Glenn Randers-Pehrson
// Copyright (c) 1996-1997 Andreas Dilger
// Copyright (c) 1995-1996 Guy Eric Schalnat, Group 42, Inc.
// This code is released under the libpng license (see LICENSE.txt).
// This is an altered (translated) version of the original software.

//! png.h: the public types and constants the translated parts use; png.c:
//! location for general purpose libpng functions.

use super::pngerror::{png_error, png_warning};
use super::pnginfo::PngInfo;
use super::pngpriv::*;
use super::pngstruct::PngStruct;
use crate::zlib::{
    crc32, Z_BUF_ERROR, Z_DATA_ERROR, Z_ERRNO, Z_MEM_ERROR, Z_NEED_DICT, Z_OK, Z_STREAM_END,
    Z_STREAM_ERROR, Z_VERSION_ERROR,
};

/* png.h */

/// The version string this translation corresponds to.
pub(crate) const PNG_LIBPNG_VER_STRING: &str = "1.6.59";

/// A `png_error()`: the `longjmp()` back to the caller's `setjmp()`. The
/// message has been reported (printed to the standard error, as libpng's
/// default error handler does) by then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PngError;

/// The result of a function that can `png_error()`.
pub(crate) type PngResult<T> = Result<T, PngError>;

/// `png_color`: three byte color data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PngColor {
    pub(crate) red: u8,
    pub(crate) green: u8,
    pub(crate) blue: u8,
}

/// `png_color_16`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PngColor16 {
    pub(crate) index: u8, /* used for palette files */
    pub(crate) red: u16,  /* for use in red green blue files */
    pub(crate) green: u16,
    pub(crate) blue: u16,
    pub(crate) gray: u16, /* for use in grayscale files */
}

/// `png_row_info`: information about the current row of pixels.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PngRowInfo {
    pub(crate) width: u32,      /* width of row */
    pub(crate) rowbytes: usize, /* number of bytes in row */
    pub(crate) color_type: u8,  /* color type of row */
    pub(crate) bit_depth: u8,   /* bit depth of row */
    pub(crate) channels: u8,    /* number of channels (1, 2, 3, or 4) */
    pub(crate) pixel_depth: u8, /* bits per pixel (depth * channels) */
}

/// `png_rw_ptr` for reading: the user's read function.
pub(crate) type PngReadPtr = fn(&mut PngStruct<'_, '_>, &mut [u8]) -> PngResult<()>;
/// `png_rw_ptr` for writing: the user's write function.
pub(crate) type PngWritePtr = fn(&mut PngStruct<'_, '_>, &[u8]) -> PngResult<()>;
/// `png_flush_ptr`
pub(crate) type PngFlushPtr = fn(&mut PngStruct<'_, '_>) -> PngResult<()>;

/* These describe the color_type field in png_info. */
/* color type masks */
pub(crate) const PNG_COLOR_MASK_PALETTE: u8 = 1;
pub(crate) const PNG_COLOR_MASK_COLOR: u8 = 2;
pub(crate) const PNG_COLOR_MASK_ALPHA: u8 = 4;

/* color types.  Note that not all combinations are legal */
pub(crate) const PNG_COLOR_TYPE_GRAY: u8 = 0;
pub(crate) const PNG_COLOR_TYPE_PALETTE: u8 = PNG_COLOR_MASK_COLOR | PNG_COLOR_MASK_PALETTE;
pub(crate) const PNG_COLOR_TYPE_RGB: u8 = PNG_COLOR_MASK_COLOR;
pub(crate) const PNG_COLOR_TYPE_RGB_ALPHA: u8 = PNG_COLOR_MASK_COLOR | PNG_COLOR_MASK_ALPHA;
pub(crate) const PNG_COLOR_TYPE_GRAY_ALPHA: u8 = PNG_COLOR_MASK_ALPHA;
/* aliases */
pub(crate) const PNG_COLOR_TYPE_RGBA: u8 = PNG_COLOR_TYPE_RGB_ALPHA;

/* This is for compression type. PNG 1.0-1.2 only define the single type. */
pub(crate) const PNG_COMPRESSION_TYPE_BASE: i32 = 0; /* Deflate method 8, 32K window */
pub(crate) const PNG_COMPRESSION_TYPE_DEFAULT: i32 = PNG_COMPRESSION_TYPE_BASE;

/* This is for filter type. PNG 1.0-1.2 only define the single type. */
pub(crate) const PNG_FILTER_TYPE_BASE: i32 = 0; /* Single row per-byte filtering */
pub(crate) const PNG_INTRAPIXEL_DIFFERENCING: i32 = 64; /* Used only in MNG datastreams */
pub(crate) const PNG_FILTER_TYPE_DEFAULT: i32 = PNG_FILTER_TYPE_BASE;

/* These are for the interlacing type.  These values should NOT be changed. */
pub(crate) const PNG_INTERLACE_NONE: i32 = 0; /* Non-interlaced image */
pub(crate) const PNG_INTERLACE_LAST: i32 = 2; /* Not a valid value */

pub(crate) const PNG_HAVE_IHDR: u32 = 0x01;
pub(crate) const PNG_HAVE_PLTE: u32 = 0x02;
pub(crate) const PNG_AFTER_IDAT: u32 = 0x08;

pub(crate) const PNG_UINT_31_MAX: u32 = 0x7fffffff;
pub(crate) const PNG_SIZE_MAX: usize = usize::MAX;

/* These are constants for fixed point values encoded in the
 * PNG specification manner (x100000)
 */
pub(crate) const PNG_FP_1: i32 = 100000;

/* Maximum number of entries in PLTE/sPLT/tRNS arrays */
pub(crate) const PNG_MAX_PALETTE_LENGTH: usize = 256;

/* These determine if an ancillary chunk's data has been successfully read
 * from the PNG header, or if the application has filled in the corresponding
 * data in the info_struct to be written into the output file.  The values
 * of the PNG_INFO_<chunk> defines should NOT be changed.
 */
pub(crate) const PNG_INFO_PLTE: u32 = 0x0008;
pub(crate) const PNG_INFO_tRNS: u32 = 0x0010;

/* Add a filler byte to 8-bit or 16-bit Gray or 24-bit or 48-bit RGB images. */
pub(crate) const PNG_FILLER_AFTER: i32 = 1;

/* Flags for png_set_filter() to say which filters to use.  The flags
 * are chosen so that they don't conflict with real filter types
 * below, in case they are supplied instead of the #defined constants.
 * These values should NOT be changed.
 */
pub(crate) const PNG_NO_FILTERS: u8 = 0x00;
pub(crate) const PNG_FILTER_NONE: u8 = 0x08;
pub(crate) const PNG_FILTER_SUB: u8 = 0x10;
pub(crate) const PNG_FILTER_UP: u8 = 0x20;
pub(crate) const PNG_FILTER_AVG: u8 = 0x40;
pub(crate) const PNG_FILTER_PAETH: u8 = 0x80;
pub(crate) const PNG_FAST_FILTERS: u8 = PNG_FILTER_NONE | PNG_FILTER_SUB | PNG_FILTER_UP;
pub(crate) const PNG_ALL_FILTERS: u8 = PNG_FAST_FILTERS | PNG_FILTER_AVG | PNG_FILTER_PAETH;

/* Filter values (not flags) - used in pngwrite.c, pngwutil.c for now.
 * These defines should NOT be changed.
 */
pub(crate) const PNG_FILTER_VALUE_NONE: u8 = 0;
pub(crate) const PNG_FILTER_VALUE_SUB: u8 = 1;
pub(crate) const PNG_FILTER_VALUE_UP: u8 = 2;
pub(crate) const PNG_FILTER_VALUE_AVG: u8 = 3;
pub(crate) const PNG_FILTER_VALUE_PAETH: u8 = 4;
pub(crate) const PNG_FILTER_VALUE_LAST: u8 = 5;

/* The values of the 'keep' argument of png_set_keep_unknown_chunks */
pub(crate) const PNG_HANDLE_CHUNK_AS_DEFAULT: i32 = 0;
pub(crate) const PNG_HANDLE_CHUNK_NEVER: i32 = 1;
pub(crate) const PNG_HANDLE_CHUNK_IF_SAFE: i32 = 2;
pub(crate) const PNG_HANDLE_CHUNK_ALWAYS: i32 = 3;

/* png.c */

/// Checks whether the supplied bytes match the PNG signature.  We allow
/// checking less than the full 8-byte signature so that those apps that
/// already read the first few bytes of a file to determine the file type
/// can simply check the remaining bytes for extra assurance.  Returns
/// an integer less than, equal to, or greater than zero if sig is found,
/// respectively, to be less than, to match, or be greater than the correct
/// PNG signature (this is the same behavior as strcmp, memcmp, etc).
/// (`png_sig_cmp`)
pub(crate) fn png_sig_cmp(sig: &[u8], start: usize, mut num_to_check: usize) -> i32 {
    static PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

    if num_to_check > 8 {
        num_to_check = 8;
    } else if num_to_check < 1 {
        return -1;
    }

    if start > 7 {
        return -1;
    }

    if start + num_to_check > 8 {
        num_to_check = 8 - start;
    }

    // (memcmp)
    for i in start..start + num_to_check {
        if sig[i] != PNG_SIGNATURE[i] {
            return sig[i] as i32 - PNG_SIGNATURE[i] as i32;
        }
    }
    0
}

/* (png_zalloc() and png_zfree(): zlib allocates with Rust allocations) */

/// Reset the CRC variable to 32 bits of 1's.  Care must be taken
/// in case CRC is > 32 bits to leave the top bits 0. (`png_reset_crc`)
pub(crate) fn png_reset_crc(png_ptr: &mut PngStruct<'_, '_>) {
    /* The cast is safe because the crc is a 32-bit value. */
    png_ptr.crc = crc32(0, None) as u32;
}

/// Calculate the CRC over a section of data.  We can only pass as
/// much data to this routine as the largest single buffer size.  We
/// also check that this data will actually be used before going to the
/// trouble of calculating it. (`png_calculate_crc`)
pub(crate) fn png_calculate_crc(png_ptr: &mut PngStruct<'_, '_>, ptr: &[u8]) {
    let mut need_crc = 1;

    if png_chunk_ancillary(png_ptr.chunk_name) != 0 {
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

    /* 'uLong' is defined in zlib.h as unsigned long; this means that on some
     * systems it is a 64-bit value.  crc32, however, returns 32 bits so the
     * following cast is safe.  'uInt' may be no more than 16 bits, so it is
     * necessary to perform a loop here.
     * (crc32() takes the whole slice here.)
     */
    if need_crc != 0 && !ptr.is_empty() {
        let crc = png_ptr.crc as u64; /* Should never issue a warning */

        /* And the following is always safe because the crc is only 32 bits. */
        png_ptr.crc = crc32(crc, Some(ptr)) as u32;
    }
}

/* (png_user_version_check(): the caller's version string is this library's,
 * PNG_LIBPNG_VER_STRING, so the check always passes) */

/// Generic function to create a png_struct for either read or write - this
/// contains the common initialization. (`png_create_png_struct`, with no
/// user error, warning or memory functions, as SDL_image passes none)
pub(crate) fn png_create_png_struct<'s, 'b>() -> Option<Box<PngStruct<'s, 'b>>> {
    let mut create_struct = PngStruct::default();

    create_struct.user_width_max = PNG_USER_WIDTH_MAX;
    create_struct.user_height_max = PNG_USER_HEIGHT_MAX;

    create_struct.user_chunk_cache_max = PNG_USER_CHUNK_CACHE_MAX;

    /* default to compile-time limit */
    create_struct.user_chunk_malloc_max = PNG_USER_CHUNK_MALLOC_MAX;

    /* (png_set_mem_fn() and png_set_error_fn(): the defaults) */

    /* (png_malloc_warn(&create_struct, sizeof *png_ptr)) */
    let mut v = Vec::new();
    if v.try_reserve_exact(1).is_err() {
        png_warning(&create_struct, "Out of memory");
        return None;
    }
    v.push(create_struct);
    v.pop().map(Box::new)
}

/// Allocate the memory for an info_struct for the application.
/// (`png_create_info_struct`)
pub(crate) fn png_create_info_struct(_png_ptr: &PngStruct<'_, '_>) -> Option<Box<PngInfo>> {
    /* Use the internal API that does not (or at least should not) error out, so
     * that this call always returns ok.  The application typically sets up the
     * error handling *after* creating the info_struct because this is the way it
     * has always been done in 'example.c'.
     */
    let mut v = Vec::new();
    v.try_reserve_exact(1).ok()?;
    v.push(PngInfo::default());
    v.pop().map(Box::new)
}

/* (png_destroy_info_struct(), png_data_freer() and png_free_data(): the
 * structures' memory is Rust's, released as they are dropped) */

/// `png_get_io_ptr`: the stream the read or write functions use.
pub(crate) fn png_get_io_ptr<'p, 's, 'b>(
    png_ptr: &'p mut PngStruct<'s, 'b>,
) -> Option<&'p mut sdl3::io::IoStream<'b>> {
    match png_ptr.io_ptr.as_mut() {
        Some(io) => Some(&mut **io),
        None => None,
    }
}

/// `png_chunk_unknown_handling` (with `png_handle_as_unknown()`): the
/// "keep" value for a chunk, from the list SDL_image never sets (so
/// always `PNG_HANDLE_CHUNK_AS_DEFAULT`).
pub(crate) fn png_chunk_unknown_handling(png_ptr: &PngStruct<'_, '_>, _chunk_name: u32) -> i32 {
    /* Check chunk_name and return "keep" value if it's on the list, else 0 */
    if png_ptr.num_chunk_list == 0 {
        return PNG_HANDLE_CHUNK_AS_DEFAULT;
    }

    /* This means that known chunks should be processed and unknown chunks should
     * be handled according to the value of png_ptr->unknown_default; this can be
     * confusing because, as a result, there are two levels of defaulting for
     * unknown chunks.
     */
    PNG_HANDLE_CHUNK_AS_DEFAULT
}

/// Ensure that png_ptr->zstream.msg holds some appropriate error message string.
/// If it doesn't 'ret' is used to set it to something appropriate, even in cases
/// like Z_OK or Z_STREAM_END where the error code is apparently a success code.
/// (`png_zstream_error`)
pub(crate) fn png_zstream_error(png_ptr: &mut PngStruct<'_, '_>, ret: i32) {
    /* Translate 'ret' into an appropriate error string, priority is given to the
     * one in zstream if set.  This always returns a string, even in cases like
     * Z_OK or Z_STREAM_END where the error code is a success code.
     */
    if png_ptr.zstream.msg.is_none() {
        png_ptr.zstream.msg = Some(match ret {
            Z_STREAM_END => {
                /* Normal exit */
                "unexpected end of LZ stream"
            }
            Z_NEED_DICT => {
                /* This means the deflate stream did not have a dictionary; this
                 * indicates a bogus PNG.
                 */
                "missing LZ dictionary"
            }
            Z_ERRNO => {
                /* gz APIs only: should not happen */
                "zlib IO error"
            }
            Z_STREAM_ERROR => {
                /* internal libpng error */
                "bad parameters to zlib"
            }
            Z_DATA_ERROR => "damaged LZ stream",
            Z_MEM_ERROR => "insufficient memory",
            Z_BUF_ERROR => {
                /* End of input or output; not a problem if the caller is doing
                 * incremental read or write.
                 */
                "truncated"
            }
            Z_VERSION_ERROR => "unsupported zlib version",
            PNG_UNEXPECTED_ZLIB_RETURN => {
                /* Compile errors here mean that zlib now uses the value co-opted in
                 * pngpriv.h for PNG_UNEXPECTED_ZLIB_RETURN; update the switch above
                 * and change pngpriv.h.  Note that this message is "... return",
                 * whereas the default/Z_OK one is "... return code".
                 */
                "unexpected zlib return"
            }
            _ /* Z_OK and the rest */ => "unexpected zlib return code",
        });
    }
    let _ = Z_OK;
}

/// `png_check_IHDR`
#[allow(non_snake_case, clippy::too_many_arguments)]
pub(crate) fn png_check_IHDR(
    png_ptr: &PngStruct<'_, '_>,
    width: u32,
    height: u32,
    bit_depth: i32,
    color_type: i32,
    interlace_type: i32,
    compression_type: i32,
    filter_type: i32,
) -> PngResult<()> {
    let mut error = 0;

    /* Check for width and height valid values */
    if width == 0 {
        png_warning(png_ptr, "Image width is zero in IHDR");
        error = 1;
    }

    if width > PNG_UINT_31_MAX {
        png_warning(png_ptr, "Invalid image width in IHDR");
        error = 1;
    }

    /* The bit mask on the first line below must be at least as big as a
     * png_uint_32.  "~7U" is not adequate on 16-bit systems because it will
     * be an unsigned 16-bit value.  Casting to (png_alloc_size_t) makes the
     * type of the result at least as bit (in bits) as the RHS of the > operator
     * which also avoids a common warning on 64-bit systems that the comparison
     * of (png_uint_32) against the constant value on the RHS will always be
     * false.
     */
    if ((width as usize + 7) & !7usize)
        > ((PNG_SIZE_MAX
           - 48        /* big_row_buf hack */
           - 1)        /* filter byte */
           / 8)        /* 8-byte RGBA pixels */
           - 1
    /* extra max_pixel_depth pad */
    {
        /* The size of the row must be within the limits of this architecture.
         * Because the read code can perform arbitrary transformations the
         * maximum size is checked here.  Because the code in png_read_start_row
         * adds extra space "for safety's sake" in several places a conservative
         * limit is used here.
         *
         * NOTE: it would be far better to check the size that is actually used,
         * but the effect in the real world is minor and the changes are more
         * extensive, therefore much more dangerous and much more difficult to
         * write in a way that avoids compiler warnings.
         */
        png_warning(png_ptr, "Image width is too large for this architecture");
        error = 1;
    }

    if width > png_ptr.user_width_max {
        png_warning(png_ptr, "Image width exceeds user limit in IHDR");
        error = 1;
    }

    if height == 0 {
        png_warning(png_ptr, "Image height is zero in IHDR");
        error = 1;
    }

    if height > PNG_UINT_31_MAX {
        png_warning(png_ptr, "Invalid image height in IHDR");
        error = 1;
    }

    if height > png_ptr.user_height_max {
        png_warning(png_ptr, "Image height exceeds user limit in IHDR");
        error = 1;
    }

    /* Check other values */
    if bit_depth != 1 && bit_depth != 2 && bit_depth != 4 && bit_depth != 8 && bit_depth != 16 {
        png_warning(png_ptr, "Invalid bit depth in IHDR");
        error = 1;
    }

    if color_type < 0 || color_type == 1 || color_type == 5 || color_type > 6 {
        png_warning(png_ptr, "Invalid color type in IHDR");
        error = 1;
    }

    if ((color_type == PNG_COLOR_TYPE_PALETTE as i32) && bit_depth > 8)
        || ((color_type == PNG_COLOR_TYPE_RGB as i32
            || color_type == PNG_COLOR_TYPE_GRAY_ALPHA as i32
            || color_type == PNG_COLOR_TYPE_RGB_ALPHA as i32)
            && bit_depth < 8)
    {
        png_warning(png_ptr, "Invalid color type/bit depth combination in IHDR");
        error = 1;
    }

    if interlace_type >= PNG_INTERLACE_LAST {
        png_warning(png_ptr, "Unknown interlace method in IHDR");
        error = 1;
    }

    if compression_type != PNG_COMPRESSION_TYPE_BASE {
        png_warning(png_ptr, "Unknown compression method in IHDR");
        error = 1;
    }

    /* Accept filter_method 64 (intrapixel differencing) only if
     * 1. Libpng was compiled with PNG_MNG_FEATURES_SUPPORTED and
     * 2. Libpng did not read a PNG signature (this filter_method is only
     *    used in PNG datastreams that are embedded in MNG datastreams) and
     * 3. The application called png_permit_mng_features with a mask that
     *    included PNG_FLAG_MNG_FILTER_64 and
     * 4. The filter_method is 64 and
     * 5. The color_type is RGB or RGBA
     */
    if (png_ptr.mode & PNG_HAVE_PNG_SIGNATURE) != 0 && png_ptr.mng_features_permitted != 0 {
        png_warning(png_ptr, "MNG features are not allowed in a PNG datastream");
    }

    if filter_type != PNG_FILTER_TYPE_BASE {
        if !((png_ptr.mng_features_permitted & PNG_FLAG_MNG_FILTER_64) != 0
            && (filter_type == PNG_INTRAPIXEL_DIFFERENCING)
            && ((png_ptr.mode & PNG_HAVE_PNG_SIGNATURE) == 0)
            && (color_type == PNG_COLOR_TYPE_RGB as i32
                || color_type == PNG_COLOR_TYPE_RGB_ALPHA as i32))
        {
            png_warning(png_ptr, "Unknown filter method in IHDR");
            error = 1;
        }

        if (png_ptr.mode & PNG_HAVE_PNG_SIGNATURE) != 0 {
            png_warning(png_ptr, "Invalid filter method in IHDR");
            error = 1;
        }
    }

    if error == 1 {
        return Err(png_error(png_ptr, "Invalid IHDR data"));
    }
    Ok(())
}

/// `png_gamma_significant`
pub(crate) fn png_gamma_significant(gamma_val: i32) -> bool {
    /* sRGB:       1/2.2 == 0.4545(45)
     * AdobeRGB:   1/(2+51/256) ~= 0.45471 5dp
     *
     * So the correction from AdobeRGB to sRGB (output) is:
     *
     *    2.2/(2+51/256) == 1.00035524
     *
     * I.e. vanishingly small (<4E-4) but still detectable in 16-bit linear (+/-
     * 23).  Note that the Adobe choice seems to be something intended to give an
     * exact number with 8 binary fractional digits - it is the closest to 2.2
     * that is possible a base 2 .8p representation.
     */
    gamma_val < PNG_FP_1 - PNG_GAMMA_THRESHOLD_FIXED
        || gamma_val > PNG_FP_1 + PNG_GAMMA_THRESHOLD_FIXED
}
