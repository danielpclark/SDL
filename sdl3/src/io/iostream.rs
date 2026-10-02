// Rust translation of src/io/SDL_iostream.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* This file provides a general interface for SDL to read and write
   data sources.  It can easily be extended to files, memory, etc.
*/

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::properties::Properties;

/// `SDL_IOStream` status, set by a read or write operation. Translation of `SDL_IOStatus`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum IoStatus {
    /// Everything is ready (no errors and not EOF).
    #[default]
    Ready,
    /// Read or write I/O error
    Error,
    /// End of file
    Eof,
    /// Non blocking I/O, not ready
    NotReady,
    /// Tried to write a read-only buffer
    ReadOnly,
    /// Tried to read a write-only buffer
    WriteOnly,
}

/// Possible `whence` values for [`IoStream::seek`]. Translation of `SDL_IOWhence`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IoWhence {
    /// Seek from the beginning of data
    Set,
    /// Seek relative to current read point
    Cur,
    /// Seek relative to the end of data
    End,
}

/// How a read, write or flush stopped short: the bytes moved before it
/// stopped, the status to report, and the error message if any (the
/// `*status = ...; SDL_SetError(...)` pair of an `SDL_IOStreamInterface`).
#[derive(Debug)]
pub struct IoStop {
    pub bytes: usize,
    pub status: IoStatus,
    pub error: Option<Error>,
}

impl IoStop {
    /// A stop with a status and no error message (EOF, not ready).
    pub fn status(bytes: usize, status: IoStatus) -> Self {
        IoStop {
            bytes,
            status,
            error: None,
        }
    }
    /// A stop with [`IoStatus::Error`] and a message.
    pub fn error(bytes: usize, error: Error) -> Self {
        IoStop {
            bytes,
            status: IoStatus::Error,
            error: Some(error),
        }
    }
}

/// The function table an [`IoStream`] dispatches to. Translation of
/// `SDL_IOStreamInterface`; implement it to provide your own stream type
/// (the `void *userdata` is `self`).
///
/// A method left at its default behaves like a `NULL` function pointer.
pub trait IoInterface: Send {
    /// Whether [`size`](Self::size) is implemented (`iface.size != NULL`).
    fn has_size(&self) -> bool {
        false
    }
    /// Return the number of bytes in this stream.
    fn size(&mut self) -> Result<i64> {
        Err(Error::unsupported())
    }
    /// Seek to `offset` relative to `whence`; return the final offset.
    fn seek(&mut self, _offset: i64, _whence: IoWhence) -> Result<i64> {
        Err(Error::unsupported())
    }
    /// Whether [`read`](Self::read) is implemented.
    fn can_read(&self) -> bool {
        false
    }
    /// Read up to `buf.len()` bytes. A short read that is not an error or
    /// EOF may return `Ok`.
    fn read(&mut self, _buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        Ok(0)
    }
    /// Whether [`write`](Self::write) is implemented.
    fn can_write(&self) -> bool {
        false
    }
    /// Write up to `buf.len()` bytes.
    fn write(&mut self, _buf: &[u8]) -> std::result::Result<usize, IoStop> {
        Ok(0)
    }
    /// Make sure written data is on its way to the destination.
    fn flush(&mut self) -> std::result::Result<(), IoStop> {
        Ok(())
    }
    /// Close and free resources. Called once, by [`IoStream::close`] or drop.
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
    /// Fill in this stream's properties the first time they are requested
    /// (the internal `setioprops` callback).
    fn set_properties(&mut self, _props: &Properties) -> Result<()> {
        Ok(())
    }
    /// The bytes of a dynamic-memory stream (`SDL_PROP_IOSTREAM_DYNAMIC_MEMORY_POINTER`).
    fn dynamic_memory(&self) -> Option<&[u8]> {
        None
    }
}

/// The read/write operation structure. Translation of `SDL_IOStream`.
///
/// Create one with [`IoStream::from_file`], [`IoStream::from_mem`],
/// [`IoStream::from_const_mem`], [`IoStream::from_dynamic_mem`] or
/// [`IoStream::open`]; it implements `std::io::{Read, Write, Seek}` as well
/// as SDL's own API. Dropping it closes it (use [`close`](Self::close) to
/// see the close error).
pub struct IoStream<'a> {
    iface: Box<dyn IoInterface + 'a>,
    status: IoStatus,
    props: Option<Properties>,
    last_error: Option<Error>,
    closed: bool,
}

impl fmt::Debug for IoStream<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IoStream")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// SDL_PROP_IOSTREAM_FILE_DESCRIPTOR_NUMBER
pub const PROP_IOSTREAM_FILE_DESCRIPTOR_NUMBER: &str = "SDL.iostream.file_descriptor";
/// SDL_PROP_IOSTREAM_MEMORY_SIZE_NUMBER
pub const PROP_IOSTREAM_MEMORY_SIZE_NUMBER: &str = "SDL.iostream.memory.size";
/// SDL_PROP_IOSTREAM_DYNAMIC_CHUNKSIZE_NUMBER: how many bytes a dynamic
/// memory stream grows by (default 1024).
pub const PROP_IOSTREAM_DYNAMIC_CHUNKSIZE_NUMBER: &str = "SDL.iostream.dynamic.chunksize";

