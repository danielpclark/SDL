// Rust translation of src/mux/muxread.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Read APIs for mux.

use crate::webp::dec::vp8_dec::vp8_get_info;
use crate::webp::dec::vp8l_dec::vp8l_get_info;
use crate::webp::decode::{
    mkfourcc, WebPMuxAnimBlend, WebPMuxAnimDispose, ALPHA_FLAG, CHUNK_HEADER_SIZE, MAX_CANVAS_SIZE,
    MAX_CHUNK_PAYLOAD, MAX_IMAGE_AREA, RIFF_HEADER_SIZE, TAG_SIZE, VP8X_CHUNK_SIZE,
};
use crate::webp::mux::muxedit::webp_mux_new;
use crate::webp::mux::muxinternal::{
    chunk_append, chunk_assign_data, chunk_disk_size, chunk_get_id_from_tag, chunk_list_emit,
    chunk_search_list, chunk_set_head, mux_chunk_list, mux_emit_riff_header,
    mux_get_chunk_list_from_id, mux_image_count, mux_image_get_nth, mux_image_push, mux_validate,
    size_with_padding, tag_of, K_CHUNKS,
};
use crate::webp::mux::{
    is_wpi, ChunkIndex, WebPChunk, WebPChunkId, WebPMux, WebPMuxAnimParams, WebPMuxError,
    WebPMuxFrameInfo, WebPMuxImage, NIL_TAG,
};
use crate::webp::utils::{get_le16, get_le24, get_le32, put_le24, put_le32};

//------------------------------------------------------------------------------
// Helper method(s).

/// Translation of `MuxGet()`: the nth chunk's data.
fn mux_get(mux: &WebPMux, idx: ChunkIndex, nth: u32) -> Result<&[u8], WebPMuxError> {
    debug_assert!(!is_wpi(K_CHUNKS[idx as usize].id));

    let list = match idx {
        ChunkIndex::Vp8x => &mux.vp8x,
        ChunkIndex::Iccp => &mux.iccp,
        ChunkIndex::Anim => &mux.anim,
        ChunkIndex::Exif => &mux.exif,
        ChunkIndex::Xmp => &mux.xmp,
        _ => {
            debug_assert!(idx != ChunkIndex::Unknown);
            return Err(WebPMuxError::NotFound);
        }
    };
    match chunk_search_list(list, nth, K_CHUNKS[idx as usize].tag) {
        Some(chunk) => Ok(&chunk.data),
        None => Err(WebPMuxError::NotFound),
    }
}

/// Fill the chunk with the given data (includes chunk header bytes), after some
/// verifications. Translation of `ChunkVerifyAndAssign()`.
fn chunk_verify_and_assign(data: &[u8], riff_size: usize) -> Result<WebPChunk, WebPMuxError> {
    // Correctness checks.
    if data.len() < CHUNK_HEADER_SIZE {
        return Err(WebPMuxError::NotEnoughData);
    }
    let chunk_size = get_le32(&data[TAG_SIZE..]);
    if chunk_size > MAX_CHUNK_PAYLOAD {
        return Err(WebPMuxError::BadData);
    }

    {
        let chunk_disk_size = size_with_padding(chunk_size as usize);
        if chunk_disk_size > riff_size {
            return Err(WebPMuxError::BadData);
        }
        if chunk_disk_size > data.len() {
            return Err(WebPMuxError::NotEnoughData);
        }
    }

    // Data assignment.
    let chunk_data = &data[CHUNK_HEADER_SIZE..CHUNK_HEADER_SIZE + chunk_size as usize];
    Ok(chunk_assign_data(chunk_data, get_le32(data)))
}

/// Update width/height/has_alpha info from chunks within wpi.
/// Also remove ALPH chunk if not needed. Translation of
/// `MuxImageFinalize()`.
pub(crate) fn mux_image_finalize(wpi: &mut WebPMuxImage) -> bool {
    let img = wpi.img.as_ref().expect("an image chunk");
    let image = &img.data;
    let is_lossless = img.tag == tag_of(ChunkIndex::Vp8l);
    let info = if is_lossless {
        vp8l_get_info(image)
    } else {
        vp8_get_info(image, image.len()).map(|(w, h)| (w, h, false))
    };
    if let Some((w, h, vp8l_has_alpha)) = info {
        // Ignore ALPH chunk accompanying VP8L.
        if is_lossless && wpi.alpha.is_some() {
            wpi.alpha = None;
        }
        wpi.width = w;
        wpi.height = h;
        wpi.has_alpha = vp8l_has_alpha || wpi.alpha.is_some();
    }
    info.is_some()
}

