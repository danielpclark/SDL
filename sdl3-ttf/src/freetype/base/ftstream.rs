// Rust translation of src/base/ftstream.c, include/freetype/ftsystem.h
// (the stream record) and include/freetype/internal/ftstream.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2000-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! I/O stream support (body).
//!
//! A stream is either memory-based (`base` holds the whole file) or reads
//! through a callback (`read`), as SDL_ttf's streams over an
//! `SDL_IOStream` do. A frame is either a window into the memory (memory
//! streams) or a block read into memory (callback streams), and the
//! `cursor`/`limit` pointers become indices into it. The
//! `FT_Stream_ReadFields()` frame descriptors are replaced by the field
//! reads they describe (`enter_frame`, the `get_*` functions in field
//! order, `exit_frame`), which is what they do.
//!
//! The `FT_PEEK_*`/`FT_NEXT_*` macros over a byte pointer become the
//! `ft_peek_*`/`ft_next_*` functions over a slice and an index; reading
//! past the end of the slice gives zero bytes instead of reading out of
//! bounds (Note (upstream): the C relies on its callers' range checks).

use std::sync::Arc;

use super::super::fttypes::*;
use super::ftmemory::ft_qalloc;

/// The callback of a stream (`FT_Stream_IoFunc`): reads up to
/// `buffer.len()` bytes at `offset` and returns the number of bytes read.
/// With an empty buffer it is a seek, which returns 0 on success.
pub type FtStreamIoFunc = Box<dyn FnMut(u64, &mut [u8]) -> u64 + Send>;

/// `FT_StreamRec`
pub struct FtStreamRec {
    /// the memory of a memory-based stream
    pub base: Option<Arc<[u8]>>,
    pub size: FtULong,
    pub pos: FtULong,
    pub read: Option<FtStreamIoFunc>,
    /* the frame of a callback stream (C's `base` while a frame is entered) */
    frame: Vec<u8>,
    /* start of the frame in `base` (memory streams) */
    frame_start: usize,
    /* cursor and limit, as offsets into the frame; `None` is C's NULL */
    cursor: Option<usize>,
    limit: usize,
}

impl std::fmt::Debug for FtStreamRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtStreamRec")
            .field("size", &self.size)
            .field("pos", &self.pos)
            .field("memory", &self.base.is_some())
            .field("read", &self.read.is_some())
            .finish()
    }
}

/// `FT_Stream`: streams are owned (and closed when dropped).
pub type FtStream = Box<FtStreamRec>;

/* FT_PEEK_* and FT_NEXT_* */

#[inline]
fn byte_at(p: &[u8], i: usize) -> u8 {
    p.get(i).copied().unwrap_or(0)
}

/// `FT_PEEK_SHORT`
#[inline]
pub fn ft_peek_short(p: &[u8], i: usize) -> i16 {
    ft_peek_ushort(p, i) as i16
}
/// `FT_PEEK_USHORT`
#[inline]
pub fn ft_peek_ushort(p: &[u8], i: usize) -> u16 {
    ((byte_at(p, i) as u16) << 8) | byte_at(p, i + 1) as u16
}
/// `FT_PEEK_LONG`
#[inline]
pub fn ft_peek_long(p: &[u8], i: usize) -> i32 {
    ft_peek_ulong(p, i) as i32
}
/// `FT_PEEK_ULONG`
#[inline]
pub fn ft_peek_ulong(p: &[u8], i: usize) -> u32 {
    ((byte_at(p, i) as u32) << 24)
        | ((byte_at(p, i + 1) as u32) << 16)
        | ((byte_at(p, i + 2) as u32) << 8)
        | byte_at(p, i + 3) as u32
}
/// `FT_PEEK_OFF3`
#[inline]
pub fn ft_peek_off3(p: &[u8], i: usize) -> i32 {
    (((byte_at(p, i) as u32) << 24)
        | ((byte_at(p, i + 1) as u32) << 16)
        | ((byte_at(p, i + 2) as u32) << 8)) as i32
        >> 8
}
/// `FT_PEEK_UOFF3`
#[inline]
pub fn ft_peek_uoff3(p: &[u8], i: usize) -> u32 {
    ((byte_at(p, i) as u32) << 16) | ((byte_at(p, i + 1) as u32) << 8) | byte_at(p, i + 2) as u32
}
/// `FT_PEEK_SHORT_LE`
#[inline]
pub fn ft_peek_short_le(p: &[u8], i: usize) -> i16 {
    ft_peek_ushort_le(p, i) as i16
}
/// `FT_PEEK_USHORT_LE`
#[inline]
pub fn ft_peek_ushort_le(p: &[u8], i: usize) -> u16 {
    ((byte_at(p, i + 1) as u16) << 8) | byte_at(p, i) as u16
}
/// `FT_PEEK_LONG_LE`
#[inline]
pub fn ft_peek_long_le(p: &[u8], i: usize) -> i32 {
    ft_peek_ulong_le(p, i) as i32
}
/// `FT_PEEK_ULONG_LE`
#[inline]
pub fn ft_peek_ulong_le(p: &[u8], i: usize) -> u32 {
    ((byte_at(p, i + 3) as u32) << 24)
        | ((byte_at(p, i + 2) as u32) << 16)
        | ((byte_at(p, i + 1) as u32) << 8)
        | byte_at(p, i) as u32
}
/// `FT_PEEK_UOFF3_LE`
#[inline]
pub fn ft_peek_uoff3_le(p: &[u8], i: usize) -> u32 {
    ((byte_at(p, i + 2) as u32) << 16) | ((byte_at(p, i + 1) as u32) << 8) | byte_at(p, i) as u32
}