/// `strerror(errno)`: the OS error text without std's " (os error N)" suffix.
pub(crate) fn strerror(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.rfind(" (os error ") {
        Some(i) => s[..i].to_owned(),
        None => s,
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// Translation of `READAHEAD_BUFFER_SIZE`.
const READAHEAD_BUFFER_SIZE: usize = 1024;
/// Translation of `WRITEBEHIND_BUFFER_SIZE`.
const WRITEBEHIND_BUFFER_SIZE: usize = 1024;

/// A buffered file stream. Translation of upstream's `IOStreamWindowsData`
/// backend (1 KiB read-ahead and write-behind buffers, append by seeking to
/// the end before each write), with the Win32 calls replaced by `std::fs`.
struct FileStream {
    h: Option<File>,
    disk: bool,
    data: Box<[u8; READAHEAD_BUFFER_SIZE]>,
    size: usize,
    left: usize,
    write_data: Box<[u8; WRITEBEHIND_BUFFER_SIZE]>,
    write_pos: usize,
    writable: bool,
    append: bool,
    autoclose: bool,
}

impl FileStream {
    fn file(&mut self) -> &mut File {
        self.h.as_mut().expect("file is open until close()")
    }

    /// Translation of `windows_flush_write_buffer()`.
    fn flush_write_buffer(&mut self) -> std::result::Result<(), IoStop> {
        if self.write_pos == 0 {
            return Ok(()); // Nothing to flush
        }

        // In append mode, seek to EOF before writing
        if self.append {
            if let Err(e) = self.file().seek(SeekFrom::End(0)) {
                return Err(IoStop::error(
                    0,
                    Error::new(format!("Error seeking in datastream: {}", strerror(&e))),
                ));
            }
        }

        let pos = self.write_pos;
        let mut buf = [0u8; WRITEBEHIND_BUFFER_SIZE];
        buf[..pos].copy_from_slice(&self.write_data[..pos]);
        if let Err(e) = self.file().write_all(&buf[..pos]) {
            return Err(IoStop::error(
                0,
                Error::new(format!("Error writing to datastream: {}", strerror(&e))),
            ));
        }
        self.write_pos = 0;
        Ok(())
    }

    fn read_error(e: &std::io::Error) -> IoStop {
        match e.kind() {
            std::io::ErrorKind::BrokenPipe => IoStop::status(0, IoStatus::Eof),
            std::io::ErrorKind::WouldBlock => IoStop::status(0, IoStatus::NotReady),
            _ => IoStop::error(
                0,
                Error::new(format!("Error reading from datastream: {}", strerror(e))),
            ),
        }
    }
}

impl IoInterface for FileStream {
    fn has_size(&self) -> bool {
        self.disk
    }

    /// Translation of `windows_file_size()`.
    fn size(&mut self) -> Result<i64> {
        if !self.disk {
            return Err(Error::unsupported());
        }
        // Upstream's Win32 backend reports the on-disk size without flushing
        // its write-behind buffer, so a size taken right after a small write
        // is stale there; the POSIX fd backend has no buffer. This backend
        // serves every platform, so it flushes and matches POSIX.
        self.flush_write_buffer().map_err(|stop| {
            stop.error
                .unwrap_or_else(|| Error::new("Error writing to datastream"))
        })?;
        self.file()
            .metadata()
            .map(|m| m.len() as i64)
            .map_err(|e| Error::new(format!("Couldn't get stream size: {}", strerror(&e))))
    }

    /// Translation of `windows_file_seek()`.
    fn seek(&mut self, mut offset: i64, whence: IoWhence) -> Result<i64> {
        if !self.disk {
            return Err(Error::unsupported());
        }
        if let Err(stop) = self.flush_write_buffer() {
            return Err(stop
                .error
                .unwrap_or_else(|| Error::new("Error writing to datastream")));
        }

        // FIXME: We may be able to satisfy the seek within buffered data
        if whence == IoWhence::Cur && self.left != 0 {
            offset -= self.left as i64;
        }
        self.left = 0;

        let pos = match whence {
            IoWhence::Set => {
                if offset < 0 {
                    return Err(Error::new("Error seeking in datastream: Invalid argument"));
                }
                SeekFrom::Start(offset as u64)
            }
            IoWhence::Cur => SeekFrom::Current(offset),
            IoWhence::End => SeekFrom::End(offset),
        };
        self.file()
            .seek(pos)
            .map(|p| p as i64)
            .map_err(|e| Error::new(format!("Error seeking in datastream: {}", strerror(&e))))
    }

    fn can_read(&self) -> bool {
        true
    }

    /// Translation of `windows_file_read()`.
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let size = buf.len();
        let mut total_need = size;
        let mut total_read = 0usize;
        let mut ptr = 0usize;

        self.flush_write_buffer()?;

        if self.left > 0 {
            let start = self.size - self.left;
            let read_ahead = total_need.min(self.left);
            buf[..read_ahead].copy_from_slice(&self.data[start..start + read_ahead]);
            self.left -= read_ahead;

            if read_ahead == total_need {
                return Ok(size);
            }
            ptr += read_ahead;
            total_need -= read_ahead;
            total_read += read_ahead;
        }

        let mut eof = false;
        if total_need < READAHEAD_BUFFER_SIZE {
            let mut data = [0u8; READAHEAD_BUFFER_SIZE];
            let bytes = match self.file().read(&mut data) {
                Ok(n) => n,
                // !!! FIXME: this should return the bytes read from any readahead we finished out before this (the `iodata->left > 0` code above). In that case, fail on the next read.
                Err(e) => return Err(Self::read_error(&e)),
            };
            if bytes == 0 {
                eof = true;
            }
            self.data[..bytes].copy_from_slice(&data[..bytes]);
            let read_ahead = total_need.min(bytes);
            buf[ptr..ptr + read_ahead].copy_from_slice(&self.data[..read_ahead]);
            self.size = bytes;
            self.left = bytes - read_ahead;
            total_read += read_ahead;
        } else {
            let bytes = match self.file().read(&mut buf[ptr..ptr + total_need]) {
                Ok(n) => n,
                // !!! FIXME: see above.
                Err(e) => return Err(Self::read_error(&e)),
            };
            if bytes == 0 {
                eof = true;
            }
            total_read += bytes;
        }

        // Like `fread()` in upstream's POSIX stdio backend (and unlike a
        // single `ReadFile()`), keep reading until the request is filled, so
        // a short read always means end of file.
        while !eof && total_read < size {
            match self.file().read(&mut buf[total_read..]) {
                Ok(0) => eof = true,
                Ok(n) => total_read += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    let mut stop = Self::read_error(&e);
                    stop.bytes = total_read;
                    return Err(stop);
                }
            }
        }

        if eof {
            Err(IoStop::status(total_read, IoStatus::Eof))
        } else {
            Ok(total_read)
        }
    }

    fn can_write(&self) -> bool {
        true
    }

    /// Translation of `windows_file_write()`.
    fn write(&mut self, src: &[u8]) -> std::result::Result<usize, IoStop> {
        let size = src.len();
        let mut remaining = size;
        let mut total_written = 0usize;
        let mut p = 0usize;

        if !self.writable {
            return Err(IoStop::status(0, IoStatus::ReadOnly));
        }

        // Invalidate read-ahead buffer if it has data
        if self.left != 0 {
            let left = self.left as i64;
            if let Err(e) = self.file().seek(SeekFrom::Current(-left)) {
                return Err(IoStop::error(
                    0,
                    Error::new(format!("Error seeking in datastream: {}", strerror(&e))),
                ));
            }
            self.left = 0;
        }

        // For large writes, flush buffer and write directly
        if size >= WRITEBEHIND_BUFFER_SIZE {
            self.flush_write_buffer()?;

            // In append mode, seek to EOF before direct write
            if self.append {
                if let Err(e) = self.file().seek(SeekFrom::End(0)) {
                    return Err(IoStop::error(
                        0,
                        Error::new(format!("Error seeking in datastream: {}", strerror(&e))),
                    ));
                }
            }

            return match self.file().write(src) {
                Ok(0) if size > 0 => Err(IoStop::status(0, IoStatus::NotReady)),
                Ok(bytes) => Ok(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    Err(IoStop::status(0, IoStatus::NotReady))
                }
                Err(e) => Err(IoStop::error(
                    0,
                    Error::new(format!("Error writing to datastream: {}", strerror(&e))),
                )),
            };
        }

        // Buffer small writes
        while remaining > 0 {
            let space = WRITEBEHIND_BUFFER_SIZE - self.write_pos;
            let to_buffer = remaining.min(space);

            self.write_data[self.write_pos..self.write_pos + to_buffer]
                .copy_from_slice(&src[p..p + to_buffer]);

            self.write_pos += to_buffer;
            p += to_buffer;
            remaining -= to_buffer;
            total_written += to_buffer;

            if self.write_pos == WRITEBEHIND_BUFFER_SIZE {
                if let Err(mut stop) = self.flush_write_buffer() {
                    stop.bytes = total_written;
                    return Err(stop);
                }
            }
        }

        Ok(total_written)
    }

    /// Translation of `windows_file_flush()` / `stdio_flush()` (data sync).
    fn flush(&mut self) -> std::result::Result<(), IoStop> {
        self.flush_write_buffer()?;

        // Sync to disk
        if self.disk && self.writable {
            if let Err(e) = self.file().sync_data() {
                return Err(IoStop::error(
                    0,
                    Error::new(format!("Error flushing datastream: {}", strerror(&e))),
                ));
            }
        }
        Ok(())
    }

    /// Translation of `windows_file_close()`.
    fn close(&mut self) -> Result<()> {
        let mut result = Ok(());
        if let Err(stop) = self.flush_write_buffer() {
            result = Err(stop
                .error
                .unwrap_or_else(|| Error::new("Error writing to datastream")));
        }
        if let Some(h) = self.h.take() {
            if !self.autoclose {
                // The caller keeps ownership of the descriptor/handle.
                std::mem::forget(h);
            }
        }
        result
    }

    fn set_properties(&mut self, props: &Properties) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if let Some(h) = &self.h {
                props.set(PROP_IOSTREAM_FILE_DESCRIPTOR_NUMBER, h.as_raw_fd() as i64)?;
            }
        }
        let _ = props;
        Ok(())
    }
}

