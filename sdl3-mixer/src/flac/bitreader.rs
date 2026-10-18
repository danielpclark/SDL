// Rust translation of src/libFLAC/bitreader.c (and
// include/private/bitreader.h) from libFLAC 1.3.4 (https://xiph.org/flac/,
// at the revision SDL_mixer's external/flac pins).
// Copyright (C) 2000-2009  Josh Coalson
// Copyright (C) 2011-2018  Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The bit reader, with 32-bit words (`ENABLE_64_BIT_WORDS` is 0 in
//! upstream's CMake build). The read callback and its `client_data` are a
//! [`BitReaderReadCallback`] passed to each call that may need more data.
//! The buffer's words are kept in host order, as upstream's are after its
//! byte swaps; the bytes the callback reads are put into them in stream
//! (big-endian) order. Not translated, as the decoder doesn't use them:
//! `FLAC__bitreader_dump()`, `FLAC__bitreader_skip_bits_no_crc()` (only
//! the CUESHEET and PICTURE readers use it), `FLAC__bitreader_read_rice_signed()`
//! and the `#if 0` Golomb readers.

use super::bitmath::{FLAC__clz2_uint32, FLAC__clz_uint32};
use super::crc::{FLAC__crc16_update_words32, FLAC__CRC16_UPDATE};

/* Things should be fastest when this matches the machine word size */
/* WATCHOUT: if you change this you must also change the following #defines down to COUNT_ZERO_MSBS2 below to match */
/* WATCHOUT: there are a few places where the code will not work unless brword is >= 32 bits wide */
/*           also, some sections currently only have fast versions for 4 or 8 bytes per word */

type brword = u32;
const FLAC__BYTES_PER_WORD: u32 = 4; /* sizeof brword */
const FLAC__BITS_PER_WORD: u32 = 32;
const FLAC__WORD_ALL_ONES: u32 = 0xffffffff;
/* counts the # of zero MSBs in a word */
#[inline]
fn COUNT_ZERO_MSBS(word: brword) -> u32 {
    FLAC__clz_uint32(word)
}
#[inline]
fn COUNT_ZERO_MSBS2(word: brword) -> u32 {
    FLAC__clz2_uint32(word)
}

/*
 * This should be at least twice as large as the largest number of words
 * required to represent any 'number' (in any encoding) you are going to
 * read.  With FLAC this is on the order of maybe a few hundred bits.
 * If the buffer is smaller than that, the decoder won't be able to read
 * in a whole number that is in a variable length encoding (e.g. Rice).
 * But to be practical it should be at least 1K bytes.
 *
 * Increase this number to decrease the number of read callbacks, at the
 * expense of using more memory.  Or decrease for the reverse effect,
 * keeping in mind the limit from the first paragraph.  The optimal size
 * also depends on the CPU cache size and other factors; some twiddling
 * may be necessary to squeeze out the best performance.
 */
const FLAC__BITREADER_DEFAULT_CAPACITY: u32 = 65536 / FLAC__BITS_PER_WORD; /* in words */

/// `FLAC__BitReaderReadCallback` with its `client_data`: reads up to
/// `*bytes` bytes into `buffer`, setting `*bytes` to the number read;
/// `false` on failure.
pub(crate) trait BitReaderReadCallback {
    fn read(&mut self, buffer: &mut [u8], bytes: &mut usize) -> bool;
}

/// `FLAC__BitReader`.
#[derive(Default, Debug)]
pub(crate) struct BitReader {
    /* any partially-consumed word at the head will stay right-justified as bits are consumed from the left */
    /* any incomplete word at the tail will be left-justified, and bytes from the read callback are added on the right */
    buffer: Vec<brword>,
    capacity: u32,       /* in words */
    words: u32,          /* # of completed words in buffer */
    bytes: u32,          /* # of bytes in incomplete word at buffer[words] */
    consumed_words: u32, /* #words ... */
    consumed_bits: u32,  /* ... + (#bits of head word) already consumed from the front of buffer */
    read_crc16: u32,     /* the running frame CRC */
    crc16_offset: u32,   /* the number of words in the current buffer that should not be CRC'd */
    crc16_align: u32, /* the number of bits in the current consumed word that should not be CRC'd */
    /// (the bytes the read callback fills, before they go into `buffer`)
    target: Vec<u8>,
}

