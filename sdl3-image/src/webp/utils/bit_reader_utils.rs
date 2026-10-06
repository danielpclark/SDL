// Rust translation of src/utils/bit_reader_utils.c, bit_reader_utils.h and
// bit_reader_inl_utils.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2010 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Boolean decoder (the VP8 lossy bit reader) and the lossless bit reader.
//!
//! The Boolean decoder needs to maintain infinite precision on the value_
//! field. However, since range_ is only 8bit, we only need an active window
//! of 8 bits for value_. Left bits (MSB) gets zeroed and shifted away when
//! value_ falls below 128, range_ is updated, and fresh bits read from the
//! bitstream are brought in as LSB. To avoid reading the fresh bits one by
//! one (slow), we cache BITS of them ahead. The total of (BITS + 8) bits
//! must fit into a natural register (with type bit_t). To fetch BITS bits
//! from bitstream we use a type lbit_t.
//!
//! BITS can be any multiple of 8 from 8 to 56 (inclusive). Here it is 56,
//! upstream's choice on 64-bit x86 and ARM (the decoded bits are the same
//! for every choice). The bit tracing tool (`BITTRACE`) is not translated.

use crate::webp::utils::bits_log2_floor;

/// Number of bits cached ahead. Translation of `BITS` (64-bit x86).
const BITS: i32 = 56;

/// The bytes read at a time: `sizeof(lbit_t)`.
const LBIT_SIZE: usize = 8;

/// Bitreader. Translation of `VP8BitReader` (the buffer pointers are
/// positions in `buf`).
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8BitReader<'a> {
    // boolean decoder  (keep the field ordering as is!)
    /// current value
    value: u64,
    /// current range minus 1. In [127, 254] interval.
    range: u32,
    /// number of valid bits left
    bits: i32,
    // read buffer
    buf: &'a [u8],
    /// next byte to be read
    pos: usize,
    /// end of read buffer
    end: usize,
    /// max packed-read position on buffer
    max: usize,
    /// true if input is exhausted
    pub(crate) eof: bool,
}

//------------------------------------------------------------------------------
// VP8BitReader

impl<'a> VP8BitReader<'a> {
    /// Sets the working read buffer. Translation of `VP8BitReaderSetBuffer()`.
    fn set_buffer(&mut self, start: &'a [u8]) {
        let size = start.len();
        self.buf = start;
        self.pos = 0;
        self.end = size;
        self.max = if size >= LBIT_SIZE {
            size - LBIT_SIZE + 1
        } else {
            0
        };
    }

