// Rust translation of src/mux/muxedit.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Set and delete APIs for mux.

use crate::webp::dec::vp8l_dec::vp8l_check_signature;
use crate::webp::decode::{
    WebPMuxAnimBlend, WebPMuxAnimDispose, ALPHA_FLAG, ANIMATION_FLAG, ANIM_CHUNK_SIZE, EXIF_FLAG,
    ICCP_FLAG, MAX_CANVAS_SIZE, MAX_CHUNK_PAYLOAD, MAX_DURATION, MAX_IMAGE_AREA, MAX_LOOP_COUNT,
    MAX_POSITION_OFFSET, RIFF_HEADER_SIZE, TAG_SIZE, VP8X_CHUNK_SIZE, XMP_FLAG,
};
use crate::webp::mux::muxinternal::{
    chunk_assign_data, chunk_get_id_from_tag, chunk_get_index_from_tag, chunk_get_tag_from_fourcc,
    chunk_list_disk_size, chunk_list_emit, chunk_set_head, chunk_set_head_list,
    mux_emit_riff_header, mux_get_chunk_list_from_id, mux_has_alpha, mux_image_count,
    mux_image_disk_size, mux_image_emit, mux_image_get_nth, mux_image_push, mux_validate, tag_of,
    K_CHUNKS,
};
use crate::webp::mux::muxread::{mux_image_finalize, webp_mux_create, webp_mux_num_chunks};
use crate::webp::mux::{
    is_wpi, ChunkIndex, WebPChunk, WebPChunkId, WebPMux, WebPMuxAnimParams, WebPMuxError,
    WebPMuxFrameInfo, WebPMuxImage,
};
use crate::webp::utils::{get_le24, put_le16, put_le24, put_le32};

//------------------------------------------------------------------------------
// Life of a mux object.

/// Translation of `WebPMuxNew()` (`WebPNewInternal()`).
pub(crate) fn webp_mux_new() -> WebPMux {
    WebPMux::default()
}

//------------------------------------------------------------------------------
// Helper method(s).

/// Translation of `MuxSet()`.
fn mux_set(mux: &mut WebPMux, tag: u32, data: &[u8]) -> WebPMuxError {
    let idx = chunk_get_index_from_tag(tag);
    debug_assert!(!is_wpi(K_CHUNKS[idx as usize].id));

    let chunk = chunk_assign_data(data, tag);
    match idx {
        ChunkIndex::Vp8x => chunk_set_head_list(chunk, &mut mux.vp8x),
        ChunkIndex::Iccp => chunk_set_head_list(chunk, &mut mux.iccp),
        ChunkIndex::Anim => chunk_set_head_list(chunk, &mut mux.anim),
        ChunkIndex::Exif => chunk_set_head_list(chunk, &mut mux.exif),
        ChunkIndex::Xmp => chunk_set_head_list(chunk, &mut mux.xmp),
        ChunkIndex::Unknown => chunk_set_head_list(chunk, &mut mux.unknown),
        _ => WebPMuxError::NotFound,
    }
}

/// Create data for frame given image data, offsets and duration.
/// Translation of `CreateFrameData()`.
fn create_frame_data(width: i32, height: i32, info: &WebPMuxFrameInfo) -> Vec<u8> {
    let frame_size = K_CHUNKS[ChunkIndex::Anmf as usize].size as usize;

    debug_assert!(width > 0 && height > 0 && info.duration >= 0);
    // Note: assertion on upper bounds is done in PutLE24().

    let mut frame_bytes = vec![0u8; frame_size];

    put_le24(&mut frame_bytes[0..], info.x_offset / 2);
    put_le24(&mut frame_bytes[3..], info.y_offset / 2);

    put_le24(&mut frame_bytes[6..], width - 1);
    put_le24(&mut frame_bytes[9..], height - 1);
    put_le24(&mut frame_bytes[12..], info.duration);
    frame_bytes[15] = (if info.blend_method == WebPMuxAnimBlend::NoBlend {
        2
    } else {
        0
    }) | (if info.dispose_method == WebPMuxAnimDispose::Background {
        1
    } else {
        0
    });

    frame_bytes
}

