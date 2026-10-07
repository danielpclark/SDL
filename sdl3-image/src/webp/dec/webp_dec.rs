// Rust translation of src/dec/webp_dec.c and the header parsing parts of
// src/dec/webpi_dec.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2010 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Main decoding functions for WEBP images: the RIFF container, the
//! bitstream features and the "Into" decoding variants (RGB(A) into the
//! caller's buffer, which is what SDL_image's loader uses). The decoding
//! options (`WebPDecode()` with a `WebPDecoderConfig`), the allocating
//! variants and YUV output are not translated.

use crate::webp::dec::buffer_dec::webp_allocate_dec_buffer;
use crate::webp::dec::io_dec::WebPDecParams;
use crate::webp::dec::vp8_dec::{vp8_decode, vp8_get_headers, vp8_get_info, vp8_new, VP8Io};
use crate::webp::dec::vp8l_dec::{
    vp8l_check_signature, vp8l_decode_header, vp8l_decode_image, vp8l_get_info, vp8l_new,
};
use crate::webp::decode::{
    VP8StatusCode, WebPBitstreamFeatures, WebPDecBuffer, WebPRGBABuffer, WebpCspMode, ALPHA_FLAG,
    ANIMATION_FLAG, CHUNK_HEADER_SIZE, MAX_CHUNK_PAYLOAD, MAX_IMAGE_AREA, RIFF_HEADER_SIZE,
    TAG_SIZE, VP8L_FRAME_HEADER_SIZE, VP8X_CHUNK_SIZE, VP8_FRAME_HEADER_SIZE,
};
use crate::webp::utils::{get_le24, get_le32};

//------------------------------------------------------------------------------
// RIFF layout is:
//   Offset  tag
//   0...3   "RIFF" 4-byte tag
//   4...7   size of image data (including metadata) starting at offset 8
//   8...11  "WEBP"   our form-type signature
// The RIFF container (12 bytes) is followed by appropriate chunks:
//   12..15  "VP8 ": 4-bytes tags, signaling the use of VP8 video format
//   16..19  size of the raw VP8 image data, starting at offset 20
//   20....  the VP8 bytes
// Or,
//   12..15  "VP8L": 4-bytes tags, signaling the use of VP8L lossless format
//   16..19  size of the raw VP8L image data, starting at offset 20
//   20....  the VP8L bytes
// Or,
//   12..15  "VP8X": 4-bytes tags, describing the extended-VP8 chunk.
//   16..19  size of the VP8X chunk starting at offset 20.
//   20..23  VP8X flags bit-map corresponding to the chunk-types present.
//   24..26  Width of the Canvas Image.
//   27..29  Height of the Canvas Image.
// There can be extra chunks after the "VP8X" chunk (ICCP, ANMF, VP8, VP8L,
// XMP, EXIF  ...)
// All sizes are in little-endian order.
// Note: chunk data size must be padded to multiple of 2 when written.

/// Structure storing a description of the RIFF headers. Translation of
/// `WebPHeaderStructure` (`data_size` is `data.len()`).
#[derive(Clone, Copy, Default)]
pub(crate) struct WebPHeaderStructure<'a> {
    /// input buffer
    pub(crate) data: &'a [u8],
    /// true if all data is known to be available
    pub(crate) have_all_data: bool,
    /// offset to main data chunk (VP8 or VP8L)
    pub(crate) offset: usize,
    /// points to alpha chunk (if present)
    pub(crate) alpha_data: Option<&'a [u8]>,
    /// VP8/VP8L compressed data size
    pub(crate) compressed_size: usize,
    /// size of the riff payload (or 0 if absent)
    pub(crate) riff_size: usize,
    /// true if a VP8L chunk is present
    pub(crate) is_lossless: bool,
}

