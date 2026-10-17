// Rust translation of src/hio.c, src/hio.h, src/memio.c, src/memio.h,
// src/callbackio.h and src/mdataio.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2024 Claudio Matsuoka and Hipolito Carraro Jr (hio.c, memio.c)
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr (mdataio.h)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The loaders' input: a module in memory (`MFILE`) or read through the
//! caller's callbacks (`CBFILE`). (The `FILE *` handles aren't used by
//! anything translated here.)

use std::borrow::Cow;

use super::dataio::*;
use super::XmpCallbacks;

/// `EOF`
pub(crate) const EOF: i32 = -1;
/// `EINVAL`
const EINVAL: i32 = 22;

pub(crate) const SEEK_SET: i32 = 0;
pub(crate) const SEEK_CUR: i32 = 1;
pub(crate) const SEEK_END: i32 = 2;

/// Translation of `MFILE`.
pub(crate) struct MFile<'a> {
    pub start: Cow<'a, [u8]>,
    pub pos: isize,
    pub size: isize,
}

/// Translation of `CBFILE`.
pub(crate) struct CbFile<'a> {
    pub priv_: &'a mut dyn XmpCallbacks,
    pub eof: i32,
}

/// The `type` and `handle` of `HIO_HANDLE`.
pub(crate) enum HioType<'a> {
    Memory(MFile<'a>),
    CbFile(CbFile<'a>),
}

/// Translation of `HIO_HANDLE`.
pub(crate) struct HioHandle<'a> {
    pub handle: HioType<'a>,
    pub size: i64,
    pub error: i32,
}

impl std::fmt::Debug for HioHandle<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HioHandle")
            .field("size", &self.size)
            .field("error", &self.error)
            .finish()
    }
}

/* memio.c */

impl MFile<'_> {
    /// `CAN_READ()` (mdataio.h's, which memio.c's equals for the sizes
    /// here).
    #[inline]
    fn can_read(&self) -> isize {
        if self.pos >= 0 {
            self.size - self.pos
        } else {
            0
        }
    }

    /// Translation of `mgetc()`.
    fn mgetc(&mut self) -> i32 {
        if self.can_read() >= 1 {
            let c = self.start[self.pos as usize];
            self.pos += 1;
            return c as i32;
        }
        EOF
    }

    /// Translation of `mread()`.
    fn mread(&mut self, buf: &mut [u8], size: usize, num: usize) -> usize {
        let should_read = size.wrapping_mul(num);
        let can_read = self.can_read();

        if size == 0 || num == 0 || can_read <= 0 {
            return 0;
        }

        let p = self.pos as usize;
        if should_read > can_read as usize {
            let n = can_read as usize;
            buf[..n].copy_from_slice(&self.start[p..p + n]);
            self.pos += can_read;

            n / size
        } else {
            buf[..should_read].copy_from_slice(&self.start[p..p + should_read]);
            self.pos += should_read as isize;

            num
        }
    }

    /// Translation of `mseek()`.
    fn mseek(&mut self, offset: i64, whence: i32) -> i32 {
        let mut ofs = offset as isize;

        match whence {
            SEEK_SET => {}
            SEEK_CUR => ofs = ofs.wrapping_add(self.pos),
            SEEK_END => ofs = ofs.wrapping_add(self.size),
            _ => return -1,
        }
        if ofs < 0 {
            return -1;
        }
        if ofs > self.size {
            ofs = self.size;
        }
        self.pos = ofs;
        0
    }

    /// Translation of `mtell()`.
    fn mtell(&self) -> i64 {
        self.pos as i64
    }

    /// Translation of `meof()`.
    fn meof(&self) -> i32 {
        (self.can_read() <= 0) as i32
    }

    /* mdataio.h */

    /// Translation of `mread8()`.
    fn mread8(&mut self, err: &mut i32) -> u8 {
        let mut x = [0xffu8];
        let r = self.mread(&mut x, 1, 1);
        *err = if r == 1 { 0 } else { EOF };
        x[0]
    }

    /// Translation of `mread8s()`.
    fn mread8s(&mut self, err: &mut i32) -> i8 {
        let r = self.mgetc();
        *err = if r < 0 { EOF } else { 0 };
        r as i8
    }

    /// The `mread16l()` family: `n` bytes decoded by `f`, or `bad` past the
    /// end (which moves to the end).
    #[inline]
    fn mread_n<T>(&mut self, n: isize, err: &mut i32, bad: T, f: fn(&[u8]) -> T) -> T {
        let can_read = self.can_read();
        if can_read >= n {
            let p = self.pos as usize;
            let v = f(&self.start[p..p + n as usize]);
            self.pos += n;
            *err = 0;
            v
        } else {
            self.pos += can_read;
            *err = EOF;
            bad
        }
    }
}

/* callbackio.h */

