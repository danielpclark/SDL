// Rust translation of the avifIO interface of include/avif/avif.h and of
// src/io.c from libavif (https://github.com/AOMediaCodec/libavif, at the
// revision SDL_image's external/libavif pins: libavif 1.1.1 with SDL's
// patches).
// Copyright 2020 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The I/O the decoder reads through. SDL_image supplies its own (over an
//! `SDL_IOStream`, see `ReadAVIFIO()` in IMG_avif.c), so libavif's memory
//! and file readers are not translated. The decoder takes the reader as
//! an argument of the calls that read, instead of keeping a pointer to it
//! (`avifDecoderSetIO()`); `avifIODestroy()` is dropping it.

use super::avif::AvifResult;

/// Translation of `avifIO`.
pub(crate) trait AvifIo {
    /// This function should return a block of memory that *must* remain valid until another read call to
    /// this avifIO struct is made (reusing a read buffer is acceptable/expected).
    ///
    /// * If offset exceeds the size of the content (past EOF), return AVIF_RESULT_IO_ERROR.
    /// * If offset is *exactly* at EOF, provide a 0-byte buffer and return AVIF_RESULT_OK.
    /// * If (offset+size) exceeds the contents' size, it must truncate the range to provide all
    ///   bytes from the offset to EOF.
    /// * If the range is unavailable yet (due to network conditions or any other reason),
    ///   return AVIF_RESULT_WAITING_ON_IO.
    /// * Otherwise, provide the range and return AVIF_RESULT_OK.
    ///
    /// Translation of `avifIOReadFunc` (the error is never
    /// `AVIF_RESULT_OK`).
    fn read(&mut self, read_flags: u32, offset: u64, size: usize) -> Result<&[u8], AvifResult>;

    /// If non-zero, this is a hint to internal structures of the max size offered by the content
    /// this avifIO structure is reading. If it is a static memory source, it should be the size of
    /// the memory buffer; if it is a file, it should be the file's size. If this information cannot
    /// be known (as it is streamed-in), set a reasonable upper boundary here (larger than the file
    /// can possibly be for your environment, but within your environment's memory constraints). This
    /// is used for sanity checks when allocating internal buffers to protect against
    /// malformed/malicious files.
    fn size_hint(&self) -> u64;

    /// If true, *all* memory regions returned from *all* calls to read are guaranteed to be
    /// persistent and exist for the lifetime of the avifIO object. If false, libavif will make
    /// in-memory copies of samples and metadata content, and a memory region returned from read must
    /// only persist until the next call to read.
    ///
    /// (The decoder copies what it reads either way here, which gives the
    /// same results.)
    fn persistent(&self) -> bool;
}