/// Validates the RIFF container (if detected) and skips over it.
/// If a RIFF container is detected, returns:
///     VP8_STATUS_BITSTREAM_ERROR for invalid header,
///     VP8_STATUS_NOT_ENOUGH_DATA for truncated data if have_all_data is true,
/// and VP8_STATUS_OK otherwise.
/// In case there are not enough bytes (partial RIFF container), return 0
/// for *riff_size. Else return the RIFF size extracted from the header.
/// Translation of `ParseRIFF()` (`*data` is `&buf[*pos..]`).
fn parse_riff(
    buf: &[u8],
    pos: &mut usize,
    have_all_data: bool,
    riff_size: &mut usize,
) -> VP8StatusCode {
    let data = &buf[*pos..];
    *riff_size = 0; // Default: no RIFF present.
    if data.len() >= RIFF_HEADER_SIZE && &data[..TAG_SIZE] == b"RIFF" {
        if &data[8..8 + TAG_SIZE] != b"WEBP" {
            return VP8StatusCode::BitstreamError; // Wrong image file signature.
        } else {
            let size = get_le32(&data[TAG_SIZE..]);
            // Check that we have at least one chunk (i.e "WEBP" + "VP8?nnnn").
            if (size as usize) < TAG_SIZE + CHUNK_HEADER_SIZE {
                return VP8StatusCode::BitstreamError;
            }
            if size > MAX_CHUNK_PAYLOAD {
                return VP8StatusCode::BitstreamError;
            }
            if have_all_data && (size as usize > data.len() - CHUNK_HEADER_SIZE) {
                return VP8StatusCode::NotEnoughData; // Truncated bitstream.
            }
            // We have a RIFF container. Skip it.
            *riff_size = size as usize;
            *pos += RIFF_HEADER_SIZE;
        }
    }
    VP8StatusCode::Ok
}

/// What `ParseVP8X()` finds: whether there is a VP8X chunk, and its
/// canvas size and flags.
#[derive(Default)]
struct VP8XInfo {
    found_vp8x: bool,
    width: i32,
    height: i32,
    flags: u32,
}

/// Validates the VP8X header and skips over it.
/// Returns VP8_STATUS_BITSTREAM_ERROR for invalid VP8X header,
///         VP8_STATUS_NOT_ENOUGH_DATA in case of insufficient data, and
///         VP8_STATUS_OK otherwise.
/// If a VP8X chunk is found, found_vp8x is set to true and *width_ptr,
/// *height_ptr and *flags_ptr are set to the corresponding values extracted
/// from the VP8X chunk. Translation of `ParseVP8X()`.
fn parse_vp8x(buf: &[u8], pos: &mut usize, info: &mut VP8XInfo) -> VP8StatusCode {
    let vp8x_size = CHUNK_HEADER_SIZE + VP8X_CHUNK_SIZE as usize;
    let data = &buf[*pos..];

    info.found_vp8x = false;

    if data.len() < CHUNK_HEADER_SIZE {
        return VP8StatusCode::NotEnoughData; // Insufficient data.
    }

    if &data[..TAG_SIZE] == b"VP8X" {
        let chunk_size = get_le32(&data[TAG_SIZE..]);
        if chunk_size != VP8X_CHUNK_SIZE {
            return VP8StatusCode::BitstreamError; // Wrong chunk size.
        }

        // Verify if enough data is available to validate the VP8X chunk.
        if data.len() < vp8x_size {
            return VP8StatusCode::NotEnoughData; // Insufficient data.
        }
        let flags = get_le32(&data[8..]);
        let width = 1 + get_le24(&data[12..]);
        let height = 1 + get_le24(&data[15..]);
        if width as u64 * height as u64 >= MAX_IMAGE_AREA {
            return VP8StatusCode::BitstreamError; // image is too large
        }

        info.flags = flags;
        info.width = width;
        info.height = height;
        // Skip over VP8X header bytes.
        *pos += vp8x_size;
        info.found_vp8x = true;
    }
    VP8StatusCode::Ok
}

