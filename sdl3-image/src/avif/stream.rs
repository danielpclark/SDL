// Rust translation of src/stream.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The read-only byte stream the box parsers read: big-endian integers,
//! bits, strings and box headers, failing (with a diagnostic) where the
//! data runs out. The writable stream (`avifRWStream`) is for encoding,
//! which is not translated.

use super::diag::{fourcc, printf, AvifDiagnostics};
use super::internal::{avif_check, AvifBoxHeader};

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
