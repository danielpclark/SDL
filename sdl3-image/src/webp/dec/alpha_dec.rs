// Rust translation of src/dec/alpha_dec.c and src/dec/alphai_dec.h from
// libwebp (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Alpha-plane decompression: the ALPH chunk of a lossy image, raw or
//! lossless-compressed, with its spatial filter. The alpha plane belongs to
//! the VP8 decoder (`alpha_plane_`, which is the alpha decoder's
//! `output_`). Alpha dithering (`WebPDequantizeLevels()`, a decoding option
//! SDL_image doesn't set) is not translated.

use crate::webp::dec::vp8_dec::{VP8Decoder, VP8Io};
use crate::webp::dec::vp8l_dec::{
    vp8l_decode_alpha_header, vp8l_decode_alpha_image_stream, VP8LDecoder,
};
use crate::webp::decode::{
    ALPHA_HEADER_LEN, ALPHA_LOSSLESS_COMPRESSION, ALPHA_NO_COMPRESSION, ALPHA_PREPROCESSED_LEVELS,
};
use crate::webp::dsp::filters::webp_unfilter;
use crate::webp::dsp::WebpFilterType;
use crate::webp::utils::safe_alloc;

/// Translation of `struct ALPHDecoder`.
pub(crate) struct ALPHDecoder<'a> {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) method: i32,
    pub(crate) filter: WebpFilterType,
    pub(crate) pre_processing: i32,
    pub(crate) vp8l_dec: Option<Box<VP8LDecoder<'a>>>,
    pub(crate) io: VP8Io<'a>,
    /// Although alpha channel requires only 1 byte per pixel, sometimes
    /// VP8LDecoder may need to allocate 4 bytes per pixel internally during
    /// decode.
    pub(crate) use_8b_decode: bool,
    /// last output row (or NULL): an offset in the output
    pub(crate) prev_line: Option<usize>,
}

//------------------------------------------------------------------------------
// ALPHDecoder object.

/// Allocates a new alpha decoder instance. Translation of `ALPHNew()`.
fn alph_new<'a>() -> ALPHDecoder<'a> {
    ALPHDecoder {
        width: 0,
        height: 0,
        method: 0,
        filter: WebpFilterType::None,
        pre_processing: 0,
        vp8l_dec: None,
        io: VP8Io::default(),
        use_8b_decode: false,
        prev_line: None,
    }
}

//------------------------------------------------------------------------------
// Decoding.

/// Initialize alpha decoding by parsing the alpha header and decoding the
/// image header for alpha data stored using lossless compression. Returns
/// false in case of error in alpha header (data too short, invalid
/// compression method or filter, error in lossless header data etc).
/// Translation of `ALPHInit()`.
fn alph_init<'a>(dec: &mut ALPHDecoder<'a>, data: &'a [u8], src_io: &VP8Io<'_>) -> bool {
    dec.width = src_io.width;
    dec.height = src_io.height;
    debug_assert!(dec.width > 0 && dec.height > 0);

    if data.len() <= ALPHA_HEADER_LEN {
        return false;
    }
    let alpha_data = &data[ALPHA_HEADER_LEN..];
    let alpha_data_size = alpha_data.len();

    dec.method = (data[0] & 0x03) as i32;
    let filter = (data[0] >> 2) & 0x03;
    dec.filter = match filter {
        0 => WebpFilterType::None,
        1 => WebpFilterType::Horizontal,
        2 => WebpFilterType::Vertical,
        _ => WebpFilterType::Gradient,
    };
    dec.pre_processing = ((data[0] >> 4) & 0x03) as i32;
    let rsrv = (data[0] >> 6) & 0x03;
    if dec.method < ALPHA_NO_COMPRESSION
        || dec.method > ALPHA_LOSSLESS_COMPRESSION
        || dec.pre_processing > ALPHA_PREPROCESSED_LEVELS
        || rsrv != 0
    {
        return false;
    }

    // Copy the necessary parameters from src_io to io
    let io = &mut dec.io;
    *io = VP8Io::default();
    io.width = src_io.width;
    io.height = src_io.height;

    io.use_cropping = src_io.use_cropping;
    io.crop_left = src_io.crop_left;
    io.crop_right = src_io.crop_right;
    io.crop_top = src_io.crop_top;
    io.crop_bottom = src_io.crop_bottom;
    // No need to copy the scaling parameters.

    if dec.method == ALPHA_NO_COMPRESSION {
        let alpha_decoded_size = dec.width as usize * dec.height as usize;
        alpha_data_size >= alpha_decoded_size
    } else {
        debug_assert!(dec.method == ALPHA_LOSSLESS_COMPRESSION);
        vp8l_decode_alpha_header(dec, alpha_data)
    }
}