/// `FT_NEXT_CHAR`
#[inline]
pub fn ft_next_char(p: &[u8], i: &mut usize) -> i8 {
    let v = byte_at(p, *i) as i8;
    *i += 1;
    v
}
/// `FT_NEXT_BYTE`
#[inline]
pub fn ft_next_byte(p: &[u8], i: &mut usize) -> u8 {
    let v = byte_at(p, *i);
    *i += 1;
    v
}
/// `FT_NEXT_SHORT`
#[inline]
pub fn ft_next_short(p: &[u8], i: &mut usize) -> i16 {
    let v = ft_peek_short(p, *i);
    *i += 2;
    v
}
/// `FT_NEXT_USHORT`
#[inline]
pub fn ft_next_ushort(p: &[u8], i: &mut usize) -> u16 {
    let v = ft_peek_ushort(p, *i);
    *i += 2;
    v
}
/// `FT_NEXT_OFF3`
#[inline]
pub fn ft_next_off3(p: &[u8], i: &mut usize) -> i32 {
    let v = ft_peek_off3(p, *i);
    *i += 3;
    v
}
/// `FT_NEXT_UOFF3`
#[inline]
pub fn ft_next_uoff3(p: &[u8], i: &mut usize) -> u32 {
    let v = ft_peek_uoff3(p, *i);
    *i += 3;
    v
}
/// `FT_NEXT_LONG`
#[inline]
pub fn ft_next_long(p: &[u8], i: &mut usize) -> i32 {
    let v = ft_peek_long(p, *i);
    *i += 4;
    v
}
/// `FT_NEXT_ULONG`
#[inline]
pub fn ft_next_ulong(p: &[u8], i: &mut usize) -> u32 {
    let v = ft_peek_ulong(p, *i);
    *i += 4;
    v
}
/// `FT_NEXT_SHORT_LE`
#[inline]
pub fn ft_next_short_le(p: &[u8], i: &mut usize) -> i16 {
    let v = ft_peek_short_le(p, *i);
    *i += 2;
    v
}
/// `FT_NEXT_USHORT_LE`
#[inline]
pub fn ft_next_ushort_le(p: &[u8], i: &mut usize) -> u16 {
    let v = ft_peek_ushort_le(p, *i);
    *i += 2;
    v
}
/// `FT_NEXT_LONG_LE`
#[inline]
pub fn ft_next_long_le(p: &[u8], i: &mut usize) -> i32 {
    let v = ft_peek_long_le(p, *i);
    *i += 4;
    v
}
/// `FT_NEXT_ULONG_LE`
#[inline]
pub fn ft_next_ulong_le(p: &[u8], i: &mut usize) -> u32 {
    let v = ft_peek_ulong_le(p, *i);
    *i += 4;
    v
}

impl FtStreamRec {
    /// `FT_Stream_OpenMemory`
    pub fn open_memory(base: Arc<[u8]>) -> FtStream {
        let size = base.len() as FtULong;
        Box::new(FtStreamRec {
            base: Some(base),
            size,
            pos: 0,
            read: None,
            frame: Vec::new(),
            frame_start: 0,
            cursor: None,
            limit: 0,
        })
    }