impl BitReader {
    #[inline]
    fn crc16_update_word_(&mut self, word: brword) {
        let mut crc = self.read_crc16;

        while self.crc16_align < FLAC__BITS_PER_WORD {
            let shift = FLAC__BITS_PER_WORD
                .wrapping_sub(8)
                .wrapping_sub(self.crc16_align);
            crc = FLAC__CRC16_UPDATE(
                if shift < FLAC__BITS_PER_WORD {
                    (word >> shift) & 0xff
                } else {
                    0
                },
                crc,
            );
            self.crc16_align += 8;
        }

        self.read_crc16 = crc;
        self.crc16_align = 0;
    }

    #[inline]
    fn crc16_update_block_(&mut self) {
        if self.consumed_words > self.crc16_offset && self.crc16_align != 0 {
            let word = self.buffer[self.crc16_offset as usize];
            self.crc16_offset += 1;
            self.crc16_update_word_(word);
        }

        /* Prevent OOB read due to wrap-around. */
        if self.consumed_words > self.crc16_offset {
            self.read_crc16 = FLAC__crc16_update_words32(
                &self.buffer[self.crc16_offset as usize..self.consumed_words as usize],
                self.read_crc16 as u16,
            ) as u32;
        }

        self.crc16_offset = 0;
    }

    fn bitreader_read_from_client_(&mut self, cb: &mut dyn BitReaderReadCallback) -> bool {
        /* first shift the unconsumed buffer data toward the front as much as possible */
        if self.consumed_words > 0 {
            self.crc16_update_block_(); /* CRC consumed words */

            let start = self.consumed_words;
            let end = self.words + if self.bytes != 0 { 1 } else { 0 };
            self.buffer.copy_within(start as usize..end as usize, 0);

            self.words -= start;
            self.consumed_words = 0;
        }

        /*
         * set the target for reading, taking into account word alignment and endianness
         */
        let mut bytes = ((self.capacity - self.words) * FLAC__BYTES_PER_WORD - self.bytes) as usize;
        if bytes == 0 {
            return false; /* no space left, buffer is too small; see note for FLAC__BITREADER_DEFAULT_CAPACITY  */
        }

        /* read in the data; note that the callback may return a smaller number of bytes */
        if !cb.read(&mut self.target[..bytes], &mut bytes) {
            return false;
        }

        /* (upstream reads into the buffer's memory past the tail bytes, with
         * the tail word swapped to big-endian order around the read; here
         * the bytes go into the host-order words in stream order, the same
         * thing) */
        let first = (self.words * FLAC__BYTES_PER_WORD + self.bytes) as usize;
        for (k, &b) in self.target[..bytes].iter().enumerate() {
            let pos = first + k;
            let shift = 24 - 8 * (pos % 4) as u32;
            let word = &mut self.buffer[pos / 4];
            *word = (*word & !(0xff << shift)) | ((b as brword) << shift);
        }

        /* finally we'll update the reader values: */
        let end = self.words * FLAC__BYTES_PER_WORD + self.bytes + bytes as u32;
        self.words = end / FLAC__BYTES_PER_WORD;
        self.bytes = end % FLAC__BYTES_PER_WORD;

        true
    }

    /***********************************************************************
     *
     * Class constructor/destructor
     *
     ***********************************************************************/

    /// `FLAC__bitreader_new()`.
    pub(crate) fn new() -> BitReader {
        BitReader::default()
    }

    /***********************************************************************
     *
     * Public class methods
     *
     ***********************************************************************/

    /// `FLAC__bitreader_init()` (`false` when the buffer can't be
    /// allocated).
    pub(crate) fn init(&mut self) -> bool {
        self.words = 0;
        self.bytes = 0;
        self.consumed_words = 0;
        self.consumed_bits = 0;
        self.capacity = FLAC__BITREADER_DEFAULT_CAPACITY;
        let Some(buffer) = try_zeroed(self.capacity as usize) else {
            return false;
        };
        let Some(target) = try_zeroed((self.capacity * FLAC__BYTES_PER_WORD) as usize) else {
            return false;
        };
        self.buffer = buffer;
        self.target = target;

        true
    }

    /// `FLAC__bitreader_free()`.
    pub(crate) fn free(&mut self) {
        self.buffer = Vec::new();
        self.target = Vec::new();
        self.capacity = 0;
        self.words = 0;
        self.bytes = 0;
        self.consumed_words = 0;
        self.consumed_bits = 0;
    }

