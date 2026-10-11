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
//! The embedding stream's descriptor (the gzip file) lives in its read
//! callback, with the source stream it shares with the face; the zlib
//! stream's buffers are the file's, given to each `inflate()` call.

use std::sync::Arc;

use super::super::base::ftmemory::ft_qalloc;
use super::super::base::ftstream::*;
use super::super::fttypes::*;
use super::zlib::*;

/***************************************************************************/
/***************************************************************************/
/*****                                                                 *****/
/*****            Z L I B   M E M O R Y   M A N A G E M E N T          *****/
/*****                                                                 *****/
/***************************************************************************/
/***************************************************************************/

/* (`ft_gzip_alloc` and `ft_gzip_free`: zlib's memory is Rust's) */

/***************************************************************************/
/***************************************************************************/
/*****                                                                 *****/
/*****               Z L I B   F I L E   D E S C R I P T O R           *****/
/*****                                                                 *****/
/***************************************************************************/
/***************************************************************************/

pub const FT_GZIP_BUFFER_SIZE: usize = 4096;

/// `FT_GZipFileRec` (`zstream` is the inflate state between calls, with
/// its buffer positions: `next_in` in `input`, `next_out` in `buffer`)
#[derive(Debug)]
pub struct FtGZipFileRec {
    pub source: FtSharedStream, /* parent/source stream        */
    zstream: ZStreamState,      /* zlib input stream           */
    next_in: usize,
    avail_in: u32,
    next_out: usize,
    avail_out: u32,

    pub start: FtULong, /* starting position, after .gz header */
    pub input: Box<[FtByte; FT_GZIP_BUFFER_SIZE]>, /* input read buffer  */

    pub buffer: Box<[FtByte; FT_GZIP_BUFFER_SIZE]>, /* output buffer      */
    pub pos: FtULong,                               /* position in output */
    pub cursor: usize,
    pub limit: usize,
}

