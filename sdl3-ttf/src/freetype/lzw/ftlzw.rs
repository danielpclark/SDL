// Rust translation of src/lzw/ftlzw.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2004-2023 by Albert Chin-A-Young.
// Based on code in `src/gzip/ftgzip.c'.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType support for .Z compressed files.
//!
//! This optional component relies on NetBSD's zopen().  It should mainly
//! be used to parse compressed PCF fonts, as found with many X11 server
//! distributions.
//!
//! `FT_CONFIG_OPTION_USE_LZW` is defined in SDL_ttf's bundled build. The
//! embedding stream's descriptor (the LZW file) lives in its read
//! callback, with the source stream it shares with the face.

use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::ftzopen::*;

/***************************************************************************/
/***************************************************************************/
/*****                                                                 *****/
/*****                   F I L E   D E S C R I P T O R                 *****/
/*****                                                                 *****/
/***************************************************************************/
/***************************************************************************/

pub const FT_LZW_BUFFER_SIZE: usize = 4096;

/// `FT_LZWFileRec`
#[derive(Debug)]
pub struct FtLzwFileRec {
    pub source: FtSharedStream, /* parent/source stream        */
    pub lzw: FtLzwStateRec,     /* lzw decompressor state      */

    pub buffer: Box<[FtByte; FT_LZW_BUFFER_SIZE]>, /* output buffer      */
    pub pos: FtULong,                              /* position in output */
    pub cursor: usize,
    pub limit: usize,
}

