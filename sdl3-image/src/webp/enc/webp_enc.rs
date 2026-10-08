// Rust translation of src/enc/webp_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebP encoder: main entry point. (`WebPEncodingSetError()` is in
//! `picture_enc.rs`; the statistics and the progress report are not
//! translated.)

use crate::webp::dec::{B_DC_PRED, NUM_MB_SEGMENTS};
use crate::webp::enc::alpha_enc::{
    vp8_enc_delete_alpha, vp8_enc_finish_alpha, vp8_enc_init_alpha, vp8_enc_start_alpha,
};
use crate::webp::enc::analysis_enc::vp8_enc_analyze;
use crate::webp::enc::config_enc::webp_validate_config;
use crate::webp::enc::frame_enc::vp8_enc_token_loop;
use crate::webp::enc::picture_csp_enc::{
    webp_picture_argb_to_yuva_dithered, webp_picture_yuva_to_argb,
};
use crate::webp::enc::picture_enc::{webp_encoding_set_error, webp_validate_picture};
use crate::webp::enc::picture_tools_enc::{
    webp_cleanup_transparent_area, webp_replace_transparent_pixels,
};
use crate::webp::enc::syntax_enc::{vp8_enc_free_bit_writers, vp8_enc_write};
use crate::webp::enc::token_enc::{vp8_tbuffer_clear, vp8_tbuffer_init};
use crate::webp::enc::tree_enc::vp8_default_probas;
use crate::webp::enc::vp8i_enc::{
    ScoreT, VP8EncFilterHeader, VP8EncSegmentHeader, VP8Encoder, VP8MBInfo, VP8RDLevel,
    VP8SegmentInfo, VP8TBuffer, ERROR_DIFFUSION_QUALITY,
};
use crate::webp::enc::vp8l_enc::vp8l_encode_image;
use crate::webp::encode::{
    WebPConfig, WebPEncodingError, WebPPicture, WEBP_MAX_DIMENSION, WEBP_YUV420,
};

//------------------------------------------------------------------------------
// VP8Encoder
//------------------------------------------------------------------------------

/// Translation of `ResetSegmentHeader()`.
fn reset_segment_header(enc: &mut VP8Encoder<'_>) {
    let hdr = &mut enc.segment_hdr;
    hdr.num_segments = enc.config.segments;
    hdr.update_map = hdr.num_segments > 1;
    hdr.size = 0;
}

/// Translation of `ResetFilterHeader()`.
fn reset_filter_header(enc: &mut VP8Encoder<'_>) {
    let hdr = &mut enc.filter_hdr;
    hdr.simple = true;
    hdr.level = 0;
    hdr.sharpness = 0;
    hdr.i4x4_lf_delta = 0;
}

/// Translation of `ResetBoundaryPredictions()`.
fn reset_boundary_predictions(enc: &mut VP8Encoder<'_>) {
    // init boundary values once for all
    // Note: actually, initializing the preds_[] is only needed for intra4.
    let preds_w = enc.preds_w as usize;
    // top = preds_ - preds_w_ (from top[-1]), left = preds_ - 1
    for i in 0..1 + 4 * enc.mb_w as usize {
        enc.preds[i] = B_DC_PRED as u8;
    }
    for i in 0..4 * enc.mb_h as usize {
        enc.preds[preds_w + i * preds_w] = B_DC_PRED as u8;
    }
    enc.nz[0] = 0; // constant
}

// Mapping from config->method_ to coding tools used.
//-------------------+---+---+---+---+---+---+---+
//   Method          | 0 | 1 | 2 | 3 |(4)| 5 | 6 |
//-------------------+---+---+---+---+---+---+---+
// fast probe        | x |   |   | x |   |   |   |
//-------------------+---+---+---+---+---+---+---+
// dynamic proba     | ~ | x | x | x | x | x | x |
//-------------------+---+---+---+---+---+---+---+
// fast mode analysis|[x]|[x]|   |   | x | x | x |
//-------------------+---+---+---+---+---+---+---+
// basic rd-opt      |   |   |   | x | x | x | x |
//-------------------+---+---+---+---+---+---+---+
// disto-refine i4/16| x | x | x |   |   |   |   |
//-------------------+---+---+---+---+---+---+---+
// disto-refine uv   |   | x | x |   |   |   |   |
//-------------------+---+---+---+---+---+---+---+
// rd-opt i4/16      |   |   | ~ | x | x | x | x |
//-------------------+---+---+---+---+---+---+---+
// token buffer (opt)|   |   |   | x | x | x | x |
//-------------------+---+---+---+---+---+---+---+
// Trellis           |   |   |   |   |   | x |Ful|
//-------------------+---+---+---+---+---+---+---+
// full-SNS          |   |   |   |   | x | x | x |
//-------------------+---+---+---+---+---+---+---+