impl CbFile<'_> {
    /// Translation of `cbread8()`.
    fn cbread8(&mut self, err: &mut i32) -> u8 {
        let mut x = [0xffu8];
        let r = self.priv_.read_func(&mut x, 1, 1);
        self.eof = if r == 1 { 0 } else { EOF };

        *err = self.eof;

        x[0]
    }

    /// Translation of `cbread8s()`.
    fn cbread8s(&mut self, err: &mut i32) -> i8 {
        self.cbread8(err) as i8
    }

    /// The `cbread16l()` family: `N` bytes decoded by `f`, or `bad` if not
    /// all of them could be read.
    #[inline]
    fn cbread_n<const N: usize, T>(&mut self, err: &mut i32, bad: T, f: fn(&[u8]) -> T) -> T {
        let mut buf = [0u8; N];
        let mut x = bad;
        let r = self.priv_.read_func(&mut buf, N, 1);
        self.eof = if r == 1 { 0 } else { EOF };

        if r != 0 {
            x = f(&buf);
        }
        *err = self.eof;

        x
    }

    /// Translation of `cbread()`.
    fn cbread(&mut self, dest: &mut [u8], len: usize, nmemb: usize) -> usize {
        let r = self.priv_.read_func(dest, len, nmemb);
        self.eof = if r < nmemb { EOF } else { 0 };
        r
    }

    /// Translation of `cbseek()`.
    fn cbseek(&mut self, offset: i64, whence: i32) -> i32 {
        self.eof = 0;
        self.priv_.seek_func(offset, whence)
    }

    /// Translation of `cbtell()`.
    fn cbtell(&mut self) -> i64 {
        self.priv_.tell_func()
    }

    /// Translation of `cbeof()`.
    fn cbeof(&self) -> i32 {
        self.eof
    }

    /// Translation of `cbfilelength()`.
    fn cbfilelength(&mut self) -> i64 {
        let pos = self.priv_.tell_func();

        if pos < 0 {
            return EOF as i64;
        }

        let r = self.priv_.seek_func(0, SEEK_END);
        if r < 0 {
            return EOF as i64;
        }

        let length = self.priv_.tell_func();
        let _ = self.priv_.seek_func(pos, SEEK_SET);

        length
    }
}

/* hio.c */

