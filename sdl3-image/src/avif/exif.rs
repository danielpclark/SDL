// Rust translation of avifGetExifTiffHeaderOffset() from src/exif.c from
// libavif (https://github.com/AOMediaCodec/libavif, at the revision
// SDL_image's external/libavif pins: libavif 1.1.1 with SDL's patches).
// Copyright 2022 Google LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Exif payload check the decoder makes. The orientation helpers are
//! for avifImageSetMetadataExif() (encoding), which is not translated.

use super::avif::AvifResult;

/// Validates the first bytes of the Exif payload and finds the TIFF header
/// offset (up to UINT32_MAX). Translation of
/// `avifGetExifTiffHeaderOffset()`.
pub(crate) fn avif_get_exif_tiff_header_offset(exif: &[u8], offset: &mut usize) -> AvifResult {
    const TIFF_HEADER_BE: [u8; 4] = [b'M', b'M', 0, 42];
    const TIFF_HEADER_LE: [u8; 4] = [b'I', b'I', 42, 0];
    let exif_size = exif.len().min(u32::MAX as usize);
    *offset = 0;
    while *offset + 4 < exif_size {
        let at = &exif[*offset..*offset + 4];
        if at == TIFF_HEADER_BE || at == TIFF_HEADER_LE {
            return AvifResult::Ok;
        }
        *offset += 1;
    }
    // Couldn't find the TIFF header
    AvifResult::InvalidExifPayload
}