/// Translation of `MapConfigToTools()`.
fn map_config_to_tools(enc: &mut VP8Encoder<'_>) {
    let config = enc.config;
    let method = config.method;
    let limit = 100 - config.partition_limit;
    enc.method = method;
    enc.rd_opt_level = if method >= 6 {
        VP8RDLevel::TrellisAll
    } else if method >= 5 {
        VP8RDLevel::Trellis
    } else if method >= 3 {
        VP8RDLevel::Basic
    } else {
        VP8RDLevel::None
    };
    enc.max_i4_header_bits = 256 * 16 * 16 *                 // upper bound: up to 16bit per 4x4 block
        (limit * limit)
        / (100 * 100); // ... modulated with a quadratic curve.

    // partition0 = 512k max.
    enc.mb_header_limit = 256 * 510 * 8 * 1024 / (enc.mb_w as ScoreT * enc.mb_h as ScoreT);

    enc.thread_level = config.thread_level;

    enc.do_search = config.target_size > 0 || config.target_psnr > 0.0;
    if config.low_memory == 0 {
        enc.use_tokens = enc.rd_opt_level >= VP8RDLevel::Basic; // need rd stats
        if enc.use_tokens {
            enc.num_parts = 1; // doesn't work with multi-partition
        }
    }
}

/// Translation of `InitVP8Encoder()`.
fn init_vp8_encoder<'a>(config: &'a WebPConfig, picture: &'a WebPPicture) -> VP8Encoder<'a> {
    let use_filter = (config.filter_strength > 0) || (config.autofilter > 0);
    let mb_w = (picture.width + 15) >> 4;
    let mb_h = (picture.height + 15) >> 4;
    let preds_w = 4 * mb_w + 1;
    let preds_h = 4 * mb_h + 1;
    let preds_size = (preds_w * preds_h) as usize;
    let top_stride = (mb_w * 16) as usize;
    let top_derr_size = if config.quality <= ERROR_DIFFUSION_QUALITY || config.pass > 1 {
        mb_w as usize
    } else {
        0
    };
    debug_assert!(config.autofilter == 0, "the autofilter is not translated");

    let mut enc = VP8Encoder {
        config,
        pic: picture,
        filter_hdr: VP8EncFilterHeader::default(),
        segment_hdr: VP8EncSegmentHeader::default(),
        profile: if use_filter {
            if config.filter_type == 1 {
                0
            } else {
                1
            }
        } else {
            2
        },
        mb_w,
        mb_h,
        preds_w,
        num_parts: 1 << config.partitions,
        bw: Default::default(),
        parts: Default::default(),
        tokens: VP8TBuffer::default(),
        has_alpha: false,
        alpha_data: Vec::new(),
        dqm: [VP8SegmentInfo::default(); NUM_MB_SEGMENTS],
        base_quant: 0,
        alpha: 0,
        uv_alpha: 0,
        dq_y1_dc: 0,
        dq_y2_dc: 0,
        dq_y2_ac: 0,
        dq_uv_dc: 0,
        dq_uv_ac: 0,
        proba: Box::default(),
        method: 0,
        rd_opt_level: VP8RDLevel::None,
        max_i4_header_bits: 0,
        mb_header_limit: 0,
        thread_level: 0,
        do_search: false,
        use_tokens: false,
        mb_info: vec![VP8MBInfo::default(); (mb_w * mb_h) as usize],
        preds: vec![0; preds_size],
        nz: vec![0; mb_w as usize + 1],
        y_top: vec![0; top_stride],
        uv_top: vec![0; top_stride],
        top_derr: vec![[[0; 2]; 2]; top_derr_size],
    };

    map_config_to_tools(&mut enc);
    vp8_default_probas(&mut enc);
    reset_segment_header(&mut enc);
    reset_filter_header(&mut enc);
    reset_boundary_predictions(&mut enc);
    vp8_enc_init_alpha(&mut enc);

    // lower quality means smaller output -> we modulate a little the page
    // size based on quality. This is just a crude 1rst-order prediction.
    {
        let scale = 1.0f32 + config.quality * 5.0f32 / 100.0f32; // in [1,6]
        vp8_tbuffer_init(
            &mut enc.tokens,
            (mb_w as f32 * mb_h as f32 * 4.0 * scale) as i32,
        );
    }
    enc
}