/// The image data, the alpha data (if any) and whether it's lossless.
type ImageData = (Vec<u8>, Option<Vec<u8>>, bool);

/// Outputs image data given a bitstream. The bitstream can either be a
/// single-image WebP file or raw VP8/VP8L data.
/// Also outputs 'is_lossless' to be true if the given bitstream is lossless.
/// Translation of `GetImageData()`: the image and the alpha (`None`
/// without).
fn get_image_data(bitstream: &[u8]) -> Result<ImageData, WebPMuxError> {
    let (image, alpha) = if bitstream.len() < TAG_SIZE || &bitstream[..TAG_SIZE] != b"RIFF" {
        // It is NOT webp file data. Return input data as is.
        (bitstream.to_vec(), None)
    } else {
        // It is webp file data. Extract image data from it.
        let Some(mux) = webp_mux_create(bitstream) else {
            return Err(WebPMuxError::BadData);
        };
        let wpi = &mux.images[0];
        let img = wpi.img.as_ref().expect("an image chunk");
        (img.data.clone(), wpi.alpha.as_ref().map(|a| a.data.clone()))
    };
    let is_lossless = vp8l_check_signature(&image);
    Ok((image, alpha, is_lossless))
}

/// Translation of `DeleteChunks()`.
fn delete_chunks(chunk_list: &mut Vec<WebPChunk>, tag: u32) -> WebPMuxError {
    let len = chunk_list.len();
    chunk_list.retain(|chunk| chunk.tag != tag);
    if chunk_list.len() != len {
        WebPMuxError::Ok
    } else {
        WebPMuxError::NotFound
    }
}

/// Translation of `MuxDeleteAllNamedData()`.
fn mux_delete_all_named_data(mux: &mut WebPMux, tag: u32) -> WebPMuxError {
    let id = chunk_get_id_from_tag(tag);
    if is_wpi(id) {
        return WebPMuxError::InvalidArgument;
    }
    delete_chunks(mux_get_chunk_list_from_id(mux, id), tag)
}

//------------------------------------------------------------------------------
// Set API(s).

/// Translation of `WebPMuxSetChunk()`.
pub(crate) fn webp_mux_set_chunk(
    mux: &mut WebPMux,
    fourcc: &[u8; 4],
    chunk_data: &[u8],
) -> WebPMuxError {
    // (empty data has no bytes: NULL upstream)
    if chunk_data.is_empty() || chunk_data.len() > MAX_CHUNK_PAYLOAD as usize {
        return WebPMuxError::InvalidArgument;
    }
    let tag = chunk_get_tag_from_fourcc(fourcc);

    // Delete existing chunk(s) with the same 'fourcc'.
    let err = mux_delete_all_named_data(mux, tag);
    if err != WebPMuxError::Ok && err != WebPMuxError::NotFound {
        return err;
    }

    // Add the given chunk.
    mux_set(mux, tag, chunk_data)
}

/// Creates a chunk from given 'data' and sets it as 1st chunk in 'chunk_list'.
/// Translation of `AddDataToChunkList()`.
fn add_data_to_chunk_list(
    data: &[u8],
    tag: u32,
    chunk_list: &mut Option<WebPChunk>,
) -> WebPMuxError {
    let chunk = chunk_assign_data(data, tag);
    chunk_set_head(chunk, chunk_list)
}