/// Skips to the next VP8/VP8L chunk header in the data given the size of
/// the RIFF chunk 'riff_size'.
/// Returns VP8_STATUS_BITSTREAM_ERROR if any invalid chunk size is encountered,
///         VP8_STATUS_NOT_ENOUGH_DATA in case of insufficient data, and
///         VP8_STATUS_OK otherwise.
/// If an alpha chunk is found, *alpha_data and *alpha_size are set
/// appropriately. Translation of `ParseOptionalChunks()`.
fn parse_optional_chunks<'a>(
    buf: &'a [u8],
    pos: &mut usize,
    riff_size: usize,
    alpha_data: &mut Option<&'a [u8]>,
) -> VP8StatusCode {
    let mut b = *pos;
    let mut total_size: u32 = (TAG_SIZE + // "WEBP".
                               CHUNK_HEADER_SIZE) as u32 + // "VP8Xnnnn".
                              VP8X_CHUNK_SIZE; // data.

    *alpha_data = None;

    loop {
        *pos = b;
        let buf_size = buf.len() - b;

        if buf_size < CHUNK_HEADER_SIZE {
            // Insufficient data.
            return VP8StatusCode::NotEnoughData;
        }

        let chunk_size = get_le32(&buf[b + TAG_SIZE..]);
        if chunk_size > MAX_CHUNK_PAYLOAD {
            return VP8StatusCode::BitstreamError; // Not a valid chunk size.
        }
        // For odd-sized chunk-payload, there's one byte padding at the end.
        let disk_chunk_size = (CHUNK_HEADER_SIZE as u32 + chunk_size + 1) & !1u32;
        total_size = total_size.wrapping_add(disk_chunk_size);

        // Check that total bytes skipped so far does not exceed riff_size.
        if riff_size > 0 && (total_size as usize > riff_size) {
            return VP8StatusCode::BitstreamError; // Not a valid chunk size.
        }

        // Start of a (possibly incomplete) VP8/VP8L chunk implies that we have
        // parsed all the optional chunks.
        // Note: This check must occur before the check 'buf_size < disk_chunk_size'
        // below to allow incomplete VP8/VP8L chunks.
        let tag = &buf[b..b + TAG_SIZE];
        if tag == b"VP8 " || tag == b"VP8L" {
            return VP8StatusCode::Ok;
        }

        if buf_size < disk_chunk_size as usize {
            // Insufficient data.
            return VP8StatusCode::NotEnoughData;
        }

        if tag == b"ALPH" {
            // A valid ALPH header.
            let start = b + CHUNK_HEADER_SIZE;
            *alpha_data = Some(&buf[start..start + chunk_size as usize]);
        }

        // We have a full and valid chunk; skip it.
        b += disk_chunk_size as usize;
    }
}

/// Validates the VP8/VP8L Header ("VP8 nnnn" or "VP8L nnnn") and skips over it.
/// Returns VP8_STATUS_BITSTREAM_ERROR for invalid (chunk larger than
///         riff_size) VP8/VP8L header,
///         VP8_STATUS_NOT_ENOUGH_DATA in case of insufficient data, and
///         VP8_STATUS_OK otherwise.
/// If a VP8/VP8L chunk is found, *chunk_size is set to the total number of bytes
/// extracted from the VP8/VP8L chunk header.
/// The flag '*is_lossless' is set to 1 in case of VP8L chunk / raw VP8L
/// data. Translation of `ParseVP8Header()`.
fn parse_vp8_header(
    buf: &[u8],
    pos: &mut usize,
    have_all_data: bool,
    riff_size: usize,
    chunk_size: &mut usize,
    is_lossless: &mut bool,
) -> VP8StatusCode {
    let data = &buf[*pos..];
    let is_vp8 = &data[..TAG_SIZE] == b"VP8 ";
    let is_vp8l = &data[..TAG_SIZE] == b"VP8L";
    let minimal_size = TAG_SIZE + CHUNK_HEADER_SIZE; // "WEBP" + "VP8 nnnn" OR
                                                     // "WEBP" + "VP8Lnnnn"

    if data.len() < CHUNK_HEADER_SIZE {
        return VP8StatusCode::NotEnoughData; // Insufficient data.
    }

    if is_vp8 || is_vp8l {
        // Bitstream contains VP8/VP8L header.
        let size = get_le32(&data[TAG_SIZE..]) as usize;
        if (riff_size >= minimal_size) && (size > riff_size - minimal_size) {
            return VP8StatusCode::BitstreamError; // Inconsistent size information.
        }
        if have_all_data && (size > data.len() - CHUNK_HEADER_SIZE) {
            return VP8StatusCode::NotEnoughData; // Truncated bitstream.
        }
        // Skip over CHUNK_HEADER_SIZE bytes from VP8/VP8L Header.
        *chunk_size = size;
        *pos += CHUNK_HEADER_SIZE;
        *is_lossless = is_vp8l;
    } else {
        // Raw VP8/VP8L bitstream (no header).
        *is_lossless = vp8l_check_signature(data);
        *chunk_size = data.len();
    }

    VP8StatusCode::Ok
}