/// Translation of `DeleteVP8Encoder()`.
fn delete_vp8_encoder(enc: &mut VP8Encoder<'_>) -> bool {
    let ok = vp8_enc_delete_alpha(enc);
    vp8_tbuffer_clear(&mut enc.tokens);
    ok
}

/// The lossy branch of `WebPEncode()`, from the transparent area cleanup:
/// the VP8 encoder's run.
fn vp8_encode(config: &WebPConfig, pic: &mut WebPPicture) -> bool {
    if config.exact == 0 {
        webp_cleanup_transparent_area(pic);
    }

    let pic: &WebPPicture = pic;
    let mut enc = init_vp8_encoder(config, pic);
    // Note: each of the tasks below account for 20% in the progress report.
    let mut ok = vp8_enc_analyze(&mut enc);

    // Analysis is done, proceed to actual coding.
    ok = ok && vp8_enc_start_alpha(&mut enc); // possibly done in parallel
    debug_assert!(
        enc.use_tokens,
        "the encoding loop without token buffer is not translated"
    );
    ok = ok && vp8_enc_token_loop(&mut enc);
    ok = ok && vp8_enc_finish_alpha(&mut enc);

    ok = ok && vp8_enc_write(&mut enc);
    if !ok {
        vp8_enc_free_bit_writers(&mut enc);
    }
    ok &= delete_vp8_encoder(&mut enc); // must always be called, even if !ok
    ok
}

//------------------------------------------------------------------------------

/// Main encoding call, after config and picture have been initialized.
/// 'picture' must be less than 16384x16384 in dimension (cf
/// WEBP_MAX_DIMENSION), and the 'config' object must be a valid one.
/// Returns false in case of error, true otherwise.
/// In case of error, picture->error_code is updated accordingly.
/// 'picture' can hold the source samples in both YUV(A) or ARGB input,
/// depending on the kind of compression used: lossy (YUV420 and ARGB are
/// converted to YUV420) or lossless (YUV420 is converted to ARGB).
/// Translation of `WebPEncode()`.
pub(crate) fn webp_encode(config: &WebPConfig, pic: &mut WebPPicture) -> bool {
    pic.error_code.set(WebPEncodingError::Ok); // all ok so far
    if !webp_validate_config(config) {
        return webp_encoding_set_error(pic, WebPEncodingError::InvalidConfiguration);
    }
    if !webp_validate_picture(pic) {
        return false;
    }
    if pic.width > WEBP_MAX_DIMENSION || pic.height > WEBP_MAX_DIMENSION {
        return webp_encoding_set_error(pic, WebPEncodingError::BadDimension);
    }

    if config.lossless == 0 {
        if pic.use_argb || pic.y.is_empty() || pic.u.is_empty() || pic.v.is_empty() {
            // Make sure we have YUVA samples.
            debug_assert!(
                config.use_sharp_yuv == 0 && (config.preprocessing & 4) == 0,
                "the sharp conversion is not translated"
            );
            let mut dithering = 0.0f32;
            if config.preprocessing & 2 != 0 {
                let x = config.quality / 100.0f32;
                let x2 = x * x;
                // slowly decreasing from max dithering at low quality (q->0)
                // to 0.5 dithering amplitude at high quality (q->100)
                dithering = 1.0f32 + (0.5f32 - 1.0f32) * x2 * x2;
            }
            if !webp_picture_argb_to_yuva_dithered(pic, WEBP_YUV420, dithering) {
                return false;
            }
        }

        vp8_encode(config, pic)
    } else {
        // Make sure we have ARGB samples.
        if pic.argb.is_empty() && !webp_picture_yuva_to_argb(pic) {
            return false;
        }

        if config.exact == 0 {
            webp_replace_transparent_pixels(pic, 0x000000);
        }

        vp8l_encode_image(config, pic) // Sets pic->error in case of problem.
    }
}
