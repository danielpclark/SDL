// Rust translation of src/utils/bit_writer_utils.c and bit_writer_utils.h
// from libwebp (https://chromium.googlesource.com/webm/libwebp, as
// SDL_image's external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Bit writing and boolean coder: the VP8 (lossy) boolean encoder and the
//! VP8L (lossless) bit writer.
//!
//! The buffers are `Vec`s whose length is the write position (`pos_` and
//! `cur_ - buf_` upstream); they grow as upstream's do, without its
//! capacity bookkeeping (`max_pos_`, `end_`), which never shows in the
//! output. The lossless writer is upstream's 64-bit variant (a 64-bit
//! accumulator flushed 32 bits at a time) on every target; the bytes it
//! writes are those of the 32-bit variant.

//------------------------------------------------------------------------------
// VP8BitWriter

/// Translation of `VP8BitWriter`.
#[derive(Clone, Default)]
pub(crate) struct VP8BitWriter {
    /// range-1
    range: i32,
    value: i32,
    /// number of outstanding bits
    run: i32,
    /// number of pending bits
    nb_bits: i32,
    /// internal buffer (its length is upstream's `pos_`).
    buf: Vec<u8>,
    /// true in case of error
    pub(crate) error: bool,
}

impl VP8BitWriter {
    /// Translation of `Flush()`.
    fn flush(&mut self) {
        let s = 8 + self.nb_bits;
        let bits = self.value >> s;
        debug_assert!(self.nb_bits >= 0);
        self.value -= bits << s;
        self.nb_bits -= 8;
        if (bits & 0xff) != 0xff {
            let pos = self.buf.len();
            if bits & 0x100 != 0 {
                // overflow -> propagate carry over pending 0xff's
                if pos > 0 {
                    self.buf[pos - 1] = self.buf[pos - 1].wrapping_add(1);
                }
            }
            if self.run > 0 {
                let value = if bits & 0x100 != 0 { 0x00 } else { 0xff };
                while self.run > 0 {
                    self.buf.push(value);
                    self.run -= 1;
                }
            }
            self.buf.push((bits & 0xff) as u8);
        } else {
            self.run += 1; // delay writing of bytes 0xff, pending eventual carry.
        }
    }

    /// Translation of `VP8PutBit()`.
    pub(crate) fn put_bit(&mut self, bit: bool, prob: i32) -> bool {
        let split = (self.range * prob) >> 8;
        if bit {
            self.value += split + 1;
            self.range -= split + 1;
        } else {
            self.range = split;
        }
        if self.range < 127 {
            // emit 'shift' bits out and renormalize
            let shift = K_NORM[self.range as usize] as i32;
            self.range = K_NEW_RANGE[self.range as usize] as i32;
            self.value <<= shift;
            self.nb_bits += shift;
            if self.nb_bits > 0 {
                self.flush();
            }
        }
        bit
    }

    /// Translation of `VP8PutBitUniform()`.
    pub(crate) fn put_bit_uniform(&mut self, bit: bool) -> bool {
        let split = self.range >> 1;
        if bit {
            self.value += split + 1;
            self.range -= split + 1;
        } else {
            self.range = split;
        }
        if self.range < 127 {
            self.range = K_NEW_RANGE[self.range as usize] as i32;
            self.value <<= 1;
            self.nb_bits += 1;
            if self.nb_bits > 0 {
                self.flush();
            }
        }
        bit
    }

    /// Translation of `VP8PutBits()`.
    pub(crate) fn put_bits(&mut self, value: u32, nb_bits: i32) {
        debug_assert!(nb_bits > 0 && nb_bits < 32);
        let mut mask = 1u32 << (nb_bits - 1);
        while mask != 0 {
            self.put_bit_uniform(value & mask != 0);
            mask >>= 1;
        }
    }

    /// Translation of `VP8PutSignedBits()`.
    pub(crate) fn put_signed_bits(&mut self, value: i32, nb_bits: i32) {
        if !self.put_bit_uniform(value != 0) {
            return;
        }
        if value < 0 {
            self.put_bits((((-value) << 1) | 1) as u32, nb_bits + 1);
        } else {
            self.put_bits((value << 1) as u32, nb_bits + 1);
        }
    }