//------------------------------------------------------------------------------

/// What `ParseHeadersInternal()` fetches: its `int*` outputs.
#[derive(Default)]
struct HeaderFeatures {
    width: i32,
    height: i32,
    has_alpha: bool,
    has_animation: bool,
    format: i32,
}

/// Fetch '*width', '*height', '*has_alpha' and fill out 'headers' based
/// on 'data'. If 'headers' is NULL only the minimal amount will be read to
/// fetch the remaining parameters. If 'headers' is non-NULL this function
/// will attempt to locate both alpha data (with or without a VP8X chunk)
/// and the bitstream chunk (VP8/VP8L).
/// Note: The following chunk sequences (before the raw VP8/VP8L data) are
/// considered valid by this function:
/// RIFF + VP8(L)
/// RIFF + VP8X + (optional chunks) + VP8(L)
/// ALPH + VP8 <-- Not a valid WebP format: only allowed for internal purpose.
/// VP8(L)     <-- Not a valid WebP format: only allowed for internal purpose.
/// Translation of `ParseHeadersInternal()` (the outputs are all fetched,
/// into `out`).
fn parse_headers_internal<'a>(
    data: &'a [u8],
    out: &mut HeaderFeatures,
    headers: Option<&mut WebPHeaderStructure<'a>>,
) -> VP8StatusCode {
    let mut image_width;
    let mut image_height;
    let have_all_data = headers.as_ref().is_some_and(|h| h.have_all_data);
    let want_headers = headers.is_some();
    let mut pos = 0usize;
    let mut status;
    let mut hdrs = WebPHeaderStructure::default();
    let mut vp8x = VP8XInfo::default();

    if data.len() < RIFF_HEADER_SIZE {
        return VP8StatusCode::NotEnoughData;
    }
    hdrs.data = data;

    // Skip over RIFF header.
    status = parse_riff(data, &mut pos, have_all_data, &mut hdrs.riff_size);
    if status != VP8StatusCode::Ok {
        return status; // Wrong RIFF header / insufficient data.
    }
    let found_riff = hdrs.riff_size > 0;

    'return_width_height: {
        // Skip over VP8X.
        {
            status = parse_vp8x(data, &mut pos, &mut vp8x);
            if status != VP8StatusCode::Ok {
                return status; // Wrong VP8X / insufficient data.
            }
            let animation_present = vp8x.flags & ANIMATION_FLAG != 0;
            if !found_riff && vp8x.found_vp8x {
                // Note: This restriction may be removed in the future, if it becomes
                // necessary to send VP8X chunk to the decoder.
                return VP8StatusCode::BitstreamError;
            }
            out.has_alpha = vp8x.flags & ALPHA_FLAG != 0;
            out.has_animation = animation_present;
            out.format = 0; // default = undefined

            image_width = vp8x.width;
            image_height = vp8x.height;
            if vp8x.found_vp8x && animation_present && !want_headers {
                status = VP8StatusCode::Ok;
                break 'return_width_height; // Just return features from VP8X header.
            }
        }

        if data.len() - pos < TAG_SIZE {
            status = VP8StatusCode::NotEnoughData;
            break 'return_width_height;
        }

        // Skip over optional chunks if data started with "RIFF + VP8X" or "ALPH".
        if (found_riff && vp8x.found_vp8x)
            || (!found_riff && !vp8x.found_vp8x && &data[pos..pos + TAG_SIZE] == b"ALPH")
        {
            status = parse_optional_chunks(data, &mut pos, hdrs.riff_size, &mut hdrs.alpha_data);
            if status != VP8StatusCode::Ok {
                break 'return_width_height; // Invalid chunk size / insufficient data.
            }
        }

        // Skip over VP8/VP8L header.
        status = parse_vp8_header(
            data,
            &mut pos,
            have_all_data,
            hdrs.riff_size,
            &mut hdrs.compressed_size,
            &mut hdrs.is_lossless,
        );
        if status != VP8StatusCode::Ok {
            break 'return_width_height; // Wrong VP8/VP8L chunk-header / insufficient data.
        }
        if hdrs.compressed_size > MAX_CHUNK_PAYLOAD as usize {
            return VP8StatusCode::BitstreamError;
        }

        if !out.has_animation {
            out.format = if hdrs.is_lossless { 2 } else { 1 };
        }

        let rest = &data[pos..];
        if !hdrs.is_lossless {
            if rest.len() < VP8_FRAME_HEADER_SIZE {
                status = VP8StatusCode::NotEnoughData;
                break 'return_width_height;
            }
            // Validates raw VP8 data.
            match vp8_get_info(rest, hdrs.compressed_size as u32 as usize) {
                Some((w, h)) => {
                    image_width = w;
                    image_height = h;
                }
                None => return VP8StatusCode::BitstreamError,
            }
        } else {
            if rest.len() < VP8L_FRAME_HEADER_SIZE {
                status = VP8StatusCode::NotEnoughData;
                break 'return_width_height;
            }
            // Validates raw VP8L data.
            match vp8l_get_info(rest) {
                Some((w, h, a)) => {
                    image_width = w;
                    image_height = h;
                    out.has_alpha = a;
                }
                None => return VP8StatusCode::BitstreamError,
            }
        }
        // Validates image size coherency.
        if vp8x.found_vp8x && (vp8x.width != image_width || vp8x.height != image_height) {
            return VP8StatusCode::BitstreamError;
        }
        if let Some(headers) = headers {
            *headers = hdrs;
            headers.offset = pos;
            debug_assert!((pos as u64) < MAX_CHUNK_PAYLOAD as u64);
        }
    }
    // ReturnWidthHeight:
    if status == VP8StatusCode::Ok
        || (status == VP8StatusCode::NotEnoughData && vp8x.found_vp8x && !want_headers)
    {
        // If the data did not contain a VP8X/VP8L chunk the only definitive way
        // to set this is by looking for alpha data (from an ALPH chunk).
        out.has_alpha |= hdrs.alpha_data.is_some();
        out.width = image_width;
        out.height = image_height;
        VP8StatusCode::Ok
    } else {
        status
    }
}

