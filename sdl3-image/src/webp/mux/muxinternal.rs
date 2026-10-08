// Rust translation of src/mux/muxinternal.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Internal objects and utils for mux.

use crate::webp::decode::{
    mkfourcc, ALPHA_FLAG, ANIMATION_FLAG, ANIM_CHUNK_SIZE, ANMF_CHUNK_SIZE, CHUNK_HEADER_SIZE,
    EXIF_FLAG, ICCP_FLAG, MAX_CHUNK_PAYLOAD, RIFF_HEADER_SIZE, VP8X_CHUNK_SIZE, XMP_FLAG,
};
use crate::webp::mux::muxread::{webp_mux_get_features, webp_mux_num_chunks};
use crate::webp::mux::{
    ChunkIndex, ChunkInfo, WebPChunk, WebPChunkId, WebPMux, WebPMuxError, WebPMuxImage, NIL_TAG,
};
use crate::webp::utils::put_le32;

const UNDEFINED_CHUNK_SIZE: u32 = u32::MAX;

/// Translation of `kChunks`.
pub(crate) const K_CHUNKS: [ChunkInfo; ChunkIndex::Nil as usize + 1] = [
    ChunkInfo {
        tag: mkfourcc(b"VP8X"),
        id: WebPChunkId::Vp8x,
        size: VP8X_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: mkfourcc(b"ICCP"),
        id: WebPChunkId::Iccp,
        size: UNDEFINED_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: mkfourcc(b"ANIM"),
        id: WebPChunkId::Anim,
        size: ANIM_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: mkfourcc(b"ANMF"),
        id: WebPChunkId::Anmf,
        size: ANMF_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: mkfourcc(b"ALPH"),
        id: WebPChunkId::Alpha,
        size: UNDEFINED_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: mkfourcc(b"VP8 "),
        id: WebPChunkId::Image,
        size: UNDEFINED_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: mkfourcc(b"VP8L"),
        id: WebPChunkId::Image,
        size: UNDEFINED_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: mkfourcc(b"EXIF"),
        id: WebPChunkId::Exif,
        size: UNDEFINED_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: mkfourcc(b"XMP "),
        id: WebPChunkId::Xmp,
        size: UNDEFINED_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: NIL_TAG,
        id: WebPChunkId::Unknown,
        size: UNDEFINED_CHUNK_SIZE,
    },
    ChunkInfo {
        tag: NIL_TAG,
        id: WebPChunkId::Nil,
        size: UNDEFINED_CHUNK_SIZE,
    },
];

/// The tag of the chunk at `idx` of `kChunks`.
pub(crate) fn tag_of(idx: ChunkIndex) -> u32 {
    K_CHUNKS[idx as usize].tag
}

//------------------------------------------------------------------------------
// Chunk misc methods.

/// Get chunk index from chunk tag. Returns IDX_UNKNOWN if not found.
/// Translation of `ChunkGetIndexFromTag()`.
pub(crate) fn chunk_get_index_from_tag(tag: u32) -> ChunkIndex {
    const INDEXES: [ChunkIndex; 9] = [
        ChunkIndex::Vp8x,
        ChunkIndex::Iccp,
        ChunkIndex::Anim,
        ChunkIndex::Anmf,
        ChunkIndex::Alpha,
        ChunkIndex::Vp8,
        ChunkIndex::Vp8l,
        ChunkIndex::Exif,
        ChunkIndex::Xmp,
    ];
    let mut i = 0;
    while K_CHUNKS[i].tag != NIL_TAG {
        if tag == K_CHUNKS[i].tag {
            return INDEXES[i];
        }
        i += 1;
    }
    ChunkIndex::Unknown
}

/// Get chunk id from chunk tag. Returns WEBP_CHUNK_UNKNOWN if not found.
/// Translation of `ChunkGetIdFromTag()`.
pub(crate) fn chunk_get_id_from_tag(tag: u32) -> WebPChunkId {
    let mut i = 0;
    while K_CHUNKS[i].tag != NIL_TAG {
        if tag == K_CHUNKS[i].tag {
            return K_CHUNKS[i].id;
        }
        i += 1;
    }
    WebPChunkId::Unknown
}