    /// `FLAC__bitreader_clear()`.
    pub(crate) fn clear(&mut self) -> bool {
        self.words = 0;
        self.bytes = 0;
        self.consumed_words = 0;
        self.consumed_bits = 0;
        true
    }

    /// `FLAC__bitreader_reset_read_crc16()`.
    pub(crate) fn reset_read_crc16(&mut self, seed: u16) {
        self.read_crc16 = seed as u32;
        self.crc16_offset = self.consumed_words;
        self.crc16_align = self.consumed_bits;
    }

    /// `FLAC__bitreader_get_read_crc16()`.
    pub(crate) fn get_read_crc16(&mut self) -> u16 {
        /* CRC consumed words up to here */
        self.crc16_update_block_();

        /* CRC any tail bytes in a partially-consumed word */
        if self.consumed_bits != 0 {
            let tail = self.buffer[self.consumed_words as usize];
            while self.crc16_align < self.consumed_bits {
                self.read_crc16 = FLAC__CRC16_UPDATE(
                    (tail >> (FLAC__BITS_PER_WORD - 8 - self.crc16_align)) & 0xff,
                    self.read_crc16,
                );
                self.crc16_align += 8;
            }
        }
        self.read_crc16 as u16
    }

    /// `FLAC__bitreader_is_consumed_byte_aligned()`.
    #[inline]
    pub(crate) fn is_consumed_byte_aligned(&self) -> bool {
        (self.consumed_bits & 7) == 0
    }

    /// `FLAC__bitreader_bits_left_for_byte_alignment()`.
    #[inline]
    pub(crate) fn bits_left_for_byte_alignment(&self) -> u32 {
        8 - (self.consumed_bits & 7)
    }

    /// `FLAC__bitreader_get_input_bits_unconsumed()`.
    #[inline]
    pub(crate) fn get_input_bits_unconsumed(&self) -> u32 {
        (self.words - self.consumed_words) * FLAC__BITS_PER_WORD + self.bytes * 8
            - self.consumed_bits
    }

    /// `FLAC__bitreader_read_raw_uint32()`.
    pub(crate) fn read_raw_uint32(
        &mut self,
        val: &mut u32,
        mut bits: u32,
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        debug_assert!(bits <= 32);

        /* WATCHOUT: code does not work with <32bit words; we can make things much faster with this assertion */

        if bits == 0 {
            /* OPT: investigate if this can ever happen, maybe change to assertion */
            *val = 0;
            return true;
        }

        while (self.words - self.consumed_words) * FLAC__BITS_PER_WORD + self.bytes * 8
            - self.consumed_bits
            < bits
        {
            if !self.bitreader_read_from_client_(cb) {
                return false;
            }
        }
        if self.consumed_words < self.words {
            /* if we've not consumed up to a partial tail word... */
            /* OPT: taking out the consumed_bits==0 "else" case below might make things faster if less code allows the compiler to inline this function */
            if self.consumed_bits != 0 {
                /* this also works when consumed_bits==0, it's just a little slower than necessary for that case */
                let n = FLAC__BITS_PER_WORD - self.consumed_bits;
                let word = self.buffer[self.consumed_words as usize];
                let mask = if self.consumed_bits < FLAC__BITS_PER_WORD {
                    FLAC__WORD_ALL_ONES >> self.consumed_bits
                } else {
                    0
                };
                if bits < n {
                    let shift = n - bits;
                    *val = if shift < FLAC__BITS_PER_WORD {
                        (word & mask) >> shift
                    } else {
                        0
                    }; /* The result has <= 32 non-zero bits */
                    self.consumed_bits += bits;
                    return true;
                }
                /* (FLAC__BITS_PER_WORD - br->consumed_bits <= bits) ==> (FLAC__WORD_ALL_ONES >> br->consumed_bits) has no more than 'bits' non-zero bits */
                *val = word & mask;
                bits -= n;
                self.consumed_words += 1;
                self.consumed_bits = 0;
                if bits != 0 {
                    /* if there are still bits left to read, there have to be less than 32 so they will all be in the next word */
                    let shift = FLAC__BITS_PER_WORD - bits;
                    *val = if bits < 32 { *val << bits } else { 0 };
                    *val |= if shift < FLAC__BITS_PER_WORD {
                        self.buffer[self.consumed_words as usize] >> shift
                    } else {
                        0
                    };
                    self.consumed_bits = bits;
                }
                true
            } else {
                /* br->consumed_bits == 0 */
                let word = self.buffer[self.consumed_words as usize];
                if bits < FLAC__BITS_PER_WORD {
                    *val = word >> (FLAC__BITS_PER_WORD - bits);
                    self.consumed_bits = bits;
                    return true;
                }
                /* at this point bits == FLAC__BITS_PER_WORD == 32; because of previous assertions, it can't be larger */
                *val = word;
                self.consumed_words += 1;
                true
            }
        } else {
            /* in this case we're starting our read at a partial tail word;
             * the reader has guaranteed that we have at least 'bits' bits
             * available to read, which makes this case simpler.
             */
            /* OPT: taking out the consumed_bits==0 "else" case below might make things faster if less code allows the compiler to inline this function */
            if self.consumed_bits != 0 {
                /* this also works when consumed_bits==0, it's just a little slower than necessary for that case */
                debug_assert!(self.consumed_bits + bits <= self.bytes * 8);
                *val = (self.buffer[self.consumed_words as usize]
                    & (FLAC__WORD_ALL_ONES >> self.consumed_bits))
                    >> (FLAC__BITS_PER_WORD - self.consumed_bits - bits);
                self.consumed_bits += bits;
                true
            } else {
                *val = self.buffer[self.consumed_words as usize] >> (FLAC__BITS_PER_WORD - bits);
                self.consumed_bits += bits;
                true
            }
        }
    }