/// Skips over all valid chunks prior to the first VP8/VP8L frame header.
/// Returns: VP8_STATUS_OK, VP8_STATUS_BITSTREAM_ERROR (invalid header/chunk),
/// VP8_STATUS_NOT_ENOUGH_DATA (partial input) or VP8_STATUS_UNSUPPORTED_FEATURE
/// in the case of non-decodable features (animation for instance).
/// In 'headers', compressed_size, offset, alpha_data, alpha_size, and lossless
/// fields are updated appropriately upon success. Translation of
/// `WebPParseHeaders()`.
pub(crate) fn webp_parse_headers(headers: &mut WebPHeaderStructure<'_>) -> VP8StatusCode {
    let mut out = HeaderFeatures::default();
    // fill out headers, ignore width/height/has_alpha.
    let data = headers.data;
    let mut status = parse_headers_internal(data, &mut out, Some(headers));
    if (status == VP8StatusCode::Ok || status == VP8StatusCode::NotEnoughData) && out.has_animation
    {
        // The WebPDemux API + libwebp can be used to decode individual
        // uncomposited frames or the WebPAnimDecoder can be used to fully
        // reconstruct them (see webp/demux.h).
        status = VP8StatusCode::UnsupportedFeature;
    }
    status
}

//------------------------------------------------------------------------------
// "Into" decoding variants