/// The access flags of an fopen-style mode string, as `windows_file_open()`
/// derives them.
fn open_options(mode: &str) -> Option<OpenOptions> {
    // "r" = reading, file must exist
    // "w" = writing, truncate existing, file may not exist
    // "wx"= writing, file must not exist
    // "r+"= reading or writing, file must exist
    // "a" = writing, append file may not exist
    // "a+"= append + read, file may not exist
    // "w+" = read, write, truncate. file may not exist
    // "w+x"= read, write, file must not exist
    let must_exist = mode.contains('r');
    let truncate = mode.contains('w');
    let r_right = mode.contains('+') || must_exist;
    let a_mode = mode.contains('a');
    let w_right = a_mode || mode.contains('+') || truncate;
    let create_new = truncate && mode.contains('x');

    if !r_right && !w_right {
        return None; // inconsistent mode
    }

    let mut o = OpenOptions::new();
    o.read(r_right).write(w_right);
    if create_new {
        o.create_new(true);
    } else if truncate {
        o.create(true).truncate(true);
    } else if a_mode {
        o.create(true);
    }
    Some(o)
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

/// Translation of `IOStreamMemData` (with `base`/`here`/`stop` as offsets).
#[derive(Debug, Default)]
struct MemData {
    here: usize,
    stop: usize,
}

impl MemData {
    /// Translation of `mem_seek()`.
    fn seek(&mut self, offset: i64, whence: IoWhence) -> i64 {
        let base = match whence {
            IoWhence::Set => 0i64,
            IoWhence::Cur => self.here as i64,
            IoWhence::End => self.stop as i64,
        };
        let newpos = base.saturating_add(offset).clamp(0, self.stop as i64);
        self.here = newpos as usize;
        self.here as i64
    }

    /// Bytes available from `here` (`mem_io()`'s clamp).
    fn available(&self, size: usize) -> usize {
        size.min(self.stop - self.here)
    }
}

/// `SDL_IOFromMem()`'s backend.
struct MemStream<'a> {
    mem: &'a mut [u8],
    data: MemData,
}