    /// `FLAC__bitreader_read_raw_int32()`.
    pub(crate) fn read_raw_int32(
        &mut self,
        val: &mut i32,
        bits: u32,
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        let mut uval: u32 = 0;
        /* OPT: inline raw uint32 code here, or make into a macro if possible in the .h file */
        if bits < 1 || !self.read_raw_uint32(&mut uval, bits, cb) {
            return false;
        }
        /* sign-extend *val assuming it is currently bits wide. */
        /* From: https://graphics.stanford.edu/~seander/bithacks.html#FixedSignExtend */
        let mask: u32 = if bits >= 33 { 0 } else { 1u32 << (bits - 1) };
        *val = (uval ^ mask).wrapping_sub(mask) as i32;
        true
    }

    /// `FLAC__bitreader_read_raw_uint64()`.
    pub(crate) fn read_raw_uint64(
        &mut self,
        val: &mut u64,
        bits: u32,
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        let mut hi: u32 = 0;
        let mut lo: u32 = 0;

        if bits > 32 {
            if !self.read_raw_uint32(&mut hi, bits - 32, cb) {
                return false;
            }
            if !self.read_raw_uint32(&mut lo, 32, cb) {
                return false;
            }
            *val = hi as u64;
            *val <<= 32;
            *val |= lo as u64;
        } else {
            if !self.read_raw_uint32(&mut lo, bits, cb) {
                return false;
            }
            *val = lo as u64;
        }
        true
    }

    /// `FLAC__bitreader_read_uint32_little_endian()`.
    pub(crate) fn read_uint32_little_endian(
        &mut self,
        val: &mut u32,
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        let mut x8: u32 = 0;
        let mut x32: u32 = 0;

        /* this doesn't need to be that fast as currently it is only used for vorbis comments */

        if !self.read_raw_uint32(&mut x32, 8, cb) {
            return false;
        }

        if !self.read_raw_uint32(&mut x8, 8, cb) {
            return false;
        }
        x32 |= x8 << 8;

        if !self.read_raw_uint32(&mut x8, 8, cb) {
            return false;
        }
        x32 |= x8 << 16;

        if !self.read_raw_uint32(&mut x8, 8, cb) {
            return false;
        }
        x32 |= x8 << 24;

        *val = x32;
        true
    }

    /// `FLAC__bitreader_skip_byte_block_aligned_no_crc()`.
    pub(crate) fn skip_byte_block_aligned_no_crc(
        &mut self,
        mut nvals: u32,
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        let mut x: u32 = 0;

        /* step 1: skip over partial head word to get word aligned */
        while nvals != 0 && self.consumed_bits != 0 {
            /* i.e. run until we read 'nvals' bytes or we hit the end of the head word */
            if !self.read_raw_uint32(&mut x, 8, cb) {
                return false;
            }
            nvals -= 1;
        }
        if 0 == nvals {
            return true;
        }
        /* step 2: skip whole words in chunks */
        while nvals >= FLAC__BYTES_PER_WORD {
            if self.consumed_words < self.words {
                self.consumed_words += 1;
                nvals -= FLAC__BYTES_PER_WORD;
            } else if !self.bitreader_read_from_client_(cb) {
                return false;
            }
        }
        /* step 3: skip any remainder from partial tail bytes */
        while nvals != 0 {
            if !self.read_raw_uint32(&mut x, 8, cb) {
                return false;
            }
            nvals -= 1;
        }

        true
    }

