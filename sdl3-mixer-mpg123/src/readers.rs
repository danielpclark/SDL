/* TODO: Check all read calls (in loops, especially!) for return value 0 (EOF)! */
/* Check if get_fileinfo should read ID3 info or not, seems a bit out of place here. */
/* #define EXTRA_DEBUG */
/*
    readers.c: reading input data

    copyright ?-2023 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp
*/
// Modified 2026-10-17: translated into Rust from reader.h and readers.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The stream readers over the caller's I/O callbacks (`READER_STREAM`,
//! and `READER_ICY_STREAM` for ICY metadata), which are what
//! `mpg123_open_handle64()` uses. The feeder and the buffered stream
//! readers (`mpg123_open_feed()`, `MPG123_SEEKBUFFER`) aren't translated:
//! SDL_mixer uses neither, and this behaves as libmpg123 built with
//! `NO_FEEDER` there.

use crate::frame::{frame_index_find, Mpg123Handle};
use crate::mpg123::*;
use crate::parse::read_frame;

pub(crate) const READER_ID3TAG: i32 = 0x2;
pub(crate) const READER_SEEKABLE: i32 = 0x4;
pub(crate) const READER_BUFFERED: i32 = 0x8;
pub(crate) const READER_NOSEEK: i32 = 0x10;
pub(crate) const READER_HANDLEIO: i32 = 0x40;

pub(crate) const READER_ERROR: i32 = MPG123_ERR;
pub(crate) const READER_MORE: i32 = MPG123_NEED_MORE;

/// `SEEK_SET`, the `whence` of an absolute seek.
pub const SEEK_SET: i32 = 0;
/// `SEEK_CUR`, the `whence` of a seek relative to the current position.
pub const SEEK_CUR: i32 = 1;
/// `SEEK_END`, the `whence` of a seek relative to the end.
pub const SEEK_END: i32 = 2;

/// The I/O callbacks of `mpg123_reader64()` and the I/O handle of
/// `mpg123_open_handle64()`, in one: `read` is `r_read`, `lseek` is
/// `r_lseek`, and dropping it is `cleanup`.
pub trait Mpg123Io: Send {
    /// `int (*r_read)(void *handle, void *buf, size_t count, size_t *got)`:
    /// read up to `buf.len()` bytes, storing how many in `got`; a negative
    /// return is an error.
    fn read(&mut self, buf: &mut [u8], got: &mut usize) -> i32;
    /// `int64_t (*r_lseek)(void *handle, int64_t offset, int whence)`: seek
    /// (`whence` is `SEEK_SET`, `SEEK_CUR` or `SEEK_END`, 0, 1 or 2) and
    /// return the new position, or a negative value on error.
    fn lseek(&mut self, offset: i64, whence: i32) -> i64;
}

/// Which reader (`fr->rd`) is in use: one of `readers[]` or the
/// `bad_reader`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReaderKind {
    Bad,
    Stream,
    IcyStream,
}

/// What `r_lseek64` is: unset, upstream's `no_lseek64()` or the I/O
/// handle's `lseek`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lseek {
    Unset,
    Missing,
    Handle,
}

/// Translation of `struct reader_data`.
pub(crate) struct ReaderData<'a> {
    /* total file length or total buffer size */
    pub filelen: i64,
    /* position in file or position in buffer chain */
    pub filepos: i64,
    /* Custom opaque I/O handle from the client. */
    pub iohandle: Option<Box<dyn Mpg123Io + 'a>>,
    pub flags: i32,
    // The one and only lowlevel reader wrapper, wrapping over all others.
    // This is either libmpg123's wrapper or directly the user-supplied functions.
    pub r_read64: bool,
    /// `r_lseek64`
    pub lseek_kind: Lseek,
}

impl Default for ReaderData<'_> {
    fn default() -> Self {
        ReaderData {
            filelen: 0,
            filepos: 0,
            iohandle: None,
            flags: 0,
            r_read64: false,
            lseek_kind: Lseek::Unset,
        }
    }
}

