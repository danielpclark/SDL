// Rust translation of src/bitwise.c (the reading half of its LSb-first
// packer, `oggpack_*`) from libogg 1.3.5 (https://xiph.org/ogg/, at the
// revision SDL_mixer's external/ogg pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2014
// by the Xiph.Org Foundation http://www.xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: packing variable sized words into an octet stream
//!
//! Only the reading half of the LSb-first (`oggpack_*`) packer that
//! libvorbis decodes packets with is translated (`oggpack_readinit()`,
//! `oggpack_look()`, `oggpack_adv()`, `oggpack_read()`,
//! `oggpack_bytes()`): not the writer, the one-bit and bit-count variants
//! libvorbis doesn't call, nor the MSb-first (`oggpackB_*`) ones. The buffer borrows the packet's data
//! (C points at it); C's `ptr` is always `buffer + endbyte` until an
//! overflow sets it to NULL, after which every read fails before looking
//! at it, so it isn't kept.

/* We're 'LSb' endian; if we write a word but read individual bits,
then we'll read the lsb first */

const MASK: [u64; 33] = [
    0x00000000, 0x00000001, 0x00000003, 0x00000007, 0x0000000f, 0x0000001f, 0x0000003f, 0x0000007f,
    0x000000ff, 0x000001ff, 0x000003ff, 0x000007ff, 0x00000fff, 0x00001fff, 0x00003fff, 0x00007fff,
    0x0000ffff, 0x0001ffff, 0x0003ffff, 0x0007ffff, 0x000fffff, 0x001fffff, 0x003fffff, 0x007fffff,
    0x00ffffff, 0x01ffffff, 0x03ffffff, 0x07ffffff, 0x0fffffff, 0x1fffffff, 0x3fffffff, 0x7fffffff,
    0xffffffff,
];

/// Translation of `oggpack_buffer` (reading). C's `long` fields are `i64`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OggpackBuffer<'a> {
    pub endbyte: i64,
    pub endbit: i32,

    pub buffer: &'a [u8],
    pub storage: i64,
}

/// Translation of `oggpack_readinit()`.
pub(crate) fn oggpack_readinit<'a>(b: &mut OggpackBuffer<'a>, buf: &'a [u8], bytes: i32) {
    *b = OggpackBuffer::default();
    b.buffer = buf;
    b.storage = bytes as i64;
}

impl OggpackBuffer<'_> {
    /// C's `b->ptr[i]`.
    fn ptr(&self, i: usize) -> u64 {
        self.buffer[self.endbyte as usize + i] as u64
    }

    /// The `overflow:` exit of the readers.
    fn overflow(&mut self) {
        /* (b->ptr=NULL;) */
        self.endbyte = self.storage;
        self.endbit = 1;
    }
}

/* Read in bits without advancing the bitptr; bits <= 32 */
/// Translation of `oggpack_look()`.
pub(crate) fn oggpack_look(b: &OggpackBuffer<'_>, mut bits: i32) -> i64 {
    let mut ret: u64;
    let m: u64;

    if !(0..=32).contains(&bits) {
        return -1;
    }
    m = MASK[bits as usize];
    bits += b.endbit;

    if b.endbyte >= b.storage - 4 {
        /* not the main path */
        if b.endbyte > b.storage - ((bits + 7) >> 3) as i64 {
            return -1;
        }
        /* special case to avoid reading b->ptr[0], which might be past the end of
        the buffer; also skips some useless accounting */
        else if bits == 0 {
            return 0;
        }
    }

    ret = b.ptr(0) >> b.endbit;
    if bits > 8 {
        ret |= b.ptr(1) << (8 - b.endbit);
        if bits > 16 {
            ret |= b.ptr(2) << (16 - b.endbit);
            if bits > 24 {
                ret |= b.ptr(3) << (24 - b.endbit);
                if bits > 32 && b.endbit != 0 {
                    ret |= b.ptr(4) << (32 - b.endbit);
                }
            }
        }
    }
    (m & ret) as i64
}

/// Translation of `oggpack_adv()`.
pub(crate) fn oggpack_adv(b: &mut OggpackBuffer<'_>, mut bits: i32) {
    bits += b.endbit;

    if b.endbyte > b.storage - ((bits + 7) >> 3) as i64 {
        b.overflow();
        return;
    }

    b.endbyte += (bits / 8) as i64;
    b.endbit = bits & 7;
}

/* bits <= 32 */
/// Translation of `oggpack_read()`.
pub(crate) fn oggpack_read(b: &mut OggpackBuffer<'_>, mut bits: i32) -> i64 {
    let mut ret: u64;
    let m: u64;

    if !(0..=32).contains(&bits) {
        b.overflow();
        return -1;
    }
    m = MASK[bits as usize];
    bits += b.endbit;

    if b.endbyte >= b.storage - 4 {
        /* not the main path */
        if b.endbyte > b.storage - ((bits + 7) >> 3) as i64 {
            b.overflow();
            return -1;
        }
        /* special case to avoid reading b->ptr[0], which might be past the end of
        the buffer; also skips some useless accounting */
        else if bits == 0 {
            return 0;
        }
    }

    ret = b.ptr(0) >> b.endbit;
    if bits > 8 {
        ret |= b.ptr(1) << (8 - b.endbit);
        if bits > 16 {
            ret |= b.ptr(2) << (16 - b.endbit);
            if bits > 24 {
                ret |= b.ptr(3) << (24 - b.endbit);
                if bits > 32 && b.endbit != 0 {
                    ret |= b.ptr(4) << (32 - b.endbit);
                }
            }
        }
    }
    ret &= m;
    b.endbyte += (bits / 8) as i64;
    b.endbit = bits & 7;
    ret as i64
}

/// Translation of `oggpack_bytes()`.
pub(crate) fn oggpack_bytes(b: &OggpackBuffer<'_>) -> i64 {
    b.endbyte + ((b.endbit + 7) / 8) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads of known bytes, across byte boundaries and past the end.
    #[test]
    fn reads_lsb_first() {
        let data = [0b1010_1100u8, 0xff, 0x01, 0x80, 0x55];
        let mut b = OggpackBuffer::default();
        oggpack_readinit(&mut b, &data, data.len() as i32);
        assert_eq!(oggpack_read(&mut b, 2), 0b00);
        assert_eq!(oggpack_read(&mut b, 3), 0b011);
        assert_eq!(oggpack_look(&b, 3), 0b101);
        assert_eq!(oggpack_read(&mut b, 1), 1);
        assert_eq!(oggpack_read(&mut b, 12), 0x7fe);
        assert_eq!(oggpack_bytes(&b), 3);
        assert_eq!(oggpack_read(&mut b, 22), 0x15_6000);
        assert_eq!(oggpack_read(&mut b, 1), -1);
        assert_eq!(oggpack_read(&mut b, 0), -1);
        assert_eq!(oggpack_look(&b, 0), -1);
    }
}
