// Rust translation of src/stream.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The read-only byte stream the box parsers read: big-endian integers,
//! bits, strings and box headers, failing (with a diagnostic) where the
//! data runs out, and the writable stream the encoder writes boxes with
//! (`avifRWStream`).

use super::avif::AvifResult;
use super::diag::{fourcc, printf, AvifDiagnostics};
use super::internal::{avif_check, avif_checkerr, avif_checkres, AvifBoxHeader};
use super::rawdata::avif_rw_data_realloc;

/// Translation of `avifROStream`. In network byte order (big-endian)
/// unless otherwise specified.
pub(crate) struct AvifROStream<'a, 'd> {
    pub(crate) raw: &'a [u8],

    /// Index of the next byte in the raw stream.
    pub(crate) offset: usize,

    /// If 0, byte-aligned functions can be used (avifROStreamRead() etc.).
    /// Otherwise, it represents the number of bits already used in the last byte
    /// (located at offset-1).
    pub(crate) num_used_bits_in_partial_byte: usize,

    // Error information, if any.
    pub(crate) diag: Option<&'d AvifDiagnostics>,
    pub(crate) diag_context: &'d str,
}

impl<'a, 'd> AvifROStream<'a, 'd> {
    /// Translation of `avifROStreamStart()` (and of read.c's
    /// `BEGIN_STREAM()`).
    pub(crate) fn start(
        raw: &'a [u8],
        diag: Option<&'d AvifDiagnostics>,
        diag_context: &'d str,
    ) -> AvifROStream<'a, 'd> {
        AvifROStream {
            raw,
            offset: 0,
            num_used_bits_in_partial_byte: 0,
            diag,
            diag_context,
        }
    }