// This is only for streams, so READER_HANDLEIO must be set.
fn fdread(fr: &mut Mpg123Handle<'_>, buf: &mut [u8]) -> isize {
    if (fr.rdat.flags & READER_HANDLEIO) != 0 && fr.rdat.r_read64 {
        if let Some(io) = fr.rdat.iohandle.as_mut() {
            let mut got = 0usize;
            let ret = io.read(buf, &mut got);
            if ret < 0 {
                // error reading %zu bytes
                return -1;
            }
            // Stupid handling, but at least some handling of never-occuring case.
            return got.min(isize::MAX as usize) as isize;
        }
    }
    // no reader setup
    -1
}

fn fdseek(fr: &mut Mpg123Handle<'_>, offset: i64, whence: i32) -> i64 {
    if (fr.rdat.flags & READER_HANDLEIO) != 0 && fr.rdat.lseek_kind != Lseek::Unset {
        return if fr.rdat.flags & READER_NOSEEK != 0 {
            -1
        } else {
            match (fr.rdat.lseek_kind, fr.rdat.iohandle.as_mut()) {
                (Lseek::Handle, Some(io)) => io.lseek(offset, whence),
                // (no_lseek64(), or a NULL handle the callback can't use)
                _ => -1,
            }
        };
    }
    // no reader setup
    -1
}

/* stream based operation  with icy meta data*/
fn icy_fullread(fr: &mut Mpg123Handle<'_>, buf: &mut [u8]) -> isize {
    let count = buf.len() as isize;
    let mut cnt: isize = 0;
    if fr.rdat.flags & READER_SEEKABLE != 0 {
        // mpg123 programmer error: I don't do ICY on seekable streams.
        return -1;
    }
    /*
        There used to be a check for expected file end here (length value or ID3 flag).
        This is not needed:
        1. EOF is indicated by fdread returning zero bytes anyway.
        2. We get false positives of EOF for either files that grew or
        3. ... files that have ID3v1 tags in between (stream with intro).
    */

    while cnt < count {
        /* all icy code is inside this if block, everything else is the plain fullread we know */
        /* debug1("read: %li left", (long) count-cnt); */
        if fr.icy.next < (count - cnt) as i64 {
            let mut temp_buff = [0u8; 1];

            /* we are near icy-metaint boundary, read up to the boundary */
            if fr.icy.next > 0 {
                let cut_pos = fr.icy.next as usize;
                let ret = fdread(fr, &mut buf[cnt as usize..cnt as usize + cut_pos]);
                if ret < 1 {
                    if ret == 0 {
                        break; /* Just EOF. */
                    }
                    // icy boundary read
                    return READER_ERROR as isize;
                }

                if fr.rdat.flags & READER_BUFFERED == 0 {
                    fr.rdat.filepos = fr.rdat.filepos.saturating_add(ret as i64);
                }
                cnt += ret;
                fr.icy.next -= ret as i64;
                if fr.icy.next > 0 {
                    continue;
                }
            }
            /* now off to read icy data */

            /* one byte icy-meta size (must be multiplied by 16 to get icy-meta length) */

            let ret = fdread(fr, &mut temp_buff); /* Getting one single byte hast to suceed. */
            if ret < 0 {
                // reading icy size
                return READER_ERROR as isize;
            }
            if ret == 0 {
                break;
            }

            if fr.rdat.flags & READER_BUFFERED == 0 {
                fr.rdat.filepos = fr.rdat.filepos.saturating_add(ret as i64); /* 1... */
            }

            let meta_size = temp_buff[0] as usize * 16;
            if meta_size != 0 {
                /* we have got some metadata */
                /* TODO: Get rid of this malloc ... perhaps hooking into the reader buffer pool? */
                let mut meta_buff = vec![0u8; meta_size + 1];
                let mut left = meta_size as isize;
                let mut ret = 0;
                while left > 0 {
                    let at = meta_size - left as usize;
                    ret = fdread(fr, &mut meta_buff[at..meta_size]);
                    /* 0 is error here, too... there _must_ be the ICY data, the server promised! */
                    if ret < 1 {
                        // reading icy-meta
                        return READER_ERROR as isize;
                    }
                    left -= ret;
                }
                meta_buff[meta_size] = 0; /* string paranoia */
                if fr.rdat.flags & READER_BUFFERED == 0 {
                    fr.rdat.filepos = fr.rdat.filepos.saturating_add(ret as i64);
                }

                fr.icy.data = Some(meta_buff);
                fr.metaflags |= MPG123_NEW_ICY;
            }
            fr.icy.next = fr.icy.interval;
        } else {
            let ret = plain_fullread(fr, &mut buf[cnt as usize..]);
            if ret < 0 {
                // reading the rest of %li
                return READER_ERROR as isize;
            }
            if ret == 0 {
                break;
            }

            cnt += ret;
            fr.icy.next -= ret as i64;
        }
    }
    /* debug1("done reading, got %li", (long)cnt); */
    cnt
}