    /// `FLAC__bitreader_read_byte_block_aligned_no_crc()`: reads
    /// `val.len()` bytes (C's `nvals`).
    pub(crate) fn read_byte_block_aligned_no_crc(
        &mut self,
        val: &mut [u8],
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        let mut x: u32 = 0;
        let mut nvals = val.len();
        let mut v = 0usize;

        /* step 1: read from partial head word to get word aligned */
        while nvals != 0 && self.consumed_bits != 0 {
            /* i.e. run until we read 'nvals' bytes or we hit the end of the head word */
            if !self.read_raw_uint32(&mut x, 8, cb) {
                return false;
            }
            val[v] = x as u8;
            v += 1;
            nvals -= 1;
        }
        if 0 == nvals {
            return true;
        }
        /* step 2: read whole words in chunks */
        while nvals >= FLAC__BYTES_PER_WORD as usize {
            if self.consumed_words < self.words {
                let word = self.buffer[self.consumed_words as usize];
                self.consumed_words += 1;
                val[v..v + 4].copy_from_slice(&word.to_be_bytes());
                v += FLAC__BYTES_PER_WORD as usize;
                nvals -= FLAC__BYTES_PER_WORD as usize;
            } else if !self.bitreader_read_from_client_(cb) {
                return false;
            }
        }
        /* step 3: read any remainder from partial tail bytes */
        while nvals != 0 {
            if !self.read_raw_uint32(&mut x, 8, cb) {
                return false;
            }
            val[v] = x as u8;
            v += 1;
            nvals -= 1;
        }

        true
    }

    /// `FLAC__bitreader_read_unary_unsigned()` (the fast version; the
    /// `#if 0` slow but readable one isn't translated).
    pub(crate) fn read_unary_unsigned(
        &mut self,
        val: &mut u32,
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        let mut i: u32;

        *val = 0;
        loop {
            while self.consumed_words < self.words {
                /* if we've not consumed up to a partial tail word... */
                let b: brword = if self.consumed_bits < FLAC__BITS_PER_WORD {
                    self.buffer[self.consumed_words as usize] << self.consumed_bits
                } else {
                    0
                };
                if b != 0 {
                    i = COUNT_ZERO_MSBS(b);
                    *val = val.wrapping_add(i);
                    i += 1;
                    self.consumed_bits += i;
                    if self.consumed_bits >= FLAC__BITS_PER_WORD {
                        /* faster way of testing if(br->consumed_bits == FLAC__BITS_PER_WORD) */
                        self.consumed_words += 1;
                        self.consumed_bits = 0;
                    }
                    return true;
                } else {
                    *val = val.wrapping_add(FLAC__BITS_PER_WORD - self.consumed_bits);
                    self.consumed_words += 1;
                    self.consumed_bits = 0;
                    /* didn't find stop bit yet, have to keep going... */
                }
            }
            /* at this point we've eaten up all the whole words; have to try
             * reading through any tail bytes before calling the read callback.
             * this is a repeat of the above logic adjusted for the fact we
             * don't have a whole word.  note though if the client is feeding
             * us data a byte at a time (unlikely), br->consumed_bits may not
             * be zero.
             */
            if self.bytes * 8 > self.consumed_bits {
                let end = self.bytes * 8;
                let b: brword = (self.buffer[self.consumed_words as usize]
                    & (FLAC__WORD_ALL_ONES << (FLAC__BITS_PER_WORD - end)))
                    << self.consumed_bits;
                if b != 0 {
                    i = COUNT_ZERO_MSBS(b);
                    *val = val.wrapping_add(i);
                    i += 1;
                    self.consumed_bits += i;
                    debug_assert!(self.consumed_bits < FLAC__BITS_PER_WORD);
                    return true;
                } else {
                    *val = val.wrapping_add(end - self.consumed_bits);
                    self.consumed_bits = end;
                    debug_assert!(self.consumed_bits < FLAC__BITS_PER_WORD);
                    /* didn't find stop bit yet, have to keep going... */
                }
            }
            if !self.bitreader_read_from_client_(cb) {
                return false;
            }
        }
    }