/// Extracts image & alpha data from the given bitstream and then sets wpi.alpha_
/// and wpi.img_ appropriately. Translation of `SetAlphaAndImageChunks()`.
fn set_alpha_and_image_chunks(bitstream: &[u8], wpi: &mut WebPMuxImage) -> WebPMuxError {
    let (image, alpha, is_lossless) = match get_image_data(bitstream) {
        Ok(data) => data,
        Err(err) => return err,
    };
    let image_tag = if is_lossless {
        tag_of(ChunkIndex::Vp8l)
    } else {
        tag_of(ChunkIndex::Vp8)
    };
    if let Some(alpha) = alpha {
        let err = add_data_to_chunk_list(&alpha, tag_of(ChunkIndex::Alpha), &mut wpi.alpha);
        if err != WebPMuxError::Ok {
            return err;
        }
    }
    let err = add_data_to_chunk_list(&image, image_tag, &mut wpi.img);
    if err != WebPMuxError::Ok {
        return err;
    }
    if mux_image_finalize(wpi) {
        WebPMuxError::Ok
    } else {
        WebPMuxError::InvalidArgument
    }
}

/// Translation of `WebPMuxSetImage()`.
pub(crate) fn webp_mux_set_image(mux: &mut WebPMux, bitstream: &[u8]) -> WebPMuxError {
    if bitstream.is_empty() || bitstream.len() > MAX_CHUNK_PAYLOAD as usize {
        return WebPMuxError::InvalidArgument;
    }

    // Only one 'simple image' can be added in mux. So, remove present images.
    mux.images.clear();

    let mut wpi = WebPMuxImage::default();
    let err = set_alpha_and_image_chunks(bitstream, &mut wpi);
    if err != WebPMuxError::Ok {
        return err;
    }

    // Add this WebPMuxImage to mux.
    mux_image_push(wpi, &mut mux.images)
}

/// Translation of `WebPMuxPushFrame()`.
pub(crate) fn webp_mux_push_frame(mux: &mut WebPMux, info: &WebPMuxFrameInfo) -> WebPMuxError {
    if info.id != WebPChunkId::Anmf {
        return WebPMuxError::InvalidArgument;
    }

    if info.bitstream.is_empty() || info.bitstream.len() > MAX_CHUNK_PAYLOAD as usize {
        return WebPMuxError::InvalidArgument;
    }

    if let Some(image) = mux.images.first() {
        let image_id = match &image.header {
            Some(header) => chunk_get_id_from_tag(header.tag),
            None => WebPChunkId::Image,
        };
        if image_id != info.id {
            return WebPMuxError::InvalidArgument; // Conflicting frame types.
        }
    }

    let mut wpi = WebPMuxImage::default();
    let err = set_alpha_and_image_chunks(&info.bitstream, &mut wpi);
    if err != WebPMuxError::Ok {
        return err;
    }
    debug_assert!(wpi.img.is_some()); // As SetAlphaAndImageChunks() was successful.

    {
        let tag = tag_of(ChunkIndex::Anmf);
        let mut tmp = info.clone();
        tmp.x_offset &= !1; // Snap offsets to even.
        tmp.y_offset &= !1;
        if tmp.x_offset < 0
            || tmp.x_offset >= MAX_POSITION_OFFSET as i32
            || tmp.y_offset < 0
            || tmp.y_offset >= MAX_POSITION_OFFSET as i32
            || (tmp.duration < 0 || tmp.duration >= MAX_DURATION as i32)
        {
            return WebPMuxError::InvalidArgument;
        }
        let frame = create_frame_data(wpi.width, wpi.height, &tmp);
        // Add frame chunk (with copy_data = 1).
        let err = add_data_to_chunk_list(&frame, tag, &mut wpi.header);
        if err != WebPMuxError::Ok {
            return err;
        }
    }

    // Add this WebPMuxImage to mux.
    mux_image_push(wpi, &mut mux.images)
}

