// Rust translation of src/dec/buffer_dec.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Everything about WebPDecBuffer: checking an external RGB(A) output
//! buffer (the internal allocation, the YUV checks, the flip and the
//! buffer copies are for the decoding options and output modes SDL_image's
//! loader doesn't use).

use crate::webp::decode::{webp_is_rgb_mode, VP8StatusCode, WebPDecBuffer, MODE_LAST};

//------------------------------------------------------------------------------
// WebPDecBuffer

/// Number of bytes per pixel for the different color-spaces.
static K_MODE_BPP: [u8; MODE_LAST] = [
    3, 4, 3, 4, 4, 2, 2, //
    4, 4, 4, 2, // pre-multiplied modes
    1, 1,
];

/// strictly speaking, the very last (or first, if flipped) row doesn't
/// require padding. Translation of `MIN_BUFFER_SIZE()`.
fn min_buffer_size(width: u64, height: i32, stride: i32) -> u64 {
    (stride as u64).wrapping_mul((height - 1) as u64).wrapping_add(width)
}

/// Translation of `CheckDecBuffer()` (for the RGB colorspaces).
fn check_dec_buffer(buffer: &WebPDecBuffer<'_>) -> VP8StatusCode {
    let mut ok = true;
    let mode = buffer.colorspace;
    let width = buffer.width;
    let height = buffer.height;
    if !webp_is_rgb_mode(mode) {
        // YUV checks
        ok = false; // (the YUV view is not translated)
    } else {
        // RGB checks
        let buf = &buffer.rgba;
        let stride = buf.stride.wrapping_abs();
        let bpp = K_MODE_BPP[mode as usize] as i32;
        let size = min_buffer_size(width as u64 * bpp as u64, height, stride);
        ok &= size <= buf.rgba.len() as u64;
        ok &= stride >= width * bpp;
    }
    if ok {
        VP8StatusCode::Ok
    } else {
        VP8StatusCode::InvalidParam
    }
}

/// Translation of `AllocateBuffer()` (with external memory: the check).
fn allocate_buffer(buffer: &mut WebPDecBuffer<'_>) -> VP8StatusCode {
    let w = buffer.width;
    let h = buffer.height;

    if w <= 0 || h <= 0 {
        return VP8StatusCode::InvalidParam;
    }

    // (buffer->is_external_memory > 0: no allocation)
    check_dec_buffer(buffer)
}

/// Prepare 'buffer' with the requested initial dimensions width/height.
/// Validate the parameters. Return an error code in case of problem
/// (invalid stride / size / dimension / etc.). Translation of
/// `WebPAllocateDecBuffer()` (without decoding options).
pub(crate) fn webp_allocate_dec_buffer(
    width: i32,
    height: i32,
    buffer: &mut WebPDecBuffer<'_>,
) -> VP8StatusCode {
    if width <= 0 || height <= 0 {
        return VP8StatusCode::InvalidParam;
    }
    buffer.width = width;
    buffer.height = height;

    // Then, allocate buffer for real.
    allocate_buffer(buffer)
}
