// Rust translation of src/getbits.c and src/getbits.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The bit reader of the OBU and header parsers. The pointers of
//! `GetBits` are offsets into the borrowed buffer (`ptr_start` is 0).

use super::intops::{inv_recenter, ulog2};

/// Translation of `GetBits`.
pub(crate) struct GetBits<'a> {
    pub(crate) state: u64,
    pub(crate) bits_left: i32,
    pub(crate) error: i32,
    pub(crate) buf: &'a [u8],
    pub(crate) ptr: usize,
    pub(crate) ptr_end: usize,
}

impl<'a> GetBits<'a> {
    /// Translation of `dav1d_init_get_bits()`.
    pub(crate) fn new(data: &'a [u8]) -> GetBits<'a> {
        debug_assert!(!data.is_empty());
        GetBits {
            state: 0,
            bits_left: 0,
            error: 0,
            buf: data,
            ptr: 0,
            ptr_end: data.len(),
        }
    }

    /// Translation of `dav1d_get_bit()`.
    pub(crate) fn get_bit(&mut self) -> u32 {
        if self.bits_left == 0 {
            if self.ptr >= self.ptr_end {
                self.error = 1;
            } else {
                let state = self.buf[self.ptr] as u32;
                self.ptr += 1;
                self.bits_left = 7;
                self.state = (state as u64) << 57;
                return state >> 7;
            }
        }

        let state = self.state;
        self.bits_left -= 1;
        self.state = state << 1;
        (state >> 63) as u32
    }

    #[inline]
    fn refill(&mut self, n: i32) {
        debug_assert!(self.bits_left >= 0 && self.bits_left < 32);
        let mut state: u32 = 0;
        loop {
            if self.ptr >= self.ptr_end {
                self.error = 1;
                if state != 0 {
                    break;
                }
                return;
            }
            state = (state << 8) | self.buf[self.ptr] as u32;
            self.ptr += 1;
            self.bits_left += 8;
            if n <= self.bits_left {
                break;
            }
        }
        self.state |= (state as u64) << (64 - self.bits_left);
    }

    /// Translation of `dav1d_get_bits()`.
    pub(crate) fn get_bits(&mut self, n: i32) -> u32 {
        debug_assert!(n > 0 && n <= 32);
        /* Unsigned cast avoids refill after eob */
        if n as u32 > self.bits_left as u32 {
            self.refill(n);
        }
        let state = self.state;
        self.bits_left -= n;
        self.state = state << n;
        (state >> (64 - n)) as u32
    }

    /// Translation of `dav1d_get_sbits()`.
    pub(crate) fn get_sbits(&mut self, n: i32) -> i32 {
        debug_assert!(n > 0 && n <= 32);
        /* Unsigned cast avoids refill after eob */
        if n as u32 > self.bits_left as u32 {
            self.refill(n);
        }
        let state = self.state;
        self.bits_left -= n;
        self.state = state << n;
        ((state as i64) >> (64 - n)) as i32
    }

    /// Translation of `dav1d_get_uleb128()`.
    pub(crate) fn get_uleb128(&mut self) -> u32 {
        let mut val: u64 = 0;
        let mut i: u32 = 0;
        let mut more;

        loop {
            let v = self.get_bits(8);
            more = v & 0x80;
            val |= ((v & 0x7F) as u64) << i;
            i += 7;
            if !(more != 0 && i < 56) {
                break;
            }
        }

        if val > u32::MAX as u64 || more != 0 {
            self.error = 1;
            return 0;
        }

        val as u32
    }

    /// Translation of `dav1d_get_uniform()`: output in range 0..max-1.
    pub(crate) fn get_uniform(&mut self, max: u32) -> u32 {
        // Output in range [0..max-1]
        // max must be > 1, or else nothing is read from the bitstream
        debug_assert!(max > 1);
        let l = ulog2(max) + 1;
        debug_assert!(l > 1);
        let m = (1u32 << l).wrapping_sub(max);
        let v = self.get_bits(l - 1);
        if v < m {
            v
        } else {
            (v << 1).wrapping_sub(m).wrapping_add(self.get_bit())
        }
    }

    /// Translation of `dav1d_get_vlc()`.
    pub(crate) fn get_vlc(&mut self) -> u32 {
        if self.get_bit() != 0 {
            return 0;
        }

        let mut n_bits = 0;
        loop {
            n_bits += 1;
            if n_bits == 32 {
                return 0xFFFFFFFF;
            }
            if self.get_bit() != 0 {
                break;
            }
        }

        ((1u32 << n_bits) - 1).wrapping_add(self.get_bits(n_bits))
    }

    fn get_bits_subexp_u(&mut self, r: u32, n: u32) -> u32 {
        let mut v: u32 = 0;

        let mut i = 0;
        loop {
            let b = if i != 0 { 3 + i - 1 } else { 3 };

            if n < v + 3 * (1 << b) {
                v += self.get_uniform(n - v + 1);
                break;
            }

            if self.get_bit() == 0 {
                v += self.get_bits(b);
                break;
            }

            v += 1 << b;
            i += 1;
        }

        if r.wrapping_mul(2) <= n {
            inv_recenter(r, v)
        } else {
            n.wrapping_sub(inv_recenter(n.wrapping_sub(r), v))
        }
    }

    /// Translation of `dav1d_get_bits_subexp()`.
    pub(crate) fn get_bits_subexp(&mut self, r: i32, n: u32) -> i32 {
        (self.get_bits_subexp_u(r.wrapping_add(1 << n) as u32, 2 << n) as i32).wrapping_sub(1 << n)
    }

    /// Translation of `dav1d_bytealign_get_bits()`: discard bits from the
    /// buffer until we're next byte-aligned.
    pub(crate) fn bytealign(&mut self) {
        // bits_left is never more than 7, because it is only incremented
        // by refill(), called by dav1d_get_bits and that never reads more
        // than 7 bits more than it needs.
        //
        // If this wasn't true, we would need to work out how many bits to
        // discard (bits_left % 8), subtract that from bits_left and then
        // shift state right by that amount.
        debug_assert!(self.bits_left <= 7);

        self.bits_left = 0;
        self.state = 0;
    }

    /// Translation of `dav1d_get_bits_pos()`: return the current bit
    /// position relative to the start of the buffer.
    #[allow(dead_code)]
    pub(crate) fn pos(&self) -> u32 {
        (self.ptr as u32) * 8 - self.bits_left as u32
    }
}