/// Translation of `MuxImageParse()`.
fn mux_image_parse(chunk: &WebPChunk, wpi: &mut WebPMuxImage) -> bool {
    let bytes = &chunk.data;
    let mut pos = 0;
    let mut size = bytes.len();

    debug_assert!(chunk.tag == tag_of(ChunkIndex::Anmf));
    debug_assert!(!wpi.is_partial);

    // ANMF.
    let hdr_size = K_CHUNKS[ChunkIndex::Anmf as usize].size as usize;
    // Each of ANMF chunk contain a header at the beginning. So, its size should
    // be at least 'hdr_size'.
    if size < hdr_size {
        return false;
    }
    let subchunk = chunk_assign_data(&bytes[..hdr_size], chunk.tag);
    // Rest of the chunks.
    let subchunk_size = chunk_disk_size(&subchunk) - CHUNK_HEADER_SIZE;
    if chunk_set_head(subchunk, &mut wpi.header) != WebPMuxError::Ok {
        return false;
    }
    wpi.is_partial = true; // Waiting for ALPH and/or VP8/VP8L chunks.

    pos += subchunk_size;
    size = size.wrapping_sub(subchunk_size);

    while pos != bytes.len() {
        let Ok(subchunk) = chunk_verify_and_assign(&bytes[pos..pos + size], size) else {
            return false;
        };
        let subchunk_size = chunk_disk_size(&subchunk);
        match chunk_get_id_from_tag(subchunk.tag) {
            WebPChunkId::Alpha => {
                if wpi.alpha.is_some() {
                    return false; // Consecutive ALPH chunks.
                }
                if chunk_set_head(subchunk, &mut wpi.alpha) != WebPMuxError::Ok {
                    return false;
                }
                wpi.is_partial = true; // Waiting for a VP8 chunk.
            }
            WebPChunkId::Image => {
                if wpi.img.is_some() {
                    return false; // Only 1 image chunk allowed.
                }
                if chunk_set_head(subchunk, &mut wpi.img) != WebPMuxError::Ok {
                    return false;
                }
                if !mux_image_finalize(wpi) {
                    return false;
                }
                wpi.is_partial = false; // wpi is completely filled.
            }
            WebPChunkId::Unknown => {
                if wpi.is_partial {
                    return false; // Encountered an unknown chunk
                                  // before some image chunks.
                }
                if chunk_append(subchunk, &mut wpi.unknown) != WebPMuxError::Ok {
                    return false;
                }
            }
            _ => return false,
        }
        pos += subchunk_size;
        size -= subchunk_size;
    }
    !wpi.is_partial
}

//------------------------------------------------------------------------------
// Create a mux object from WebP-RIFF data.