impl IoInterface for MemStream<'_> {
    fn has_size(&self) -> bool {
        true
    }
    fn size(&mut self) -> Result<i64> {
        Ok(self.data.stop as i64)
    }
    fn seek(&mut self, offset: i64, whence: IoWhence) -> Result<i64> {
        Ok(self.data.seek(offset, whence))
    }
    fn can_read(&self) -> bool {
        true
    }
    /// Translation of `mem_read()`.
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let n = self.data.available(buf.len());
        buf[..n].copy_from_slice(&self.mem[self.data.here..self.data.here + n]);
        self.data.here += n;
        if n < buf.len() && self.data.stop == self.data.here {
            return Err(IoStop::status(n, IoStatus::Eof));
        }
        Ok(n)
    }
    fn can_write(&self) -> bool {
        true
    }
    /// Translation of `mem_write()`.
    fn write(&mut self, buf: &[u8]) -> std::result::Result<usize, IoStop> {
        let n = self.data.available(buf.len());
        self.mem[self.data.here..self.data.here + n].copy_from_slice(&buf[..n]);
        self.data.here += n;
        if n < buf.len() && self.data.stop == self.data.here {
            return Err(IoStop::error(n, Error::new("Memory buffer is full")));
        }
        Ok(n)
    }
    /// Translation of `mem_setioprops()`.
    fn set_properties(&mut self, props: &Properties) -> Result<()> {
        props.set(PROP_IOSTREAM_MEMORY_SIZE_NUMBER, self.mem.len() as i64)
    }
}

/// `SDL_IOFromConstMem()`'s backend (no write function).
struct ConstMemStream<'a> {
    mem: &'a [u8],
    data: MemData,
}

impl IoInterface for ConstMemStream<'_> {
    fn has_size(&self) -> bool {
        true
    }
    fn size(&mut self) -> Result<i64> {
        Ok(self.data.stop as i64)
    }
    fn seek(&mut self, offset: i64, whence: IoWhence) -> Result<i64> {
        Ok(self.data.seek(offset, whence))
    }
    fn can_read(&self) -> bool {
        true
    }
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let n = self.data.available(buf.len());
        buf[..n].copy_from_slice(&self.mem[self.data.here..self.data.here + n]);
        self.data.here += n;
        if n < buf.len() && self.data.stop == self.data.here {
            return Err(IoStop::status(n, IoStatus::Eof));
        }
        Ok(n)
    }
    // leave write as NULL.
    fn set_properties(&mut self, props: &Properties) -> Result<()> {
        props.set(PROP_IOSTREAM_MEMORY_SIZE_NUMBER, self.mem.len() as i64)
    }
}

/// `SDL_IOFromDynamicMem()`'s backend. Translation of `IOStreamDynamicMemData`.
#[derive(Default)]
struct DynamicMemStream {
    props: Option<Properties>,
    /// The allocation (`base`..`end`); `data.stop` is the logical size.
    buf: Vec<u8>,
    data: MemData,
}

impl DynamicMemStream {
    /// Translation of `dynamic_mem_realloc()`.
    fn realloc(&mut self, size: usize) {
        let mut chunksize = self
            .props
            .as_ref()
            .and_then(|p| p.get_number(PROP_IOSTREAM_DYNAMIC_CHUNKSIZE_NUMBER))
            .unwrap_or(1024) as usize;
        if chunksize == 0 {
            chunksize = 1024;
        }

        // We're intentionally allocating more memory than needed so it can be null terminated
        let chunks = ((self.buf.len() + size) / chunksize) + 1;
        let length = chunks * chunksize;
        self.buf.resize(length, 0);
    }
}

impl IoInterface for DynamicMemStream {
    fn has_size(&self) -> bool {
        true
    }
    fn size(&mut self) -> Result<i64> {
        Ok(self.data.stop as i64)
    }
    fn seek(&mut self, offset: i64, whence: IoWhence) -> Result<i64> {
        Ok(self.data.seek(offset, whence))
    }
    fn can_read(&self) -> bool {
        true
    }
    /// Translation of `dynamic_mem_read()`.
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
        let n = self.data.available(buf.len());
        buf[..n].copy_from_slice(&self.buf[self.data.here..self.data.here + n]);
        self.data.here += n;
        if n < buf.len() && self.data.stop == self.data.here {
            return Err(IoStop::status(n, IoStatus::Eof));
        }
        Ok(n)
    }
    fn can_write(&self) -> bool {
        true
    }
    /// Translation of `dynamic_mem_write()`.
    fn write(&mut self, src: &[u8]) -> std::result::Result<usize, IoStop> {
        let size = src.len();
        if size > self.data.stop - self.data.here {
            if size > self.buf.len() - self.data.here {
                self.realloc(size);
            }
            self.data.stop = self.data.here + size;
        }
        let n = self.data.available(size);
        self.buf[self.data.here..self.data.here + n].copy_from_slice(&src[..n]);
        self.data.here += n;
        crate::sdl_assert!(n == size); // we should have allocated enough to cover this!
        Ok(n)
    }
    fn set_properties(&mut self, props: &Properties) -> Result<()> {
        self.props = Some(props.clone());
        Ok(())
    }
    fn dynamic_memory(&self) -> Option<&[u8]> {
        Some(&self.buf[..self.data.stop])
    }
}

