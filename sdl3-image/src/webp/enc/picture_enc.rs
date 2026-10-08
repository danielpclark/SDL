// Rust translation of src/enc/picture_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), with `WebPEncodingSetError()` of
// src/enc/webp_enc.c.
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebPPicture class basis: the plane allocations and the memory writer.
//! (The simple `WebPEncodeRGB()` and friends are not translated:
//! SDL_image encodes through `WebPEncode()`.)

use crate::webp::encode::{
    WebPEncodingError, WebPMemoryWriter, WebPPicture, WEBP_CSP_ALPHA_BIT, WEBP_YUV420, WEBP_YUV420A,
};
use crate::webp::utils::safe_alloc;

//------------------------------------------------------------------------------
// WebPPicture
//------------------------------------------------------------------------------

/// Should always be called, to initialize the structure. Returns false in
/// case of version mismatch. WebPPictureInit() must have succeeded before
/// using the 'picture' object. Translation of `WebPPictureInitInternal()`
/// (the writer is none: `DummyWriter()`).
pub(crate) fn webp_picture_init(picture: &mut WebPPicture) -> bool {
    *picture = WebPPicture::default();
    webp_encoding_set_error(picture, WebPEncodingError::Ok);
    true
}

//------------------------------------------------------------------------------

/// Assign an error code to a picture. Return false for convenience.
/// Translation of `WebPEncodingSetError()`.
pub(crate) fn webp_encoding_set_error(pic: &WebPPicture, error: WebPEncodingError) -> bool {
    // The oldest error reported takes precedence over the new one.
    if pic.error_code.get() == WebPEncodingError::Ok {
        pic.error_code.set(error);
    }
    false
}

/// Returns true if 'picture' is non-NULL and dimensions/colorspace are
/// within their valid ranges. If returning false, the 'error_code' in
/// 'picture' is updated. Translation of `WebPValidatePicture()`.
pub(crate) fn webp_validate_picture(picture: &WebPPicture) -> bool {
    if picture.width <= 0 || picture.height <= 0 {
        return webp_encoding_set_error(picture, WebPEncodingError::BadDimension);
    }
    if picture.width <= 0
        || picture.width / 4 > i32::MAX / 4
        || picture.height <= 0
        || picture.height / 4 > i32::MAX / 4
    {
        return webp_encoding_set_error(picture, WebPEncodingError::BadDimension);
    }
    if picture.colorspace != WEBP_YUV420 && picture.colorspace != WEBP_YUV420A {
        return webp_encoding_set_error(picture, WebPEncodingError::InvalidConfiguration);
    }
    true
}

/// Translation of `WebPPictureResetBufferARGB()`.
fn webp_picture_reset_buffer_argb(picture: &mut WebPPicture) {
    picture.argb = Vec::new();
    picture.argb_stride = 0;
}

/// Translation of `WebPPictureResetBufferYUVA()`.
fn webp_picture_reset_buffer_yuva(picture: &mut WebPPicture) {
    picture.y = Vec::new();
    picture.u = Vec::new();
    picture.v = Vec::new();
    picture.a = Vec::new();
    picture.y_stride = 0;
    picture.uv_stride = 0;
    picture.a_stride = 0;
}

/// Remove reference to the ARGB/YUVA buffer (doesn't free anything).
/// Translation of `WebPPictureResetBuffers()`.
pub(crate) fn webp_picture_reset_buffers(picture: &mut WebPPicture) {
    webp_picture_reset_buffer_argb(picture);
    webp_picture_reset_buffer_yuva(picture);
}

/// Allocates ARGB buffer of given dimension (previous one is always free'd).
/// Preserves the YUV(A) buffer. Returns false in case of error (invalid
/// param, out-of-memory). Translation of `WebPPictureAllocARGB()`.
pub(crate) fn webp_picture_alloc_argb(picture: &mut WebPPicture) -> bool {
    let width = picture.width;
    let height = picture.height;
    let argb_size = width as u64 * height as u64;

    if !webp_validate_picture(picture) {
        return false;
    }

    webp_picture_reset_buffer_argb(picture);

    // allocate a new buffer.
    // (malloc'd upstream: its pixels are all written before they're read)
    let Some(memory) = safe_alloc(argb_size, 0u32) else {
        return webp_encoding_set_error(picture, WebPEncodingError::OutOfMemory);
    };
    picture.argb = memory;
    picture.argb_stride = width;
    true
}

