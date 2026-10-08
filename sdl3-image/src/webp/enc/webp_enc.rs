// Rust translation of src/enc/webp_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebP encoder: main entry point. (`WebPEncodingSetError()` is in
//! `picture_enc.rs`; the statistics and the progress report are not
//! translated.)

use crate::webp::enc::config_enc::webp_validate_config;
use crate::webp::enc::picture_csp_enc::{
    webp_picture_argb_to_yuva_dithered, webp_picture_yuva_to_argb,
};
use crate::webp::enc::picture_enc::{webp_encoding_set_error, webp_validate_picture};
use crate::webp::enc::picture_tools_enc::webp_replace_transparent_pixels;
use crate::webp::enc::vp8l_enc::vp8l_encode_image;
use crate::webp::encode::{
    WebPConfig, WebPEncodingError, WebPPicture, WEBP_MAX_DIMENSION, WEBP_YUV420,
};

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

        crate::webp::enc::vp8_encode_lossy(config, pic)
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