/// Translation of `WebPMuxSetAnimationParams()`.
pub(crate) fn webp_mux_set_animation_params(
    mux: &mut WebPMux,
    params: &WebPMuxAnimParams,
) -> WebPMuxError {
    let mut data = [0u8; ANIM_CHUNK_SIZE as usize];

    if params.loop_count < 0 || params.loop_count >= MAX_LOOP_COUNT as i32 {
        return WebPMuxError::InvalidArgument;
    }

    // Delete any existing ANIM chunk(s).
    let err = mux_delete_all_named_data(mux, tag_of(ChunkIndex::Anim));
    if err != WebPMuxError::Ok && err != WebPMuxError::NotFound {
        return err;
    }

    // Set the animation parameters.
    put_le32(&mut data[0..], params.bgcolor);
    put_le16(&mut data[4..], params.loop_count);
    mux_set(mux, tag_of(ChunkIndex::Anim), &data)
}

/// Translation of `WebPMuxSetCanvasSize()`.
pub(crate) fn webp_mux_set_canvas_size(mux: &mut WebPMux, width: i32, height: i32) -> WebPMuxError {
    if width < 0 || height < 0 || width as u32 > MAX_CANVAS_SIZE || height as u32 > MAX_CANVAS_SIZE
    {
        return WebPMuxError::InvalidArgument;
    }
    if width as u64 * height as u64 >= MAX_IMAGE_AREA {
        return WebPMuxError::InvalidArgument;
    }
    if width.wrapping_mul(height) == 0 && (width | height) != 0 {
        // one of width / height is zero, but not both -> invalid!
        return WebPMuxError::InvalidArgument;
    }
    // If we already assembled a VP8X chunk, invalidate it.
    let err = mux_delete_all_named_data(mux, tag_of(ChunkIndex::Vp8x));
    if err != WebPMuxError::Ok && err != WebPMuxError::NotFound {
        return err;
    }

    mux.canvas_width = width;
    mux.canvas_height = height;
    WebPMuxError::Ok
}

//------------------------------------------------------------------------------
// Assembly of the WebP RIFF file.

/// Translation of `GetFrameInfo()`: the offsets and duration.
fn get_frame_info(frame_chunk: &WebPChunk) -> Result<(i32, i32, i32), WebPMuxError> {
    let data = &frame_chunk.data;
    let expected_data_size = K_CHUNKS[ChunkIndex::Anmf as usize].size as usize;
    debug_assert!(frame_chunk.tag == tag_of(ChunkIndex::Anmf));
    if data.len() != expected_data_size {
        return Err(WebPMuxError::InvalidArgument);
    }

    let x_offset = 2 * get_le24(&data[0..]);
    let y_offset = 2 * get_le24(&data[3..]);
    let duration = get_le24(&data[12..]);
    Ok((x_offset, y_offset, duration))
}

/// Translation of `GetImageInfo()`: the offsets, duration, width and
/// height.
fn get_image_info(wpi: &WebPMuxImage) -> Result<(i32, i32, i32, i32, i32), WebPMuxError> {
    let frame_chunk = wpi.header.as_ref().expect("a frame chunk");

    // Get offsets and duration from ANMF chunk.
    let (x_offset, y_offset, duration) = get_frame_info(frame_chunk)?;

    // Get width and height from VP8/VP8L chunk.
    Ok((x_offset, y_offset, duration, wpi.width, wpi.height))
}

/// Returns the tightest dimension for the canvas considering the image list.
/// Translation of `GetAdjustedCanvasSize()`.
fn get_adjusted_canvas_size(mux: &WebPMux) -> Result<(i32, i32), WebPMuxError> {
    let wpi = &mux.images[0];
    debug_assert!(wpi.img.is_some());

    if mux.images.len() > 1 {
        let mut max_x = 0;
        let mut max_y = 0;
        // if we have a chain of wpi's, header_ is necessarily set
        debug_assert!(wpi.header.is_some());
        // Aggregate the bounding box for animation frames.
        for wpi in &mux.images {
            let (x_offset, y_offset, _duration, w, h) = get_image_info(wpi)?;
            let max_x_pos = x_offset + w;
            let max_y_pos = y_offset + h;
            debug_assert!(x_offset < MAX_POSITION_OFFSET as i32);
            debug_assert!(y_offset < MAX_POSITION_OFFSET as i32);

            if max_x_pos > max_x {
                max_x = max_x_pos;
            }
            if max_y_pos > max_y {
                max_y = max_y_pos;
            }
        }
        Ok((max_x, max_y))
    } else {
        // For a single image, canvas dimensions are same as image dimensions.
        Ok((wpi.width, wpi.height))
    }
}