/// Decodes, unfilters and dequantizes *at least* 'num_rows' rows of alpha
/// starting from row number 'row'. It assumes that rows up to (row - 1)
/// have already been decoded. Returns false in case of bitstream error.
/// Translation of `ALPHDecode()`.
fn alph_decode(dec: &mut VP8Decoder<'_>, row: i32, num_rows: i32) -> bool {
    let Some(alph_dec) = dec.alph_dec.as_mut() else {
        return false;
    };
    let width = alph_dec.width as usize;
    let height = alph_dec.io.crop_bottom;
    if alph_dec.method == ALPHA_NO_COMPRESSION {
        let mut prev_line = dec.alpha_prev_line;
        let alpha_data = dec.alpha_data.unwrap_or(&[]);
        let mut deltas = ALPHA_HEADER_LEN + row as usize * width;
        let mut dst = row as usize * width;
        debug_assert!(deltas <= alpha_data.len());
        // (the deltas are copied to the plane, then unfiltered in place)
        for _ in 0..num_rows {
            dec.alpha_plane[dst..dst + width].copy_from_slice(&alpha_data[deltas..deltas + width]);
            if alph_dec.filter != WebpFilterType::None {
                webp_unfilter(alph_dec.filter, &mut dec.alpha_plane, prev_line, dst, width);
            }
            prev_line = Some(dst);
            dst += width;
            deltas += width;
        }
        dec.alpha_prev_line = prev_line;
    } else {
        // alph_dec->method_ == ALPHA_LOSSLESS_COMPRESSION
        debug_assert!(alph_dec.vp8l_dec.is_some());
        if !vp8l_decode_alpha_image_stream(alph_dec, row + num_rows, &mut dec.alpha_plane) {
            return false;
        }
    }

    if row + num_rows >= height {
        dec.is_alpha_decoded = true;
    }
    true
}

/// Translation of `AllocateAlphaPlane()`.
fn allocate_alpha_plane(dec: &mut VP8Decoder<'_>, io: &VP8Io<'_>) -> bool {
    let stride = io.width;
    let height = io.crop_bottom;
    let alpha_size = stride as u64 * height as u64;
    debug_assert!(dec.alpha_plane.is_empty());
    let Some(plane) = safe_alloc(alpha_size, 0u8) else {
        return false;
    };
    dec.alpha_plane = plane;
    dec.alpha_prev_line = None;
    true
}

/// Deallocate memory associated to dec->alpha_plane_ decoding.
/// Translation of `WebPDeallocateAlphaMemory()`.
pub(crate) fn webp_deallocate_alpha_memory(dec: &mut VP8Decoder<'_>) {
    dec.alpha_plane = Vec::new();
    dec.alph_dec = None;
}

//------------------------------------------------------------------------------
// Main entry point.

/// Translation of `VP8DecompressAlphaRows()`: the offset of the current
/// decoded row in the alpha plane, or `None` on an error.
pub(crate) fn vp8_decompress_alpha_rows(
    dec: &mut VP8Decoder<'_>,
    io: &VP8Io<'_>,
    row: i32,
    mut num_rows: i32,
) -> Option<usize> {
    let width = io.width;
    let height = io.crop_bottom;

    if row < 0 || num_rows <= 0 || row + num_rows > height {
        return None;
    }

    'error: {
        if !dec.is_alpha_decoded {
            if dec.alph_dec.is_none() {
                // Initialize decoder.
                dec.alph_dec = Some(Box::new(alph_new()));
                if !allocate_alpha_plane(dec, io) {
                    break 'error;
                }
                let alpha_data = dec.alpha_data.unwrap_or(&[]);
                let Some(alph_dec) = dec.alph_dec.as_mut() else {
                    break 'error;
                };
                if !alph_init(alph_dec, alpha_data, io) {
                    break 'error;
                }
                // if we allowed use of alpha dithering, check whether it's needed at all
                if alph_dec.pre_processing != ALPHA_PREPROCESSED_LEVELS {
                    dec.alpha_dithering = 0; // disable dithering
                } else {
                    num_rows = height - row; // decode everything in one pass
                }
            }

            debug_assert!(dec.alph_dec.is_some());
            debug_assert!(row + num_rows <= height);
            if !alph_decode(dec, row, num_rows) {
                break 'error;
            }

            if dec.is_alpha_decoded {
                // finished?
                dec.alph_dec = None;
                // (dec->alpha_dithering_ > 0 needs a decoding option)
            }
        }

        // Return a pointer to the current decoded row.
        return Some(row as usize * width as usize);
    }

    webp_deallocate_alpha_memory(dec);
    None
}