/// Convert a fourcc string to a tag. Translation of
/// `ChunkGetTagFromFourCC()`.
pub(crate) fn chunk_get_tag_from_fourcc(fourcc: &[u8; 4]) -> u32 {
    mkfourcc(fourcc)
}

//------------------------------------------------------------------------------
// Chunk search methods.

/// Search for nth chunk with given 'tag' in the chunk list.
/// nth = 0 means "last of the list". Translation of `ChunkSearchList()`.
pub(crate) fn chunk_search_list(list: &[WebPChunk], nth: u32, tag: u32) -> Option<&WebPChunk> {
    let mut matching = list.iter().filter(|c| c.tag == tag);
    let mut first = matching.next()?;

    let mut iter = nth;
    loop {
        iter = iter.wrapping_sub(1);
        if iter == 0 {
            break;
        }
        match matching.next() {
            Some(next_chunk) => first = next_chunk,
            None => break,
        }
    }
    if (nth > 0) && (iter > 0) {
        None
    } else {
        Some(first)
    }
}

//------------------------------------------------------------------------------
// Chunk writer methods.

/// Fill the chunk with the given data. Translation of
/// `ChunkAssignData()` (the data is always copied).
pub(crate) fn chunk_assign_data(data: &[u8], tag: u32) -> WebPChunk {
    WebPChunk {
        tag,
        data: data.to_vec(),
    }
}

/// Sets 'chunk' as the only element in 'chunk_list' if it is empty.
/// Translation of `ChunkSetHead()`.
pub(crate) fn chunk_set_head(chunk: WebPChunk, chunk_list: &mut Option<WebPChunk>) -> WebPMuxError {
    if chunk_list.is_some() {
        return WebPMuxError::NotFound;
    }
    *chunk_list = Some(chunk);
    WebPMuxError::Ok
}

/// Sets 'chunk' as the only element in the list 'chunk_list' if it is
/// empty. Translation of `ChunkSetHead()` for a list.
pub(crate) fn chunk_set_head_list(
    chunk: WebPChunk,
    chunk_list: &mut Vec<WebPChunk>,
) -> WebPMuxError {
    if !chunk_list.is_empty() {
        return WebPMuxError::NotFound;
    }
    chunk_list.push(chunk);
    WebPMuxError::Ok
}

/// Sets 'chunk' at last position in the 'chunk_list'.
/// Translation of `ChunkAppend()`.
pub(crate) fn chunk_append(chunk: WebPChunk, chunk_list: &mut Vec<WebPChunk>) -> WebPMuxError {
    chunk_list.push(chunk);
    WebPMuxError::Ok
}

//------------------------------------------------------------------------------
// Chunk serialization methods.

/// Returns size of the chunk including chunk header and padding byte (if any).
/// Translation of `SizeWithPadding()`.
pub(crate) fn size_with_padding(chunk_size: usize) -> usize {
    debug_assert!(chunk_size <= MAX_CHUNK_PAYLOAD as usize);
    CHUNK_HEADER_SIZE + ((chunk_size + 1) & !1)
}

/// Size of a chunk including header and padding. Translation of
/// `ChunkDiskSize()`.
pub(crate) fn chunk_disk_size(chunk: &WebPChunk) -> usize {
    size_with_padding(chunk.data.len())
}

/// Translation of `ChunkEmit()`.
fn chunk_emit(chunk: &WebPChunk, dst: &mut Vec<u8>) {
    let chunk_size = chunk.data.len();
    debug_assert!(chunk.tag != NIL_TAG);
    let mut hdr = [0u8; CHUNK_HEADER_SIZE];
    put_le32(&mut hdr[0..], chunk.tag);
    put_le32(&mut hdr[4..], chunk_size as u32);
    debug_assert!(chunk_size == chunk_size as u32 as usize);
    dst.extend_from_slice(&hdr);
    dst.extend_from_slice(&chunk.data);
    if chunk_size & 1 != 0 {
        dst.push(0); // Add padding.
    }
}

/// Write out the given list of chunks into 'dst'. Translation of
/// `ChunkListEmit()`.
pub(crate) fn chunk_list_emit<'c>(
    chunk_list: impl IntoIterator<Item = &'c WebPChunk>,
    dst: &mut Vec<u8>,
) {
    for chunk in chunk_list {
        chunk_emit(chunk, dst);
    }
}