    /// Translation of `avifROStreamCurrent()`: the bytes from the offset.
    pub(crate) fn current(&self) -> &'a [u8] {
        &self.raw[self.offset.min(self.raw.len())..]
    }

    /// Translation of `avifROStreamHasBytesLeft()`.
    pub(crate) fn has_bytes_left(&self, byte_count: usize) -> bool {
        byte_count <= (self.raw.len() - self.offset)
    }

    /// Translation of `avifROStreamRemainingBytes()`.
    pub(crate) fn remaining_bytes(&self) -> usize {
        self.raw.len() - self.offset
    }

    /// Translation of `avifROStreamOffset()`.
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Translation of `avifROStreamSetOffset()`.
    pub(crate) fn set_offset(&mut self, offset: usize) {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        self.offset = offset;
        if self.offset > self.raw.len() {
            self.offset = self.raw.len();
        }
    }

    /// Translation of `avifROStreamSkip()`.
    pub(crate) fn skip(&mut self, byte_count: usize) -> bool {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        if !self.has_bytes_left(byte_count) {
            printf(
                self.diag,
                format_args!(
                    "{}: Failed to skip {} bytes, truncated data?",
                    self.diag_context, byte_count
                ),
            );
            return false;
        }
        self.offset += byte_count;
        true
    }

    /// Translation of `avifROStreamRead()`.
    pub(crate) fn read(&mut self, data: &mut [u8]) -> bool {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let size = data.len();
        if !self.has_bytes_left(size) {
            printf(
                self.diag,
                format_args!(
                    "{}: Failed to read {} bytes, truncated data?",
                    self.diag_context, size
                ),
            );
            return false;
        }

        data.copy_from_slice(&self.raw[self.offset..self.offset + size]);
        self.offset += size;
        true
    }

    /// Reads a factor*8 sized uint, saves in v. If factor is 0, reads nothing and saves 0 in v.
    /// Translation of `avifROStreamReadUX8()`.
    pub(crate) fn read_ux8(&mut self, v: &mut u64, factor: u64) -> bool {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        if factor == 0 {
            // Don't read anything, just set to 0
            *v = 0;
        } else if factor == 1 {
            let mut tmp = 0u8;
            avif_check!(self.read_u8(&mut tmp));
            *v = tmp as u64;
        } else if factor == 2 {
            let mut tmp = 0u16;
            avif_check!(self.read_u16(&mut tmp));
            *v = tmp as u64;
        } else if factor == 4 {
            let mut tmp = 0u32;
            avif_check!(self.read_u32(&mut tmp));
            *v = tmp as u64;
        } else if factor == 8 {
            let mut tmp = 0u64;
            avif_check!(self.read_u64(&mut tmp));
            *v = tmp;
        } else {
            // Unsupported factor
            printf(
                self.diag,
                format_args!(
                    "{}: Failed to read UX8 value; Unsupported UX8 factor [{}]",
                    self.diag_context, factor
                ),
            );
            return false;
        }
        true
    }

    /// `avifROStreamRead(stream, &v, 1)`.
    pub(crate) fn read_u8(&mut self, v: &mut u8) -> bool {
        let mut b = [0u8; 1];
        avif_check!(self.read(&mut b));
        *v = b[0];
        true
    }

    /// Translation of `avifROStreamReadU16()`.
    pub(crate) fn read_u16(&mut self, v: &mut u16) -> bool {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let mut b = [0u8; 2];
        avif_check!(self.read(&mut b));
        *v = u16::from_be_bytes(b);
        true
    }

    /// Translation of `avifROStreamReadU16Endianness()`.
    pub(crate) fn read_u16_endianness(&mut self, v: &mut u16, little_endian: bool) -> bool {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let mut b = [0u8; 2];
        avif_check!(self.read(&mut b));
        *v = if little_endian {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        };
        true
    }

    /// Translation of `avifROStreamReadU32()`.
    pub(crate) fn read_u32(&mut self, v: &mut u32) -> bool {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let mut b = [0u8; 4];
        avif_check!(self.read(&mut b));
        *v = u32::from_be_bytes(b);
        true
    }

    /// Translation of `avifROStreamReadU32Endianness()`.
    pub(crate) fn read_u32_endianness(&mut self, v: &mut u32, little_endian: bool) -> bool {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let mut b = [0u8; 4];
        avif_check!(self.read(&mut b));
        *v = if little_endian {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        };
        true
    }

    /// Translation of `avifROStreamReadU64()`.
    pub(crate) fn read_u64(&mut self, v: &mut u64) -> bool {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let mut b = [0u8; 8];
        avif_check!(self.read(&mut b));
        *v = u64::from_be_bytes(b);
        true
    }

    /// Override of avifROStreamReadBits() for convenient uint8_t output.
    /// Translation of `avifROStreamReadBits8()`.
    pub(crate) fn read_bits8(&mut self, v: &mut u8, bit_count: usize) -> bool {
        debug_assert!(bit_count <= 8);
        let mut v32 = 0u32;
        if !self.read_bits(&mut v32, bit_count) {
            return false;
        }
        *v = v32 as u8;
        true
    }

    /// The following functions can write non-aligned bits. Translation of
    /// `avifROStreamReadBits()`.
    pub(crate) fn read_bits(&mut self, v: &mut u32, mut bit_count: usize) -> bool {
        debug_assert!(bit_count <= 32);
        *v = 0;
        while bit_count != 0 {
            if self.num_used_bits_in_partial_byte == 0 {
                avif_check!(self.skip(1)); // Book a new partial byte in the stream.
            }
            debug_assert!(self.offset > 0);
            let packed_bits = self.raw[self.offset - 1];

            let num_bits = bit_count.min(8 - self.num_used_bits_in_partial_byte);
            self.num_used_bits_in_partial_byte += num_bits;
            bit_count -= num_bits;
            // The stream bits are packed starting with the most significant bit of the first input byte.
            // This way, packed bits can be found in the same order in the bit stream.
            let bits = ((packed_bits as u32) >> (8 - self.num_used_bits_in_partial_byte))
                & ((1u32 << num_bits) - 1);
            // The value bits are ordered from the most significant bit to the least significant bit.
            // In the case where avifROStreamReadBits() is used to parse the unsigned integer value *v
            // over multiple aligned bytes, this order corresponds to big endianness.
            *v |= bits << bit_count;

            if self.num_used_bits_in_partial_byte == 8 {
                // Start a new partial byte the next time a bit is needed.
                self.num_used_bits_in_partial_byte = 0;
            }
        }
        true
    }

    /// Translation of `avifROStreamReadString()`: the string (without its
    /// NUL terminator) goes to `output`, clamped to `output_size - 1`
    /// bytes, when given.
    pub(crate) fn read_string(&mut self, output: Option<&mut Vec<u8>>, output_size: usize) -> bool {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.

        // Check for the presence of a null terminator in the stream.
        let p = self.current();
        let Some(mut string_len) = p.iter().position(|&b| b == 0) else {
            printf(
                self.diag,
                format_args!(
                    "{}: Failed to find a NULL terminator when reading a string",
                    self.diag_context
                ),
            );
            return false;
        };

        let stream_string = &p[..string_len];
        self.offset += string_len + 1; // update the stream to have read the "whole string" in

        if let Some(output) = output {
            if output_size != 0 {
                // clamp to our output buffer
                if string_len >= output_size {
                    string_len = output_size - 1;
                }
                output.clear();
                output.extend_from_slice(&stream_string[..string_len]);
            }
        }
        true
    }

    /// This doesn't require that the full box can fit in the stream.
    /// Translation of `avifROStreamReadBoxHeaderPartial()`.
    pub(crate) fn read_box_header_partial(
        &mut self,
        header: &mut AvifBoxHeader,
        top_level: bool,
    ) -> bool {
        // Section 4.2.2 of ISO/IEC 14496-12.
        let start_offset = self.offset;

        let mut small_size = 0u32;
        avif_check!(self.read_u32(&mut small_size)); // unsigned int(32) size;
        avif_check!(self.read(&mut header.type_)); // unsigned int(32) type = boxtype;

        let mut size = small_size as u64;
        if size == 1 {
            avif_check!(self.read_u64(&mut size)); // unsigned int(64) largesize;
        }

        if &header.type_ == b"uuid" {
            avif_check!(self.skip(16)); // unsigned int(8) usertype[16] = extended_type;
        }

        let bytes_read = self.offset - start_offset;
        if size == 0 {
            // Section 4.2.2 of ISO/IEC 14496-12.
            //   if size is 0, then this box shall be in a top-level box (i.e. not contained in another
            //   box), and be the last box in its 'file', and its payload extends to the end of that
            //   enclosing 'file'. This is normally only used for a MediaDataBox.
            if !top_level {
                printf(
                    self.diag,
                    format_args!("{}: Non-top-level box with size 0", self.diag_context),
                );
                return false;
            }

            // The given stream may be incomplete and there is no guarantee that sizeHint is available and accurate.
            // Otherwise size could be set to avifROStreamRemainingBytes(stream) + (stream->offset - startOffset) right now.

            // Wait for avifIOReadFunc() to return AVIF_RESULT_OK.
            header.is_size_zero_box = true;
            header.size = 0;
            return true;
        }

        if (size < bytes_read as u64) || ((size - bytes_read as u64) > usize::MAX as u64) {
            printf(
                self.diag,
                format_args!("{}: Header size overflow check failure", self.diag_context),
            );
            return false;
        }
        header.is_size_zero_box = false;
        header.size = (size - bytes_read as u64) as usize;
        true
    }

    /// This fails if the size reported by the header cannot fit in the
    /// stream. Translation of `avifROStreamReadBoxHeader()`.
    pub(crate) fn read_box_header(&mut self, header: &mut AvifBoxHeader) -> bool {
        avif_check!(self.read_box_header_partial(header, /*topLevel=*/ false));
        if header.size > self.remaining_bytes() {
            printf(
                self.diag,
                format_args!(
                    "{}: Child box too large, possibly truncated data",
                    self.diag_context
                ),
            );
            return false;
        }
        true
    }

    /// version and flags ptrs are both optional. Translation of
    /// `avifROStreamReadVersionAndFlags()`.
    pub(crate) fn read_version_and_flags(
        &mut self,
        version: Option<&mut u8>,
        flags: Option<&mut u32>,
    ) -> bool {
        let mut version_and_flags = [0u8; 4];
        avif_check!(self.read(&mut version_and_flags));
        if let Some(version) = version {
            *version = version_and_flags[0];
        }
        if let Some(flags) = flags {
            *flags = ((version_and_flags[1] as u32) << 16)
                + ((version_and_flags[2] as u32) << 8)
                + (version_and_flags[3] as u32);
        }
        true
    }

    /// currently discards flags. Translation of
    /// `avifROStreamReadAndEnforceVersion()`.
    pub(crate) fn read_and_enforce_version(&mut self, enforced_version: u8) -> bool {
        let mut version = 0u8;
        avif_check!(self.read_version_and_flags(Some(&mut version), None));
        if version != enforced_version {
            printf(
                self.diag,
                format_args!(
                    "{}: Expecting box version {}, got version {}",
                    self.diag_context, enforced_version, version
                ),
            );
            return false;
        }
        true
    }
}