// ---------------------------------------------------------------------------
// The stream object
// ---------------------------------------------------------------------------

impl<'a> IoStream<'a> {
    /// Create a stream over a custom implementation. Translation of `SDL_OpenIO()`.
    pub fn open(iface: impl IoInterface + 'a) -> IoStream<'a> {
        IoStream {
            iface: Box::new(iface),
            status: IoStatus::Ready,
            props: None,
            last_error: None,
            closed: false,
        }
    }

    /// A stream over a mutable memory buffer of fixed size (writes past its
    /// end fail with "Memory buffer is full"). Translation of `SDL_IOFromMem()`.
    pub fn from_mem(mem: &'a mut [u8]) -> IoStream<'a> {
        let stop = mem.len();
        IoStream::open(MemStream {
            mem,
            data: MemData { here: 0, stop },
        })
    }

    /// A read-only stream over memory. Translation of `SDL_IOFromConstMem()`.
    pub fn from_const_mem(mem: &'a [u8]) -> IoStream<'a> {
        let stop = mem.len();
        IoStream::open(ConstMemStream {
            mem,
            data: MemData { here: 0, stop },
        })
    }

    /// The status of the last read or write. Translation of `SDL_GetIOStatus()`.
    pub fn status(&self) -> IoStatus {
        self.status
    }

    /// The error message of the last failed operation (what `SDL_GetError()`
    /// would report after it).
    pub fn last_error(&self) -> Option<&Error> {
        self.last_error.as_ref()
    }

    fn fail<T>(&mut self, e: Error) -> Result<T> {
        self.last_error = Some(e.clone());
        Err(e)
    }

    /// This stream's properties, created on first use. Translation of `SDL_GetIOProperties()`.
    pub fn properties(&mut self) -> Result<Properties> {
        if let Some(p) = &self.props {
            return Ok(p.clone());
        }
        let props = Properties::new();
        self.iface.set_properties(&props)?;
        self.props = Some(props.clone());
        Ok(props)
    }

    /// The size of the data stream. Translation of `SDL_GetIOSize()`.
    pub fn size(&mut self) -> Result<i64> {
        if !self.iface.has_size() {
            let pos = self.seek(0, IoWhence::Cur)?;
            let size = self.seek(0, IoWhence::End);
            let _ = self.seek(pos, IoWhence::Set);
            return size;
        }
        match self.iface.size() {
            Ok(s) => Ok(s),
            Err(e) => self.fail(e),
        }
    }

    /// Seek within the data stream; returns the final offset.
    /// Translation of `SDL_SeekIO()`.
    pub fn seek(&mut self, offset: i64, whence: IoWhence) -> Result<i64> {
        match self.iface.seek(offset, whence) {
            Ok(p) => Ok(p),
            Err(e) => self.fail(e),
        }
    }

    /// The current read/write offset. Translation of `SDL_TellIO()`.
    pub fn tell(&mut self) -> Result<i64> {
        self.seek(0, IoWhence::Cur)
    }

    /// Read up to `buf.len()` bytes; check [`status`](Self::status) when
    /// fewer arrive. Translation of `SDL_ReadIO()`.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        if !self.iface.can_read() {
            self.status = IoStatus::WriteOnly;
            self.last_error = Some(Error::unsupported());
            return 0;
        }

        if buf.is_empty() {
            return 0; // context->status doesn't change for this.
        }

        self.status = IoStatus::Ready;
        self.last_error = None;
        match self.iface.read(buf) {
            Ok(n) => n,
            Err(stop) => {
                self.status = stop.status;
                self.last_error = stop.error;
                stop.bytes
            }
        }
    }

    /// Write up to `buf.len()` bytes; check [`status`](Self::status) when
    /// fewer are written. Translation of `SDL_WriteIO()`.
    pub fn write(&mut self, buf: &[u8]) -> usize {
        if !self.iface.can_write() {
            self.status = IoStatus::ReadOnly;
            self.last_error = Some(Error::unsupported());
            return 0;
        }

        if buf.is_empty() {
            return 0; // context->status doesn't change for this.
        }

        self.status = IoStatus::Ready;
        self.last_error = None;
        match self.iface.write(buf) {
            Ok(n) => n,
            Err(stop) => {
                self.status = stop.status;
                self.last_error = stop.error;
                stop.bytes
            }
        }
    }

    /// Flush buffered writes. Translation of `SDL_FlushIO()`.
    pub fn flush(&mut self) -> Result<()> {
        self.status = IoStatus::Ready;
        self.last_error = None;
        match self.iface.flush() {
            Ok(()) => Ok(()),
            Err(stop) => {
                self.status = if stop.status == IoStatus::Ready {
                    IoStatus::Error
                } else {
                    stop.status
                };
                let e = stop
                    .error
                    .unwrap_or_else(|| Error::new("Error flushing datastream"));
                self.fail(e)
            }
        }
    }

    /// Close the stream, reporting any error from flushing or closing.
    /// Translation of `SDL_CloseIO()`.
    pub fn close(mut self) -> Result<()> {
        self.closed = true;
        self.iface.close()
    }

    /// Read everything to the end. Translation of `SDL_LoadFile_IO()`.
    ///
    /// Upstream returns the data even when the stream reports an error and
    /// leaves the status for the caller to check; here that case is `Err`.
    pub fn load_all(&mut self) -> Result<Vec<u8>> {
        const FILE_CHUNK_SIZE: i64 = 1024;
        let mut loading_chunks = false;

        let mut size = match self.size() {
            Ok(s) if s >= 0 => s,
            _ => {
                loading_chunks = true;
                FILE_CHUNK_SIZE
            }
        };
        if size as u64 >= usize::MAX as u64 - 1 {
            return Err(Error::out_of_memory());
        }

        let mut data = vec![0u8; size as usize];
        let mut size_total: i64 = 0;
        loop {
            if loading_chunks && (size_total + FILE_CHUNK_SIZE) > size {
                size = size_total + FILE_CHUNK_SIZE;
                data.resize(size as usize, 0);
            }

            let size_read = self.read(&mut data[size_total as usize..size as usize]);
            if size_read > 0 {
                size_total += size_read as i64;
                continue;
            } else if self.status == IoStatus::NotReady {
                // Wait for the stream to be ready
                crate::timer::delay(Duration::from_millis(1));
                continue;
            }

            // The stream status will remain set for the caller to check
            break;
        }
        data.truncate(size_total as usize);
        if self.status == IoStatus::Error {
            let e = self
                .last_error
                .clone()
                .unwrap_or_else(|| Error::new("Error reading from datastream"));
            return Err(e);
        }
        Ok(data)
    }

    /// Write all of `data`. Translation of `SDL_SaveFile_IO()`.
    pub fn save_all(&mut self, data: &[u8]) -> Result<()> {
        let mut size_total = 0usize;
        while size_total < data.len() {
            let size_written = self.write(&data[size_total..]);
            if size_written == 0 {
                if self.status == IoStatus::NotReady {
                    // Wait for the stream to be ready
                    crate::timer::delay(Duration::from_millis(1));
                    continue;
                } else {
                    let e = self
                        .last_error
                        .clone()
                        .unwrap_or_else(|| Error::new("Error writing to datastream"));
                    return Err(e);
                }
            }
            size_total += size_written;
        }
        Ok(())
    }

    /// The bytes written to a [`from_dynamic_mem`](IoStream::from_dynamic_mem)
    /// stream (`SDL_PROP_IOSTREAM_DYNAMIC_MEMORY_POINTER`); `None` for other streams.
    pub fn dynamic_memory(&self) -> Option<&[u8]> {
        self.iface.dynamic_memory()
    }

    /// Format and write text. Translation of `SDL_IOprintf()`; use with `format_args!`.
    pub fn print(&mut self, args: fmt::Arguments<'_>) -> usize {
        let s = fmt::format(args);
        self.write(s.as_bytes())
    }

    fn read_exact_sdl<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut data = [0u8; N];
        if self.read(&mut data) == N {
            Ok(data)
        } else {
            // Upstream returns false and leaves SDL_GetError() as the read set it
            // (empty at EOF).
            Err(self
                .last_error
                .clone()
                .unwrap_or_else(|| Error::new("End of stream")))
        }
    }
}