    /// `FLAC__bitreader_read_rice_signed_block()`: reads `nvals` values into
    /// `vals`.
    ///
    /// FIXME (upstream): with no values to read (a first partition with
    /// as many samples as the predictor's order) and the whole words of
    /// the buffer consumed, the jump to `process_tail` reads a value
    /// anyway, into `vals[0]` (the next partition's first residual, which
    /// it then overwrites), consuming its bits. That is kept: `vals` may
    /// go on past `nvals` (danielpclark/SDL#142).
    ///
    /// this is by far the most heavily used reader call.  it ain't pretty but it's fast
    pub(crate) fn read_rice_signed_block(
        &mut self,
        vals: &mut [i32],
        nvals: usize,
        parameter: u32,
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        /* try and get br->consumed_words and br->consumed_bits into register;
         * must remember to flush them back to *br before calling other
         * bitreader functions that use them, and before returning */
        let mut cwords: u32;
        let mut words: u32;
        let mut lsbs: u32 = 0;
        let mut msbs: u32 = 0;
        let mut x: u32;
        let mut y: u32;
        let mut ucbits: u32; /* keep track of the number of unconsumed bits in word */
        let mut b: brword;
        let end = nvals;
        let mut val = 0usize;

        debug_assert!(parameter < 32);
        /* WATCHOUT: code does not work with <32bit words; we can make things much faster with this assertion */
        /* the above two asserts also guarantee that the binary part never straddles more than 2 words, so we don't have to loop to read it */

        if parameter == 0 {
            while val < end {
                /* read the unary MSBs and end bit */
                if !self.read_unary_unsigned(&mut msbs, cb) {
                    return false;
                }

                vals[val] = (msbs >> 1) as i32 ^ ((msbs & 1) as i32).wrapping_neg();
                val += 1;
            }

            return true;
        }

        cwords = self.consumed_words;
        words = self.words;

        /* (C's gotos into the tail loop: where the main loop's reading
         * enters it) */
        #[derive(PartialEq)]
        enum Tail {
            /// `goto process_tail`
            Process,
            /// `goto incomplete_msbs`
            IncompleteMsbs,
            /// `goto incomplete_lsbs`
            IncompleteLsbs,
        }

        let mut tail: Option<Tail> = None;
        /* if we've not consumed up to a partial tail word... */
        if cwords >= words {
            x = 0;
            ucbits = 0; // (unset in C; the tail sets it before use)
            b = 0;
            tail = Some(Tail::Process);
        } else {
            ucbits = FLAC__BITS_PER_WORD - self.consumed_bits;
            b = self.buffer[cwords as usize] << self.consumed_bits; /* keep unconsumed bits aligned to left */
            x = 0;
        }

        while val < end || tail.is_some() {
            if tail.is_none() {
                /* read the unary MSBs and end bit */
                y = COUNT_ZERO_MSBS2(b);
                x = y;
                let mut incomplete = false;
                if x == FLAC__BITS_PER_WORD {
                    x = ucbits;
                    loop {
                        /* didn't find stop bit yet, have to keep going... */
                        cwords += 1;
                        if cwords >= words {
                            incomplete = true;
                            break;
                        }
                        b = self.buffer[cwords as usize];
                        y = COUNT_ZERO_MSBS2(b);
                        x = x.wrapping_add(y);
                        if y != FLAC__BITS_PER_WORD {
                            break;
                        }
                    }
                }
                if incomplete {
                    tail = Some(Tail::IncompleteMsbs);
                } else {
                    b <<= y;
                    b <<= 1; /* account for stop bit */
                    ucbits = ucbits.wrapping_sub(x).wrapping_sub(1) % FLAC__BITS_PER_WORD;
                    msbs = x;

                    /* read the binary LSBs */
                    x = b >> (FLAC__BITS_PER_WORD - parameter); /* parameter < 32, so we can cast to 32-bit uint32_t */
                    if parameter <= ucbits {
                        ucbits -= parameter;
                        b <<= parameter;
                    } else {
                        /* there are still bits left to read, they will all be in the next word */
                        cwords += 1;
                        if cwords >= words {
                            tail = Some(Tail::IncompleteLsbs);
                        } else {
                            b = self.buffer[cwords as usize];
                            ucbits += FLAC__BITS_PER_WORD - parameter;
                            x |= b >> ucbits;
                            b <<= FLAC__BITS_PER_WORD - ucbits;
                        }
                    }
                    if tail.is_none() {
                        lsbs = x;

                        /* compose the value */
                        x = (msbs << parameter) | lsbs;
                        vals[val] = (x >> 1) as i32 ^ ((x & 1) as i32).wrapping_neg();
                        val += 1;

                        continue;
                    }
                }
            }

            /* at this point we've eaten up all the whole words */
            // process_tail:
            loop {
                let entry = tail.take();
                if entry != Some(Tail::IncompleteLsbs) {
                    if entry == Some(Tail::IncompleteMsbs) {
                        // incomplete_msbs:
                        self.consumed_bits = 0;
                        self.consumed_words = cwords;
                    }

                    /* read the unary MSBs and end bit */
                    if !self.read_unary_unsigned(&mut msbs, cb) {
                        return false;
                    }
                    msbs = msbs.wrapping_add(x);
                    x = 0;
                    ucbits = 0;
                } else {
                    // incomplete_lsbs:
                    self.consumed_bits = 0;
                    self.consumed_words = cwords;
                }

                /* read the binary LSBs */
                if !self.read_raw_uint32(&mut lsbs, parameter - ucbits, cb) {
                    return false;
                }
                lsbs = x | lsbs;

                /* compose the value */
                x = (msbs << parameter) | lsbs;
                if let Some(v) = vals.get_mut(val) {
                    *v = (x >> 1) as i32 ^ ((x & 1) as i32).wrapping_neg();
                }
                val += 1;
                x = 0;

                cwords = self.consumed_words;
                words = self.words;
                ucbits = FLAC__BITS_PER_WORD - self.consumed_bits;
                b = if cwords < self.capacity {
                    self.buffer[cwords as usize].wrapping_shl(self.consumed_bits)
                } else {
                    0
                };
                if !(cwords >= words && val < end) {
                    break;
                }
            }
        }

        if ucbits == 0 && cwords < words {
            /* don't leave the head word with no unconsumed bits */
            cwords += 1;
            ucbits = FLAC__BITS_PER_WORD;
        }

        self.consumed_bits = FLAC__BITS_PER_WORD - ucbits;
        self.consumed_words = cwords;

        true
    }