/// Translation of `WebPMuxCreate()` (`WebPMuxCreateInternal()`): `None`
/// for invalid data.
pub(crate) fn webp_mux_create(bitstream: &[u8]) -> Option<WebPMux> {
    let mut data = bitstream;
    let mut size = data.len();

    if size < RIFF_HEADER_SIZE + CHUNK_HEADER_SIZE {
        return None;
    }
    if get_le32(data) != mkfourcc(b"RIFF")
        || get_le32(&data[CHUNK_HEADER_SIZE..]) != mkfourcc(b"WEBP")
    {
        return None;
    }

    let mut mux = webp_mux_new();

    let tag = get_le32(&data[RIFF_HEADER_SIZE..]);
    if tag != tag_of(ChunkIndex::Vp8)
        && tag != tag_of(ChunkIndex::Vp8l)
        && tag != tag_of(ChunkIndex::Vp8x)
    {
        return None; // First chunk should be VP8, VP8L or VP8X.
    }

    let mut riff_size = get_le32(&data[TAG_SIZE..]) as usize;
    if riff_size > MAX_CHUNK_PAYLOAD as usize {
        return None;
    }

    // Note this padding is historical and differs from demux.c which does not
    // pad the file size.
    riff_size = size_with_padding(riff_size);
    if riff_size < CHUNK_HEADER_SIZE {
        return None;
    }
    if riff_size > size {
        return None;
    }
    // There's no point in reading past the end of the RIFF chunk.
    if size > riff_size + CHUNK_HEADER_SIZE {
        size = riff_size + CHUNK_HEADER_SIZE;
    }

    data = &data[RIFF_HEADER_SIZE..size];
    size -= RIFF_HEADER_SIZE;

    let mut wpi = WebPMuxImage::default();

    // Loop over chunks.
    while !data.is_empty() {
        let chunk = chunk_verify_and_assign(&data[..size], riff_size).ok()?;
        let data_size = chunk_disk_size(&chunk);
        let id = chunk_get_id_from_tag(chunk.tag);
        let mut push_image = false;
        match id {
            WebPChunkId::Alpha => {
                if wpi.alpha.is_some() {
                    return None; // Consecutive ALPH chunks.
                }
                if chunk_set_head(chunk, &mut wpi.alpha) != WebPMuxError::Ok {
                    return None;
                }
                wpi.is_partial = true; // Waiting for a VP8 chunk.
            }
            WebPChunkId::Image => {
                if chunk_set_head(chunk, &mut wpi.img) != WebPMuxError::Ok {
                    return None;
                }
                if !mux_image_finalize(&mut wpi) {
                    return None;
                }
                wpi.is_partial = false; // wpi is completely filled.
                push_image = true;
            }
            WebPChunkId::Anmf => {
                if wpi.is_partial {
                    return None; // Previous wpi is still incomplete.
                }
                if !mux_image_parse(&chunk, &mut wpi) {
                    return None;
                }
                push_image = true;
            }
            _ => {
                // A non-image chunk.
                if wpi.is_partial {
                    return None; // Encountered a non-image chunk before
                                 // getting all chunks of an image.
                }
                // (the list to add this chunk to)
                if chunk_append(chunk, mux_get_chunk_list_from_id(&mut mux, id)) != WebPMuxError::Ok
                {
                    return None;
                }
                if id == WebPChunkId::Vp8x {
                    // grab global specs
                    if data_size < CHUNK_HEADER_SIZE + VP8X_CHUNK_SIZE as usize {
                        return None;
                    }
                    mux.canvas_width = get_le24(&data[12..]) + 1;
                    mux.canvas_height = get_le24(&data[15..]) + 1;
                }
            }
        }
        if push_image {
            // PushImage: Add this to mux->images_ list.
            if mux_image_push(std::mem::take(&mut wpi), &mut mux.images) != WebPMuxError::Ok {
                return None;
            }
            // (MuxImageInit(): reset for reading next image)
        }
        data = &data[data_size..];
        size -= data_size;
    }

    // Incomplete image.
    if wpi.is_partial {
        return None;
    }

    // Validate mux if complete.
    if mux_validate(&mux) != WebPMuxError::Ok {
        return None;
    }

    Some(mux) // All OK;
}

//------------------------------------------------------------------------------
// Get API(s).

/// Validates that the given mux has a single image.
/// Translation of `ValidateForSingleImage()`.
fn validate_for_single_image(mux: &WebPMux) -> WebPMuxError {
    let num_images = mux_image_count(&mux.images, WebPChunkId::Image);
    let num_frames = mux_image_count(&mux.images, WebPChunkId::Anmf);

    if num_images == 0 {
        // No images in mux.
        WebPMuxError::NotFound
    } else if num_images == 1 && num_frames == 0 {
        // Valid case (single image).
        WebPMuxError::Ok
    } else {
        // Frame case OR an invalid mux.
        WebPMuxError::InvalidArgument
    }
}

