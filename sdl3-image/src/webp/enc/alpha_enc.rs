// Rust translation of src/enc/alpha_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Alpha-plane compression. The alpha is compressed on the calling thread,
//! as upstream does without `thread_level`; the level quantization
//! (`QuantizeLevels()`, for an `alpha_quality` under 100, which SDL_image
//! doesn't set) and the statistics are not translated.

use crate::webp::decode::{
    ALPHA_HEADER_LEN, ALPHA_LOSSLESS_COMPRESSION, ALPHA_NO_COMPRESSION, ALPHA_PREPROCESSED_LEVELS,
};
use crate::webp::dsp::alpha_processing::webp_dispatch_alpha_to_green;
use crate::webp::dsp::filters::webp_filter;
use crate::webp::dsp::{WebpFilterType, WEBP_FILTER_LAST};
use crate::webp::enc::config_enc::webp_config_init;
use crate::webp::enc::picture_csp_enc::webp_picture_has_transparency;
use crate::webp::enc::picture_enc::{
    webp_encoding_set_error, webp_picture_alloc, webp_picture_free, webp_picture_init,
};
use crate::webp::enc::vp8i_enc::VP8Encoder;
use crate::webp::enc::vp8l_enc::vp8l_encode_stream;
use crate::webp::encode::{WebPConfig, WebPEncodingError, WebPPicture};
use crate::webp::utils::bit_writer_utils::{VP8BitWriter, VP8LBitWriter};
use crate::webp::utils::filters_utils::webp_estimate_best_filter;
use crate::webp::utils::webp_copy_plane;

/// The meta-type trying all the filters. Translation of
/// `WEBP_FILTER_BEST`.
const WEBP_FILTER_BEST: i32 = WEBP_FILTER_LAST + 1;
/// The meta-type estimating the best filter. Translation of
/// `WEBP_FILTER_FAST`.
const WEBP_FILTER_FAST: i32 = WEBP_FILTER_LAST + 2;

/// The filter type numbered `filter`.
fn filter_type(filter: i32) -> WebpFilterType {
    match filter {
        0 => WebpFilterType::None,
        1 => WebpFilterType::Horizontal,
        2 => WebpFilterType::Vertical,
        _ => WebpFilterType::Gradient,
    }
}

// -----------------------------------------------------------------------------
// Encodes the given alpha data via specified compression method 'method'.
// The pre-processing (quantization) is performed if 'quality' is less than 100.
// For such cases, the encoding is lossy. The valid range is [0, 100] for
// 'quality' and [0, 1] for 'method':
//   'method = 0' - No compression;
//   'method = 1' - Use lossless coder on the alpha plane only
// 'filter' values [0, 4] correspond to prediction modes none, horizontal,
// vertical & gradient filters. The prediction mode 4 will try all the
// prediction modes 0 to 3 and pick the best one.
// 'effort_level': specifies how much effort must be spent to try and reduce
//  the compressed output size. In range 0 (quick) to 6 (slow).
//
// 'output' corresponds to the buffer containing compressed alpha data.
//          This buffer is allocated by this method and caller should call
//          WebPSafeFree(*output) when done.
// 'output_size' corresponds to size of this compressed alpha buffer.
//
// Returns 1 on successfully encoding the alpha and
//         0 if either:
//           invalid quality or method, or
//           memory allocation for the compressed data fails.

