// Rust translation of lib/jxl/dec_bit_reader.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Bounds-checked bit reader; 64-bit buffer with support for deferred refills
//! and switching to reading byte-aligned words.

use super::base::{jxl_failure, load_le64, Status, K_BITS_PER_BYTE};

/// Reads bits previously written to memory by BitWriter. Uses unaligned
/// 8-byte little-endian loads. Translation of `BitReader` (the pointers are
/// indices into the slice; `Close()` is not required before dropping).
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    buf: u64,
    bits_in_buf: usize, // [0, 64)
    next_byte: usize,

    // Number of bytes past the end that were loaded into the buf_. These bytes
    // are not read from memory, but instead assumed 0. It is an error (likely due
    // to an invalid stream) to Consume() more bits than specified in the range
    // passed to the constructor.
    overread_bytes: u64,
    close_called: bool,

    checked_out_of_bounds_bits: u64,
}

impl<'a> BitReader<'a> {
    pub(crate) const K_MAX_BITS_PER_CALL: usize = 56;

    // bytes need not be aligned nor padded!
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        let mut br = BitReader {
            data: bytes,
            buf: 0,
            bits_in_buf: 0,
            next_byte: 0,
            overread_bytes: 0,
            close_called: false,
            checked_out_of_bounds_bits: 0,
        };
        br.refill();
        br
    }

    /// The data the reader reads (`FirstByte()` with `TotalBytes()`).
    pub(crate) fn data(&self) -> &'a [u8] {
        self.data
    }

    // For time-critical reads, refills can be shared by multiple reads.
    // Based on variant 4 (plus bounds-checking), see
    // fgiesen.wordpress.com/2018/02/20/reading-bits-in-far-too-many-ways-part-2/
    #[inline]
    pub(crate) fn refill(&mut self) {
        if self.next_byte + 8 > self.data.len() {
            self.bounds_checked_refill();
        } else {
            // It's safe to load 64 bits; insert valid (possibly nonzero) bits above
            // bits_in_buf_. The shift requires bits_in_buf_ < 64.
            self.buf |= load_le64(&self.data[self.next_byte..]) << self.bits_in_buf;

            // Advance by bytes fully absorbed into the buffer.
            self.next_byte += (63 - self.bits_in_buf) >> 3;

            // We absorbed a multiple of 8 bits, so the lower 3 bits of bits_in_buf_
            // must remain unchanged, otherwise the next refill's shifted bits will
            // not align with buf_. Set the three upper bits so the result >= 56.
            self.bits_in_buf |= 56;
        }
    }

    // Returns the bits that would be returned by Read without calling Advance().
    // It is legal to PEEK at more bits than present in the bitstream (required
    // by Huffman), and those bits will be zero.
    #[inline]
    pub(crate) fn peek_fixed_bits<const N: usize>(&self) -> u64 {
        self.buf & ((1u64 << N) - 1)
    }

    #[inline]
    pub(crate) fn peek_bits(&self, nbits: usize) -> u64 {
        let mask = (1u64 << nbits) - 1;
        self.buf & mask
    }

    // Removes bits from the buffer. Need not match the previous Peek size, but
    // the buffer must contain at least num_bits (this prevents consuming more
    // than the total number of bits).
    #[inline]
    pub(crate) fn consume(&mut self, num_bits: usize) {
        self.bits_in_buf -= num_bits;
        self.buf >>= num_bits;
    }

    #[inline]
    pub(crate) fn read_bits(&mut self, nbits: usize) -> u64 {
        self.refill();
        let bits = self.peek_bits(nbits);
        self.consume(nbits);
        bits
    }

    #[inline]
    pub(crate) fn read_fixed_bits<const N: usize>(&mut self) -> u64 {
        self.refill();
        let bits = self.peek_fixed_bits::<N>();
        self.consume(N);
        bits
    }

    // Equivalent to calling ReadFixedBits(1) `skip` times, but much faster.
    // `skip` is typically large.
    pub(crate) fn skip_bits(&mut self, mut skip: usize) {
        // Buffer is large enough - don't zero buf_ below.
        if skip <= self.bits_in_buf {
            self.consume(skip);
            return;
        }

        // First deduct what we can satisfy from the buffer
        skip -= self.bits_in_buf;
        self.bits_in_buf = 0;
        // Not enough to call Advance - that may leave some bits in the buffer
        // which were previously ABOVE bits_in_buf.
        self.buf = 0;

        // Skip whole bytes
        let whole_bytes = skip / K_BITS_PER_BYTE;
        skip %= K_BITS_PER_BYTE;
        if whole_bytes > self.data.len() - self.next_byte {
            // This is already an overflow condition (skipping past the end of the bit
            // stream). However if we increase next_byte_ too much we risk overflowing
            // that value and potentially making it valid again (next_byte_ < end).
            // This will set next_byte_ to the end of the stream and still consume
            // some bits in overread_bytes_, however the TotalBitsConsumed() will be
            // incorrect (still larger than the TotalBytes()).
            self.next_byte = self.data.len();
            skip += K_BITS_PER_BYTE;
        } else {
            self.next_byte += whole_bytes;
        }

        self.refill();
        self.consume(skip);
    }

    pub(crate) fn total_bits_consumed(&self) -> usize {
        let bytes_read = self.next_byte;
        ((bytes_read as u64 + self.overread_bytes) as usize)
            .wrapping_mul(K_BITS_PER_BYTE)
            .wrapping_sub(self.bits_in_buf)
    }

    pub(crate) fn jump_to_byte_boundary(&mut self) -> Status {
        let remainder = self.total_bits_consumed() % K_BITS_PER_BYTE;
        if remainder == 0 {
            return Ok(());
        }
        if self.read_bits(K_BITS_PER_BYTE - remainder) != 0 {
            return jxl_failure!("Non-zero padding bits");
        }
        Ok(())
    }

    // For interoperability with other bitreaders (for resuming at
    // non-byte-aligned positions).
    pub(crate) fn total_bytes(&self) -> usize {
        self.data.len()
    }

    // Returns span of the remaining (unconsumed) bytes, e.g. for passing to
    // external decoders such as Brotli.
    #[allow(dead_code)]
    pub(crate) fn get_span(&self) -> &'a [u8] {
        let offset = self.total_bits_consumed() / K_BITS_PER_BYTE; // no remainder
        &self.data[offset.min(self.data.len())..]
    }

    // Returns whether all the bits read so far have been within the input bounds.
    // When reading past the EOF, the Read*() and Consume() functions return zeros
    // but flag a failure when calling Close() without checking this function.
    pub(crate) fn all_reads_within_bounds(&mut self) -> bool {
        // Mark up to which point the user checked the out of bounds condition. If
        // the user handles the condition at higher level (e.g. fetch more bytes
        // from network, return a custom JXL_FAILURE, ...), Close() should not
        // output a debug error (which would break tests with JXL_CRASH_ON_ERROR
        // even when legitimately handling the situation at higher level). This is
        // used by Bundle::CanRead.
        self.checked_out_of_bounds_bits = self.total_bits_consumed() as u64;
        if self.total_bits_consumed() > self.total_bytes() * K_BITS_PER_BYTE {
            return false;
        }
        true
    }

    // Close the bit reader and return whether all the previous reads were
    // successful. Close must be called once.
    pub(crate) fn close(&mut self) -> Status {
        self.close_called = true;
        if self.total_bits_consumed() as u64 > self.checked_out_of_bounds_bits
            && self.total_bits_consumed() > self.total_bytes() * K_BITS_PER_BYTE
        {
            return jxl_failure!("Read more bits than available in the bit_reader");
        }
        Ok(())
    }

    // Separate function avoids inlining this relatively cold code into callers.
    #[inline(never)]
    fn bounds_checked_refill(&mut self) {
        let end = self.data.len();

        // Read whole bytes until we have [56, 64) bits (same as LoadLE64)
        while self.bits_in_buf < 64 - K_BITS_PER_BYTE {
            if self.next_byte >= end {
                break;
            }
            self.buf |= (self.data[self.next_byte] as u64) << self.bits_in_buf;
            self.next_byte += 1;
            self.bits_in_buf += K_BITS_PER_BYTE;
        }

        // Add extra bytes as 0 at the end of the stream in the bit_buffer_. If
        // these bits are read, Close() will return a failure.
        let extra_bytes = (63 - self.bits_in_buf) / K_BITS_PER_BYTE;
        self.overread_bytes += extra_bytes as u64;
        self.bits_in_buf += extra_bytes * K_BITS_PER_BYTE;
    }
}
