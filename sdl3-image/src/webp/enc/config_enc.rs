// Rust translation of src/enc/config_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Coding tools configuration. (`WebPConfigLosslessPreset()` is not
//! translated: SDL_image doesn't call it.)

use crate::webp::encode::{WebPConfig, WebPImageHint, WebPPreset};

//------------------------------------------------------------------------------
// WebPConfig
//------------------------------------------------------------------------------

/// Should always be called, to initialize a fresh WebPConfig structure
/// before modification, with the settings of a preset at a quality.
/// Returns false in case of version mismatch or an invalid configuration
/// (a quality out of [0, 100]). Translation of `WebPConfigInitInternal()`
/// (the ABI version check always passes).
pub(crate) fn webp_config_init_internal(
    config: &mut WebPConfig,
    preset: WebPPreset,
    quality: f32,
) -> bool {
    config.quality = quality;
    config.target_size = 0;
    config.target_psnr = 0.0;
    config.method = 4;
    config.sns_strength = 50;
    config.filter_strength = 60; // mid-filtering
    config.filter_sharpness = 0;
    config.filter_type = 1; // default: strong (so U/V is filtered too)
    config.partitions = 0;
    config.segments = 4;
    config.pass = 1;
    config.qmin = 0;
    config.qmax = 100;
    config.show_compressed = 0;
    config.preprocessing = 0;
    config.autofilter = 0;
    config.partition_limit = 0;
    config.alpha_compression = 1;
    config.alpha_filtering = 1;
    config.alpha_quality = 100;
    config.lossless = 0;
    config.exact = 0;
    config.image_hint = WebPImageHint::Default;
    config.emulate_jpeg_size = 0;
    config.thread_level = 0;
    config.low_memory = 0;
    config.near_lossless = 100;
    config.use_delta_palette = 0;
    config.use_sharp_yuv = 0;

    // TODO(skal): tune.
    match preset {
        WebPPreset::Picture => {
            config.sns_strength = 80;
            config.filter_sharpness = 4;
            config.filter_strength = 35;
            config.preprocessing &= !2; // no dithering
        }
        WebPPreset::Photo => {
            config.sns_strength = 80;
            config.filter_sharpness = 3;
            config.filter_strength = 30;
            config.preprocessing |= 2;
        }
        WebPPreset::Drawing => {
            config.sns_strength = 25;
            config.filter_sharpness = 6;
            config.filter_strength = 10;
        }
        WebPPreset::Icon => {
            config.sns_strength = 0;
            config.filter_strength = 0; // disable filtering to retain sharpness
            config.preprocessing &= !2; // no dithering
        }
        WebPPreset::Text => {
            config.sns_strength = 0;
            config.filter_strength = 0; // disable filtering to retain sharpness
            config.preprocessing &= !2; // no dithering
            config.segments = 2;
        }
        WebPPreset::Default => {}
    }
    webp_validate_config(config)
}

/// Should always be called, to initialize a fresh WebPConfig structure
/// before modification: the default preset at quality 75. Translation of
/// `WebPConfigInit()`.
pub(crate) fn webp_config_init(config: &mut WebPConfig) -> bool {
    webp_config_init_internal(config, WebPPreset::Default, 75.0)
}

/// Returns true if 'config' is non-NULL and all configuration parameters
/// are within their valid ranges. Translation of `WebPValidateConfig()`.
pub(crate) fn webp_validate_config(config: &WebPConfig) -> bool {
    // (a NaN quality passes these checks, as upstream's comparisons let it)
    if config.quality < 0.0 || config.quality > 100.0 {
        return false;
    }
    if config.target_size < 0 {
        return false;
    }
    if config.target_psnr < 0.0 {
        return false;
    }
    if config.method < 0 || config.method > 6 {
        return false;
    }
    if config.segments < 1 || config.segments > 4 {
        return false;
    }
    if config.sns_strength < 0 || config.sns_strength > 100 {
        return false;
    }
    if config.filter_strength < 0 || config.filter_strength > 100 {
        return false;
    }
    if config.filter_sharpness < 0 || config.filter_sharpness > 7 {
        return false;
    }
    if config.filter_type < 0 || config.filter_type > 1 {
        return false;
    }
    if config.autofilter < 0 || config.autofilter > 1 {
        return false;
    }
    if config.pass < 1 || config.pass > 10 {
        return false;
    }
    if config.qmin < 0 || config.qmax > 100 || config.qmin > config.qmax {
        return false;
    }
    if config.show_compressed < 0 || config.show_compressed > 1 {
        return false;
    }
    if config.preprocessing < 0 || config.preprocessing > 7 {
        return false;
    }
    if config.partitions < 0 || config.partitions > 3 {
        return false;
    }
    if config.partition_limit < 0 || config.partition_limit > 100 {
        return false;
    }
    if config.alpha_compression < 0 {
        return false;
    }
    if config.alpha_filtering < 0 {
        return false;
    }
    if config.alpha_quality < 0 || config.alpha_quality > 100 {
        return false;
    }
    if config.lossless < 0 || config.lossless > 1 {
        return false;
    }
    if config.near_lossless < 0 || config.near_lossless > 100 {
        return false;
    }
    // (config->image_hint >= WEBP_HINT_LAST can't be: it is an enum here)
    if config.emulate_jpeg_size < 0 || config.emulate_jpeg_size > 1 {
        return false;
    }
    if config.thread_level < 0 || config.thread_level > 1 {
        return false;
    }
    if config.low_memory < 0 || config.low_memory > 1 {
        return false;
    }
    if config.exact < 0 || config.exact > 1 {
        return false;
    }
    if config.use_delta_palette < 0 || config.use_delta_palette > 1 {
        return false;
    }
    if config.use_sharp_yuv < 0 || config.use_sharp_yuv > 1 {
        return false;
    }

    true
}