// VP8X format:
// Total Size : 10,
// Flags  : 4 bytes,
// Width  : 3 bytes,
// Height : 3 bytes.
/// Translation of `CreateVP8XChunk()`.
fn create_vp8x_chunk(mux: &mut WebPMux) -> WebPMuxError {
    let mut flags: u32 = 0;
    let mut data = [0u8; VP8X_CHUNK_SIZE as usize];

    match mux.images.first().and_then(|images| images.img.as_ref()) {
        Some(img) if !img.data.is_empty() => {}
        _ => return WebPMuxError::InvalidArgument,
    }

    // If VP8X chunk(s) is(are) already present, remove them (and later add new
    // VP8X chunk with updated flags).
    let err = mux_delete_all_named_data(mux, tag_of(ChunkIndex::Vp8x));
    if err != WebPMuxError::Ok && err != WebPMuxError::NotFound {
        return err;
    }

    // Set flags.
    // (a chunk's data_.bytes is NULL when it is empty)
    let has_data = |list: &[WebPChunk]| list.first().is_some_and(|c| !c.data.is_empty());
    if has_data(&mux.iccp) {
        flags |= ICCP_FLAG;
    }
    if has_data(&mux.exif) {
        flags |= EXIF_FLAG;
    }
    if has_data(&mux.xmp) {
        flags |= XMP_FLAG;
    }
    let images = &mux.images[0];
    if let Some(header) = &images.header {
        if header.tag == tag_of(ChunkIndex::Anmf) {
            // This is an image with animation.
            flags |= ANIMATION_FLAG;
        }
    }
    if mux_image_count(&mux.images, WebPChunkId::Alpha) > 0 {
        flags |= ALPHA_FLAG; // Some images have an alpha channel.
    }

    let (mut width, mut height) = match get_adjusted_canvas_size(mux) {
        Ok(size) => size,
        Err(err) => return err,
    };

    if width <= 0 || height <= 0 {
        return WebPMuxError::InvalidArgument;
    }
    if width as u32 > MAX_CANVAS_SIZE || height as u32 > MAX_CANVAS_SIZE {
        return WebPMuxError::InvalidArgument;
    }

    if mux.canvas_width != 0 || mux.canvas_height != 0 {
        if width > mux.canvas_width || height > mux.canvas_height {
            return WebPMuxError::InvalidArgument;
        }
        width = mux.canvas_width;
        height = mux.canvas_height;
    }

    if flags == 0 && mux.unknown.is_empty() {
        // For simple file format, VP8X chunk should not be added.
        return WebPMuxError::Ok;
    }

    if mux_has_alpha(&mux.images) {
        // This means some frames explicitly/implicitly contain alpha.
        // Note: This 'flags' update must NOT be done for a lossless image
        // without a VP8X chunk!
        flags |= ALPHA_FLAG;
    }

    put_le32(&mut data[0..], flags); // VP8X chunk flags.
    put_le24(&mut data[4..], width - 1); // canvas width.
    put_le24(&mut data[7..], height - 1); // canvas height.

    mux_set(mux, tag_of(ChunkIndex::Vp8x), &data)
}