/* stream based operation */
fn plain_fullread(fr: &mut Mpg123Handle<'_>, buf: &mut [u8]) -> isize {
    let count = buf.len() as isize;
    let mut cnt: isize = 0;

    /*
        There used to be a check for expected file end here (length value or ID3 flag).
        This is not needed:
        1. EOF is indicated by fdread returning zero bytes anyway.
        2. We get false positives of EOF for either files that grew or
        3. ... files that have ID3v1 tags in between (stream with intro).
    */
    while cnt < count {
        let ret = fdread(fr, &mut buf[cnt as usize..]);
        if ret < 0 {
            return READER_ERROR as isize;
        }
        if ret == 0 {
            break;
        }
        if fr.rdat.flags & READER_BUFFERED == 0 {
            fr.rdat.filepos = fr.rdat.filepos.saturating_add(ret as i64);
        }
        cnt += ret;
    }
    cnt
}

fn stream_lseek(fr: &mut Mpg123Handle<'_>, pos: i64, whence: i32) -> i64 {
    let mut ret = fdseek(fr, pos, whence);
    if ret >= 0 {
        fr.rdat.filepos = ret;
    } else {
        fr.err = MPG123_LSEEK_FAILED;
        ret = READER_ERROR as i64; /* not the original value */
    }
    ret
}

fn stream_close(fr: &mut Mpg123Handle<'_>) {
    if fr.rdat.flags & READER_HANDLEIO != 0 {
        // (cleanup_handle(): dropping the handle)
        fr.rdat.iohandle = None;
    }
}

fn stream_seek_frame(fr: &mut Mpg123Handle<'_>, newframe: i64) -> i32 {
    /* Seekable streams can go backwards and jump forwards.
    Non-seekable streams still can go forward, just not jump. */
    if (fr.rdat.flags & READER_SEEKABLE != 0) || (newframe >= fr.num) {
        let mut preframe = 0; /* a leading frame we jump to */
        /*
            now seek to nearest leading index position and read from there until newframe is reached.
            We use skip_bytes, which handles seekable and non-seekable streams
            (the latter only for positive offset, which we ensured before entering here).
        */
        let seek_to = frame_index_find(fr, newframe, &mut preframe); /* the byte offset we want to reach */
        /* No need to seek to index position if we are closer already.
        But I am picky about fr->num == newframe, play safe by reading the frame again.
        If you think that's stupid, don't call a seek to the current frame. */
        if fr.num >= newframe || fr.num < preframe {
            let to_skip = seek_to - rd_tell(fr); /* bytes to skip to get there (can be negative) */
            if rd_skip_bytes(fr, to_skip) != seek_to {
                return READER_ERROR;
            }

            fr.num = preframe - 1; /* Watch out! I am going to read preframe... fr->num should indicate the frame before! */
        }
        while fr.num < newframe {
            /* try to be non-fatal now... frameNum only gets advanced on success anyway */
            if read_frame(fr) == 0 {
                break;
            }
        }
        /* Now the wanted frame should be ready for decoding. */

        MPG123_OK
    } else {
        fr.err = MPG123_NO_SEEK;
        READER_ERROR /* invalid, no seek happened */
    }
}