    /// A stream reading through a callback (an `FT_StreamRec` with its
    /// `read` function set, as SDL_ttf's over an `SDL_IOStream`).
    pub fn open_callback(size: FtULong, read: FtStreamIoFunc) -> FtStream {
        Box::new(FtStreamRec {
            base: None,
            size,
            pos: 0,
            read: Some(read),
            frame: Vec::new(),
            frame_start: 0,
            cursor: None,
            limit: 0,
        })
    }

    /// `FT_Stream_Seek`
    pub fn seek(&mut self, pos: FtULong) -> FtResult<()> {
        let mut error = FT_ERR_OK;

        if let Some(read) = self.read.as_mut() {
            if read(pos, &mut []) != 0 {
                error = FT_ERR_INVALID_STREAM_OPERATION;
            }
        }
        /* note that seeking to the first position after the file is valid */
        else if pos > self.size {
            error = FT_ERR_INVALID_STREAM_OPERATION;
        }

        if error == 0 {
            self.pos = pos;
            Ok(())
        } else {
            Err(error)
        }
    }

    /// `FT_Stream_Skip`
    pub fn skip(&mut self, distance: FtLong) -> FtResult<()> {
        if distance < 0 {
            return Err(FT_ERR_INVALID_STREAM_OPERATION);
        }

        self.seek(self.pos.wrapping_add(distance as FtULong))
    }

    /// `FT_Stream_Pos`
    pub fn pos(&self) -> FtULong {
        self.pos
    }

    /// `FT_Stream_Read`
    pub fn read(&mut self, buffer: &mut [u8]) -> FtResult<()> {
        self.read_at(self.pos, buffer)
    }

    /// `FT_Stream_ReadAt`
    pub fn read_at(&mut self, pos: FtULong, buffer: &mut [u8]) -> FtResult<()> {
        let count = buffer.len() as FtULong;
        let read_bytes: FtULong;

        if pos >= self.size {
            return Err(FT_ERR_INVALID_STREAM_OPERATION);
        }

        if let Some(read) = self.read.as_mut() {
            read_bytes = read(pos, buffer);
        } else {
            let mut n = self.size - pos;
            if n > count {
                n = count;
            }

            /* Allow "reading" zero bytes without UB even if buffer is NULL */
            if count != 0 {
                let base = self.base.as_ref().map(|b| &b[..]).unwrap_or(&[]);
                let p = pos as usize;
                buffer[..n as usize].copy_from_slice(&base[p..p + n as usize]);
            }
            read_bytes = n;
        }

        self.pos = pos.wrapping_add(read_bytes);

        if read_bytes < count {
            return Err(FT_ERR_INVALID_STREAM_OPERATION);
        }

        Ok(())
    }

    /// `FT_Stream_TryRead`
    pub fn try_read(&mut self, buffer: &mut [u8]) -> FtULong {
        let count = buffer.len() as FtULong;
        let mut read_bytes: FtULong = 0;

        if self.pos >= self.size {
            return read_bytes;
        }

        if let Some(read) = self.read.as_mut() {
            read_bytes = read(self.pos, buffer);
        } else {
            read_bytes = self.size - self.pos;
            if read_bytes > count {
                read_bytes = count;
            }

            /* Allow "reading" zero bytes without UB even if buffer is NULL */
            if count != 0 {
                let base = self.base.as_ref().map(|b| &b[..]).unwrap_or(&[]);
                let p = self.pos as usize;
                buffer[..read_bytes as usize].copy_from_slice(&base[p..p + read_bytes as usize]);
            }
        }

        self.pos = self.pos.wrapping_add(read_bytes);

        read_bytes
    }

    /// `FT_Stream_ExtractFrame`: the frame's bytes, owned (for a memory
    /// stream, a copy of the window C would point into).
    pub fn extract_frame(&mut self, count: FtULong) -> FtResult<Vec<u8>> {
        self.enter_frame(count)?;

        let bytes = if self.read.is_some() {
            std::mem::take(&mut self.frame)
        } else {
            let mut v = ft_qalloc(count as FtLong)?;
            v.copy_from_slice(self.frame_data());
            v
        };

        /* equivalent to FT_Stream_ExitFrame(), with no memory block release */
        self.cursor = None;
        self.limit = 0;

        Ok(bytes)
    }