/// Get the canvas width, height and flags after validating that VP8X/VP8/VP8L
/// chunk and canvas size are valid. Translation of `MuxGetCanvasInfo()`.
fn mux_get_canvas_info(mux: &WebPMux) -> Result<(i32, i32, u32), WebPMuxError> {
    let w;
    let h;
    let mut f = 0u32;

    // Check if VP8X chunk is present.
    if let Ok(data) = mux_get(mux, ChunkIndex::Vp8x, 1) {
        if data.len() < VP8X_CHUNK_SIZE as usize {
            return Err(WebPMuxError::BadData);
        }
        f = get_le32(data);
        w = get_le24(&data[4..]) + 1;
        h = get_le24(&data[7..]) + 1;
    } else {
        let wpi = mux.images.first();
        // Grab user-forced canvas size as default.
        let mut ww = mux.canvas_width;
        let mut hh = mux.canvas_height;
        if ww == 0 && hh == 0 && validate_for_single_image(mux) == WebPMuxError::Ok {
            // single image and not forced canvas size => use dimension of first frame
            let wpi = wpi.expect("an image");
            ww = wpi.width;
            hh = wpi.height;
        }
        if let Some(wpi) = wpi {
            if wpi.has_alpha {
                f |= ALPHA_FLAG;
            }
        }
        w = ww;
        h = hh;
    }
    if w as u64 * h as u64 >= MAX_IMAGE_AREA {
        return Err(WebPMuxError::BadData);
    }

    Ok((w, h, f))
}

/// Translation of `WebPMuxGetCanvasSize()`.
pub(crate) fn webp_mux_get_canvas_size(mux: &WebPMux) -> Result<(i32, i32), WebPMuxError> {
    mux_get_canvas_info(mux).map(|(w, h, _)| (w, h))
}

/// Translation of `WebPMuxGetFeatures()`.
pub(crate) fn webp_mux_get_features(mux: &WebPMux, flags: &mut u32) -> WebPMuxError {
    match mux_get_canvas_info(mux) {
        Ok((_, _, f)) => {
            *flags = f;
            WebPMuxError::Ok
        }
        Err(err) => err,
    }
}

/// Translation of `EmitVP8XChunk()`.
fn emit_vp8x_chunk(dst: &mut Vec<u8>, width: i32, height: i32, flags: u32) {
    let vp8x_size = CHUNK_HEADER_SIZE + VP8X_CHUNK_SIZE as usize;
    debug_assert!(width >= 1 && height >= 1);
    debug_assert!(width as u32 <= MAX_CANVAS_SIZE && height as u32 <= MAX_CANVAS_SIZE);
    debug_assert!((width as u64 * height as u64) < MAX_IMAGE_AREA);
    let mut chunk = vec![0u8; vp8x_size];
    put_le32(&mut chunk[0..], mkfourcc(b"VP8X"));
    put_le32(&mut chunk[TAG_SIZE..], VP8X_CHUNK_SIZE);
    put_le32(&mut chunk[CHUNK_HEADER_SIZE..], flags);
    put_le24(&mut chunk[CHUNK_HEADER_SIZE + 4..], width - 1);
    put_le24(&mut chunk[CHUNK_HEADER_SIZE + 7..], height - 1);
    dst.extend_from_slice(&chunk);
}

/// Assemble a single image WebP bitstream from 'wpi'.
/// Translation of `SynthesizeBitstream()`.
fn synthesize_bitstream(wpi: &WebPMuxImage) -> Vec<u8> {
    // Allocate data.
    let need_vp8x = wpi.alpha.is_some();
    let vp8x_size = if need_vp8x {
        CHUNK_HEADER_SIZE + VP8X_CHUNK_SIZE as usize
    } else {
        0
    };
    let alpha_size = wpi.alpha.as_ref().map_or(0, chunk_disk_size);
    let img = wpi.img.as_ref().expect("an image chunk");
    // Note: No need to output ANMF chunk for a single image.
    let size = RIFF_HEADER_SIZE + vp8x_size + alpha_size + chunk_disk_size(img);
    let mut data = Vec::with_capacity(size);

    // Main RIFF header.
    mux_emit_riff_header(&mut data, size);

    if need_vp8x {
        emit_vp8x_chunk(&mut data, wpi.width, wpi.height, ALPHA_FLAG); // VP8X.
        chunk_list_emit(&wpi.alpha, &mut data); // ALPH.
    }

    // Bitstream.
    chunk_list_emit(&wpi.img, &mut data);
    debug_assert!(data.len() == size);

    // Output.
    data
}

/// Translation of `MuxGetImageInternal()`.
fn mux_get_image_internal(wpi: &WebPMuxImage, info: &mut WebPMuxFrameInfo) -> WebPMuxError {
    // Set some defaults for unrelated fields.
    info.x_offset = 0;
    info.y_offset = 0;
    info.duration = 1;
    info.dispose_method = WebPMuxAnimDispose::None;
    info.blend_method = WebPMuxAnimBlend::Blend;
    // Extract data for related fields.
    info.id = chunk_get_id_from_tag(wpi.img.as_ref().expect("an image chunk").tag);
    info.bitstream = synthesize_bitstream(wpi);
    WebPMuxError::Ok
}