impl IoStream<'static> {
    /// Open a file. `mode` is an `fopen()` mode string ("rb", "w+b", "a",
    /// "wx", ...). Translation of `SDL_IOFromFile()`.
    pub fn from_file(file: impl AsRef<std::path::Path>, mode: &str) -> Result<IoStream<'static>> {
        let path = file.as_ref();
        if path.as_os_str().is_empty() {
            return Err(Error::invalid_param("file"));
        }
        if mode.is_empty() {
            return Err(Error::invalid_param("mode"));
        }
        let display = path.display();
        let Some(opts) = open_options(mode) else {
            return Err(Error::new(format!(
                "Couldn't open {display}: Invalid argument"
            )));
        };
        let fp = opts
            .open(path)
            .map_err(|e| Error::new(format!("Couldn't open {display}: {}", strerror(&e))))?;
        if fp.metadata().map(|m| m.is_dir()).unwrap_or(false) {
            return Err(Error::new(format!("{display} is a directory")));
        }
        Ok(IoStream::from_std_file(fp, mode, true))
    }

    /// Wrap an already-open file. With `autoclose == false` the file is not
    /// closed when the stream is (its descriptor is leaked back to the
    /// caller, as with C). Translation of `SDL_IOFromHandle()` /
    /// `SDL_IOFromFP()` / `SDL_IOFromFD()`.
    pub fn from_std_file(file: File, mode: &str, autoclose: bool) -> IoStream<'static> {
        let disk = file.metadata().map(|m| m.is_file()).unwrap_or(false);
        IoStream::open(FileStream {
            h: Some(file),
            disk,
            data: Box::new([0; READAHEAD_BUFFER_SIZE]),
            size: 0,
            left: 0,
            write_data: Box::new([0; WRITEBEHIND_BUFFER_SIZE]),
            write_pos: 0,
            writable: mode.contains('w') || mode.contains('a') || mode.contains('+'),
            append: mode.contains('a'),
            autoclose,
        })
    }

    /// A growable in-memory stream; get the bytes back with
    /// [`dynamic_memory`](IoStream::dynamic_memory). Translation of `SDL_IOFromDynamicMem()`.
    pub fn from_dynamic_mem() -> IoStream<'static> {
        IoStream::open(DynamicMemStream::default())
    }
}

impl Drop for IoStream<'_> {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.iface.close();
        }
    }
}