/// Total size of a list of chunks. Translation of `ChunkListDiskSize()`.
pub(crate) fn chunk_list_disk_size<'c>(
    chunk_list: impl IntoIterator<Item = &'c WebPChunk>,
) -> usize {
    chunk_list.into_iter().map(chunk_disk_size).sum()
}

//------------------------------------------------------------------------------
// MuxImage search methods.

/// Get a reference to appropriate chunk list within an image given chunk tag.
/// Translation of `GetChunkListFromId()`.
fn get_chunk_list_from_id(wpi: &WebPMuxImage, id: WebPChunkId) -> &Option<WebPChunk> {
    match id {
        WebPChunkId::Anmf => &wpi.header,
        WebPChunkId::Alpha => &wpi.alpha,
        WebPChunkId::Image => &wpi.img,
        _ => unreachable!("not an image chunk"),
    }
}

/// Count number of images matching the given tag id in the 'wpi_list'.
/// If id == WEBP_CHUNK_NIL, all images will be matched.
/// Translation of `MuxImageCount()`.
pub(crate) fn mux_image_count(wpi_list: &[WebPMuxImage], id: WebPChunkId) -> i32 {
    let mut count = 0;
    for current in wpi_list {
        if id == WebPChunkId::Nil {
            count += 1; // Special case: count all images.
        } else if let Some(wpi_chunk) = get_chunk_list_from_id(current, id) {
            let wpi_chunk_id = chunk_get_id_from_tag(wpi_chunk.tag);
            if wpi_chunk_id == id {
                count += 1; // Count images with a matching 'id'.
            }
        }
    }
    count
}

/// The index of the nth image (the last for 0), if found. Translation
/// of `SearchImageToGetOrDelete()`.
fn search_image_to_get_or_delete(wpi_list: &[WebPMuxImage], mut nth: u32) -> Option<usize> {
    if nth == 0 {
        nth = mux_image_count(wpi_list, WebPChunkId::Nil) as u32;
        if nth == 0 {
            return None; // Not found.
        }
    }
    if (nth as usize) <= wpi_list.len() {
        Some(nth as usize - 1) // Found.
    } else {
        None // Not found.
    }
}

//------------------------------------------------------------------------------
// MuxImage writer methods.

/// Pushes 'wpi' at the end of 'wpi_list'. Translation of `MuxImagePush()`.
pub(crate) fn mux_image_push(wpi: WebPMuxImage, wpi_list: &mut Vec<WebPMuxImage>) -> WebPMuxError {
    wpi_list.push(wpi);
    WebPMuxError::Ok
}

//------------------------------------------------------------------------------
// MuxImage reader methods.

/// Get nth image in the image list. Translation of `MuxImageGetNth()`.
pub(crate) fn mux_image_get_nth(
    wpi_list: &[WebPMuxImage],
    nth: u32,
) -> Result<usize, WebPMuxError> {
    search_image_to_get_or_delete(wpi_list, nth).ok_or(WebPMuxError::NotFound)
}

//------------------------------------------------------------------------------
// MuxImage serialization methods.

/// Size of an image. Translation of `MuxImageDiskSize()`.
pub(crate) fn mux_image_disk_size(wpi: &WebPMuxImage) -> usize {
    let mut size = 0;
    if let Some(header) = &wpi.header {
        size += chunk_disk_size(header);
    }
    if let Some(alpha) = &wpi.alpha {
        size += chunk_disk_size(alpha);
    }
    if let Some(img) = &wpi.img {
        size += chunk_disk_size(img);
    }
    size += chunk_list_disk_size(&wpi.unknown);
    size
}

/// Special case as ANMF chunk encapsulates other image chunks.
/// Translation of `ChunkEmitSpecial()`.
fn chunk_emit_special(header: &WebPChunk, total_size: usize, dst: &mut Vec<u8>) {
    let header_size = header.data.len();
    let offset_to_next = total_size - CHUNK_HEADER_SIZE;
    debug_assert!(header.tag == tag_of(ChunkIndex::Anmf));
    let mut hdr = [0u8; CHUNK_HEADER_SIZE];
    put_le32(&mut hdr[0..], header.tag);
    put_le32(&mut hdr[4..], offset_to_next as u32);
    debug_assert!(header_size == header_size as u32 as usize);
    dst.extend_from_slice(&hdr);
    dst.extend_from_slice(&header.data);
    if header_size & 1 != 0 {
        dst.push(0); // Add padding.
    }
}