    /// Initialize the bit reader and the boolean decoder.
    /// Translation of `VP8InitBitReader()`.
    pub(crate) fn new(start: &'a [u8]) -> VP8BitReader<'a> {
        debug_assert!(start.len() < (1usize << 31)); // limit ensured by format and upstream checks
        let mut br = VP8BitReader {
            range: 255 - 1,
            value: 0,
            bits: -8, // to load the very first 8bits
            eof: false,
            ..VP8BitReader::default()
        };
        br.set_buffer(start);
        br.load_new_bytes();
        br
    }

    /// special case for the tail byte-reading. Translation of
    /// `VP8LoadFinalBytes()`.
    fn load_final_bytes(&mut self) {
        // Only read 8bits at a time
        if self.pos < self.end {
            self.bits += 8;
            self.value = self.buf[self.pos] as u64 | (self.value << 8);
            self.pos += 1;
        } else if !self.eof {
            self.value <<= 8;
            self.bits += 8;
            self.eof = true;
        } else {
            self.bits = 0; // This is to avoid undefined behaviour with shifts.
        }
    }

    //--------------------------------------------------------------------------
    // Inlined critical functions

    /// makes sure br->value_ has at least BITS bits worth of data.
    /// Translation of `VP8LoadNewBytes()`.
    fn load_new_bytes(&mut self) {
        // Read 'BITS' bits at a time if possible.
        if self.pos < self.max {
            // convert memory type to register type (with some zero'ing!)
            let mut in_bits = [0u8; LBIT_SIZE];
            in_bits.copy_from_slice(&self.buf[self.pos..self.pos + LBIT_SIZE]);
            self.pos += (BITS >> 3) as usize;
            let mut bits = u64::from_be_bytes(in_bits);
            bits >>= 64 - BITS;
            self.value = bits | (self.value << BITS);
            self.bits += BITS;
        } else {
            self.load_final_bytes(); // no need to be inlined
        }
    }

    /// Read a bit with proba 'prob'. Speed-critical function!
    /// Translation of `VP8GetBit()`.
    pub(crate) fn get_bit(&mut self, prob: i32) -> i32 {
        // Don't move this declaration! It makes a big speed difference to store
        // 'range' *before* calling VP8LoadNewBytes(), even if this function doesn't
        // alter br->range_ value.
        let mut range = self.range;
        if self.bits < 0 {
            self.load_new_bytes();
        }
        let pos = self.bits;
        let split = (range * prob as u32) >> 8;
        let value = (self.value >> pos) as u32;
        let bit = value > split;
        if bit {
            range -= split;
            self.value = self.value.wrapping_sub(((split + 1) as u64) << pos);
        } else {
            range = split + 1;
        }
        let shift = 7 ^ bits_log2_floor(range);
        range <<= shift;
        self.bits -= shift;
        self.range = range - 1;
        bit as i32
    }

    /// simplified version of VP8GetBit() for prob=0x80 (note shift is always
    /// 1 here). Translation of `VP8GetSigned()`.
    pub(crate) fn get_signed(&mut self, v: i32) -> i32 {
        if self.bits < 0 {
            self.load_new_bytes();
        }
        let pos = self.bits;
        let split = self.range >> 1;
        let value = (self.value >> pos) as u32;
        let mask = (split.wrapping_sub(value) as i32) >> 31; // -1 or 0
        self.bits -= 1;
        self.range = self.range.wrapping_add(mask as u32);
        self.range |= 1;
        self.value = self
            .value
            .wrapping_sub((((split + 1) & mask as u32) as u64) << pos);
        (v ^ mask) - mask
    }

    //--------------------------------------------------------------------------
    // Higher-level calls

    /// return the next value made of 'num_bits' bits.
    /// Translation of `VP8GetValue()`.
    pub(crate) fn get_value(&mut self, mut bits: i32) -> u32 {
        let mut v: u32 = 0;
        while bits > 0 {
            bits -= 1;
            v |= (self.get_bit(0x80) as u32) << bits;
        }
        v
    }

    /// Translation of `VP8Get()`: one bit at probability 1/2.
    pub(crate) fn get(&mut self) -> i32 {
        self.get_value(1) as i32
    }

    /// return the next value with sign-extension.
    /// Translation of `VP8GetSignedValue()`.
    pub(crate) fn get_signed_value(&mut self, bits: i32) -> i32 {
        let value = self.get_value(bits) as i32;
        if self.get() != 0 {
            -value
        } else {
            value
        }
    }
}

// -----------------------------------------------------------------------------
// Bitreader for lossless format

/// maximum number of bits (inclusive) the bit-reader can handle.
/// Translation of `VP8L_MAX_NUM_BIT_READ`.
pub(crate) const VP8L_MAX_NUM_BIT_READ: i32 = 24;

/// Number of bits prefetched (= bit-size of vp8l_val_t).
const VP8L_LBITS: i32 = 64;
/// Minimum number of bytes ready after VP8LFillBitWindow.
const VP8L_WBITS: i32 = 32;
/// Number of bytes needed to store VP8L_WBITS bits.
const VP8L_LOG8_WBITS: usize = 4;

static K_BIT_MASK: [u32; VP8L_MAX_NUM_BIT_READ as usize + 1] = [
    0, 0x000001, 0x000003, 0x000007, 0x00000f, 0x00001f, 0x00003f, 0x00007f, 0x0000ff, 0x0001ff,
    0x0003ff, 0x0007ff, 0x000fff, 0x001fff, 0x003fff, 0x007fff, 0x00ffff, 0x01ffff, 0x03ffff,
    0x07ffff, 0x0fffff, 0x1fffff, 0x3fffff, 0x7fffff, 0xffffff,
];

/// The lossless bit reader. Translation of `VP8LBitReader`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8LBitReader<'a> {
    /// pre-fetched bits
    val: u64,
    /// input byte buffer (its length is `len_`)
    buf: &'a [u8],
    /// byte position in buf_
    pos: usize,
    /// current bit-reading position in val_
    pub(crate) bit_pos: i32,
    /// true if a bit was read past the end of buffer
    pub(crate) eos: bool,
}