    /// `FLAC__bitreader_read_utf8_uint32()`: on return, if `*val ==
    /// 0xffffffff` then the utf-8 sequence was invalid, but the return value
    /// will be true. The bytes read are appended to `raw[*rawlen..]`.
    pub(crate) fn read_utf8_uint32(
        &mut self,
        val: &mut u32,
        raw: &mut [u8],
        rawlen: &mut u32,
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        let mut v: u32;
        let mut x: u32 = 0;
        let mut i: u32;

        if !self.read_raw_uint32(&mut x, 8, cb) {
            return false;
        }
        raw[*rawlen as usize] = x as u8;
        *rawlen += 1;
        if x & 0x80 == 0 {
            /* 0xxxxxxx */
            v = x;
            i = 0;
        } else if x & 0xC0 != 0 && x & 0x20 == 0 {
            /* 110xxxxx */
            v = x & 0x1F;
            i = 1;
        } else if x & 0xE0 != 0 && x & 0x10 == 0 {
            /* 1110xxxx */
            v = x & 0x0F;
            i = 2;
        } else if x & 0xF0 != 0 && x & 0x08 == 0 {
            /* 11110xxx */
            v = x & 0x07;
            i = 3;
        } else if x & 0xF8 != 0 && x & 0x04 == 0 {
            /* 111110xx */
            v = x & 0x03;
            i = 4;
        } else if x & 0xFC != 0 && x & 0x02 == 0 {
            /* 1111110x */
            v = x & 0x01;
            i = 5;
        } else {
            *val = 0xffffffff;
            return true;
        }
        while i != 0 {
            if !self.read_raw_uint32(&mut x, 8, cb) {
                return false;
            }
            raw[*rawlen as usize] = x as u8;
            *rawlen += 1;
            if x & 0x80 == 0 || x & 0x40 != 0 {
                /* 10xxxxxx */
                *val = 0xffffffff;
                return true;
            }
            v <<= 6;
            v |= x & 0x3F;
            i -= 1;
        }
        *val = v;
        true
    }