/// Write out the given image into 'dst'. Translation of `MuxImageEmit()`.
pub(crate) fn mux_image_emit(wpi: &WebPMuxImage, dst: &mut Vec<u8>) {
    // Ordering of chunks to be emitted is strictly as follows:
    // 1. ANMF chunk (if present).
    // 2. ALPH chunk (if present).
    // 3. VP8/VP8L chunk.
    if let Some(header) = &wpi.header {
        chunk_emit_special(header, mux_image_disk_size(wpi), dst);
    }
    if let Some(alpha) = &wpi.alpha {
        chunk_emit(alpha, dst);
    }
    if let Some(img) = &wpi.img {
        chunk_emit(img, dst);
    }
    chunk_list_emit(&wpi.unknown, dst);
}

//------------------------------------------------------------------------------
// Helper methods for mux.

/// Checks if the given image list contains at least one image with alpha.
/// Translation of `MuxHasAlpha()`.
pub(crate) fn mux_has_alpha(images: &[WebPMuxImage]) -> bool {
    images.iter().any(|image| image.has_alpha)
}

/// Write out RIFF header into 'data', given total data size 'size'.
/// Translation of `MuxEmitRiffHeader()`.
pub(crate) fn mux_emit_riff_header(data: &mut Vec<u8>, size: usize) {
    let mut hdr = [0u8; RIFF_HEADER_SIZE];
    put_le32(&mut hdr[0..], mkfourcc(b"RIFF"));
    put_le32(
        &mut hdr[4..],
        (size as u32).wrapping_sub(CHUNK_HEADER_SIZE as u32),
    );
    debug_assert!(size == size as u32 as usize);
    put_le32(&mut hdr[8..], mkfourcc(b"WEBP"));
    data.extend_from_slice(&hdr);
}

/// Returns the list where chunk with given ID is to be inserted in mux.
/// Translation of `MuxGetChunkListFromId()`.
pub(crate) fn mux_get_chunk_list_from_id(
    mux: &mut WebPMux,
    id: WebPChunkId,
) -> &mut Vec<WebPChunk> {
    match id {
        WebPChunkId::Vp8x => &mut mux.vp8x,
        WebPChunkId::Iccp => &mut mux.iccp,
        WebPChunkId::Anim => &mut mux.anim,
        WebPChunkId::Exif => &mut mux.exif,
        WebPChunkId::Xmp => &mut mux.xmp,
        _ => &mut mux.unknown,
    }
}

/// The list of [`mux_get_chunk_list_from_id`], to read.
pub(crate) fn mux_chunk_list(mux: &WebPMux, id: WebPChunkId) -> &[WebPChunk] {
    match id {
        WebPChunkId::Vp8x => &mux.vp8x,
        WebPChunkId::Iccp => &mux.iccp,
        WebPChunkId::Anim => &mux.anim,
        WebPChunkId::Exif => &mux.exif,
        WebPChunkId::Xmp => &mux.xmp,
        _ => &mux.unknown,
    }
}

/// Translation of `IsNotCompatible()`.
fn is_not_compatible(feature: u32, num_items: i32) -> bool {
    (feature != 0) != (num_items > 0)
}

const NO_FLAG: u32 = 0;

/// Test basic constraints:
/// retrieval, maximum number of chunks by index (use -1 to skip)
/// and feature incompatibility (use NO_FLAG to skip).
/// On success returns WEBP_MUX_OK and stores the chunk count in *num.
/// Translation of `ValidateChunk()`.
fn validate_chunk(
    mux: &WebPMux,
    idx: ChunkIndex,
    feature: u32,
    vp8x_flags: u32,
    max: i32,
    num: &mut i32,
) -> WebPMuxError {
    let err = webp_mux_num_chunks(mux, K_CHUNKS[idx as usize].id, num);
    if err != WebPMuxError::Ok {
        return err;
    }
    if max > -1 && *num > max {
        return WebPMuxError::InvalidArgument;
    }
    if feature != NO_FLAG && is_not_compatible(vp8x_flags & feature, *num) {
        return WebPMuxError::InvalidArgument;
    }
    WebPMuxError::Ok
}

