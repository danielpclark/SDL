// File_Extractor 0.4.0. http://www.slack.net/~ant/

/* Copyright (C) 2005-2006 Shay Green. This module is free software; you
can redistribute it and/or modify it under the terms of the GNU Lesser
General Public License as published by the Free Software Foundation; either
version 2.1 of the License, or (at your option) any later version. This
module is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
details. You should have received a copy of the GNU Lesser General Public
License along with this module; if not, write to the Free Software Foundation,
Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA */
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! Data reader interface for uniform access (`Data_Reader.h`,
//! `Data_Reader.cpp`): the memory, subset and remaining readers the
//! emulators read files through. SDL_mixer builds libgme without zlib
//! (`GME_ZLIB` off), so `Mem_File_Reader` doesn't inflate gzipped data;
//! the file readers (`Std_File_Reader`) and `Callback_Reader`, which
//! SDL_mixer doesn't reach, are left out.

use crate::blargg_common::BlarggErr;

/// returned by read() when request goes beyond end
pub const EOF_ERROR: &str = "Unexpected end of file";

const CORRUPT_FILE: &str = "Corrupt file";

/// Supports reading and finding out how many bytes are remaining
pub trait DataReader {
    /// Read at most count bytes and return number actually read, or <= 0 if error
    fn read_avail(&mut self, p: &mut [u8]) -> i64;

    /// Read exactly count bytes and return error if they couldn't be read
    fn read(&mut self, p: &mut [u8]) -> BlarggErr {
        data_reader_read(self, p)
    }

    /// Number of bytes remaining until end of file
    fn remain(&self) -> i64;

    /// Read and discard count bytes
    fn skip(&mut self, count: i64) -> BlarggErr {
        data_reader_skip(self, count)
    }
}

/// `Data_Reader::read()`
pub fn data_reader_read<R: DataReader + ?Sized>(r: &mut R, p: &mut [u8]) -> BlarggErr {
    let s = p.len() as i64;
    if s <= 0 {
        return Err(CORRUPT_FILE);
    }

    let result = r.read_avail(p);
    if result != s {
        if result >= 0 && result < s {
            return Err(EOF_ERROR);
        }

        return Err("Read error");
    }

    Ok(())
}

/// `Data_Reader::skip()`
pub fn data_reader_skip<R: DataReader + ?Sized>(r: &mut R, mut count: i64) -> BlarggErr {
    if count < 0 {
        return Err(CORRUPT_FILE);
    }

    let mut buf = [0u8; 512];
    while count != 0 {
        let mut n = buf.len() as i64;
        if n > count {
            n = count;
        }
        count -= n;
        r.read(&mut buf[..n as usize])?;
    }
    Ok(())
}

/// Treats range of memory as a file (`Mem_File_Reader`, a `File_Reader`)
#[derive(Debug)]
pub struct MemFileReader<'a> {
    m_begin: &'a [u8],
    m_pos: i64,
}

impl<'a> MemFileReader<'a> {
    pub fn new(p: &'a [u8]) -> Self {
        MemFileReader {
            m_begin: p,
            m_pos: 0,
        }
    }

    /// Size of file
    pub fn size(&self) -> i64 {
        self.m_begin.len() as i64
    }

    /// Current position in file
    pub fn tell(&self) -> i64 {
        self.m_pos
    }

    /// Go to new position
    pub fn seek(&mut self, n: i64) -> BlarggErr {
        if n < 0 {
            return Err(CORRUPT_FILE);
        }
        if n > self.size() {
            return Err(EOF_ERROR);
        }
        self.m_pos = n;
        Ok(())
    }
}

impl DataReader for MemFileReader<'_> {
    fn read_avail(&mut self, p: &mut [u8]) -> i64 {
        let r = self.remain();
        let mut s = p.len() as i64;
        if s > r {
            s = r;
        }
        let pos = self.m_pos as usize;
        p[..s as usize].copy_from_slice(&self.m_begin[pos..pos + s as usize]);
        self.m_pos += s;
        s
    }

    /// `File_Reader::remain()`
    fn remain(&self) -> i64 {
        self.size() - self.tell()
    }

    /// `File_Reader::skip()`
    fn skip(&mut self, n: i64) -> BlarggErr {
        if n < 0 {
            return Err(CORRUPT_FILE);
        }

        if n == 0 {
            return Ok(());
        }
        let t = self.tell();
        self.seek(t + n)
    }
}

/// Makes it look like there are only count bytes remaining
#[allow(missing_debug_implementations)]
pub struct SubsetReader<'r> {
    input: &'r mut dyn DataReader,
    remain_: i64,
}

impl<'r> SubsetReader<'r> {
    pub fn new(dr: &'r mut dyn DataReader, size: i64) -> Self {
        let mut remain_ = dr.remain();
        if remain_ > size {
            remain_ = size.max(0);
        }
        SubsetReader { input: dr, remain_ }
    }
}

impl DataReader for SubsetReader<'_> {
    fn remain(&self) -> i64 {
        self.remain_
    }

    fn read_avail(&mut self, p: &mut [u8]) -> i64 {
        let mut s = (p.len() as i64).max(0);
        if s > self.remain_ {
            s = self.remain_;
        }
        self.remain_ -= s;
        self.input.read_avail(&mut p[..s as usize])
    }
}

/// Joins already-read header and remaining data into original file (to avoid
/// having to seek back to beginning of file)
#[allow(missing_debug_implementations)]
pub struct RemainingReader<'r> {
    header: &'r [u8],
    input: &'r mut dyn DataReader,
}

impl<'r> RemainingReader<'r> {
    pub fn new(header: &'r [u8], input: &'r mut dyn DataReader) -> Self {
        RemainingReader { header, input }
    }

    fn read_first(&mut self, out: &mut [u8]) -> i64 {
        let count = out.len() as i64;
        let mut first = self.header.len() as i64;
        if first != 0 {
            if first > count || first < 0 {
                first = count;
            }
            let (old, rest) = self.header.split_at(first as usize);
            self.header = rest;
            out[..first as usize].copy_from_slice(old);
        }
        first
    }
}

impl DataReader for RemainingReader<'_> {
    fn remain(&self) -> i64 {
        self.header.len() as i64 + self.input.remain()
    }

    fn read_avail(&mut self, out: &mut [u8]) -> i64 {
        let count = out.len() as i64;
        let first = self.read_first(out);
        let mut second = (count - first).max(0);
        if second != 0 {
            second = self.input.read_avail(&mut out[first as usize..]);
            if second <= 0 {
                return second;
            }
        }
        first + second
    }

    fn read(&mut self, out: &mut [u8]) -> BlarggErr {
        let count = out.len() as i64;
        let first = self.read_first(out);
        let second = (count - first).max(0);
        if second == 0 {
            return Ok(());
        }
        self.input.read(&mut out[first as usize..])
    }
}