macro_rules! endian_io {
    ($($(#[$m:meta])* $read:ident, $write:ident: $t:ty, $from:ident, $to:ident;)*) => {
        impl IoStream<'_> {
            $(
                $(#[$m])*
                pub fn $read(&mut self) -> Result<$t> {
                    Ok(<$t>::$from(self.read_exact_sdl::<{ std::mem::size_of::<$t>() }>()?))
                }

                $(#[$m])*
                pub fn $write(&mut self, value: $t) -> Result<()> {
                    let bytes = value.$to();
                    if self.write(&bytes) == bytes.len() {
                        Ok(())
                    } else {
                        Err(self.last_error.clone().unwrap_or_else(|| Error::new("Error writing to datastream")))
                    }
                }
            )*
        }
    };
}

endian_io! {
    /// Translation of `SDL_ReadU8()` / `SDL_WriteU8()`.
    read_u8, write_u8: u8, from_le_bytes, to_le_bytes;
    /// Translation of `SDL_ReadS8()` / `SDL_WriteS8()`.
    read_s8, write_s8: i8, from_le_bytes, to_le_bytes;
    /// Translation of `SDL_ReadU16LE()` / `SDL_WriteU16LE()`.
    read_u16_le, write_u16_le: u16, from_le_bytes, to_le_bytes;
    /// Translation of `SDL_ReadS16LE()` / `SDL_WriteS16LE()`.
    read_s16_le, write_s16_le: i16, from_le_bytes, to_le_bytes;
    /// Translation of `SDL_ReadU16BE()` / `SDL_WriteU16BE()`.
    read_u16_be, write_u16_be: u16, from_be_bytes, to_be_bytes;
    /// Translation of `SDL_ReadS16BE()` / `SDL_WriteS16BE()`.
    read_s16_be, write_s16_be: i16, from_be_bytes, to_be_bytes;
    /// Translation of `SDL_ReadU32LE()` / `SDL_WriteU32LE()`.
    read_u32_le, write_u32_le: u32, from_le_bytes, to_le_bytes;
    /// Translation of `SDL_ReadS32LE()` / `SDL_WriteS32LE()`.
    read_s32_le, write_s32_le: i32, from_le_bytes, to_le_bytes;
    /// Translation of `SDL_ReadU32BE()` / `SDL_WriteU32BE()`.
    read_u32_be, write_u32_be: u32, from_be_bytes, to_be_bytes;
    /// Translation of `SDL_ReadS32BE()` / `SDL_WriteS32BE()`.
    read_s32_be, write_s32_be: i32, from_be_bytes, to_be_bytes;
    /// Translation of `SDL_ReadU64LE()` / `SDL_WriteU64LE()`.
    read_u64_le, write_u64_le: u64, from_le_bytes, to_le_bytes;
    /// Translation of `SDL_ReadS64LE()` / `SDL_WriteS64LE()`.
    read_s64_le, write_s64_le: i64, from_le_bytes, to_le_bytes;
    /// Translation of `SDL_ReadU64BE()` / `SDL_WriteU64BE()`.
    read_u64_be, write_u64_be: u64, from_be_bytes, to_be_bytes;
    /// Translation of `SDL_ReadS64BE()` / `SDL_WriteS64BE()`.
    read_s64_be, write_s64_be: i64, from_be_bytes, to_be_bytes;
}

fn to_io_error(e: Option<&Error>, fallback: &str) -> std::io::Error {
    std::io::Error::other(
        e.map(|e| e.to_string())
            .unwrap_or_else(|| fallback.to_owned()),
    )
}

impl std::io::Read for IoStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = IoStream::read(self, buf);
        match self.status {
            IoStatus::Ready | IoStatus::Eof => Ok(n),
            IoStatus::NotReady if n == 0 => Err(std::io::ErrorKind::WouldBlock.into()),
            IoStatus::NotReady => Ok(n),
            _ if n > 0 => Ok(n),
            _ => Err(to_io_error(
                self.last_error.as_ref(),
                "Error reading from datastream",
            )),
        }
    }
}

impl std::io::Write for IoStream<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = IoStream::write(self, buf);
        if n == 0 && !buf.is_empty() {
            if self.status == IoStatus::NotReady {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            return Err(to_io_error(
                self.last_error.as_ref(),
                "Error writing to datastream",
            ));
        }
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        IoStream::flush(self).map_err(|e| std::io::Error::other(e.to_string()))
    }
}

impl std::io::Seek for IoStream<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let (offset, whence) = match pos {
            SeekFrom::Start(o) => (
                i64::try_from(o)
                    .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?,
                IoWhence::Set,
            ),
            SeekFrom::Current(o) => (o, IoWhence::Cur),
            SeekFrom::End(o) => (o, IoWhence::End),
        };
        IoStream::seek(self, offset, whence)
            .map(|p| p as u64)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }
}

/// Load all the data from a file. Translation of `SDL_LoadFile()`.
pub fn load_file(file: impl AsRef<std::path::Path>) -> Result<Vec<u8>> {
    let mut stream = IoStream::from_file(file, "rb")?;
    let data = stream.load_all()?;
    stream.close()?;
    Ok(data)
}