/// Translation of `MuxGetFrameInternal()`.
fn mux_get_frame_internal(wpi: &WebPMuxImage, frame: &mut WebPMuxFrameInfo) -> WebPMuxError {
    let header = wpi.header.as_ref().expect("a frame chunk"); // Already checked by WebPMuxGetFrame().
    let is_frame = header.tag == tag_of(ChunkIndex::Anmf);
    if !is_frame {
        return WebPMuxError::InvalidArgument;
    }
    // Get frame chunk.
    let frame_data = &header.data;
    if frame_data.len() < K_CHUNKS[ChunkIndex::Anmf as usize].size as usize {
        return WebPMuxError::BadData;
    }
    // Extract info.
    frame.x_offset = 2 * get_le24(&frame_data[0..]);
    frame.y_offset = 2 * get_le24(&frame_data[3..]);
    {
        let bits = frame_data[15];
        frame.duration = get_le24(&frame_data[12..]);
        frame.dispose_method = if bits & 1 != 0 {
            WebPMuxAnimDispose::Background
        } else {
            WebPMuxAnimDispose::None
        };
        frame.blend_method = if bits & 2 != 0 {
            WebPMuxAnimBlend::NoBlend
        } else {
            WebPMuxAnimBlend::Blend
        };
    }
    frame.id = chunk_get_id_from_tag(header.tag);
    frame.bitstream = synthesize_bitstream(wpi);
    WebPMuxError::Ok
}

/// Translation of `WebPMuxGetFrame()`.
pub(crate) fn webp_mux_get_frame(
    mux: &WebPMux,
    nth: u32,
    frame: &mut WebPMuxFrameInfo,
) -> WebPMuxError {
    // Get the nth WebPMuxImage.
    let wpi = match mux_image_get_nth(&mux.images, nth) {
        Ok(n) => &mux.images[n],
        Err(err) => return err,
    };

    // Get frame info.
    if wpi.header.is_none() {
        mux_get_image_internal(wpi, frame)
    } else {
        mux_get_frame_internal(wpi, frame)
    }
}

/// Translation of `WebPMuxGetAnimationParams()`.
pub(crate) fn webp_mux_get_animation_params(
    mux: &WebPMux,
    params: &mut WebPMuxAnimParams,
) -> WebPMuxError {
    let anim = match mux_get(mux, ChunkIndex::Anim, 1) {
        Ok(anim) => anim,
        Err(err) => return err,
    };
    if anim.len() < K_CHUNKS[WebPChunkId::Anim as usize].size as usize {
        return WebPMuxError::BadData;
    }
    params.bgcolor = get_le32(anim);
    params.loop_count = get_le16(&anim[4..]);

    WebPMuxError::Ok
}

/// Get chunk index from chunk id. Returns IDX_NIL if not found.
/// Translation of `ChunkGetIndexFromId()`.
fn chunk_get_index_from_id(id: WebPChunkId) -> usize {
    let mut i = 0;
    while K_CHUNKS[i].id != WebPChunkId::Nil {
        if id == K_CHUNKS[i].id {
            return i;
        }
        i += 1;
    }
    ChunkIndex::Nil as usize
}

/// Count number of chunks matching 'tag' in the 'chunk_list'.
/// If tag == NIL_TAG, any tag will be matched.
/// Translation of `CountChunks()`.
fn count_chunks(chunk_list: &[WebPChunk], tag: u32) -> i32 {
    chunk_list
        .iter()
        .filter(|c| tag == NIL_TAG || c.tag == tag)
        .count() as i32
}

/// Translation of `WebPMuxNumChunks()`.
pub(crate) fn webp_mux_num_chunks(
    mux: &WebPMux,
    id: WebPChunkId,
    num_elements: &mut i32,
) -> WebPMuxError {
    if is_wpi(id) {
        *num_elements = mux_image_count(&mux.images, id);
    } else {
        let chunk_list = mux_chunk_list(mux, id);
        let idx = chunk_get_index_from_id(id);
        *num_elements = count_chunks(chunk_list, K_CHUNKS[idx].tag);
    }

    WebPMuxError::Ok
}