    /// Translation of `VP8BitWriterInit()` (the expected size is a capacity
    /// hint).
    pub(crate) fn new(expected_size: usize) -> VP8BitWriter {
        VP8BitWriter {
            range: 255 - 1,
            value: 0,
            run: 0,
            nb_bits: -8,
            buf: Vec::with_capacity(expected_size),
            error: false,
        }
    }

    /// Finalize the bitstream coding. Returns the buffer. Translation of
    /// `VP8BitWriterFinish()`.
    pub(crate) fn finish(&mut self) -> &[u8] {
        self.put_bits(0, 9 - self.nb_bits);
        self.nb_bits = 0; // pad with zeroes
        self.flush();
        &self.buf
    }

    /// Appends some bytes to the internal buffer. Data is copied.
    /// Translation of `VP8BitWriterAppend()`.
    pub(crate) fn append(&mut self, data: &[u8]) -> bool {
        if self.nb_bits != -8 {
            return false; // Flush() must have been called
        }
        self.buf.extend_from_slice(data);
        true
    }

    /// Release any pending memory and zeroes the object. Translation of
    /// `VP8BitWriterWipeOut()`.
    pub(crate) fn wipe_out(&mut self) {
        *self = VP8BitWriter::default();
    }

    /// Return approximate write position (in bits). Translation of
    /// `VP8BitWriterPos()`.
    pub(crate) fn pos(&self) -> u64 {
        let nb_bits = (8 + self.nb_bits) as u64; // bw->nb_bits_ is <= 0, note
        (self.buf.len() as u64 + self.run as u64) * 8 + nb_bits
    }

    /// Returns a pointer to the internal buffer. Translation of
    /// `VP8BitWriterBuf()`.
    pub(crate) fn buf(&self) -> &[u8] {
        &self.buf
    }

    /// Returns the size of the internal buffer. Translation of
    /// `VP8BitWriterSize()`.
    pub(crate) fn size(&self) -> usize {
        self.buf.len()
    }
}

//------------------------------------------------------------------------------
// renormalization

/// renorm_sizes[i] = 8 - log2(i)
const K_NORM: [u8; 128] = [
    7, 6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, //
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, //
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, //
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, //
    0,
];

/// range = ((range + 1) << kVP8Log2Range[range]) - 1
const K_NEW_RANGE: [u8; 128] = [
    127, 127, 191, 127, 159, 191, 223, 127, 143, 159, 175, 191, 207, 223, 239, //
    127, 135, 143, 151, 159, 167, 175, 183, 191, 199, 207, 215, 223, 231, 239, //
    247, 127, 131, 135, 139, 143, 147, 151, 155, 159, 163, 167, 171, 175, 179, //
    183, 187, 191, 195, 199, 203, 207, 211, 215, 219, 223, 227, 231, 235, 239, //
    243, 247, 251, 127, 129, 131, 133, 135, 137, 139, 141, 143, 145, 147, 149, //
    151, 153, 155, 157, 159, 161, 163, 165, 167, 169, 171, 173, 175, 177, 179, //
    181, 183, 185, 187, 189, 191, 193, 195, 197, 199, 201, 203, 205, 207, 209, //
    211, 213, 215, 217, 219, 221, 223, 225, 227, 229, 231, 233, 235, 237, 239, //
    241, 243, 245, 247, 249, 251, 253, 127,
];

//------------------------------------------------------------------------------
// VP8LBitWriter

/// accumulator type. Translation of `vp8l_atype_t` (64-bit).
type Vp8lAtype = u64;
/// 8 * sizeof(vp8l_wtype_t). Translation of `VP8L_WRITER_BITS`.
const VP8L_WRITER_BITS: i32 = 32;

/// Translation of `VP8LBitWriter`.
#[derive(Clone, Default)]
pub(crate) struct VP8LBitWriter {
    /// bit accumulator
    bits: Vp8lAtype,
    /// number of bits used in accumulator
    used: i32,
    /// the bytes written (its length is upstream's `cur_ - buf_`)
    buf: Vec<u8>,
    /// After all bits are written (VP8LBitWriterFinish()), the caller must
    /// observe the state of error_. A value of 1 indicates that a memory
    /// allocation failure has happened during bit writing. A value of 0
    /// indicates successful writing of bits.
    pub(crate) error: bool,
}