    /// `FT_Stream_EnterFrame`
    pub fn enter_frame(&mut self, count: FtULong) -> FtResult<()> {
        let mut error = FT_ERR_OK;

        /* check for nested frame access */
        debug_assert!(self.cursor.is_none());

        if self.read.is_some() {
            /* allocate the frame in memory */

            /* simple sanity check */
            if count > self.size {
                return Err(FT_ERR_INVALID_STREAM_OPERATION);
            }

            self.frame = ft_qalloc(count as FtLong)?;

            /* read it */
            let pos = self.pos;
            let read = self.read.as_mut().unwrap();
            let read_bytes = read(pos, &mut self.frame);
            if read_bytes < count {
                self.frame = Vec::new();
                error = FT_ERR_INVALID_STREAM_OPERATION;
            }

            self.cursor = Some(0);
            self.limit = count as usize;
            self.pos = self.pos.wrapping_add(read_bytes);
        } else {
            /* check current and new position */
            if self.pos >= self.size || self.size - self.pos < count {
                return Err(FT_ERR_INVALID_STREAM_OPERATION);
            }

            /* set cursor */
            self.frame_start = self.pos as usize;
            self.cursor = Some(0);
            self.limit = count as usize;
            self.pos += count;
        }

        if error != 0 {
            self.cursor = None;
            self.limit = 0;
            return Err(error);
        }
        Ok(())
    }

    /// `FT_Stream_ExitFrame`
    pub fn exit_frame(&mut self) {
        /* IMPORTANT: The assertion stream->cursor != 0 was removed, given    */
        /*            that it is possible to access a frame of length 0 in    */
        /*            some weird fonts (usually, when accessing an array of   */
        /*            0 records, like in some strange kern tables).           */
        /*                                                                    */
        /*  In this case, the loader code handles the 0-length table          */
        /*  gracefully; however, stream.cursor is really set to 0 by the      */
        /*  FT_Stream_EnterFrame() call, and this is not an error.            */

        if self.read.is_some() {
            self.frame = Vec::new();
        }

        self.cursor = None;
        self.limit = 0;
    }

    /// The whole current frame (from its start to `limit`).
    pub fn frame_data(&self) -> &[u8] {
        if self.read.is_some() {
            &self.frame[..self.limit.min(self.frame.len())]
        } else {
            match self.base.as_ref() {
                Some(b) => &b[self.frame_start..self.frame_start + self.limit],
                None => &[],
            }
        }
    }

    /// `stream->cursor`, as an offset into [`frame_data`](Self::frame_data).
    pub fn cursor(&self) -> usize {
        self.cursor.unwrap_or(0)
    }

    /// The offset of the frame's start from C's `stream->base`: the frame's
    /// position in the memory of a memory-based stream, and 0 for a
    /// callback stream (whose `base` is the frame block).
    pub fn frame_origin(&self) -> usize {
        if self.read.is_some() {
            0
        } else {
            self.frame_start
        }
    }