/// Save all the data into a file. Translation of `SDL_SaveFile()`.
pub fn save_file(file: impl AsRef<std::path::Path>, data: &[u8]) -> Result<()> {
    let mut stream = IoStream::from_file(file, "wb")?;
    stream.save_all(data)?;
    stream.close()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("sdl3-rs-iostream-{}-{name}", std::process::id()));
        p
    }

    #[test]
    fn memory_streams() {
        let mut buf = [0u8; 8];
        let mut s = IoStream::from_mem(&mut buf);
        assert_eq!(s.size().unwrap(), 8);
        s.write_u32_be(0x01020304).unwrap();
        s.write_u16_le(0x0605).unwrap();
        assert_eq!(s.tell().unwrap(), 6);
        assert_eq!(s.write(b"abc"), 2);
        assert_eq!(s.status(), IoStatus::Error);
        assert_eq!(s.last_error().unwrap().message(), "Memory buffer is full");
        assert_eq!(
            s.seek(-100, IoWhence::Cur).unwrap(),
            0,
            "seeks clamp to the start"
        );
        assert_eq!(s.read_u8().unwrap(), 1);
        assert_eq!(s.seek(100, IoWhence::Set).unwrap(), 8, "and to the end");
        let mut b = [0u8; 4];
        assert_eq!(s.read(&mut b), 0);
        assert_eq!(s.status(), IoStatus::Eof);
        assert_eq!(
            s.properties()
                .unwrap()
                .get_number(PROP_IOSTREAM_MEMORY_SIZE_NUMBER),
            Some(8)
        );
        drop(s);
        assert_eq!(&buf, &[1, 2, 3, 4, 5, 6, b'a', b'b']);

        let data = [9u8, 8, 7];
        let mut s = IoStream::from_const_mem(&data);
        assert_eq!(s.write(b"x"), 0);
        assert_eq!(s.status(), IoStatus::ReadOnly);
        assert_eq!(s.read_u16_be().unwrap(), 0x0908);
        assert!(s.read_u16_be().is_err(), "short read fails");
        assert_eq!(s.status(), IoStatus::Eof);
        s.seek(0, IoWhence::Set).unwrap();
        assert_eq!(s.load_all().unwrap(), vec![9, 8, 7]);
    }

    #[test]
    fn dynamic_memory() {
        let mut s = IoStream::from_dynamic_mem();
        s.properties()
            .unwrap()
            .set(PROP_IOSTREAM_DYNAMIC_CHUNKSIZE_NUMBER, 4i64)
            .unwrap();
        for i in 0..10u8 {
            s.write_u8(i).unwrap();
        }
        assert_eq!(s.dynamic_memory().unwrap(), &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        s.seek(2, IoWhence::Set).unwrap();
        s.write_u16_be(0xAABB).unwrap();
        assert_eq!(s.size().unwrap(), 10);
        s.seek(0, IoWhence::End).unwrap();
        assert_eq!(s.print(format_args!("{}!", 42)), 3);
        assert_eq!(
            s.dynamic_memory().unwrap(),
            &[0, 1, 0xAA, 0xBB, 4, 5, 6, 7, 8, 9, b'4', b'2', b'!']
        );
        use std::io::{Read, Seek};
        s.rewind().unwrap();
        let mut v = Vec::new();
        s.read_to_end(&mut v).unwrap();
        assert_eq!(v.len(), 13);
    }

    #[test]
    fn files() {
        let path = tmp("files");
        assert!(IoStream::from_file(&path, "rb")
            .unwrap_err()
            .message()
            .starts_with("Couldn't open "));
        assert!(IoStream::from_file(&path, "").is_err());

        // Small buffered writes, then a large direct one.
        let mut s = IoStream::from_file(&path, "wb").unwrap();
        for i in 0..2000u32 {
            s.write_u32_le(i).unwrap();
        }
        let big = vec![0x5Au8; 5000];
        assert_eq!(s.write(&big), 5000);
        s.flush().unwrap();
        assert_eq!(s.size().unwrap(), 13000);
        s.close().unwrap();

        let data = load_file(&path).unwrap();
        assert_eq!(data.len(), 13000);
        assert_eq!(&data[4..8], &1u32.to_le_bytes());

        // Read-ahead buffering and SEEK_CUR accounting
        let mut s = IoStream::from_file(&path, "rb").unwrap();
        assert_eq!(s.read_u32_le().unwrap(), 0);
        assert_eq!(s.tell().unwrap(), 4, "tell accounts for read-ahead");
        s.seek(4 * 1999, IoWhence::Set).unwrap();
        assert_eq!(s.read_u32_le().unwrap(), 1999);
        assert_eq!(s.write(b"x"), 0);
        assert_eq!(s.status(), IoStatus::ReadOnly);
        s.seek(-1, IoWhence::End).unwrap();
        assert_eq!(s.read_u8().unwrap(), 0x5A);
        let mut b = [0u8; 4];
        assert_eq!(s.read(&mut b), 0);
        assert_eq!(s.status(), IoStatus::Eof);
        drop(s);

        // Append mode writes at the end even after seeking
        let mut s = IoStream::from_file(&path, "a+b").unwrap();
        s.seek(0, IoWhence::Set).unwrap();
        s.write(b"END");
        s.close().unwrap();
        let data = load_file(&path).unwrap();
        assert_eq!(&data[data.len() - 3..], b"END");

        // "wx" refuses to overwrite
        assert!(IoStream::from_file(&path, "wx").is_err());

        save_file(&path, b"hello").unwrap();
        assert_eq!(load_file(&path).unwrap(), b"hello");
        std::fs::remove_file(&path).unwrap();

        let dir = std::env::temp_dir();
        if cfg!(unix) {
            assert!(IoStream::from_file(&dir, "rb")
                .unwrap_err()
                .message()
                .ends_with("is a directory"));
        }
    }

    #[test]
    fn custom_interface_without_seek() {
        struct Counter(u8);
        impl IoInterface for Counter {
            fn can_read(&self) -> bool {
                true
            }
            fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, IoStop> {
                if self.0 == 0 {
                    return Err(IoStop::status(0, IoStatus::Eof));
                }
                buf[0] = self.0;
                self.0 -= 1;
                Ok(1)
            }
        }
        let mut s = IoStream::open(Counter(3));
        assert_eq!(s.size().unwrap_err().kind(), crate::ErrorKind::Unsupported);
        assert_eq!(
            s.load_all().unwrap(),
            vec![3, 2, 1],
            "loads in chunks when the size is unknown"
        );
        assert_eq!(s.write(b"x"), 0);
        assert_eq!(s.status(), IoStatus::ReadOnly);
    }
}