/// Validates the given mux object. Translation of `MuxValidate()`.
pub(crate) fn mux_validate(mux: &WebPMux) -> WebPMuxError {
    let mut num_iccp = 0;
    let mut num_exif = 0;
    let mut num_xmp = 0;
    let mut num_anim = 0;
    let mut num_frames = 0;
    let mut num_vp8x = 0;
    let mut num_images = 0;
    let mut num_alpha = 0;
    let mut flags = 0u32;

    // Verify mux has at least one image.
    if mux.images.is_empty() {
        return WebPMuxError::InvalidArgument;
    }

    let err = webp_mux_get_features(mux, &mut flags);
    if err != WebPMuxError::Ok {
        return err;
    }

    // At most one color profile chunk.
    let err = validate_chunk(mux, ChunkIndex::Iccp, ICCP_FLAG, flags, 1, &mut num_iccp);
    if err != WebPMuxError::Ok {
        return err;
    }

    // At most one EXIF metadata.
    let err = validate_chunk(mux, ChunkIndex::Exif, EXIF_FLAG, flags, 1, &mut num_exif);
    if err != WebPMuxError::Ok {
        return err;
    }

    // At most one XMP metadata.
    let err = validate_chunk(mux, ChunkIndex::Xmp, XMP_FLAG, flags, 1, &mut num_xmp);
    if err != WebPMuxError::Ok {
        return err;
    }

    // Animation: ANIMATION_FLAG, ANIM chunk and ANMF chunk(s) are consistent.
    // At most one ANIM chunk.
    let err = validate_chunk(mux, ChunkIndex::Anim, NO_FLAG, flags, 1, &mut num_anim);
    if err != WebPMuxError::Ok {
        return err;
    }
    let err = validate_chunk(mux, ChunkIndex::Anmf, NO_FLAG, flags, -1, &mut num_frames);
    if err != WebPMuxError::Ok {
        return err;
    }

    {
        let has_animation = (flags & ANIMATION_FLAG) != 0;
        if has_animation && (num_anim == 0 || num_frames == 0) {
            return WebPMuxError::InvalidArgument;
        }
        if !has_animation && (num_anim == 1 || num_frames > 0) {
            return WebPMuxError::InvalidArgument;
        }
        if !has_animation {
            let images = &mux.images;
            // There can be only one image.
            if images.len() != 1 {
                return WebPMuxError::InvalidArgument;
            }
            // Size must match.
            if mux.canvas_width > 0
                && (images[0].width != mux.canvas_width || images[0].height != mux.canvas_height)
            {
                return WebPMuxError::InvalidArgument;
            }
        }
    }

    // Verify either VP8X chunk is present OR there is only one elem in
    // mux->images_.
    let err = validate_chunk(mux, ChunkIndex::Vp8x, NO_FLAG, flags, 1, &mut num_vp8x);
    if err != WebPMuxError::Ok {
        return err;
    }
    let err = validate_chunk(mux, ChunkIndex::Vp8, NO_FLAG, flags, -1, &mut num_images);
    if err != WebPMuxError::Ok {
        return err;
    }
    if num_vp8x == 0 && num_images != 1 {
        return WebPMuxError::InvalidArgument;
    }

    // ALPHA_FLAG & alpha chunk(s) are consistent.
    // Note: ALPHA_FLAG can be set when there is actually no Alpha data present.
    if mux_has_alpha(&mux.images) {
        if num_vp8x > 0 {
            // VP8X chunk is present, so it should contain ALPHA_FLAG.
            if flags & ALPHA_FLAG == 0 {
                return WebPMuxError::InvalidArgument;
            }
        } else {
            // VP8X chunk is not present, so ALPH chunks should NOT be present either.
            let err = webp_mux_num_chunks(mux, WebPChunkId::Alpha, &mut num_alpha);
            if err != WebPMuxError::Ok {
                return err;
            }
            if num_alpha > 0 {
                return WebPMuxError::InvalidArgument;
            }
        }
    }

    WebPMuxError::Ok
}