    /// Sets `stream->cursor` (an offset into the frame).
    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = Some(cursor);
    }

    /// `stream->limit`, as an offset into the frame.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// The memory of a memory-based stream (`stream->base`).
    pub fn memory_base(&self) -> Option<&Arc<[u8]>> {
        self.base.as_ref()
    }

    /// `FT_Stream_GetByte`
    pub fn get_byte(&mut self) -> u8 {
        let mut result = 0;
        let c = self.cursor();

        if c < self.limit {
            result = byte_at(self.frame_data(), c);
            self.cursor = Some(c + 1);
        }
        result
    }

    /// `FT_GET_CHAR`
    pub fn get_char(&mut self) -> i8 {
        self.get_byte() as i8
    }

    /// `FT_Stream_GetUShort`
    pub fn get_ushort(&mut self) -> u16 {
        let mut result = 0;
        let c = self.cursor();

        if c + 1 < self.limit {
            result = ft_peek_ushort(self.frame_data(), c);
            self.cursor = Some(c + 2);
        }
        result
    }

    /// `FT_GET_SHORT`
    pub fn get_short(&mut self) -> i16 {
        self.get_ushort() as i16
    }

    /// `FT_Stream_GetUShortLE`
    pub fn get_ushort_le(&mut self) -> u16 {
        let mut result = 0;
        let c = self.cursor();

        if c + 1 < self.limit {
            result = ft_peek_ushort_le(self.frame_data(), c);
            self.cursor = Some(c + 2);
        }
        result
    }

    /// `FT_Stream_GetUOffset`
    pub fn get_uoffset(&mut self) -> u32 {
        let mut result = 0;
        let c = self.cursor();

        if c + 2 < self.limit {
            result = ft_peek_uoff3(self.frame_data(), c);
            self.cursor = Some(c + 3);
        }
        result
    }

    /// `FT_Stream_GetULong`
    pub fn get_ulong(&mut self) -> u32 {
        let mut result = 0;
        let c = self.cursor();

        if c + 3 < self.limit {
            result = ft_peek_ulong(self.frame_data(), c);
            self.cursor = Some(c + 4);
        }
        result
    }

    /// `FT_GET_LONG`
    pub fn get_long(&mut self) -> i32 {
        self.get_ulong() as i32
    }

    /// `FT_Stream_GetULongLE`
    pub fn get_ulong_le(&mut self) -> u32 {
        let mut result = 0;
        let c = self.cursor();

        if c + 3 < self.limit {
            result = ft_peek_ulong_le(self.frame_data(), c);
            self.cursor = Some(c + 4);
        }
        result
    }

    /// Reads `N` bytes at the stream position, for `FT_Stream_ReadByte`
    /// and friends.
    fn read_small<const N: usize>(&mut self) -> FtResult<[u8; N]> {
        let mut reads = [0u8; N];

        if self.pos + (N as u64 - 1) < self.size {
            if let Some(read) = self.read.as_mut() {
                if read(self.pos, &mut reads) != N as u64 {
                    return Err(FT_ERR_INVALID_STREAM_OPERATION);
                }
            } else {
                let base = self.base.as_ref().map(|b| &b[..]).unwrap_or(&[]);
                let p = self.pos as usize;
                reads.copy_from_slice(&base[p..p + N]);
            }
        } else {
            return Err(FT_ERR_INVALID_STREAM_OPERATION);
        }

        self.pos += N as u64;
        Ok(reads)
    }

    /// `FT_Stream_ReadByte`
    pub fn read_byte(&mut self) -> FtResult<u8> {
        Ok(self.read_small::<1>()?[0])
    }

    /// `FT_READ_CHAR`
    pub fn read_char(&mut self) -> FtResult<i8> {
        Ok(self.read_byte()? as i8)
    }

    /// `FT_Stream_ReadUShort`
    pub fn read_ushort(&mut self) -> FtResult<u16> {
        let r = self.read_small::<2>()?;
        Ok(ft_peek_ushort(&r, 0))
    }

    /// `FT_READ_SHORT`
    pub fn read_short(&mut self) -> FtResult<i16> {
        Ok(self.read_ushort()? as i16)
    }

    /// `FT_Stream_ReadUShortLE`
    pub fn read_ushort_le(&mut self) -> FtResult<u16> {
        let r = self.read_small::<2>()?;
        Ok(ft_peek_ushort_le(&r, 0))
    }

    /// `FT_READ_SHORT_LE`
    pub fn read_short_le(&mut self) -> FtResult<i16> {
        Ok(self.read_ushort_le()? as i16)
    }

    /// `FT_Stream_ReadUOffset`
    pub fn read_uoffset(&mut self) -> FtResult<u32> {
        let r = self.read_small::<3>()?;
        Ok(ft_peek_uoff3(&r, 0))
    }

    /// `FT_Stream_ReadULong`
    pub fn read_ulong(&mut self) -> FtResult<u32> {
        let r = self.read_small::<4>()?;
        Ok(ft_peek_ulong(&r, 0))
    }

    /// `FT_READ_LONG`
    pub fn read_long(&mut self) -> FtResult<i32> {
        Ok(self.read_ulong()? as i32)
    }

    /// `FT_Stream_ReadULongLE`
    pub fn read_ulong_le(&mut self) -> FtResult<u32> {
        let r = self.read_small::<4>()?;
        Ok(ft_peek_ulong_le(&r, 0))
    }

    /// `FT_READ_LONG_LE`
    pub fn read_long_le(&mut self) -> FtResult<i32> {
        Ok(self.read_ulong_le()? as i32)
    }

    /// `FT_Stream_ReadFields` with an `ft_frame_bytes` field: copies `len`
    /// bytes from the frame at the cursor.
    pub fn get_bytes(&mut self, out: &mut [u8]) -> FtResult<()> {
        let c = self.cursor();
        let len = out.len();

        if c + len > self.limit {
            return Err(FT_ERR_INVALID_STREAM_OPERATION);
        }
        out.copy_from_slice(&self.frame_data()[c..c + len]);
        self.cursor = Some(c + len);
        Ok(())
    }

    /// `FT_Stream_ReadFields` with an `ft_frame_skip` field.
    pub fn skip_bytes(&mut self, len: usize) -> FtResult<()> {
        let c = self.cursor();

        if c + len > self.limit {
            return Err(FT_ERR_INVALID_STREAM_OPERATION);
        }
        self.cursor = Some(c + len);
        Ok(())
    }
}