/* return FALSE on error, TRUE on success, READER_MORE on occasion */
fn generic_head_read(fr: &mut Mpg123Handle<'_>, newhead: &mut u64) -> i32 {
    let mut hbuf = [0u8; 4];
    let ret = rd_fullread(fr, &mut hbuf) as i32;
    if ret == READER_MORE {
        return ret;
    }
    if ret != 4 {
        return 0;
    }

    *newhead = ((hbuf[0] as u64) << 24)
        | ((hbuf[1] as u64) << 16)
        | ((hbuf[2] as u64) << 8)
        | (hbuf[3] as u64);

    1
}

/* return FALSE on error, TRUE on success, READER_MORE on occasion */
fn generic_head_shift(fr: &mut Mpg123Handle<'_>, head: &mut u64) -> i32 {
    let mut hbuf = [0u8; 1];
    let ret = rd_fullread(fr, &mut hbuf) as i32;
    if ret == READER_MORE {
        return ret;
    }
    if ret != 1 {
        return 0;
    }

    *head <<= 8;
    *head |= hbuf[0] as u64;
    *head &= 0xffffffff;
    1
}

/* returns reached position... negative ones are bad... */
fn stream_skip_bytes(fr: &mut Mpg123Handle<'_>, mut len: i64) -> i64 {
    if fr.rdat.flags & READER_SEEKABLE != 0 {
        let ret = stream_lseek(fr, len, SEEK_CUR);
        if ret < 0 {
            READER_ERROR as i64
        } else {
            ret
        }
    } else if len >= 0 {
        let mut buf = [0u8; 1024]; /* ThOr: Compaq cxx complained and it makes sense to me... or should one do a cast? What for? */
        while len > 0 {
            let num = if len < buf.len() as i64 {
                len as usize
            } else {
                buf.len()
            };
            let ret = rd_fullread(fr, &mut buf[..num]);
            if ret < 0 {
                return ret as i64;
            } else if ret == 0 {
                break; /* EOF... an error? interface defined to tell the actual position... */
            }
            len -= ret as i64;
        }
        rd_tell(fr)
    } else {
        fr.err = MPG123_NO_SEEK;
        READER_ERROR as i64
    }
}

/* Return 0 on success... */
fn stream_back_bytes(fr: &mut Mpg123Handle<'_>, bytes: i64) -> i32 {
    let want = rd_tell(fr).wrapping_sub(bytes);
    if want < 0 {
        return READER_ERROR;
    }
    if stream_skip_bytes(fr, bytes.wrapping_neg()) != want {
        return READER_ERROR;
    }

    0
}

/* returns size on success... otherwise an error code < 0 */
fn generic_read_frame_body(fr: &mut Mpg123Handle<'_>, buf: &mut [u8]) -> i32 {
    let size = buf.len() as isize;
    let l = rd_fullread(fr, buf);
    if l >= 0 && l < size {
        READER_ERROR
    } else {
        l as i32
    }
}

fn generic_tell(fr: &mut Mpg123Handle<'_>) -> i64 {
    fr.rdat.filepos
}

/* This does not (fully) work for non-seekable streams... You have to check for that flag, pal! */
#[allow(dead_code)]
fn stream_rewind(fr: &mut Mpg123Handle<'_>) {
    if fr.rdat.flags & READER_SEEKABLE != 0 {
        fr.rdat.filepos = stream_lseek(fr, 0, SEEK_SET);
    }
}

/*
 * returns length of a file (if filept points to a file)
 * reads the last 128 bytes information into buffer
 * ... that is not totally safe...
 */
fn get_fileinfo(fr: &mut Mpg123Handle<'_>) -> i64 {
    let mut len = fdseek(fr, 0, SEEK_END);
    if len < 0 {
        // cannot seek to end
        return -1;
    } else if len >= 128 {
        if fdseek(fr, -128, SEEK_END) < 0 {
            // cannot seek to END-128
            return -1;
        }
        let mut id3buf = [0u8; 128];
        let got = rd_fullread(fr, &mut id3buf);
        fr.id3buf = id3buf;
        if got != 128 {
            // cannot read ID3v1?!
            return -1;
        }
        if &fr.id3buf[..3] == b"TAG" {
            len -= 128;
        } else {
            fr.id3buf = [0; 128];
        }
    } else {
        // stream too short for ID3
    }

    if fdseek(fr, 0, SEEK_SET) < 0 {
        // cannot seek back
        return -1;
    }
    fr.rdat.filepos = 0; // un-do our seeking here

    len
}