/// `ft_lzw_check_header`: check and skip .Z header
fn ft_lzw_check_header(stream: &mut FtStreamRec) -> FtResult<()> {
    let mut head = [0u8; 2];

    stream.seek(0)?;
    stream.read(&mut head)?;

    /* head[0] && head[1] are the magic numbers */
    if head[0] != 0x1F || head[1] != 0x9D {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    Ok(())
}

/// `ft_lzw_file_init`
fn ft_lzw_file_init(source: &FtSharedStream) -> FtResult<FtLzwFileRec> {
    let mut zip = FtLzwFileRec {
        source: source.clone(),
        lzw: FtLzwStateRec::default(),
        buffer: Box::new([0; FT_LZW_BUFFER_SIZE]),
        pos: 0,
        cursor: FT_LZW_BUFFER_SIZE,
        limit: FT_LZW_BUFFER_SIZE,
    };

    /* check and skip .Z header */
    ft_lzw_check_header(&mut ft_lock_stream(source))?;

    /* initialize internal lzw variable */
    ft_lzwstate_init(&mut zip.lzw);

    Ok(zip)
}

/// `ft_lzw_file_done`
fn ft_lzw_file_done(zip: &mut FtLzwFileRec) {
    /* clear the rest */
    ft_lzwstate_done(&mut zip.lzw);
}

/// `ft_lzw_file_reset`
fn ft_lzw_file_reset(zip: &mut FtLzwFileRec) -> FtResult<()> {
    let error = ft_lock_stream(&zip.source).seek(0);
    if error.is_ok() {
        ft_lzwstate_reset(&mut zip.lzw);

        zip.limit = FT_LZW_BUFFER_SIZE;
        zip.cursor = zip.limit;
        zip.pos = 0;
    }

    error
}

/// `ft_lzw_file_fill_output`
fn ft_lzw_file_fill_output(zip: &mut FtLzwFileRec) -> FtResult<()> {
    zip.cursor = 0;

    let count = {
        let mut source = ft_lock_stream(&zip.source);
        ft_lzwstate_io(
            &mut zip.lzw,
            &mut source,
            Some(&mut zip.buffer[..]),
            FT_LZW_BUFFER_SIZE as FtULong,
        )
    };

    zip.limit = zip.cursor + count as usize;

    if count == 0 {
        return Err(FT_ERR_INVALID_STREAM_OPERATION);
    }

    Ok(())
}

/// `ft_lzw_file_skip_output`: fill output buffer; `count' must be <=
/// FT_LZW_BUFFER_SIZE
fn ft_lzw_file_skip_output(zip: &mut FtLzwFileRec, mut count: FtULong) -> FtResult<()> {
    let mut error = Ok(());

    /* first, we skip what we can from the output buffer */
    {
        let mut delta = (zip.limit - zip.cursor) as FtULong;

        if delta >= count {
            delta = count;
        }

        zip.cursor += delta as usize;
        zip.pos += delta;

        count -= delta;
    }

    /* next, we skip as many bytes remaining as possible */
    while count > 0 {
        let mut delta = FT_LZW_BUFFER_SIZE as FtULong;

        if delta > count {
            delta = count;
        }

        let numread = {
            let mut source = ft_lock_stream(&zip.source);
            ft_lzwstate_io(&mut zip.lzw, &mut source, None, delta)
        };
        if numread < delta {
            /* not enough bytes */
            error = Err(FT_ERR_INVALID_STREAM_OPERATION);
            break;
        }

        zip.pos += delta;
        count -= delta;
    }

    error
}

/// `ft_lzw_file_io`
fn ft_lzw_file_io(zip: &mut FtLzwFileRec, pos: FtULong, buffer: &mut [FtByte]) -> FtULong {
    let mut result: FtULong = 0;
    let mut count = buffer.len() as FtULong;

    'exit: {
        /* seeking backwards. */
        if pos < zip.pos {
            /* If the new position is within the output buffer, simply       */
            /* decrement pointers, otherwise we reset the stream completely! */
            if (zip.pos - pos) <= zip.cursor as FtULong {
                zip.cursor -= (zip.pos - pos) as usize;
                zip.pos = pos;
            } else if ft_lzw_file_reset(zip).is_err() {
                break 'exit;
            }
        }

        /* skip unwanted bytes */
        if pos > zip.pos && ft_lzw_file_skip_output(zip, pos - zip.pos).is_err() {
            break 'exit;
        }

        if count == 0 {
            break 'exit;
        }

        /* now read the data */
        loop {
            let mut delta = (zip.limit - zip.cursor) as FtULong;
            if delta >= count {
                delta = count;
            }

            let (r, d) = (result as usize, delta as usize);
            buffer[r..r + d].copy_from_slice(&zip.buffer[zip.cursor..zip.cursor + d]);
            result += delta;
            zip.cursor += d;
            zip.pos += delta;

            count -= delta;
            if count == 0 {
                break;
            }

            if ft_lzw_file_fill_output(zip).is_err() {
                break;
            }
        }
    }

    /* Exit: */
    result
}

/***************************************************************************/
/***************************************************************************/
/*****                                                                 *****/
/*****            L Z W   E M B E D D I N G   S T R E A M              *****/
/*****                                                                 *****/
/***************************************************************************/
/***************************************************************************/

/// `ft_lzw_stream_close`: the stream's callback, with the LZW file in it,
/// goes when the stream does.
impl Drop for FtLzwFileRec {
    fn drop(&mut self) {
        /* finalize lzw file descriptor */
        ft_lzw_file_done(self);
    }
}

/// `FT_Stream_OpenLZW`: a stream reading the .Z compressed `source`.
pub fn ft_stream_open_lzw(source: &FtSharedStream) -> FtResult<FtStream> {
    /*
     * Check the header right now; this prevents allocation of a huge
     * LZWFile object (400 KByte of heap memory) if not necessary.
     *
     * Did I mention that you should never use .Z compressed font
     * files?
     */
    ft_lzw_check_header(&mut ft_lock_stream(source))?;

    let mut zip = ft_lzw_file_init(source)?;
    /* (`stream->descriptor.pointer = zip') */

    /* `ft_lzw_stream_io` */
    let read: FtStreamIoFunc =
        Box::new(move |offset, buffer: &mut [u8]| ft_lzw_file_io(&mut zip, offset, buffer));

    let stream = FtStreamRec::open_callback(0x7FFFFFFF, read); /* don't know the real size! */

    Ok(stream)
}