impl<'a> VP8LBitReader<'a> {
    /// Translation of `VP8LInitBitReader()`.
    pub(crate) fn new(start: &'a [u8]) -> VP8LBitReader<'a> {
        debug_assert!((start.len() as u64) < 0xfffffff8); // can't happen with a RIFF chunk.
        let length = start.len().min(8);
        let mut value: u64 = 0;
        for (i, &b) in start[..length].iter().enumerate() {
            value |= (b as u64) << (8 * i);
        }
        VP8LBitReader {
            val: value,
            buf: start,
            pos: length,
            bit_pos: 0,
            eos: false,
        }
    }

    /// Return the prefetched bits, so they can be looked up.
    /// Translation of `VP8LPrefetchBits()`.
    pub(crate) fn prefetch_bits(&self) -> u32 {
        (self.val >> (self.bit_pos & (VP8L_LBITS - 1))) as u32
    }

    /// Returns true if there was an attempt at reading bit past the end of
    /// the buffer. Doesn't set br->eos_ flag. Translation of
    /// `VP8LIsEndOfStream()`.
    pub(crate) fn is_end_of_stream(&self) -> bool {
        debug_assert!(self.pos <= self.buf.len());
        self.eos || ((self.pos == self.buf.len()) && (self.bit_pos > VP8L_LBITS))
    }

    /// For jumping over a number of bits in the bit stream when accessed
    /// with VP8LPrefetchBits and VP8LFillBitWindow. This function does *not*
    /// set br->eos_, since it's speed-critical. Use with extreme care!
    /// Translation of `VP8LSetBitPos()`.
    pub(crate) fn set_bit_pos(&mut self, val: i32) {
        self.bit_pos = val;
    }

    /// Translation of `VP8LSetEndOfStream()`.
    fn set_end_of_stream(&mut self) {
        self.eos = true;
        self.bit_pos = 0; // To avoid undefined behaviour with shifts.
    }

    /// If not at EOS, reload up to VP8L_LBITS byte-by-byte.
    /// Translation of `ShiftBytes()`.
    fn shift_bytes(&mut self) {
        while self.bit_pos >= 8 && self.pos < self.buf.len() {
            self.val >>= 8;
            self.val |= (self.buf[self.pos] as u64) << (VP8L_LBITS - 8);
            self.pos += 1;
            self.bit_pos -= 8;
        }
        if self.is_end_of_stream() {
            self.set_end_of_stream();
        }
    }

    /// Translation of `VP8LDoFillBitWindow()` (with upstream's fast load,
    /// as on x86 and ARM).
    fn do_fill_bit_window(&mut self) {
        debug_assert!(self.bit_pos >= VP8L_WBITS);
        if self.pos + 8 < self.buf.len() {
            self.val >>= VP8L_WBITS;
            self.bit_pos -= VP8L_WBITS;
            let mut b = [0u8; 4];
            b.copy_from_slice(&self.buf[self.pos..self.pos + 4]);
            self.val |= (u32::from_le_bytes(b) as u64) << (VP8L_LBITS - VP8L_WBITS);
            self.pos += VP8L_LOG8_WBITS;
            return;
        }
        self.shift_bytes(); // Slow path.
    }

    /// Advances the read buffer by 4 bytes to make room for reading next 32
    /// bits. Translation of `VP8LFillBitWindow()`.
    pub(crate) fn fill_bit_window(&mut self) {
        if self.bit_pos >= VP8L_WBITS {
            self.do_fill_bit_window();
        }
    }

    /// Reads the specified number of bits from read buffer. Flags an error
    /// in case end_of_stream or n_bits is more than the allowed limit of
    /// VP8L_MAX_NUM_BIT_READ (inclusive). Flags eos_ if this read attempt is
    /// going to cross the read buffer. Translation of `VP8LReadBits()`.
    pub(crate) fn read_bits(&mut self, n_bits: i32) -> u32 {
        debug_assert!(n_bits >= 0);
        // Flag an error if end_of_stream or n_bits is more than allowed limit.
        if !self.eos && n_bits <= VP8L_MAX_NUM_BIT_READ {
            let val = self.prefetch_bits() & K_BIT_MASK[n_bits as usize];
            let new_bits = self.bit_pos + n_bits;
            self.bit_pos = new_bits;
            self.shift_bytes();
            val
        } else {
            self.set_end_of_stream();
            0
        }
    }
}