macro_rules! hio_read_fn {
    ($(#[$doc:meta])* $name:ident, $t:ty, $mem:expr, $cb:expr) => {
        $(#[$doc])*
        pub(crate) fn $name(h: &mut HioHandle<'_>) -> $t {
            let mut err = 0;
            let ret = match &mut h.handle {
                HioType::Memory(m) => $mem(m, &mut err),
                HioType::CbFile(f) => $cb(f, &mut err),
            };

            if err != 0 {
                h.error = err;
            }
            ret
        }
    };
}

hio_read_fn!(
    /// Translation of `hio_read8s()`.
    hio_read8s, i8, |m: &mut MFile, e| m.mread8s(e), |f: &mut CbFile, e| f.cbread8s(e)
);
hio_read_fn!(
    /// Translation of `hio_read8()`.
    hio_read8, u8, |m: &mut MFile, e| m.mread8(e), |f: &mut CbFile, e| f.cbread8(e)
);
hio_read_fn!(
    /// Translation of `hio_read16l()`.
    hio_read16l, u16,
    |m: &mut MFile, e| m.mread_n(2, e, 0xffff, readmem16l),
    |f: &mut CbFile, e| f.cbread_n::<2, u16>(e, 0xffff, readmem16l)
);
hio_read_fn!(
    /// Translation of `hio_read16b()`.
    hio_read16b, u16,
    |m: &mut MFile, e| m.mread_n(2, e, 0xffff, readmem16b),
    |f: &mut CbFile, e| f.cbread_n::<2, u16>(e, 0xffff, readmem16b)
);
hio_read_fn!(
    /// Translation of `hio_read24l()`.
    hio_read24l, u32,
    |m: &mut MFile, e| m.mread_n(3, e, 0xffffffff, readmem24l),
    |f: &mut CbFile, e| f.cbread_n::<3, u32>(e, 0xffffffff, readmem24l)
);
hio_read_fn!(
    /// Translation of `hio_read24b()`.
    hio_read24b, u32,
    |m: &mut MFile, e| m.mread_n(3, e, 0xffffffff, readmem24b),
    |f: &mut CbFile, e| f.cbread_n::<3, u32>(e, 0xffffffff, readmem24b)
);
hio_read_fn!(
    /// Translation of `hio_read32l()`.
    hio_read32l, u32,
    |m: &mut MFile, e| m.mread_n(4, e, 0xffffffff, readmem32l),
    |f: &mut CbFile, e| f.cbread_n::<4, u32>(e, 0xffffffff, readmem32l)
);
hio_read_fn!(
    /// Translation of `hio_read32b()`.
    hio_read32b, u32,
    |m: &mut MFile, e| m.mread_n(4, e, 0xffffffff, readmem32b),
    |f: &mut CbFile, e| f.cbread_n::<4, u32>(e, 0xffffffff, readmem32b)
);

/// Translation of `hio_read()`: read `num` items of `size` bytes into
/// `buf` (which must hold them).
pub(crate) fn hio_read(buf: &mut [u8], size: usize, num: usize, h: &mut HioHandle<'_>) -> usize {
    let ret;

    match &mut h.handle {
        HioType::Memory(m) => {
            ret = m.mread(buf, size, num);
            if ret != num {
                h.error = EOF;
            }
        }
        HioType::CbFile(f) => {
            ret = f.cbread(buf, size, num);
            if ret != num {
                h.error = EOF;
            }
        }
    }

    ret
}

/// `hio_read(buf, 1, buf.len(), h)`
#[inline]
pub(crate) fn hio_read_bytes(buf: &mut [u8], h: &mut HioHandle<'_>) -> usize {
    let n = buf.len();
    hio_read(buf, 1, n, h)
}

/// Translation of `hio_seek()`.
pub(crate) fn hio_seek(h: &mut HioHandle<'_>, offset: i64, whence: i32) -> i32 {
    let ret;

    match &mut h.handle {
        HioType::Memory(m) => {
            ret = m.mseek(offset, whence);
            if ret < 0 {
                h.error = EINVAL;
            } else if h.error == EOF {
                h.error = 0;
            }
        }
        HioType::CbFile(f) => {
            ret = f.cbseek(offset, whence);
            if ret < 0 {
                h.error = EINVAL;
            } else if h.error == EOF {
                h.error = 0;
            }
        }
    }

    ret
}

/// Translation of `hio_tell()`.
pub(crate) fn hio_tell(h: &mut HioHandle<'_>) -> i64 {
    let ret;

    match &mut h.handle {
        HioType::Memory(m) => {
            ret = m.mtell();
            if ret < 0 {
                /* should _not_ happen! */
                h.error = EINVAL;
            }
        }
        HioType::CbFile(f) => {
            ret = f.cbtell();
            if ret < 0 {
                h.error = EINVAL;
            }
        }
    }

    ret
}

/// Translation of `hio_eof()`.
pub(crate) fn hio_eof(h: &HioHandle<'_>) -> i32 {
    match &h.handle {
        HioType::Memory(m) => m.meof(),
        HioType::CbFile(f) => f.cbeof(),
    }
}

/// Translation of `hio_error()`.
pub(crate) fn hio_error(h: &mut HioHandle<'_>) -> i32 {
    let error = h.error;
    h.error = 0;
    error
}

/// Translation of `hio_open_const_mem()` (and `hio_open_mem()`, for data
/// the handle owns).
pub(crate) fn hio_open_const_mem<'a>(ptr: Cow<'a, [u8]>) -> Option<HioHandle<'a>> {
    let size = ptr.len() as i64;
    if size <= 0 {
        return None;
    }

    Some(HioHandle {
        handle: HioType::Memory(MFile {
            start: ptr,
            pos: 0,
            size: size as isize,
        }),
        size,
        error: 0,
    })
}

/// Translation of `hio_open_callbacks()`.
pub(crate) fn hio_open_callbacks<'a>(priv_: &'a mut dyn XmpCallbacks) -> Option<HioHandle<'a>> {
    let mut f = CbFile { priv_, eof: 0 };

    let size = f.cbfilelength();
    if size < 0 {
        return None;
    }
    Some(HioHandle {
        handle: HioType::CbFile(f),
        size,
        error: 0,
    })
}

/// Translation of `hio_reopen_mem()`: `hio_close` + `hio_open_mem`,
/// reusing the same `HIO_HANDLE`.
pub(crate) fn hio_reopen_mem(ptr: Vec<u8>, h: &mut HioHandle<'_>) -> i32 {
    let size = ptr.len() as i64;
    if size <= 0 {
        return -1;
    }

    h.handle = HioType::Memory(MFile {
        start: Cow::Owned(ptr),
        pos: 0,
        size: size as isize,
    });
    h.size = size;
    0
}

/// Translation of `hio_size()`.
pub(crate) fn hio_size(h: &HioHandle<'_>) -> i64 {
    h.size
}

/// Translation of `hio_get_underlying_memory()`: the whole contents of
/// `h`, if they are in memory.
pub(crate) fn hio_get_underlying_memory<'b>(h: &'b HioHandle<'_>) -> Option<&'b [u8]> {
    match &h.handle {
        HioType::CbFile(_) => None,
        HioType::Memory(m) => Some(&m.start),
    }
}

/// Translation of `hio_open()` (for reading, `"rb"`): the file is read
/// into memory, which the loaders then read as they would the `FILE *`.
pub(crate) fn hio_open(path: &[u8]) -> Option<HioHandle<'static>> {
    let data = std::fs::read(super::filetype::os_path(path)).ok()?;
    let size = data.len() as i64;

    Some(HioHandle {
        handle: HioType::Memory(MFile {
            start: Cow::Owned(data),
            pos: 0,
            size: size as isize,
        }),
        size,
        error: 0,
    })
}