impl FtGZipFileRec {
    /// Runs `f` on the zlib stream over the file's buffers.
    fn with_zstream<R>(&mut self, f: impl FnOnce(&mut ZStream<'_>) -> R) -> R {
        let st = std::mem::take(&mut self.zstream);
        let mut zstream = ZStream::with_state(
            &self.input[..],
            self.next_in,
            self.avail_in,
            &mut self.buffer[..],
            self.next_out,
            self.avail_out,
            st,
        );
        let r = f(&mut zstream);
        self.next_in = zstream.next_in;
        self.avail_in = zstream.avail_in;
        self.next_out = zstream.next_out;
        self.avail_out = zstream.avail_out;
        self.zstream = zstream.into_state();
        r
    }
}

/* gzip flag byte */
const FT_GZIP_ASCII_FLAG: u8 = 0x01; /* bit 0 set: file probably ascii text */
const FT_GZIP_HEAD_CRC: u8 = 0x02; /* bit 1 set: header CRC present */
const FT_GZIP_EXTRA_FIELD: u8 = 0x04; /* bit 2 set: extra field present */
const FT_GZIP_ORIG_NAME: u8 = 0x08; /* bit 3 set: original file name present */
const FT_GZIP_COMMENT: u8 = 0x10; /* bit 4 set: file comment present */
const FT_GZIP_RESERVED: u8 = 0xE0; /* bits 5..7: reserved */

/// `ft_gzip_check_header`: check and skip .gz header - we don't support
/// `transparent' compression
fn ft_gzip_check_header(stream: &mut FtStreamRec) -> FtResult<()> {
    let mut head = [0u8; 4];

    stream.seek(0)?;
    stream.read(&mut head)?;

    /* head[0] && head[1] are the magic numbers;    */
    /* head[2] is the method, and head[3] the flags */
    if head[0] != 0x1F
        || head[1] != 0x8B
        || head[2] as u32 != Z_DEFLATED
        || (head[3] & FT_GZIP_RESERVED) != 0
    {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* skip time, xflags and os code */
    /* (the macro sets `error') */
    let mut error = stream.skip(6);

    /* skip the extra field */
    if head[3] & FT_GZIP_EXTRA_FIELD != 0 {
        let len = stream.read_ushort_le()? as FtUInt;
        stream.skip(len as FtLong)?;
        error = Ok(());
    }

    /* skip original file name */
    if head[3] & FT_GZIP_ORIG_NAME != 0 {
        loop {
            let c = stream.read_byte()? as FtUInt;
            error = Ok(());

            if c == 0 {
                break;
            }
        }
    }

    /* skip .gz comment */
    if head[3] & FT_GZIP_COMMENT != 0 {
        loop {
            let c = stream.read_byte()? as FtUInt;
            error = Ok(());

            if c == 0 {
                break;
            }
        }
    }

    /* skip CRC */
    if head[3] & FT_GZIP_HEAD_CRC != 0 {
        stream.skip(2)?;
        error = Ok(());
    }

    /* Exit: */
    error
}

/// `ft_gzip_file_init`
fn ft_gzip_file_init(source: &FtSharedStream) -> FtResult<FtGZipFileRec> {
    let mut zip = FtGZipFileRec {
        source: source.clone(),
        zstream: ZStreamState::default(),
        next_in: 0,
        avail_in: 0,
        next_out: 0,
        avail_out: 0,
        start: 0,
        input: Box::new([0; FT_GZIP_BUFFER_SIZE]),
        buffer: Box::new([0; FT_GZIP_BUFFER_SIZE]),
        pos: 0,
        cursor: FT_GZIP_BUFFER_SIZE,
        limit: FT_GZIP_BUFFER_SIZE,
    };

    /* check and skip .gz header */
    {
        let mut stream = ft_lock_stream(source);

        ft_gzip_check_header(&mut stream)?;

        zip.start = stream.pos();
    }

    /* initialize zlib -- there is no zlib header in the compressed stream */
    zip.avail_in = 0;
    /* (`next_in' is the output buffer: it is not read before it is set) */
    zip.next_in = 0;

    if zip.with_zstream(|zstream| inflate_init2(zstream, -MAX_WBITS)) != Z_OK {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    Ok(zip)
}

/// `ft_gzip_file_done`
fn ft_gzip_file_done(zip: &mut FtGZipFileRec) {
    zip.with_zstream(inflate_end);

    /* clear the rest */
    zip.next_in = 0;
    zip.next_out = 0;
    zip.avail_in = 0;
    zip.avail_out = 0;
}

/// `ft_gzip_file_reset`
fn ft_gzip_file_reset(zip: &mut FtGZipFileRec) -> FtResult<()> {
    let error = ft_lock_stream(&zip.source).seek(zip.start);
    if error.is_ok() {
        zip.with_zstream(inflate_reset);

        zip.avail_in = 0;
        zip.next_in = 0;
        zip.avail_out = 0;
        zip.next_out = 0;

        zip.limit = FT_GZIP_BUFFER_SIZE;
        zip.cursor = zip.limit;
        zip.pos = 0;
    }

    error
}

/// `ft_gzip_file_fill_input`
fn ft_gzip_file_fill_input(zip: &mut FtGZipFileRec) -> FtResult<()> {
    let mut stream = ft_lock_stream(&zip.source);
    let size: FtULong;

    let pos = stream.pos;
    if let Some(read) = stream.read.as_mut() {
        size = read(pos, &mut zip.input[..]);
        if size == 0 {
            zip.limit = zip.cursor;
            return Err(FT_ERR_INVALID_STREAM_OPERATION);
        }
    } else {
        let mut n = stream.size.wrapping_sub(stream.pos);
        if n > FT_GZIP_BUFFER_SIZE as FtULong {
            n = FT_GZIP_BUFFER_SIZE as FtULong;
        }

        if n == 0 {
            zip.limit = zip.cursor;
            return Err(FT_ERR_INVALID_STREAM_OPERATION);
        }

        let base = stream
            .memory_base()
            .cloned()
            .unwrap_or_else(|| Arc::from(Vec::new()));
        let p = pos as usize;
        let n_ = (n as usize).min(base.len().saturating_sub(p));
        zip.input[..n_].copy_from_slice(&base[p..p + n_]);
        size = n;
    }
    stream.pos = stream.pos.wrapping_add(size);

    zip.next_in = 0;
    zip.avail_in = size as u32;

    Ok(())
}

/// `ft_gzip_file_fill_output`
fn ft_gzip_file_fill_output(zip: &mut FtGZipFileRec) -> FtResult<()> {
    let mut error = Ok(());

    zip.cursor = 0;
    zip.next_out = zip.cursor;
    zip.avail_out = FT_GZIP_BUFFER_SIZE as u32;

    while zip.avail_out > 0 {
        if zip.avail_in == 0 {
            error = ft_gzip_file_fill_input(zip);
            if error.is_err() {
                break;
            }
        }

        let err = zip.with_zstream(|zstream| inflate(zstream, Z_NO_FLUSH));

        if err == Z_STREAM_END {
            zip.limit = zip.next_out;
            if zip.limit == zip.cursor {
                error = Err(FT_ERR_INVALID_STREAM_OPERATION);
            }
            break;
        } else if err != Z_OK {
            zip.limit = zip.cursor;
            error = Err(FT_ERR_INVALID_STREAM_OPERATION);
            break;
        }
    }

    error
}

/// `ft_gzip_file_skip_output`: fill output buffer; `count' must be <=
/// FT_GZIP_BUFFER_SIZE
fn ft_gzip_file_skip_output(zip: &mut FtGZipFileRec, mut count: FtULong) -> FtResult<()> {
    let mut error = Ok(());

    loop {
        let mut delta = (zip.limit - zip.cursor) as FtULong;

        if delta >= count {
            delta = count;
        }

        zip.cursor += delta as usize;
        zip.pos += delta;

        count -= delta;
        if count == 0 {
            break;
        }

        error = ft_gzip_file_fill_output(zip);
        if error.is_err() {
            break;
        }
    }

    error
}

/// `ft_gzip_file_io` (an empty `buffer` is C's NULL with a count of 0)
fn ft_gzip_file_io(zip: &mut FtGZipFileRec, pos: FtULong, buffer: &mut [FtByte]) -> FtULong {
    let mut result: FtULong = 0;
    let mut count = buffer.len() as FtULong;

    'exit: {
        /* Reset inflate stream if we're seeking backwards.        */
        /* Yes, that is not too efficient, but it saves memory :-) */
        if pos < zip.pos && ft_gzip_file_reset(zip).is_err() {
            break 'exit;
        }

        /* skip unwanted bytes */
        if pos > zip.pos && ft_gzip_file_skip_output(zip, pos - zip.pos).is_err() {
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

            if ft_gzip_file_fill_output(zip).is_err() {
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
/*****               G Z   E M B E D D I N G   S T R E A M             *****/
/*****                                                                 *****/
/***************************************************************************/
/***************************************************************************/

/// `ft_gzip_stream_close`: the stream's callback, with the gzip file in
/// it, goes when the stream does (and the memory of a memory-based one
/// with it).
impl Drop for FtGZipFileRec {
    fn drop(&mut self) {
        /* finalize gzip file descriptor */
        ft_gzip_file_done(self);
    }
}

/// `ft_gzip_get_uncompressed_size`
fn ft_gzip_get_uncompressed_size(stream: &mut FtStreamRec) -> FtULong {
    let mut result: FtULong = 0;

    let old_pos = stream.pos;
    if stream.seek(stream.size.wrapping_sub(4)).is_ok() {
        result = stream.read_ulong_le().unwrap_or(0) as FtULong;

        let _ = stream.seek(old_pos);
    }

    result
}

/// `FT_Stream_OpenGzip`: a stream reading the gzip-compressed `source`.
pub fn ft_stream_open_gzip(source: &FtSharedStream) -> FtResult<FtStream> {
    /*
     * check the header right now; this prevents allocating un-necessary
     * objects when we don't need them
     */
    ft_gzip_check_header(&mut ft_lock_stream(source))?;

    let mut zip = ft_gzip_file_init(source)?;
    /* (`stream->descriptor.pointer = zip') */

    let size: FtULong;

    /*
     * We use the following trick to try to dramatically improve the
     * performance while dealing with small files.  If the original stream
     * size is less than a certain threshold, we try to load the whole font
     * file into memory.  This saves us from using the 32KB buffer needed
     * to inflate the file, plus the two 4KB intermediate input/output
     * buffers used in the `FT_GZipFile' structure.
     */
    {
        let zip_size = ft_gzip_get_uncompressed_size(&mut ft_lock_stream(source));

        if zip_size != 0 && zip_size < 40 * 1024 {
            if let Ok(mut zip_buff) = ft_qalloc(zip_size as FtLong) {
                let count = ft_gzip_file_io(&mut zip, 0, &mut zip_buff);
                if count == zip_size {
                    drop(zip);

                    return Ok(FtStreamRec::open_memory(Arc::from(zip_buff)));
                }

                ft_gzip_file_io(&mut zip, 0, &mut []);
            }
        }

        size = if zip_size != 0 {
            zip_size
        } else {
            0x7FFFFFFF /* don't know the real size! */
        };
    }

    /* `ft_gzip_stream_io` */
    let read: FtStreamIoFunc =
        Box::new(move |offset, buffer: &mut [u8]| ft_gzip_file_io(&mut zip, offset, buffer));

    Ok(FtStreamRec::open_callback(size, read))
}

/* documentation is in ftgzip.h */
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