/// Main flow. Translation of `DecodeInto()`.
fn decode_into(data: &[u8], params: &mut WebPDecParams<'_>) -> VP8StatusCode {
    let mut status;
    let mut headers = WebPHeaderStructure {
        data,
        have_all_data: true,
        ..WebPHeaderStructure::default()
    };
    status = webp_parse_headers(&mut headers); // Process Pre-VP8 chunks.
    if status != VP8StatusCode::Ok {
        return status;
    }

    let mut io = VP8Io {
        data: &headers.data[headers.offset..],
        ..VP8Io::default()
    };
    // (WebPInitCustomIo(): the hooks are io_dec.c's, with 'params')

    if !headers.is_lossless {
        let mut dec = vp8_new();
        dec.alpha_data = headers.alpha_data;

        // Decode bitstream header, update io->width/io->height.
        if !vp8_get_headers(&mut dec, &mut io) {
            status = dec.status; // An error occurred. Grab error status.
        } else {
            // Allocate/check output buffers.
            status = webp_allocate_dec_buffer(io.width, io.height, &mut params.output);
            if status == VP8StatusCode::Ok {
                // Decode
                // (VP8GetThreadMethod() and VP8InitDithering() without options:
                // single-threaded, no dithering)
                dec.mt_method = 0;
                if !vp8_decode(&mut dec, &mut io, params) {
                    status = dec.status;
                }
            }
        }
    } else {
        let mut dec = vp8l_new();
        dec.io = io;
        if !vp8l_decode_header(&mut dec) {
            status = dec.status; // An error occurred. Grab error status.
        } else {
            // Allocate/check output buffers.
            status = webp_allocate_dec_buffer(dec.io.width, dec.io.height, &mut params.output);
            if status == VP8StatusCode::Ok {
                // Decode
                if !vp8l_decode_image(&mut dec, params) {
                    status = dec.status;
                }
            }
        }
    }

    // (WebPFreeDecBuffer() of external memory, and the flip option, are
    // no-ops here)
    status
}

/// Helpers. Translation of `DecodeIntoRGBABuffer()`: `None` on failure.
fn decode_into_rgba_buffer(
    colorspace: WebpCspMode,
    data: &[u8],
    rgba: &mut [u8],
    stride: i32,
) -> Option<()> {
    let buf = WebPDecBuffer {
        colorspace,
        width: 0,
        height: 0,
        is_external_memory: 1,
        rgba: WebPRGBABuffer { rgba, stride },
    };
    let mut params = WebPDecParams::new(buf);
    if decode_into(data, &mut params) != VP8StatusCode::Ok {
        return None;
    }
    Some(())
}

/// Decodes WebP images pointed to by 'data' and writes RGB samples into a
/// pre-allocated buffer 'output_buffer' (whose size is its length), with
/// 'output_stride' bytes per row. Returns `None` on failure. Translation of
/// `WebPDecodeRGBInto()`.
pub(crate) fn webp_decode_rgb_into(data: &[u8], output: &mut [u8], stride: i32) -> Option<()> {
    decode_into_rgba_buffer(WebpCspMode::Rgb, data, output, stride)
}

/// The RGBA variant of [`webp_decode_rgb_into`]. Translation of
/// `WebPDecodeRGBAInto()`.
pub(crate) fn webp_decode_rgba_into(data: &[u8], output: &mut [u8], stride: i32) -> Option<()> {
    decode_into_rgba_buffer(WebpCspMode::Rgba, data, output, stride)
}