/// Translation of `EncodeLossless()`.
fn encode_lossless(
    data: &[u8],
    width: i32,
    height: i32,
    effort_level: i32, // in [0..6] range
    use_quality_100: bool,
    bw: &mut VP8LBitWriter,
) -> bool {
    let mut config = WebPConfig::default();
    let mut picture = WebPPicture::default();

    if !webp_picture_init(&mut picture) {
        return false;
    }
    picture.width = width;
    picture.height = height;
    picture.use_argb = true;
    if !webp_picture_alloc(&mut picture) {
        return false;
    }

    // Transfer the alpha values to the green channel.
    let argb_stride = picture.argb_stride as usize;
    webp_dispatch_alpha_to_green(
        data,
        width as usize,
        picture.width as usize,
        picture.height as usize,
        &mut picture.argb,
        argb_stride,
    );

    webp_config_init(&mut config);
    config.lossless = 1;
    // Enable exact, or it would alter RGB values of transparent alpha, which is
    // normally OK but not here since we are not encoding the input image but  an
    // internal encoding-related image containing necessary exact information in
    // RGB channels.
    config.exact = 1;
    config.method = effort_level; // impact is very small
                                  // Set a low default quality for encoding alpha. Ensure that Alpha quality at
                                  // lower methods (3 and below) is less than the threshold for triggering
                                  // costly 'BackwardReferencesTraceBackwards'.
                                  // If the alpha quality is set to 100 and the method to 6, allow for a high
                                  // lossless quality to trigger the cruncher.
    config.quality = if use_quality_100 && effort_level == 6 {
        100.0
    } else {
        8.0 * effort_level as f32
    };
    debug_assert!(config.quality >= 0.0 && config.quality <= 100.0);

    // TODO(urvang): Temporary fix to avoid generating images that trigger
    // a decoder bug related to alpha with color cache.
    // See: https://code.google.com/p/webp/issues/detail?id=239
    // Need to re-enable this later.
    let mut ok = vp8l_encode_stream(&config, &picture, bw, /*use_cache=*/ false);
    webp_picture_free(&mut picture);
    ok = ok && !bw.error;
    if !ok {
        bw.wipe_out();
        return false;
    }
    true
}

// -----------------------------------------------------------------------------

/// Small struct to hold the result of a filter mode compression attempt.
/// Translation of `FilterTrial` (without the statistics).
struct FilterTrial {
    score: usize,
    bw: VP8BitWriter,
}

/// This function always returns an initialized 'bw' object, even upon error.
/// Translation of `EncodeAlphaInternal()`.
#[allow(clippy::too_many_arguments)]
fn encode_alpha_internal(
    data: &[u8],
    width: i32,
    height: i32,
    mut method: i32,
    filter: i32,
    reduce_levels: bool,
    effort_level: i32, // in [0..6] range
    tmp_alpha: &mut [u8],
    result: &mut FilterTrial,
) -> bool {
    let mut ok = false;
    let data_size = (width * height) as usize;
    let mut tmp_bw = VP8LBitWriter::default();

    debug_assert!((0..WEBP_FILTER_LAST).contains(&filter));
    debug_assert!(method >= ALPHA_NO_COMPRESSION);
    debug_assert!(method <= ALPHA_LOSSLESS_COMPRESSION);

    let alpha_src: &[u8] = if filter != WebpFilterType::None as i32 {
        let w = width as usize;
        webp_filter(filter_type(filter), data, w, height as usize, w, tmp_alpha);
        tmp_alpha
    } else {
        data
    };

    let mut output_size = 0;
    if method != ALPHA_NO_COMPRESSION {
        tmp_bw = VP8LBitWriter::new(data_size >> 3);
        ok = encode_lossless(
            alpha_src,
            width,
            height,
            effort_level,
            !reduce_levels,
            &mut tmp_bw,
        );
        if ok {
            tmp_bw.finish();
            if tmp_bw.error {
                tmp_bw.wipe_out();
                result.bw = VP8BitWriter::default();
                return false;
            }
            output_size = tmp_bw.num_bytes();
            if output_size > data_size {
                // compressed size is larger than source! Revert to uncompressed mode.
                method = ALPHA_NO_COMPRESSION;
                tmp_bw.wipe_out();
            }
        } else {
            tmp_bw.wipe_out();
            result.bw = VP8BitWriter::default();
            return false;
        }
    }

    let output: &[u8] = if method == ALPHA_NO_COMPRESSION {
        output_size = data_size;
        ok = true;
        alpha_src
    } else {
        tmp_bw.finish()
    };

    // Emit final result.
    let mut header = (method | (filter << 2)) as u8;
    if reduce_levels {
        header |= (ALPHA_PREPROCESSED_LEVELS << 4) as u8;
    }

    result.bw = VP8BitWriter::new(ALPHA_HEADER_LEN + output_size);
    ok = ok && result.bw.append(&[header]);
    ok = ok && result.bw.append(&output[..output_size]);

    ok = ok && !result.bw.error;
    result.score = result.bw.size();
    ok
}