/*****************************************************************
 * read frame helper
 */

fn bugger_off(mh: &mut Mpg123Handle<'_>) -> i32 {
    mh.err = MPG123_NO_READER;
    MPG123_ERR
}

fn stream_init(fr: &mut Mpg123Handle<'_>) -> i32 {
    /* ICY streams of any sort shall not be seekable. */
    if fr.p.icy_interval > 0 {
        fr.rdat.flags |= READER_NOSEEK;
    }

    fr.rdat.filepos = 0;
    fr.rdat.filelen = if fr.p.flags & MPG123_NO_PEEK_END != 0 {
        -1
    } else {
        get_fileinfo(fr)
    };
    if fr.p.flags & MPG123_FORCE_SEEKABLE != 0 {
        fr.rdat.flags |= READER_SEEKABLE;
        fr.rdat.flags &= !READER_NOSEEK;
    }
    /*
        Don't enable seeking on ICY streams, just plain normal files.
        This check is necessary since the client can enforce ICY parsing on files that would otherwise be seekable.
        It is a task for the future to make the ICY parsing safe with seeks ... or not.
    */
    if fr.rdat.filelen >= 0 {
        fr.rdat.flags |= READER_SEEKABLE;
        if &fr.id3buf[..3] == b"TAG" {
            fr.rdat.flags |= READER_ID3TAG;
            fr.metaflags |= MPG123_NEW_ID3;
        }
    }
    /* Switch reader to a buffered one, if allowed. */
    else if fr.p.flags & MPG123_SEEKBUFFER != 0 {
        // (NO_FEEDER: see the module docs)
        // Buffered readers not supported in this build.
        fr.err = MPG123_MISSING_FEATURE;
        return -1;
    }
    0
}

/// Translation of `INT123_open_bad()`.
pub(crate) fn open_bad(mh: &mut Mpg123Handle<'_>) {
    crate::icy::clear_icy(&mut mh.icy);
    mh.rd = ReaderKind::Bad;
    mh.rdat.flags = 0;
    mh.rdat.filelen = -1;
}

/* Final code common to open_stream and INT123_open_stream_handle. */
/// Translation of `INT123_open_stream_handle()`.
pub(crate) fn open_stream_handle<'a>(
    fr: &mut Mpg123Handle<'a>,
    iohandle: Option<Box<dyn Mpg123Io + 'a>>,
) -> i32 {
    crate::icy::clear_icy(&mut fr.icy); /* can be done inside frame_clear ...? */
    fr.rdat.filelen = -1;
    fr.rdat.iohandle = iohandle;
    fr.rdat.flags = 0;
    fr.rdat.flags |= READER_HANDLEIO;

    if fr.p.icy_interval > 0 {
        fr.icy.interval = fr.p.icy_interval;
        fr.icy.next = fr.icy.interval;
        fr.rd = ReaderKind::IcyStream;
    } else {
        fr.rd = ReaderKind::Stream;
    }

    if rd_init(fr) < 0 {
        return -1;
    }

    MPG123_OK
}

// The reader functions (`fr->rd->...`), dispatched on the reader in use.

/// `fr->rd->init()`
pub(crate) fn rd_init(fr: &mut Mpg123Handle<'_>) -> i32 {
    match fr.rd {
        ReaderKind::Bad => bugger_off(fr),
        ReaderKind::Stream | ReaderKind::IcyStream => stream_init(fr),
    }
}

/// `fr->rd->close()`
pub(crate) fn rd_close(fr: &mut Mpg123Handle<'_>) {
    match fr.rd {
        ReaderKind::Bad => {}
        ReaderKind::Stream | ReaderKind::IcyStream => stream_close(fr),
    }
}

/// `fr->rd->fullread()`
pub(crate) fn rd_fullread(fr: &mut Mpg123Handle<'_>, buf: &mut [u8]) -> isize {
    match fr.rd {
        ReaderKind::Bad => bugger_off(fr) as isize,
        ReaderKind::Stream => plain_fullread(fr, buf),
        ReaderKind::IcyStream => icy_fullread(fr, buf),
    }
}