/// The type of a box header, for diagnostics (`%.4s`).
pub(crate) fn box_type(header: &AvifBoxHeader) -> String {
    fourcc(&header.type_)
}

// ---------------------------------------------------------------------------
// avifRWStream

/// `avifBoxMarker`: the offset of a box being written.
pub(crate) type AvifBoxMarker = usize;

/// This is to make it clear that the box size is currently unknown, and will be determined later (with a call to avifRWStreamFinishBox)
pub(crate) const AVIF_BOX_SIZE_TBD: usize = 0;

/// Translation of `avifRWStream`. The stream owns the `avifRWData` it
/// writes to (`raw`, whose length is the data's size): avifRWStreamStart()
/// is [`AvifRWStream::start`], and the data is taken back from `raw`
/// after avifRWStreamFinishWrite().
#[derive(Debug, Default)]
pub(crate) struct AvifRWStream {
    pub(crate) raw: Vec<u8>,

    /// Index of the next byte in the raw stream.
    pub(crate) offset: usize,

    /// If 0, byte-aligned functions can be used (avifRWStreamWrite() etc.).
    /// Otherwise, it represents the number of bits already used in the last byte
    /// (located at offset-1).
    pub(crate) num_used_bits_in_partial_byte: usize,
}

const AVIF_STREAM_BUFFER_INCREMENT: usize = 1024 * 1024;