// -----------------------------------------------------------------------------

/// Translation of `GetNumColors()`.
fn get_num_colors(data: &[u8], width: usize, height: usize, stride: usize) -> i32 {
    let mut color = [0u8; 256];

    for j in 0..height {
        let p = &data[j * stride..j * stride + width];
        for &v in p {
            color[v as usize] = 1;
        }
    }
    color.iter().filter(|&&c| c > 0).count() as i32
}

const FILTER_TRY_NONE: u32 = 1 << WebpFilterType::None as u32;
const FILTER_TRY_ALL: u32 = (1 << WEBP_FILTER_LAST) - 1;

/// Given the input 'filter' option, return an OR'd bit-set of filters to try.
/// Translation of `GetFilterMap()`.
fn get_filter_map(alpha: &[u8], width: i32, height: i32, filter: i32, effort_level: i32) -> u32 {
    let mut bit_map = 0u32;
    if filter == WEBP_FILTER_FAST {
        // Quick estimate of the best candidate.
        let try_filter_none = effort_level > 3;
        const K_MIN_COLORS_FOR_FILTER_NONE: i32 = 16;
        const K_MAX_COLORS_FOR_FILTER_NONE: i32 = 192;
        let num_colors = get_num_colors(alpha, width as usize, height as usize, width as usize);
        // For low number of colors, NONE yields better compression.
        let filter = if num_colors <= K_MIN_COLORS_FOR_FILTER_NONE {
            WebpFilterType::None
        } else {
            webp_estimate_best_filter(alpha, width, height, width)
        };
        bit_map |= 1 << filter as u32;
        // For large number of colors, try FILTER_NONE in addition to the best
        // filter as well.
        if try_filter_none || num_colors > K_MAX_COLORS_FOR_FILTER_NONE {
            bit_map |= FILTER_TRY_NONE;
        }
    } else if filter == WebpFilterType::None as i32 {
        bit_map = FILTER_TRY_NONE;
    } else {
        // WEBP_FILTER_BEST -> try all
        bit_map = FILTER_TRY_ALL;
    }
    bit_map
}

/// Translation of `InitFilterTrial()`.
fn init_filter_trial() -> FilterTrial {
    FilterTrial {
        score: !0u32 as usize,
        bw: VP8BitWriter::new(0),
    }
}

/// Translation of `ApplyFiltersAndEncode()`.
#[allow(clippy::too_many_arguments)]
fn apply_filters_and_encode(
    alpha: &[u8],
    width: i32,
    height: i32,
    data_size: usize,
    method: i32,
    filter: i32,
    reduce_levels: bool,
    effort_level: i32,
    output: &mut Vec<u8>,
) -> bool {
    let mut ok = true;
    let mut try_map = get_filter_map(alpha, width, height, filter, effort_level);
    let mut best = init_filter_trial();

    if try_map != FILTER_TRY_NONE {
        let mut filtered_alpha = vec![0u8; data_size];

        let mut filter = WebpFilterType::None as i32;
        while ok && try_map != 0 {
            if try_map & 1 != 0 {
                let mut trial = FilterTrial {
                    score: 0,
                    bw: VP8BitWriter::default(),
                };
                ok = encode_alpha_internal(
                    alpha,
                    width,
                    height,
                    method,
                    filter,
                    reduce_levels,
                    effort_level,
                    &mut filtered_alpha,
                    &mut trial,
                );
                if ok && trial.score < best.score {
                    best = trial;
                }
            }
            filter += 1;
            try_map >>= 1;
        }
    } else {
        ok = encode_alpha_internal(
            alpha,
            width,
            height,
            method,
            WebpFilterType::None as i32,
            reduce_levels,
            effort_level,
            &mut [],
            &mut best,
        );
    }
    if ok {
        *output = best.bw.buf().to_vec();
    }
    ok
}