/// `fr->rd->head_read()`: succ: TRUE, else <= 0 (FALSE or READER_MORE)
pub(crate) fn rd_head_read(fr: &mut Mpg123Handle<'_>, newhead: &mut u64) -> i32 {
    match fr.rd {
        ReaderKind::Bad => bugger_off(fr),
        ReaderKind::Stream | ReaderKind::IcyStream => generic_head_read(fr, newhead),
    }
}

/// `fr->rd->head_shift()`: succ: TRUE, else <= 0 (FALSE or READER_MORE)
pub(crate) fn rd_head_shift(fr: &mut Mpg123Handle<'_>, head: &mut u64) -> i32 {
    match fr.rd {
        ReaderKind::Bad => bugger_off(fr),
        ReaderKind::Stream | ReaderKind::IcyStream => generic_head_shift(fr, head),
    }
}

/// `fr->rd->skip_bytes()`: succ: >=0, else error or READER_MORE
pub(crate) fn rd_skip_bytes(fr: &mut Mpg123Handle<'_>, len: i64) -> i64 {
    match fr.rd {
        ReaderKind::Bad => bugger_off(fr) as i64,
        ReaderKind::Stream | ReaderKind::IcyStream => stream_skip_bytes(fr, len),
    }
}

/// `fr->rd->read_frame_body()`
pub(crate) fn rd_read_frame_body(fr: &mut Mpg123Handle<'_>, buf: &mut [u8]) -> i32 {
    match fr.rd {
        ReaderKind::Bad => bugger_off(fr),
        ReaderKind::Stream | ReaderKind::IcyStream => generic_read_frame_body(fr, buf),
    }
}

/// `fr->rd->read_frame_body()` of `size` bytes appended to `v`, whose
/// capacity has been reserved for them: the reads go into the vector as
/// they come, so a huge size from a corrupt header costs no more memory
/// than the data that is there.
pub(crate) fn rd_read_frame_body_vec(
    fr: &mut Mpg123Handle<'_>,
    v: &mut Vec<u8>,
    size: usize,
) -> i32 {
    if fr.rd == ReaderKind::Bad {
        return bugger_off(fr);
    }
    // (generic_read_frame_body() over fullread(), which reads until it has
    // all or the stream ends; reading in steps does the same)
    let mut chunk = vec![0u8; size.min(1 << 16)];
    let mut l: isize = 0;
    while (l as usize) < size {
        let n = (size - l as usize).min(chunk.len());
        let got = rd_fullread(fr, &mut chunk[..n]);
        if got < 0 {
            return got as i32;
        }
        v.extend_from_slice(&chunk[..got as usize]);
        l += got;
        if (got as usize) < n {
            break;
        }
    }
    if (l as usize) < size {
        READER_ERROR
    } else {
        l as i32
    }
}

/// `fr->rd->back_bytes()`
pub(crate) fn rd_back_bytes(fr: &mut Mpg123Handle<'_>, bytes: i64) -> i32 {
    match fr.rd {
        ReaderKind::Bad => bugger_off(fr),
        ReaderKind::Stream | ReaderKind::IcyStream => stream_back_bytes(fr, bytes),
    }
}

/// `fr->rd->seek_frame()`
pub(crate) fn rd_seek_frame(fr: &mut Mpg123Handle<'_>, num: i64) -> i32 {
    match fr.rd {
        ReaderKind::Bad => bugger_off(fr),
        ReaderKind::Stream | ReaderKind::IcyStream => stream_seek_frame(fr, num),
    }
}

/// `fr->rd->tell()`
pub(crate) fn rd_tell(fr: &mut Mpg123Handle<'_>) -> i64 {
    match fr.rd {
        ReaderKind::Bad => bugger_off(fr) as i64,
        ReaderKind::Stream | ReaderKind::IcyStream => generic_tell(fr),
    }
}

/// `if(fr->rd->forget != NULL) fr->rd->forget(fr);`: the stream readers
/// have no `forget` (only the feeder and the buffered readers do).
pub(crate) fn rd_forget(_fr: &mut Mpg123Handle<'_>) {}