/// Cleans up 'mux' by removing any unnecessary chunks.
/// Translation of `MuxCleanup()`.
fn mux_cleanup(mux: &mut WebPMux) -> WebPMuxError {
    let mut num_frames = 0;
    let mut num_anim_chunks = 0;

    // If we have an image with a single frame, and its rectangle
    // covers the whole canvas, convert it to a non-animated image
    // (to avoid writing ANMF chunk unnecessarily).
    let err = webp_mux_num_chunks(mux, K_CHUNKS[ChunkIndex::Anmf as usize].id, &mut num_frames);
    if err != WebPMuxError::Ok {
        return err;
    }
    if num_frames == 1 {
        let frame = match mux_image_get_nth(&mux.images, 1) {
            Ok(n) => n,
            Err(err) => return err,
        };
        let (canvas_width, canvas_height) = (mux.canvas_width, mux.canvas_height);
        let frame = &mut mux.images[frame];
        if frame.header.is_some()
            && ((canvas_width == 0 && canvas_height == 0)
                || (frame.width == canvas_width && frame.height == canvas_height))
        {
            debug_assert!(frame.header.as_ref().unwrap().tag == tag_of(ChunkIndex::Anmf));
            frame.header = None; // Removes ANMF chunk.
            num_frames = 0;
        }
    }
    // Remove ANIM chunk if this is a non-animated image.
    let err = webp_mux_num_chunks(
        mux,
        K_CHUNKS[ChunkIndex::Anim as usize].id,
        &mut num_anim_chunks,
    );
    if err != WebPMuxError::Ok {
        return err;
    }
    if num_anim_chunks >= 1 && num_frames == 0 {
        let err = mux_delete_all_named_data(mux, tag_of(ChunkIndex::Anim));
        if err != WebPMuxError::Ok {
            return err;
        }
    }
    WebPMuxError::Ok
}

/// Total size of a list of images. Translation of `ImageListDiskSize()`.
fn image_list_disk_size(wpi_list: &[WebPMuxImage]) -> usize {
    wpi_list.iter().map(mux_image_disk_size).sum()
}

/// Write out the given list of images into 'dst'. Translation of
/// `ImageListEmit()`.
fn image_list_emit(wpi_list: &[WebPMuxImage], dst: &mut Vec<u8>) {
    for wpi in wpi_list {
        mux_image_emit(wpi, dst);
    }
}

/// Translation of `WebPMuxAssemble()`: the assembled data (empty on error).
pub(crate) fn webp_mux_assemble(mux: &mut WebPMux, assembled_data: &mut Vec<u8>) -> WebPMuxError {
    // Clean up returned data, in case something goes wrong.
    assembled_data.clear();

    // Finalize mux.
    let err = mux_cleanup(mux);
    if err != WebPMuxError::Ok {
        return err;
    }
    let err = create_vp8x_chunk(mux);
    if err != WebPMuxError::Ok {
        return err;
    }

    // Allocate data.
    let size = chunk_list_disk_size(&mux.vp8x)
        + chunk_list_disk_size(&mux.iccp)
        + chunk_list_disk_size(&mux.anim)
        + image_list_disk_size(&mux.images)
        + chunk_list_disk_size(&mux.exif)
        + chunk_list_disk_size(&mux.xmp)
        + chunk_list_disk_size(&mux.unknown)
        + RIFF_HEADER_SIZE;

    let mut data = Vec::new();
    if data.try_reserve_exact(size).is_err() {
        return WebPMuxError::MemoryError;
    }

    // Emit header & chunks.
    mux_emit_riff_header(&mut data, size);
    chunk_list_emit(&mux.vp8x, &mut data);
    chunk_list_emit(&mux.iccp, &mut data);
    chunk_list_emit(&mux.anim, &mut data);
    image_list_emit(&mux.images, &mut data);
    chunk_list_emit(&mux.exif, &mut data);
    chunk_list_emit(&mux.xmp, &mut data);
    chunk_list_emit(&mux.unknown, &mut data);
    debug_assert!(data.len() == size);

    // Validate mux.
    let err = mux_validate(mux);
    if err != WebPMuxError::Ok {
        data = Vec::new();
    }

    // Finalize data.
    *assembled_data = data;

    err
}