/// Translation of `EncodeAlpha()`.
fn encode_alpha(
    enc: &VP8Encoder<'_>,
    quality: i32,
    method: i32,
    mut filter: i32,
    effort_level: i32,
    output: &mut Vec<u8>,
) -> bool {
    let pic = enc.pic;
    let width = pic.width;
    let height = pic.height;

    let data_size = (width * height) as usize;
    let reduce_levels = quality < 100;

    // quick correctness checks
    debug_assert!(!pic.a.is_empty());
    debug_assert!(width > 0 && height > 0);
    debug_assert!(pic.a_stride >= width);
    debug_assert!(filter >= WebpFilterType::None as i32 && filter <= WEBP_FILTER_FAST);

    if !(0..=100).contains(&quality) {
        return webp_encoding_set_error(pic, WebPEncodingError::InvalidConfiguration);
    }

    if !(ALPHA_NO_COMPRESSION..=ALPHA_LOSSLESS_COMPRESSION).contains(&method) {
        return webp_encoding_set_error(pic, WebPEncodingError::InvalidConfiguration);
    }

    if method == ALPHA_NO_COMPRESSION {
        // Don't filter, as filtering will make no impact on compressed size.
        filter = WebpFilterType::None as i32;
    }

    let mut quant_alpha = vec![0u8; data_size];

    // Extract alpha data (width x height) from raw_data (stride x height).
    webp_copy_plane(
        &pic.a,
        0,
        pic.a_stride as usize,
        &mut quant_alpha,
        0,
        width as usize,
        width as usize,
        height as usize,
    );

    // (No Quantization required for 'quality = 100'.)
    debug_assert!(
        !reduce_levels,
        "the alpha level quantization is not translated"
    );

    let ok = apply_filters_and_encode(
        &quant_alpha,
        width,
        height,
        data_size,
        method,
        filter,
        reduce_levels,
        effort_level,
        output,
    );
    if !ok {
        webp_encoding_set_error(pic, WebPEncodingError::OutOfMemory); // imprecise
    }
    ok
}

//------------------------------------------------------------------------------
// Main calls

/// Translation of `CompressAlphaJob()`.
fn compress_alpha_job(enc: &mut VP8Encoder<'_>) -> bool {
    let config = enc.config;
    let mut alpha_data = Vec::new();
    let effort_level = config.method; // maps to [0..6]
    let filter = if config.alpha_filtering == 0 {
        WebpFilterType::None as i32
    } else if config.alpha_filtering == 1 {
        WEBP_FILTER_FAST
    } else {
        WEBP_FILTER_BEST
    };
    if !encode_alpha(
        enc,
        config.alpha_quality,
        config.alpha_compression,
        filter,
        effort_level,
        &mut alpha_data,
    ) {
        return false;
    }
    if alpha_data.len() != alpha_data.len() as u32 as usize {
        // Soundness check.
        return false;
    }
    enc.alpha_data = alpha_data;
    true
}

/// initialize alpha compression. Translation of `VP8EncInitAlpha()`.
pub(crate) fn vp8_enc_init_alpha(enc: &mut VP8Encoder<'_>) {
    enc.has_alpha = webp_picture_has_transparency(enc.pic);
    enc.alpha_data = Vec::new();
}

/// start alpha coding process. Translation of `VP8EncStartAlpha()`.
pub(crate) fn vp8_enc_start_alpha(enc: &mut VP8Encoder<'_>) -> bool {
    if enc.has_alpha {
        return compress_alpha_job(enc); // just do the job right away
    }
    true
}

/// finalize compressed data. Translation of `VP8EncFinishAlpha()`.
pub(crate) fn vp8_enc_finish_alpha(_enc: &mut VP8Encoder<'_>) -> bool {
    true
}

/// delete compressed data. Translation of `VP8EncDeleteAlpha()`.
pub(crate) fn vp8_enc_delete_alpha(enc: &mut VP8Encoder<'_>) -> bool {
    enc.alpha_data = Vec::new();
    enc.has_alpha = false;
    true
}