impl AvifRWStream {
    /// Translation of `makeRoom()`.
    fn make_room(&mut self, size: usize) -> AvifResult {
        if size > usize::MAX - self.offset {
            return AvifResult::OutOfMemory;
        }
        let needed_size = self.offset + size;
        let mut new_size = self.raw.len();
        while new_size < needed_size {
            new_size += AVIF_STREAM_BUFFER_INCREMENT;
        }
        avif_rw_data_realloc(&mut self.raw, new_size)
    }

    /// Translation of `avifRWStreamStart()`.
    pub(crate) fn start(raw: Vec<u8>) -> AvifRWStream {
        AvifRWStream {
            raw,
            offset: 0,
            num_used_bits_in_partial_byte: 0,
        }
    }

    /// Translation of `avifRWStreamOffset()`.
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Translation of `avifRWStreamSetOffset()`.
    pub(crate) fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
        if self.offset > self.raw.len() {
            self.offset = self.raw.len();
        }
    }

    /// Translation of `avifRWStreamFinishWrite()`.
    pub(crate) fn finish_write(&mut self) {
        if self.raw.len() != self.offset {
            if self.offset != 0 {
                self.raw.truncate(self.offset);
            } else {
                self.raw = Vec::new();
            }
        }
    }

    /// Translation of `avifRWStreamWrite()`.
    pub(crate) fn write(&mut self, data: &[u8]) -> AvifResult {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let size = data.len();
        if size != 0 {
            avif_checkres!(self.make_room(size));
            self.raw[self.offset..self.offset + size].copy_from_slice(data);
            self.offset += size;
        }
        AvifResult::Ok
    }

    /// Translation of `avifRWStreamWriteChars()`.
    pub(crate) fn write_chars(&mut self, chars: &[u8]) -> AvifResult {
        self.write(chars)
    }

    /// Translation of `avifRWStreamWriteFullBox()` (`version` -1 for a
    /// plain box; `marker` optional).
    pub(crate) fn write_full_box(
        &mut self,
        type_: &[u8],
        content_size: usize,
        version: i32,
        flags: u32,
        marker: Option<&mut AvifBoxMarker>,
    ) -> AvifResult {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        if let Some(marker) = marker {
            *marker = self.offset;
        }
        let mut header_size = std::mem::size_of::<u32>() + 4 /* size of type */;
        if version != -1 {
            header_size += 4;
        }

        avif_checkres!(self.make_room(header_size));
        let o = self.offset;
        self.raw[o..o + header_size].fill(0);
        let no_size = ((header_size + content_size) as u32).to_be_bytes();
        self.raw[o..o + 4].copy_from_slice(&no_size);
        self.raw[o + 4..o + 8].copy_from_slice(&type_[..4]);
        if version != -1 {
            self.raw[o + 8] = version as u8;
            self.raw[o + 9] = ((flags >> 16) & 0xff) as u8;
            self.raw[o + 10] = ((flags >> 8) & 0xff) as u8;
            self.raw[o + 11] = (flags & 0xff) as u8;
        }
        self.offset += header_size;

        AvifResult::Ok
    }

    /// Translation of `avifRWStreamWriteBox()`.
    pub(crate) fn write_box(
        &mut self,
        type_: &[u8],
        content_size: usize,
        marker: Option<&mut AvifBoxMarker>,
    ) -> AvifResult {
        self.write_full_box(type_, content_size, -1, 0, marker)
    }

    /// Translation of `avifRWStreamFinishBox()`.
    pub(crate) fn finish_box(&mut self, marker: AvifBoxMarker) {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let no_size = ((self.offset - marker) as u32).to_be_bytes();
        self.raw[marker..marker + 4].copy_from_slice(&no_size);
    }

    /// Translation of `avifRWStreamWriteU8()`.
    pub(crate) fn write_u8(&mut self, v: u8) -> AvifResult {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        avif_checkres!(self.make_room(1));
        self.raw[self.offset] = v;
        self.offset += 1;
        AvifResult::Ok
    }

    /// Translation of `avifRWStreamWriteU16()`.
    pub(crate) fn write_u16(&mut self, v: u16) -> AvifResult {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let size = std::mem::size_of::<u16>();
        avif_checkres!(self.make_room(size));
        self.raw[self.offset..self.offset + size].copy_from_slice(&v.to_be_bytes());
        self.offset += size;
        AvifResult::Ok
    }

    /// Translation of `avifRWStreamWriteU32()`.
    pub(crate) fn write_u32(&mut self, v: u32) -> AvifResult {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let size = std::mem::size_of::<u32>();
        avif_checkres!(self.make_room(size));
        self.raw[self.offset..self.offset + size].copy_from_slice(&v.to_be_bytes());
        self.offset += size;
        AvifResult::Ok
    }

    /// Translation of `avifRWStreamWriteU64()`.
    pub(crate) fn write_u64(&mut self, v: u64) -> AvifResult {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        let size = std::mem::size_of::<u64>();
        avif_checkres!(self.make_room(size));
        self.raw[self.offset..self.offset + size].copy_from_slice(&v.to_be_bytes());
        self.offset += size;
        AvifResult::Ok
    }

    /// Translation of `avifRWStreamWriteZeros()`.
    pub(crate) fn write_zeros(&mut self, byte_count: usize) -> AvifResult {
        debug_assert!(self.num_used_bits_in_partial_byte == 0); // Byte alignment is required.
        avif_checkres!(self.make_room(byte_count));
        self.raw[self.offset..self.offset + byte_count].fill(0);
        self.offset += byte_count;
        AvifResult::Ok
    }

    /// The following functions can write non-aligned bits. Translation of
    /// `avifRWStreamWriteBits()`.
    pub(crate) fn write_bits(&mut self, v: u32, mut bit_count: usize) -> AvifResult {
        avif_checkerr!(
            bit_count >= 32 || (v >> bit_count) == 0,
            AvifResult::InvalidArgument
        );
        while bit_count != 0 {
            if self.num_used_bits_in_partial_byte == 0 {
                avif_checkres!(self.make_room(1)); // Book a new partial byte in the stream.
                self.raw[self.offset] = 0;
                self.offset += 1;
            }
            debug_assert!(self.offset > 0);
            let packed_bits = self.offset - 1;

            let num_bits = bit_count.min(8 - self.num_used_bits_in_partial_byte);
            self.num_used_bits_in_partial_byte += num_bits;
            bit_count -= num_bits;
            // Order the input bits from the most significant bit to the least significant bit.
            // In the case where avifRWStreamWriteBits() is used to write the unsigned integer value v
            // over multiple aligned bytes, this order corresponds to big endianness.
            let bits = (((v as u64) >> bit_count) & ((1u64 << num_bits) - 1)) as u32;
            // Pack bits starting with the most significant bit of the first output byte.
            // This way, packed bits can be found in the same order in the bit stream.
            self.raw[packed_bits] |= (bits << (8 - self.num_used_bits_in_partial_byte)) as u8;

            if self.num_used_bits_in_partial_byte == 8 {
                // Start a new partial byte the next time a bit is needed.
                self.num_used_bits_in_partial_byte = 0;
            }
        }
        AvifResult::Ok
    }
}
