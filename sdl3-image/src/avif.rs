// Rust translation of IMG_isAVIF() and ReadAVIFHeader() from src/IMG_avif.c
// from SDL_image, and of avifPeekCompatibleFileType() and the functions it
// calls from libavif's src/read.c and src/stream.c.
// IMG_avif.c: Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// libavif: Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! AVIF images: the detector. Upstream asks libavif whether the file type
//! box names a compatible brand; that check (a few dozen lines of libavif)
//! is translated here. The decoder, which needs libavif and a codec, is
//! not translated.

use sdl3::io::{IoStream, IoWhence};

use crate::util::read_ok;

/// Read the file type box at the start of an AVIF file. Translation of
/// `ReadAVIFHeader()` (`None` for false).
fn read_avif_header(src: &mut IoStream<'_>) -> Option<Vec<u8>> {
    let mut magic = [0u8; 16];
    let mut read: u64 = 0;

    if !read_ok(src, &mut magic[..8]) {
        return None;
    }
    read += 8;

    if &magic[4..8] != b"ftyp" {
        return None;
    }

    let mut size = u32::from_be_bytes([magic[0], magic[1], magic[2], magic[3]]) as u64;
    if size == 1 {
        /* 64-bit header size */
        if !read_ok(src, &mut magic[8..16]) {
            return None;
        }
        read += 8;

        size = u64::from_be_bytes([
            magic[8], magic[9], magic[10], magic[11], magic[12], magic[13], magic[14], magic[15],
        ]);
    }

    if size > usize::MAX as u64 {
        return None;
    }
    if size <= read {
        return None;
    }

    /* Read in the header */
    // (read in pieces rather than allocated up front, so that a box size
    // past the end of the stream fails like upstream's short read without
    // a huge allocation first)
    let mut data = magic[..read as usize].to_vec();
    let mut remaining = size - read;
    let mut chunk = [0u8; 4096];
    while remaining > 0 {
        let n = remaining.min(chunk.len() as u64) as usize;
        if !read_ok(src, &mut chunk[..n]) {
            return None;
        }
        data.extend_from_slice(&chunk[..n]);
        remaining -= n as u64;
    }
    Some(data)
}

/// Whether the file type box names an AVIF brand ("avif" or "avis").
/// Translation of libavif's `avifPeekCompatibleFileType()`, with
/// `avifROStreamReadBoxHeaderPartial()` (top level),
/// `avifParseFileTypeBox()`, `avifFileTypeHasBrand()` and
/// `avifFileTypeIsCompatible()` (as built without
/// `AVIF_ENABLE_EXPERIMENTAL_MINI`).
fn avif_peek_compatible_file_type(input: &[u8]) -> bool {
    // avifROStreamReadBoxHeaderPartial(): Section 4.2.2 of ISO/IEC 14496-12.
    let mut offset = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let bytes = input.get(offset..offset.checked_add(n)?)?;
        offset += n;
        Some(bytes)
    };
    let header = (|| {
        let small_size = u32::from_be_bytes(take(4)?.try_into().ok()?); // unsigned int(32) size;
        let box_type: [u8; 4] = take(4)?.try_into().ok()?; // unsigned int(32) type = boxtype;

        let mut size = small_size as u64;
        if size == 1 {
            size = u64::from_be_bytes(take(8)?.try_into().ok()?); // unsigned int(64) largesize;
        }

        if &box_type == b"uuid" {
            take(16)?; // unsigned int(8) usertype[16] = extended_type;
        }
        Some((box_type, size))
    })();
    let Some((box_type, size)) = header else {
        return false;
    };
    let bytes_read = offset as u64;
    if &box_type != b"ftyp" {
        return false;
    }
    if size == 0 {
        // The ftyp box goes on till the end of the file. Either there is no brand requiring anything in the file but a
        // FileTypebox (so not AVIF), or it is invalid.
        return false;
    }
    if size < bytes_read || (size - bytes_read) > usize::MAX as u64 {
        // Header size overflow check failure
        return false;
    }
    let header_size = (size - bytes_read) as usize;
    let Some(contents) = input.get(offset..).filter(|rest| rest.len() >= header_size) else {
        return false;
    };
    let contents = &contents[..header_size];

    // avifParseFileTypeBox()
    if contents.len() < 8 {
        return false;
    }
    let major_brand = &contents[0..4];
    let _minor_version = &contents[4..8];
    let compatible_brands = &contents[8..];
    if compatible_brands.len() % 4 != 0 {
        // Box[ftyp] contains a compatible brands section that isn't divisible by 4
        return false;
    }

    // avifFileTypeHasBrand()
    let has_brand = |brand: &[u8; 4]| {
        major_brand == brand || compatible_brands.chunks_exact(4).any(|b| b == brand)
    };
    // avifFileTypeIsCompatible()
    has_brand(b"avif") || has_brand(b"avis")
}

/* See if an image is contained in a data source */

/// Whether `src` holds an AVIF image (or image sequence); the stream
/// position is unchanged. Translation of `IMG_isAVIF()`.
pub fn is_avif(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_avif = false;
    if let Some(data) = read_avif_header(src) {
        /* This might be AVIF, do more thorough checks */
        is_avif = avif_peek_compatible_file_type(&data);
    }
    let _ = src.seek(start, IoWhence::Set);
    is_avif
}