/// Allocates YUVA buffer according to set width/height (previous one is
/// always free'd). Uses picture->csp to determine whether an alpha buffer
/// is needed. Preserves the ARGB buffer. Returns false in case of error
/// (invalid param, out-of-memory). Translation of
/// `WebPPictureAllocYUVA()`.
pub(crate) fn webp_picture_alloc_yuva(picture: &mut WebPPicture) -> bool {
    let has_alpha = picture.colorspace & WEBP_CSP_ALPHA_BIT != 0;
    let width = picture.width;
    let height = picture.height;
    let y_stride = width;
    let uv_width = ((width as i64 + 1) >> 1) as i32;
    let uv_height = ((height as i64 + 1) >> 1) as i32;
    let uv_stride = uv_width;

    if !webp_validate_picture(picture) {
        return false;
    }

    webp_picture_reset_buffer_yuva(picture);

    // alpha
    let a_width = if has_alpha { width } else { 0 };
    let a_stride = a_width;
    let y_size = y_stride as u64 * height as u64;
    let uv_size = uv_stride as u64 * uv_height as u64;
    let a_size = a_stride as u64 * height as u64;

    // Security and validation checks
    if width <= 0 || height <= 0 || uv_width <= 0 || uv_height <= 0 {
        // luma/alpha param error, u/v param error
        return webp_encoding_set_error(picture, WebPEncodingError::BadDimension);
    }
    // allocate a new buffer.
    let (Some(y), Some(u), Some(v), Some(a)) = (
        safe_alloc(y_size, 0u8),
        safe_alloc(uv_size, 0u8),
        safe_alloc(uv_size, 0u8),
        safe_alloc(a_size, 0u8),
    ) else {
        return webp_encoding_set_error(picture, WebPEncodingError::OutOfMemory);
    };

    // From now on, we're in the clear, we can no longer fail...
    picture.y_stride = y_stride;
    picture.uv_stride = uv_stride;
    picture.a_stride = a_stride;

    // TODO(skal): we could align the y/u/v planes and adjust stride.
    picture.y = y;
    picture.u = u;
    picture.v = v;
    picture.a = a;
    true
}

/// Convenience allocation / deallocation based on picture->width/height:
/// Allocate y/u/v buffers as per colorspace/width/height specification.
/// Note! This function will free the previous buffer if needed.
/// Returns false in case of memory error. Translation of
/// `WebPPictureAlloc()`.
pub(crate) fn webp_picture_alloc(picture: &mut WebPPicture) -> bool {
    webp_picture_free(picture); // erase previous buffer

    if !picture.use_argb {
        webp_picture_alloc_yuva(picture)
    } else {
        webp_picture_alloc_argb(picture)
    }
}

/// Release the memory allocated by WebPPictureAlloc() or
/// WebPPictureImport*(). Note that this function does _not_ free the
/// memory used by the 'picture' object itself. Besides memory (which is
/// reclaimed) all other fields of 'picture' are preserved. Translation of
/// `WebPPictureFree()`.
pub(crate) fn webp_picture_free(picture: &mut WebPPicture) {
    webp_picture_reset_buffers(picture);
}

//------------------------------------------------------------------------------
// WebPMemoryWriter: Write-to-memory

/// The custom writer to be used with WebPMemoryWriter as custom_ptr. Upon
/// completion, writer.mem and writer.size will hold the coded data.
/// Translation of `WebPMemoryWrite()` (and of `DummyWriter()` for a
/// picture without a writer): the writer function of every picture.
pub(crate) fn webp_picture_write(picture: &WebPPicture, data: &[u8]) -> bool {
    if let Some(w) = picture.writer.borrow_mut().as_mut() {
        if w.mem.try_reserve(data.len()).is_err() {
            return false;
        }
        w.mem.extend_from_slice(data);
    }
    true
}

/// The memory writer of a picture, from now on (upstream's
/// `pic->writer = WebPMemoryWrite; pic->custom_ptr = &writer;` after
/// `WebPMemoryWriterInit(&writer)`).
pub(crate) fn webp_picture_set_memory_writer(picture: &mut WebPPicture) {
    *picture.writer.get_mut() = Some(WebPMemoryWriter::default());
}

/// The bytes the memory writer of a picture holds, leaving it empty.
pub(crate) fn webp_picture_take_written(picture: &mut WebPPicture) -> Vec<u8> {
    picture
        .writer
        .get_mut()
        .as_mut()
        .map(|w| std::mem::take(&mut w.mem))
        .unwrap_or_default()
}
