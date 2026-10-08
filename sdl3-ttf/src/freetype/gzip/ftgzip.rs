// Rust translation of src/gzip/ftgzip.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2002-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType support for .gz compressed files.
//!
//! This optional component relies on zlib.  It should mainly be used to
//! parse compressed PCF fonts, as found with many X11 server
//! distributions.
//!
//! Only `FT_Gzip_Uncompress` is translated; the gzip stream
//! (`FT_Stream_OpenGzip` and its `FT_GZipFile` machinery) serves the PCF
//! driver alone, which is not translated.

use super::super::fttypes::*;
use super::zlib::*;

/// `FT_Gzip_Uncompress`: decompresses `input` (zlib or gzip data) into
/// `output`; `*output_len` is the size of `output` on entry and the number
/// of bytes written on success.  An empty `output` stands for C's NULL.
pub fn ft_gzip_uncompress(
    output: &mut [u8],
    output_len: &mut FtULong,
    input: &[u8],
) -> FtResult<()> {
    /* check for `input' delayed to `inflate' */

    if output.is_empty() {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* this function is modeled after zlib's `uncompress' function */

    let avail_out = (*output_len as u32 as usize).min(output.len());
    let mut stream = ZStream::new(input, &mut output[..avail_out]);
    /* (`avail_in' and `avail_out' are `uInt': the lengths are truncated) */
    stream.avail_in = input.len() as u32;
    stream.avail_out = *output_len as u32;

    let mut err = inflate_init2(&mut stream, MAX_WBITS | 32);

    if err != Z_OK {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    err = inflate(&mut stream, Z_FINISH);
    if err != Z_STREAM_END {
        inflate_end(&mut stream);
        if err == Z_OK {
            err = Z_BUF_ERROR;
        }
    } else {
        *output_len = stream.total_out as FtULong;

        err = inflate_end(&mut stream);
    }

    if err == Z_MEM_ERROR {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }

    if err == Z_BUF_ERROR {
        return Err(FT_ERR_ARRAY_TOO_LARGE);
    }

    if err == Z_DATA_ERROR {
        return Err(FT_ERR_INVALID_TABLE);
    }

    if err == Z_NEED_DICT {
        return Err(FT_ERR_INVALID_TABLE);
    }

    Ok(())
}