/// The state `VP8LBitWriterReset()` rewinds to: upstream's struct copy of a
/// writer (`VP8LBitWriter bw_init = *bw`), of which it uses the
/// accumulator, the position and the error flag.
#[derive(Clone, Copy)]
pub(crate) struct VP8LBitWriterState {
    bits: Vp8lAtype,
    used: i32,
    cur: usize,
    error: bool,
}

impl VP8LBitWriter {
    /// Returns the number of bytes written. Translation of
    /// `VP8LBitWriterNumBytes()`.
    pub(crate) fn num_bytes(&self) -> usize {
        self.buf.len() + ((self.used + 7) >> 3) as usize
    }

    /// Translation of `VP8LBitWriterInit()` (the expected size is a capacity
    /// hint).
    pub(crate) fn new(expected_size: usize) -> VP8LBitWriter {
        VP8LBitWriter {
            buf: Vec::with_capacity(expected_size),
            ..VP8LBitWriter::default()
        }
    }

    /// Initialize 'dst' as a copy of 'src'. Translation of
    /// `VP8LBitWriterClone()`.
    pub(crate) fn clone_into(&self, dst: &mut VP8LBitWriter) -> bool {
        dst.buf.clear();
        dst.buf.extend_from_slice(&self.buf);
        dst.bits = self.bits;
        dst.used = self.used;
        dst.error = self.error;
        true
    }

    /// The writer's state, to rewind to with [`Self::reset`].
    pub(crate) fn state(&self) -> VP8LBitWriterState {
        VP8LBitWriterState {
            bits: self.bits,
            used: self.used,
            cur: self.buf.len(),
            error: self.error,
        }
    }

    /// Resets the cursor of the BitWriter bw to when it was like in
    /// bw_init. Translation of `VP8LBitWriterReset()`.
    pub(crate) fn reset(&mut self, bw_init: &VP8LBitWriterState) {
        self.bits = bw_init.bits;
        self.used = bw_init.used;
        debug_assert!(bw_init.cur <= self.buf.len());
        self.buf.truncate(bw_init.cur);
        self.error = bw_init.error;
    }

    /// Swaps the memory held by two BitWriters. Translation of
    /// `VP8LBitWriterSwap()`.
    pub(crate) fn swap(src: &mut VP8LBitWriter, dst: &mut VP8LBitWriter) {
        std::mem::swap(src, dst);
    }

    /// Internal function for VP8LPutBits flushing 32 bits from the written
    /// state. Translation of `VP8LPutBitsFlushBits()`.
    fn put_bits_flush_bits(&mut self) {
        self.buf
            .extend_from_slice(&(self.bits as u32).to_le_bytes());
        self.bits >>= VP8L_WRITER_BITS;
        self.used -= VP8L_WRITER_BITS;
    }

    /// This function writes bits into bytes in increasing addresses (little
    /// endian), and within a byte least-significant-bit first. This
    /// function can write up to 32 bits in one go, but VP8LBitReader can
    /// only read 24 bits max (VP8L_MAX_NUM_BIT_READ). Translation of
    /// `VP8LPutBits()`.
    pub(crate) fn put_bits(&mut self, bits: u32, n_bits: i32) {
        if n_bits > 0 {
            if self.used >= 32 {
                self.put_bits_flush_bits();
            }
            self.bits |= (bits as Vp8lAtype) << self.used;
            self.used += n_bits;
        }
    }

    /// Finalize the bitstream coding. Returns the buffer. Translation of
    /// `VP8LBitWriterFinish()`.
    pub(crate) fn finish(&mut self) -> &[u8] {
        // flush leftover bits
        while self.used > 0 {
            self.buf.push(self.bits as u8);
            self.bits >>= 8;
            self.used -= 8;
        }
        self.used = 0;
        &self.buf
    }

    /// Release any pending memory and zeroes the object. Translation of
    /// `VP8LBitWriterWipeOut()`.
    pub(crate) fn wipe_out(&mut self) {
        *self = VP8LBitWriter::default();
    }
}