    /// `FLAC__bitreader_read_utf8_uint64()`: on return, if `*val ==
    /// 0xffffffffffffffff` then the utf-8 sequence was invalid, but the
    /// return value will be true. The bytes read are appended to
    /// `raw[*rawlen..]`.
    pub(crate) fn read_utf8_uint64(
        &mut self,
        val: &mut u64,
        raw: &mut [u8],
        rawlen: &mut u32,
        cb: &mut dyn BitReaderReadCallback,
    ) -> bool {
        let mut v: u64;
        let mut x: u32 = 0;
        let mut i: u32;

        if !self.read_raw_uint32(&mut x, 8, cb) {
            return false;
        }
        raw[*rawlen as usize] = x as u8;
        *rawlen += 1;
        if x & 0x80 == 0 {
            /* 0xxxxxxx */
            v = x as u64;
            i = 0;
        } else if x & 0xC0 != 0 && x & 0x20 == 0 {
            /* 110xxxxx */
            v = (x & 0x1F) as u64;
            i = 1;
        } else if x & 0xE0 != 0 && x & 0x10 == 0 {
            /* 1110xxxx */
            v = (x & 0x0F) as u64;
            i = 2;
        } else if x & 0xF0 != 0 && x & 0x08 == 0 {
            /* 11110xxx */
            v = (x & 0x07) as u64;
            i = 3;
        } else if x & 0xF8 != 0 && x & 0x04 == 0 {
            /* 111110xx */
            v = (x & 0x03) as u64;
            i = 4;
        } else if x & 0xFC != 0 && x & 0x02 == 0 {
            /* 1111110x */
            v = (x & 0x01) as u64;
            i = 5;
        } else if x & 0xFE != 0 && x & 0x01 == 0 {
            /* 11111110 */
            v = 0;
            i = 6;
        } else {
            *val = 0xffff_ffff_ffff_ffff;
            return true;
        }
        while i != 0 {
            if !self.read_raw_uint32(&mut x, 8, cb) {
                return false;
            }
            raw[*rawlen as usize] = x as u8;
            *rawlen += 1;
            if x & 0x80 == 0 || x & 0x40 != 0 {
                /* 10xxxxxx */
                *val = 0xffff_ffff_ffff_ffff;
                return true;
            }
            v <<= 6;
            v |= (x & 0x3F) as u64;
            i -= 1;
        }
        *val = v;
        true
    }
}

/// A zeroed `len`-element vector, `None` when it can't be allocated
/// (C's `malloc()` failing).
fn try_zeroed<T: Clone + Default>(len: usize) -> Option<Vec<T>> {
    crate::internal::try_alloc(len).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds the reader a few bytes at a time.
    struct Src<'a>(&'a [u8], usize);

    impl BitReaderReadCallback for Src<'_> {
        fn read(&mut self, buffer: &mut [u8], bytes: &mut usize) -> bool {
            let n = (*bytes).min(self.1).min(self.0.len());
            buffer[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            *bytes = n;
            n > 0
        }
    }

    /// Rice codes of a few parameters, split over the reads in every
    /// way: the block reader agrees with a bit-by-bit decoding.
    #[test]
    fn rice_block_across_reads() {
        let vals: Vec<i32> = (0..300).map(|i| ((i * 37) % 201) - 100).collect();
        for parameter in [0u32, 1, 3, 7, 14, 20] {
            // encode
            let mut bits: Vec<bool> = Vec::new();
            for &v in &vals {
                let u = if v < 0 {
                    ((-v as u32) << 1) - 1
                } else {
                    (v as u32) << 1
                };
                bits.resize(bits.len() + (u >> parameter) as usize, false);
                bits.push(true);
                for k in (0..parameter).rev() {
                    bits.push((u >> k) & 1 != 0);
                }
            }
            while bits.len() % 8 != 0 {
                bits.push(false);
            }
            let bytes: Vec<u8> = bits
                .chunks(8)
                .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | b as u8))
                .collect();
            for chunk in [1usize, 3, 5, 7, 4096] {
                let mut br = BitReader::new();
                assert!(br.init());
                let mut src = Src(&bytes, chunk);
                let mut out = vec![0i32; vals.len()];
                let mut skip = 0u32;
                assert!(br.read_raw_uint32(&mut skip, 0, &mut src));
                let n = out.len();
                assert!(br.read_rice_signed_block(&mut out, n, parameter, &mut src));
                assert_eq!(out, vals, "parameter {parameter} chunk {chunk}");
            }
        }
    }
}