/// The ARGB variant. Translation of `WebPDecodeARGBInto()`.
#[allow(dead_code)]
pub(crate) fn webp_decode_argb_into(data: &[u8], output: &mut [u8], stride: i32) -> Option<()> {
    decode_into_rgba_buffer(WebpCspMode::Argb, data, output, stride)
}

/// The BGR variant. Translation of `WebPDecodeBGRInto()`.
#[allow(dead_code)]
pub(crate) fn webp_decode_bgr_into(data: &[u8], output: &mut [u8], stride: i32) -> Option<()> {
    decode_into_rgba_buffer(WebpCspMode::Bgr, data, output, stride)
}

/// The BGRA variant. Translation of `WebPDecodeBGRAInto()`.
#[allow(dead_code)]
pub(crate) fn webp_decode_bgra_into(data: &[u8], output: &mut [u8], stride: i32) -> Option<()> {
    decode_into_rgba_buffer(WebpCspMode::Bgra, data, output, stride)
}

//------------------------------------------------------------------------------

/// Translation of `GetFeatures()` (with `DefaultFeatures()`).
fn get_features(data: &[u8], features: &mut WebPBitstreamFeatures) -> VP8StatusCode {
    *features = WebPBitstreamFeatures::default();

    // Only parse enough of the data to retrieve the features.
    let mut out = HeaderFeatures::default();
    let status = parse_headers_internal(data, &mut out, None);
    // (upstream writes the features through pointers as it goes: the flags
    // even when it fails)
    features.width = out.width;
    features.height = out.height;
    features.has_alpha = out.has_alpha;
    features.has_animation = out.has_animation;
    features.format = out.format;
    status
}

//------------------------------------------------------------------------------
// WebPGetInfo()

/// Retrieve basic header information: width, height. Returns `None` in
/// case of formatting error. Translation of `WebPGetInfo()`.
#[allow(dead_code)]
pub(crate) fn webp_get_info(data: &[u8]) -> Option<(i32, i32)> {
    let mut features = WebPBitstreamFeatures::default();

    if get_features(data, &mut features) != VP8StatusCode::Ok {
        return None;
    }

    Some((features.width, features.height))
}

//------------------------------------------------------------------------------
// Advance decoding API

/// Retrieve features from the bitstream. The `features` are filled with
/// information gathered from the bitstream. Returns VP8_STATUS_OK when the
/// features are successfully retrieved. Returns VP8_STATUS_NOT_ENOUGH_DATA
/// when more data is needed to retrieve the features from headers. Returns
/// error in other cases. Translation of `WebPGetFeatures()` (and
/// `WebPGetFeaturesInternal()`).
pub(crate) fn webp_get_features(
    data: &[u8],
    features: &mut WebPBitstreamFeatures,
) -> VP8StatusCode {
    get_features(data, features)
}

//------------------------------------------------------------------------------
// Cropping and rescaling.

/// Setup crop_xxx fields, mb_w and mb_h in io. 'src_colorspace' refers to
/// the *compressed* format, not the output one. Translation of
/// `WebPIoInitFromOptions()` without options (no cropping, no scaling,
/// in-loop filtering and fancy upsampling).
pub(crate) fn webp_io_init_from_options(io: &mut VP8Io<'_>, _src_colorspace: WebpCspMode) -> bool {
    let w = io.width;
    let h = io.height;
    let (x, y) = (0, 0);

    // Cropping
    io.use_cropping = false;
    io.crop_left = x;
    io.crop_top = y;
    io.crop_right = x + w;
    io.crop_bottom = y + h;
    io.mb_w = w;
    io.mb_h = h;

    // Scaling
    io.use_scaling = false;

    // Filter
    io.bypass_filtering = false;

    // Fancy upsampler
    io.fancy_upsampling = true;

    true
}
