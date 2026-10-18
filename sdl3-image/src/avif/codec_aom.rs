// Rust translation of the encoder of src/codec_aom.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches), as SDL_image
// builds it (AVIF_CODEC_AOM_ENCODE, without AVIF_CODEC_AOM_DECODE).
// Copyright 2019 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The AV1 encoder glue: libavif's aom codec, over the translation of
//! libaom's all intra encoder in `crate::aom`. As there is one codec that
//! can encode, `avifCodec` is this struct (its function pointers are its
//! methods; the codec specific options and the diagnostics it shares with
//! the encoder are the encoder's, passed in).
//!
//! The translated libaom codes what SDL_image's saver and animation
//! encoder ask of it: a single still frame (`AOM_USAGE_ALL_INTRA`) of
//! 10-bit samples, or a sequence (`AOM_USAGE_REALTIME`) of 8-bit or 10-bit
//! frames, 4:4:4 color or monochrome alpha. Other requests fail where
//! libaom would be called for what is not translated: the good quality
//! usage (`aom_codec_enc_config_default()` fails), other bit depths and
//! chroma subsamplings, `aom_codec_set_option()` (codec specific options),
//! `aom_codec_enc_config_set()`, spatial layers, scaling modes and frame
//! flags other than `AOM_EFLAG_FORCE_KF`.

use crate::aom::av1::av1_cx_iface::{
    aom_codec_enc_config_default, ctrl_set_chroma_sample_position, ctrl_set_color_primaries,
    ctrl_set_color_range, ctrl_set_cpuused, ctrl_set_cq_level, ctrl_set_lossless,
    ctrl_set_matrix_coefficients, ctrl_set_skip_postproc_filtering, ctrl_set_tile_columns,
    ctrl_set_tile_rows, ctrl_set_transfer_characteristics, ctrl_set_tuning, encoder_encode,
    encoder_init, AomCodecAlgPriv, AomCodecCxFramePkt, AomCodecEncCfg, AomImage,
    AOM_CODEC_USE_HIGHBITDEPTH, AOM_CR_FULL_RANGE, AOM_FRAME_IS_KEY, AOM_IMG_FMT_HIGHBITDEPTH,
    AOM_IMG_FMT_I420, AOM_IMG_FMT_I42016, AOM_IMG_FMT_I422, AOM_IMG_FMT_I42216, AOM_IMG_FMT_I444,
    AOM_KF_DISABLED, AOM_USAGE_ALL_INTRA,
};
use crate::aom::av1::encoder::encoder::{
    AOM_CBR, AOM_CQ, AOM_Q, AOM_TUNE_PSNR, AOM_TUNE_SSIM, AOM_VBR,
};

use super::avif::{
    avif_get_pixel_format_info, avif_image_plane_height, avif_image_plane_width,
    avif_result_to_string, AvifAddImageFlags, AvifEncoder, AvifFraction, AvifImage,
    AvifPixelFormat, AvifPixelFormatInfo, AvifRange, AvifResult,
    AVIF_ADD_IMAGE_FLAG_FORCE_KEYFRAME, AVIF_ADD_IMAGE_FLAG_SINGLE,
    AVIF_CHROMA_SAMPLE_POSITION_UNKNOWN, AVIF_COLOR_PRIMARIES_UNSPECIFIED,
    AVIF_MATRIX_COEFFICIENTS_UNSPECIFIED, AVIF_QUANTIZER_LOSSLESS, AVIF_SPEED_DEFAULT,
    AVIF_TRANSFER_CHARACTERISTICS_UNSPECIFIED,
};
use super::diag::{printf, AvifDiagnostics};
use super::internal::{avif_checkres, AvifCodecEncodeOutput, AvifEncoderChanges};
use super::internal::{
    AVIF_ENCODER_CHANGE_CODEC_SPECIFIC, AVIF_ENCODER_CHANGE_MAX_QUANTIZER,
    AVIF_ENCODER_CHANGE_MAX_QUANTIZER_ALPHA, AVIF_ENCODER_CHANGE_MIN_QUANTIZER,
    AVIF_ENCODER_CHANGE_MIN_QUANTIZER_ALPHA, AVIF_ENCODER_CHANGE_QUANTIZER,
    AVIF_ENCODER_CHANGE_QUANTIZER_ALPHA, AVIF_ENCODER_CHANGE_SCALING_MODE,
    AVIF_ENCODER_CHANGE_TILE_COLS_LOG2, AVIF_ENCODER_CHANGE_TILE_ROWS_LOG2,
};
use super::utils::avif_fraction_simplify;
use super::write::avif_codec_encode_output_add_sample;

// (The aom_codec_set_option() function is available in libaom 3.6.1
// (HAVE_AOM_CODEC_SET_OPTION), and so are speeds 7 to 9 in all intra mode
// (ALL_INTRA_HAS_SPEEDS_7_TO_9).)

/// `AOM_USAGE_GOOD_QUALITY`
const AOM_USAGE_GOOD_QUALITY: u32 = 0;
/// `AOM_USAGE_REALTIME`
const AOM_USAGE_REALTIME: u32 = 1;

/// `AOM_IMG_FMT_NONE`
const AOM_IMG_FMT_NONE: u32 = 0;

/// `aom_codec_version()` of the libaom SDL_image builds (3.6.1).
const AOM_CODEC_VERSION: i32 = (3 << 16) | (6 << 8) | 1;

/// `AOM_SCALING_MODE`
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum AomScalingMode {
    #[default]
    Normal = 0,
    FourFive = 1,
    ThreeFive = 2,
    ThreeFour = 3,
    OneFour = 4,
    OneEight = 5,
    OneTwo = 6,
}

/// `aom_scaling_mode_t`
#[derive(Clone, Copy, Debug, Default)]
struct AomScalingModeT {
    h_scaling_mode: AomScalingMode,
    v_scaling_mode: AomScalingMode,
}

/// Translation of `struct avifCodecInternal` (the encoder's members; the
/// encoder context is `Some` while it is initialized).
#[derive(Debug, Default)]
struct AvifCodecInternal {
    encoder_initialized: bool,
    encoder: Option<AomCodecAlgPriv>,
    cfg: AomCodecEncCfg,
    format_info: AvifPixelFormatInfo,
    aom_format: u32,
    monochrome_enabled: bool,
    /// Whether 'tuning' (of the specified distortion metric) was set with an
    /// avifEncoderSetCodecSpecificOption(encoder, "tune", value) call.
    tuning_set: bool,
    current_layer: u32,
}

/// Translation of `avifCodec` for aom (`aomCodecDestroyInternal()` and
/// `avifCodecDestroy()` are dropping it).
#[derive(Debug, Default)]
pub(crate) struct AvifCodec {
    internal: AvifCodecInternal,
}

/// Translation of `avifImageCalcAOMFmt()`.
fn avif_image_calc_aom_fmt(image: &AvifImage, alpha: bool) -> u32 {
    let mut fmt;
    if alpha {
        // We're going monochrome, who cares about chroma quality
        fmt = AOM_IMG_FMT_I420;
    } else {
        match image.yuv_format {
            AvifPixelFormat::Yuv444 => fmt = AOM_IMG_FMT_I444,
            AvifPixelFormat::Yuv422 => fmt = AOM_IMG_FMT_I422,
            AvifPixelFormat::Yuv420 | AvifPixelFormat::Yuv400 => fmt = AOM_IMG_FMT_I420,
            AvifPixelFormat::None => return AOM_IMG_FMT_NONE,
        }
    }

    if image.depth > 8 {
        fmt |= AOM_IMG_FMT_HIGHBITDEPTH;
    }

    fmt
}

// (aomOptionParseInt() and aomOptionParseUInt(): only without
// HAVE_AOM_CODEC_SET_OPTION)

/// Translation of `struct aomOptionEnumList`.
struct AomOptionEnumList {
    name: &'static str,
    val: i32,
}

/// The value `strtol()` parses when it parses the whole string.
fn strtol_whole(s: &str) -> Option<i64> {
    let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // (strtol() clamps an overflow to LONG_MIN or LONG_MAX)
    let mut v: i64 = 0;
    for b in digits.bytes() {
        v = v.saturating_mul(10).saturating_add((b - b'0') as i64);
    }
    Some(if neg { v.saturating_neg() } else { v })
}

/// Translation of `aomOptionParseEnum()` (the list has no sentinel).
fn aom_option_parse_enum(s: &str, enums: &[AomOptionEnumList], val: &mut i32) -> bool {
    // First see if the value can be parsed as a raw value.
    if let Some(rawval) = strtol_whole(s) {
        // Got a raw value, make sure it's valid.
        for listptr in enums {
            if listptr.val as i64 == rawval {
                *val = rawval as i32;
                return true;
            }
        }
    }

    // Next see if it can be parsed as a string.
    for listptr in enums {
        if s == listptr.name {
            *val = listptr.val;
            return true;
        }
    }

    false
}

static END_USAGE_ENUM: &[AomOptionEnumList] = &[
    AomOptionEnumList {
        name: "vbr",
        val: AOM_VBR,
    }, // Variable Bit Rate (VBR) mode
    AomOptionEnumList {
        name: "cbr",
        val: AOM_CBR,
    }, // Constant Bit Rate (CBR) mode
    AomOptionEnumList {
        name: "cq",
        val: AOM_CQ,
    }, // Constrained Quality (CQ) mode
    AomOptionEnumList {
        name: "q",
        val: AOM_Q,
    }, // Constant Quality (Q) mode
];

/// Returns true if <key> equals <name> or <prefix><name>, where <prefix> is "color:" or "alpha:"
/// or the abbreviated form "c:" or "a:".
/// Translation of `avifKeyEqualsName()`.
fn avif_key_equals_name(key: &str, name: &str, alpha: bool) -> bool {
    let prefix = if alpha { "alpha:" } else { "color:" };
    let short_prefix = if alpha { "a:" } else { "c:" };
    key == name
        || key.strip_prefix(prefix) == Some(name)
        || key.strip_prefix(short_prefix) == Some(name)
}

/// Translation of `avifProcessAOMOptionsPreInit()`.
fn avif_process_aom_options_pre_init(
    encoder: &AvifEncoder,
    alpha: bool,
    cfg: &mut AomCodecEncCfg,
) -> bool {
    for entry in &encoder.cs_options {
        let mut val = 0;
        if avif_key_equals_name(&entry.key, "end-usage", alpha) {
            // Rate control mode
            if !aom_option_parse_enum(&entry.value, END_USAGE_ENUM, &mut val) {
                printf(
                    Some(&encoder.diag),
                    format_args!("Invalid value for end-usage: {}", entry.value),
                );
                return false;
            }
            cfg.rc_end_usage = val;
        }
    }
    true
}

// (aomOptionType, struct aomOptionDef, tuningEnum and aomOptionDefs: only
// without HAVE_AOM_CODEC_SET_OPTION; they are kept for the record of the
// tune values)
#[allow(dead_code)]
static TUNING_ENUM: &[AomOptionEnumList] = &[
    AomOptionEnumList {
        name: "psnr",
        val: AOM_TUNE_PSNR,
    },
    AomOptionEnumList {
        name: "ssim",
        val: AOM_TUNE_SSIM,
    },
];

/// Translation of `avifProcessAOMOptionsPostInit()`, with
/// HAVE_AOM_CODEC_SET_OPTION: `aom_codec_set_option()` (libaom's argument
/// parser) is not translated, so it fails for every option it gets.
fn avif_process_aom_options_post_init(
    codec: &mut AvifCodec,
    encoder: &AvifEncoder,
    alpha: bool,
) -> bool {
    for entry in &encoder.cs_options {
        // Skip options for the other kind of plane.
        let other_prefix = if alpha { "color:" } else { "alpha:" };
        let other_short_prefix = if alpha { "c:" } else { "a:" };
        if entry.key.starts_with(other_prefix) || entry.key.starts_with(other_short_prefix) {
            continue;
        }

        // Skip options processed by avifProcessAOMOptionsPreInit.
        if avif_key_equals_name(&entry.key, "end-usage", alpha) {
            continue;
        }

        let prefix = if alpha { "alpha:" } else { "color:" };
        let short_prefix = if alpha { "a:" } else { "c:" };
        let mut key = entry.key.as_str();
        if let Some(k) = key.strip_prefix(prefix) {
            key = k;
        } else if let Some(k) = key.strip_prefix(short_prefix) {
            key = k;
        }
        // (aom_codec_set_option(): not translated)
        printf(
            Some(&encoder.diag),
            format_args!(
                "aom_codec_set_option(\"{}\", \"{}\") failed: {}: {}",
                key, entry.value, "Unspecified internal error", "not translated"
            ),
        );
        if key == "tune" {
            codec.internal.tuning_set = true;
        }
        return false;
    }
    true
}

/// Translation of `struct aomScalingModeMapList`.
struct AomScalingModeMapList {
    avif_mode: AvifFraction,
    aom_mode: AomScalingMode,
}

static SCALING_MODE_MAP: &[AomScalingModeMapList] = &[
    AomScalingModeMapList {
        avif_mode: AvifFraction { n: 1, d: 1 },
        aom_mode: AomScalingMode::Normal,
    },
    AomScalingModeMapList {
        avif_mode: AvifFraction { n: 1, d: 2 },
        aom_mode: AomScalingMode::OneTwo,
    },
    AomScalingModeMapList {
        avif_mode: AvifFraction { n: 1, d: 4 },
        aom_mode: AomScalingMode::OneFour,
    },
    AomScalingModeMapList {
        avif_mode: AvifFraction { n: 1, d: 8 },
        aom_mode: AomScalingMode::OneEight,
    },
    AomScalingModeMapList {
        avif_mode: AvifFraction { n: 3, d: 4 },
        aom_mode: AomScalingMode::ThreeFour,
    },
    AomScalingModeMapList {
        avif_mode: AvifFraction { n: 3, d: 5 },
        aom_mode: AomScalingMode::ThreeFive,
    },
    AomScalingModeMapList {
        avif_mode: AvifFraction { n: 4, d: 5 },
        aom_mode: AomScalingMode::FourFive,
    },
];

/// Translation of `avifFindAOMScalingMode()`.
fn avif_find_aom_scaling_mode(avif_mode: &AvifFraction, aom_mode: &mut AomScalingMode) -> bool {
    let mut simplified_fraction = *avif_mode;
    avif_fraction_simplify(&mut simplified_fraction);
    for m in SCALING_MODE_MAP {
        if m.avif_mode.n == simplified_fraction.n && m.avif_mode.d == simplified_fraction.d {
            *aom_mode = m.aom_mode;
            return true;
        }
    }

    false
}

/// Translation of `doesLevelMatch()`.
fn does_level_match(
    width: i32,
    height: i32,
    level_width: i32,
    level_height: i32,
    level_dim_mult: i32,
) -> bool {
    let level_luma_pels = level_width as i64 * level_height as i64;
    let luma_pels = width as i64 * height as i64;
    luma_pels <= level_luma_pels
        && width <= level_width * level_dim_mult
        && height <= level_height * level_dim_mult
}

/// `aom_codec_error()` and `aom_codec_error_detail()` of a failed call into
/// the translated encoder (which reports no details).
const AOM_CODEC_ERROR: &str = "Unspecified internal error";

/// The packets of an `aom_codec_get_cx_data()` loop: every packet is an
/// `AOM_CODEC_CX_FRAME_PKT`.
fn add_frame_packets(
    output: &mut AvifCodecEncodeOutput,
    pkts: Vec<AomCodecCxFramePkt>,
) -> AvifResult {
    for pkt in pkts {
        avif_checkres!(avif_codec_encode_output_add_sample(
            output,
            &pkt.buf,
            (pkt.flags & AOM_FRAME_IS_KEY) != 0
        ));
    }
    AvifResult::Ok
}

impl AvifCodec {
    /// Translation of `aomCodecEncodeImage()`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_image(
        &mut self,
        encoder: &AvifEncoder,
        image: &AvifImage,
        alpha: bool,
        tile_rows_log2: i32,
        tile_cols_log2: i32,
        quantizer: i32,
        mut encoder_changes: AvifEncoderChanges,
        disable_lagged_output: bool,
        add_image_flags: AvifAddImageFlags,
        output: &mut AvifCodecEncodeOutput,
    ) -> AvifResult {
        let diag = Some(&encoder.diag);
        let mut quantizer_updated = false;

        // For encoder->scalingMode.horizontal and encoder->scalingMode.vertical to take effect in AOM
        // encoder, config should be applied for each frame, so we don't care about changes on these
        // two fields.
        encoder_changes &= !AVIF_ENCODER_CHANGE_SCALING_MODE;

        if !self.internal.encoder_initialized {
            // Map encoder speed to AOM usage + CpuUsed:
            // Speed  0: GoodQuality CpuUsed 0
            // Speed  1: GoodQuality CpuUsed 1
            // Speed  2: GoodQuality CpuUsed 2
            // Speed  3: GoodQuality CpuUsed 3
            // Speed  4: GoodQuality CpuUsed 4
            // Speed  5: GoodQuality CpuUsed 5
            // Speed  6: GoodQuality CpuUsed 6
            // Speed  7: RealTime    CpuUsed 7
            // Speed  8: RealTime    CpuUsed 8
            // Speed  9: RealTime    CpuUsed 9
            // Speed 10: RealTime    CpuUsed 9
            let mut aom_usage = AOM_USAGE_GOOD_QUALITY;
            // Use the new AOM_USAGE_ALL_INTRA (added in https://crbug.com/aomedia/2959) for still
            // image encoding if it is available.
            if add_image_flags & AVIF_ADD_IMAGE_FLAG_SINGLE != 0 {
                aom_usage = AOM_USAGE_ALL_INTRA;
            }
            let mut aom_cpu_used = -1;
            if encoder.speed != AVIF_SPEED_DEFAULT {
                aom_cpu_used = encoder.speed.clamp(0, 9);
                if aom_cpu_used >= 7 {
                    if add_image_flags & AVIF_ADD_IMAGE_FLAG_SINGLE == 0 {
                        aom_usage = AOM_USAGE_REALTIME;
                    }
                }
            }

            // aom_codec.h says: aom_codec_version() == (major<<16 | minor<<8 | patch)
            const AOM_VERSION_2_0_0: i32 = 2 << 16;
            let aom_version = AOM_CODEC_VERSION;
            if (aom_version < AOM_VERSION_2_0_0) && (image.depth > 8) {
                // Due to a known issue with libaom v1.0.0-errata1-avif, 10bpc and
                // 12bpc image encodes will call the wrong variant of
                // aom_subtract_block when cpu-used is 7 or 8, and crash. Until we get
                // a new tagged release from libaom with the fix and can verify we're
                // running with that version of libaom, we must avoid using
                // cpu-used=7/8 on any >8bpc image encodes.
                //
                // Context:
                //   * https://github.com/AOMediaCodec/libavif/issues/49
                //   * https://bugs.chromium.org/p/aomedia/issues/detail?id=2587
                //
                // Continued bug tracking here:
                //   * https://github.com/AOMediaCodec/libavif/issues/56

                if aom_cpu_used > 6 {
                    aom_cpu_used = 6;
                }
            }

            self.internal.aom_format = avif_image_calc_aom_fmt(image, alpha);
            if self.internal.aom_format == AOM_IMG_FMT_NONE {
                return AvifResult::UnknownError;
            }
            // (the translated libaom codes 8-bit and 10-bit 4:4:4 color and
            // monochrome alpha: see the module documentation)
            if (image.depth != 8 && image.depth != 10)
                || (!alpha && image.yuv_format != AvifPixelFormat::Yuv444)
            {
                printf(
                    diag,
                    format_args!(
                        "aom_codec_enc_init() failed: {}: not translated for this format",
                        AOM_CODEC_ERROR
                    ),
                );
                return AvifResult::UnknownError;
            }

            self.internal.format_info = avif_get_pixel_format_info(image.yuv_format);

            let Some(default_cfg) = aom_codec_enc_config_default(aom_usage) else {
                // (only the all intra usage is translated)
                printf(
                    diag,
                    format_args!(
                        "aom_codec_enc_config_default() failed: {}",
                        "Invalid parameter"
                    ),
                );
                return AvifResult::UnknownError;
            };
            self.internal.cfg = default_cfg;
            let cfg = &mut self.internal.cfg;

            // Set our own default cfg->rc_end_usage value, which may differ from libaom's default.
            match aom_usage {
                AOM_USAGE_GOOD_QUALITY => {
                    // libaom's default is AOM_VBR. Change the default to AOM_Q since we don't need to
                    // hit a certain target bit rate. It's easier to control the worst quality in Q
                    // mode.
                    cfg.rc_end_usage = AOM_Q;
                }
                AOM_USAGE_REALTIME => {
                    // For real-time mode we need to use CBR rate control mode. AOM_Q doesn't fit the
                    // rate control requirements for real-time mode. CBR does.
                    cfg.rc_end_usage = AOM_CBR;
                }
                AOM_USAGE_ALL_INTRA => {
                    cfg.rc_end_usage = AOM_Q;
                }
                _ => {}
            }

            // Profile 0.  8-bit and 10-bit 4:2:0 and 4:0:0 only.
            // Profile 1.  8-bit and 10-bit 4:4:4
            // Profile 2.  8-bit and 10-bit 4:2:2
            //            12-bit 4:0:0, 4:2:0, 4:2:2 and 4:4:4
            let mut seq_profile: u8 = 0;
            if image.depth == 12 {
                // Only seqProfile 2 can handle 12 bit
                seq_profile = 2;
            } else {
                // 8-bit or 10-bit

                if alpha {
                    seq_profile = 0;
                } else {
                    match image.yuv_format {
                        AvifPixelFormat::Yuv444 => seq_profile = 1,
                        AvifPixelFormat::Yuv422 => seq_profile = 2,
                        AvifPixelFormat::Yuv420 => seq_profile = 0,
                        AvifPixelFormat::Yuv400 => seq_profile = 0,
                        AvifPixelFormat::None => {}
                    }
                }
            }

            cfg.g_profile = seq_profile as u32;
            cfg.g_bit_depth = image.depth as i32;
            cfg.g_input_bit_depth = image.depth;
            cfg.g_w = image.width;
            cfg.g_h = image.height;

            // Detect the libaom v3.6.0 bug described in
            // https://crbug.com/aomedia/2871#c12. See the changes to
            // av1/encoder/encoder.c in
            // https://aomedia-review.googlesource.com/c/aom/+/174421.
            const AOM_VERSION_3_6_0: i32 = (3 << 16) | (6 << 8);
            if aom_version == AOM_VERSION_3_6_0 {
                // Detect the use of levels 7.x and 8.x, which use a larger max
                // tile area (4096 * 4608) than MAX_TILE_AREA (4096 * 2304). The
                // larger max tile area may not result in a different bitstream
                // (see the tile_info() function in the AV1 spec, Section 5.9.15),
                // so this is just a necessary condition for the bug.
                let (w, h) = (image.width as i32, image.height as i32);
                if !does_level_match(w, h, 8192, 4352, 2)
                    && (does_level_match(w, h, 16384, 8704, 2)
                        || does_level_match(w, h, 32768, 17408, 2))
                {
                    printf(
                        diag,
                        format_args!(
                            "Detected libaom v3.6.0 bug with large images. Upgrade to libaom v3.6.1 or later."
                        ),
                    );
                    return AvifResult::UnknownError;
                }
            }

            if add_image_flags & AVIF_ADD_IMAGE_FLAG_SINGLE != 0 {
                // Set the maximum number of frames to encode to 1. This instructs
                // libaom to set still_picture and reduced_still_picture_header to
                // 1 in AV1 sequence headers.
                cfg.g_limit = 1;

                // Use the default settings of the new AOM_USAGE_ALL_INTRA (added in
                // https://crbug.com/aomedia/2959).
                //
                // Set g_lag_in_frames to 0 to reduce the number of frame buffers
                // (from 20 to 2) in libaom's lookahead structure. This reduces
                // memory consumption when encoding a single image.
                cfg.g_lag_in_frames = 0;
                // Disable automatic placement of key frames by the encoder.
                cfg.kf_mode = AOM_KF_DISABLED;
                // Tell libaom that all frames will be key frames.
                cfg.kf_max_dist = 0;
            } else if encoder.keyframe_interval > 0 {
                cfg.kf_max_dist = encoder.keyframe_interval as u32;
            }
            if encoder.extra_layer_count > 0 {
                cfg.g_limit = encoder.extra_layer_count + 1;
                // For layered image, disable lagged encoding to always get output
                // frame for each input frame.
                cfg.g_lag_in_frames = 0;
            }
            if disable_lagged_output {
                cfg.g_lag_in_frames = 0;
            }
            if encoder.max_threads > 1 {
                // libaom fails if cfg->g_threads is greater than 64 threads. See MAX_NUM_THREADS in
                // aom/aom_util/aom_thread.h.
                cfg.g_threads = encoder.max_threads.min(64) as u32;
            }

            self.internal.monochrome_enabled = false;
            if aom_version > AOM_VERSION_2_0_0 {
                // There exists a bug in libaom's chroma_check() function where it will attempt to
                // access nonexistent UV planes when encoding monochrome at faster libavif "speeds". It
                // was fixed shortly after the 2.0.0 libaom release, and the fix exists in both the
                // master and applejack branches. This ensures that the next version *after* 2.0.0 will
                // have the fix, and we must avoid cfg->monochrome until then.
                //
                // Bugfix Change-Id: https://aomedia-review.googlesource.com/q/I26a39791f820b4d4e1d63ff7141f594c3c7181f5

                if alpha || (image.yuv_format == AvifPixelFormat::Yuv400) {
                    self.internal.monochrome_enabled = true;
                    cfg.monochrome = 1;
                }
            }

            if !avif_process_aom_options_pre_init(encoder, alpha, cfg) {
                return AvifResult::InvalidCodecSpecificOption;
            }

            let mut min_quantizer;
            let mut max_quantizer;
            if alpha {
                min_quantizer = encoder.min_quantizer_alpha;
                max_quantizer = encoder.max_quantizer_alpha;
            } else {
                min_quantizer = encoder.min_quantizer;
                max_quantizer = encoder.max_quantizer;
            }
            min_quantizer = min_quantizer.clamp(0, 63);
            max_quantizer = max_quantizer.clamp(0, 63);
            if (cfg.rc_end_usage == AOM_VBR) || (cfg.rc_end_usage == AOM_CBR) {
                // cq-level is ignored in these two end-usage modes, so adjust minQuantizer and
                // maxQuantizer to the target quantizer.
                if quantizer == AVIF_QUANTIZER_LOSSLESS {
                    min_quantizer = AVIF_QUANTIZER_LOSSLESS;
                    max_quantizer = AVIF_QUANTIZER_LOSSLESS;
                } else {
                    min_quantizer = (quantizer - 4).max(min_quantizer);
                    max_quantizer = (quantizer + 4).min(max_quantizer);
                }
            }
            cfg.rc_min_quantizer = min_quantizer as u32;
            cfg.rc_max_quantizer = max_quantizer as u32;
            quantizer_updated = true;
            let _ = quantizer_updated; // (only read for an encoder initialized before)

            let mut encoder_flags = 0;
            if image.depth > 8 {
                encoder_flags |= AOM_CODEC_USE_HIGHBITDEPTH;
            }
            let rc_end_usage = cfg.rc_end_usage;
            let g_usage = cfg.g_usage;
            let Ok(ctx) = encoder_init(cfg, encoder_flags) else {
                printf(
                    diag,
                    format_args!(
                        "aom_codec_enc_init() failed: {}: {}",
                        AOM_CODEC_ERROR, "(null)"
                    ),
                );
                return AvifResult::UnknownError;
            };
            self.internal.encoder_initialized = true;
            let ctx = self.internal.encoder.insert(ctx);

            if (rc_end_usage == AOM_CQ) || (rc_end_usage == AOM_Q) {
                let _ = ctrl_set_cq_level(ctx, quantizer as u32);
            }
            let lossless = quantizer == AVIF_QUANTIZER_LOSSLESS;
            if lossless {
                let _ = ctrl_set_lossless(ctx, 1);
            }
            if tile_rows_log2 != 0 {
                let _ = ctrl_set_tile_rows(ctx, tile_rows_log2 as u32);
            }
            if tile_cols_log2 != 0 {
                let _ = ctrl_set_tile_columns(ctx, tile_cols_log2 as u32);
            }
            if encoder.extra_layer_count > 0 {
                // (AOME_SET_NUMBER_SPATIAL_LAYERS: spatial layers are not
                // translated)
                return AvifResult::UnknownError;
            }
            if aom_cpu_used != -1 && ctrl_set_cpuused(ctx, aom_cpu_used).is_err() {
                return AvifResult::UnknownError;
            }

            // Set color_config() in the sequence header OBU.
            if alpha {
                let _ = ctrl_set_color_range(ctx, AOM_CR_FULL_RANGE);
            } else {
                // libaom's defaults are AOM_CICP_CP_UNSPECIFIED, AOM_CICP_TC_UNSPECIFIED,
                // AOM_CICP_MC_UNSPECIFIED, AOM_CSP_UNKNOWN, and 0 (studio/limited range). Call
                // aom_codec_control() only if the values are not the defaults.
                if image.color_primaries != AVIF_COLOR_PRIMARIES_UNSPECIFIED {
                    let _ = ctrl_set_color_primaries(ctx, image.color_primaries as i32);
                }
                if image.transfer_characteristics != AVIF_TRANSFER_CHARACTERISTICS_UNSPECIFIED {
                    let _ = ctrl_set_transfer_characteristics(
                        ctx,
                        image.transfer_characteristics as i32,
                    );
                }
                if image.matrix_coefficients != AVIF_MATRIX_COEFFICIENTS_UNSPECIFIED {
                    let _ = ctrl_set_matrix_coefficients(ctx, image.matrix_coefficients as i32);
                }
                if image.yuv_chroma_sample_position != AVIF_CHROMA_SAMPLE_POSITION_UNKNOWN {
                    let _ = ctrl_set_chroma_sample_position(
                        ctx,
                        image.yuv_chroma_sample_position as i32,
                    );
                }
                if image.yuv_range != AvifRange::Limited {
                    let _ = ctrl_set_color_range(ctx, image.yuv_range as i32);
                }
            }

            if g_usage == AOM_USAGE_ALL_INTRA {
                // Enable AV1E_SET_SKIP_POSTPROC_FILTERING for still-picture encoding, which is
                // disabled by default.
                let _ = ctrl_set_skip_postproc_filtering(ctx, 1);
            }

            if !avif_process_aom_options_post_init(self, encoder, alpha) {
                return AvifResult::InvalidCodecSpecificOption;
            }
            if !self.internal.tuning_set {
                let Some(ctx) = self.internal.encoder.as_mut() else {
                    return AvifResult::UnknownError;
                };
                if ctrl_set_tuning(ctx, AOM_TUNE_SSIM).is_err() {
                    return AvifResult::UnknownError;
                }
            }
        } else {
            let dimensions_changed = false;
            let cfg = &mut self.internal.cfg;
            if (cfg.g_w != image.width) || (cfg.g_h != image.height) {
                // We are not ready for dimension change for now.
                return AvifResult::NotImplemented;
            }
            if alpha {
                if encoder_changes
                    & (AVIF_ENCODER_CHANGE_MIN_QUANTIZER_ALPHA
                        | AVIF_ENCODER_CHANGE_MAX_QUANTIZER_ALPHA)
                    != 0
                {
                    cfg.rc_min_quantizer = encoder.min_quantizer_alpha.clamp(0, 63) as u32;
                    cfg.rc_max_quantizer = encoder.max_quantizer_alpha.clamp(0, 63) as u32;
                    quantizer_updated = true;
                }
            } else if encoder_changes
                & (AVIF_ENCODER_CHANGE_MIN_QUANTIZER | AVIF_ENCODER_CHANGE_MAX_QUANTIZER)
                != 0
            {
                cfg.rc_min_quantizer = encoder.min_quantizer.clamp(0, 63) as u32;
                cfg.rc_max_quantizer = encoder.max_quantizer.clamp(0, 63) as u32;
                quantizer_updated = true;
            }
            let quantizer_changed_bit = if alpha {
                AVIF_ENCODER_CHANGE_QUANTIZER_ALPHA
            } else {
                AVIF_ENCODER_CHANGE_QUANTIZER
            };
            if encoder_changes & quantizer_changed_bit != 0
                && ((cfg.rc_end_usage == AOM_VBR) || (cfg.rc_end_usage == AOM_CBR))
            {
                // cq-level is ignored in these two end-usage modes, so adjust minQuantizer and
                // maxQuantizer to the target quantizer.
                if quantizer == AVIF_QUANTIZER_LOSSLESS {
                    cfg.rc_min_quantizer = AVIF_QUANTIZER_LOSSLESS as u32;
                    cfg.rc_max_quantizer = AVIF_QUANTIZER_LOSSLESS as u32;
                } else {
                    let mut min_quantizer;
                    let mut max_quantizer;
                    if alpha {
                        min_quantizer = encoder.min_quantizer_alpha;
                        max_quantizer = encoder.max_quantizer_alpha;
                    } else {
                        min_quantizer = encoder.min_quantizer;
                        max_quantizer = encoder.max_quantizer;
                    }
                    min_quantizer = min_quantizer.clamp(0, 63);
                    max_quantizer = max_quantizer.clamp(0, 63);
                    cfg.rc_min_quantizer = (quantizer - 4).max(min_quantizer) as u32;
                    cfg.rc_max_quantizer = (quantizer + 4).min(max_quantizer) as u32;
                }
                quantizer_updated = true;
            }
            let rc_end_usage = cfg.rc_end_usage;
            if quantizer_updated || dimensions_changed {
                // (aom_codec_enc_config_set(): not translated)
                printf(
                    diag,
                    format_args!(
                        "aom_codec_enc_config_set() failed: {}: {}",
                        AOM_CODEC_ERROR, "not translated"
                    ),
                );
                return AvifResult::UnknownError;
            }
            let Some(ctx) = self.internal.encoder.as_mut() else {
                return AvifResult::UnknownError;
            };
            if encoder_changes & AVIF_ENCODER_CHANGE_TILE_ROWS_LOG2 != 0 {
                let _ = ctrl_set_tile_rows(ctx, tile_rows_log2 as u32);
            }
            if encoder_changes & AVIF_ENCODER_CHANGE_TILE_COLS_LOG2 != 0 {
                let _ = ctrl_set_tile_columns(ctx, tile_cols_log2 as u32);
            }
            if encoder_changes & quantizer_changed_bit != 0 {
                if (rc_end_usage == AOM_CQ) || (rc_end_usage == AOM_Q) {
                    let _ = ctrl_set_cq_level(ctx, quantizer as u32);
                }
                let lossless = quantizer == AVIF_QUANTIZER_LOSSLESS;
                let _ = ctrl_set_lossless(ctx, lossless as u32);
            }
            if encoder_changes & AVIF_ENCODER_CHANGE_CODEC_SPECIFIC != 0
                && !avif_process_aom_options_post_init(self, encoder, alpha)
            {
                return AvifResult::InvalidCodecSpecificOption;
            }
        }

        if self.internal.current_layer > encoder.extra_layer_count {
            printf(
                diag,
                format_args!(
                    "Too many layers sent. Expected {} layers, but got {} layers.",
                    encoder.extra_layer_count + 1,
                    self.internal.current_layer + 1
                ),
            );
            return AvifResult::InvalidArgument;
        }
        // (extraLayerCount > 0 (AOME_SET_SPATIAL_LAYER_ID) failed above)

        let mut aom_scaling_mode = AomScalingModeT::default();
        if !avif_find_aom_scaling_mode(
            &encoder.scaling_mode.horizontal,
            &mut aom_scaling_mode.h_scaling_mode,
        ) {
            return AvifResult::NotImplemented;
        }
        if !avif_find_aom_scaling_mode(
            &encoder.scaling_mode.vertical,
            &mut aom_scaling_mode.v_scaling_mode,
        ) {
            return AvifResult::NotImplemented;
        }
        if (aom_scaling_mode.h_scaling_mode != AomScalingMode::Normal)
            || (aom_scaling_mode.v_scaling_mode != AomScalingMode::Normal)
        {
            // AOME_SET_SCALEMODE only applies to next frame (layer), so we have to set it every time.
            // (AOME_SET_SCALEMODE: frame scaling is not translated)
            return AvifResult::NotImplemented;
        }

        let aom_format = self.internal.aom_format;
        // We prefer to simply set the aomImage.planes[] pointers to the plane buffers in 'image'. When
        // doing this, we set aomImage.w equal to aomImage.d_w and aomImage.h equal to aomImage.d_h and
        // do not "align" aomImage.w and aomImage.h. Unfortunately this exposes a bug in libaom
        // (https://crbug.com/aomedia/3113) if chroma is subsampled and image->width or image->height is
        // equal to 1. To work around this libaom bug, we allocate the aomImage.planes[] buffers and
        // copy the image YUV data if image->width or image->height is equal to 1. This bug has been
        // fixed in libaom v3.1.3.
        //
        // Note: The exact condition for the bug is
        //   ((image->width == 1) && (chroma is subsampled horizontally)) ||
        //   ((image->height == 1) && (chroma is subsampled vertically))
        // Since an image width or height of 1 is uncommon in practice, we test an inexact but simpler
        // condition.
        let aom_image_allocated = (image.width == 1) || (image.height == 1);
        // (the planes aom_img_alloc() allocates)
        let mut allocated_planes: [Vec<u16>; 3] = Default::default();
        // (libaom reads 8-bit planes as bytes; the translated libaom takes
        // samples, so the planes of an 8-bit image are widened: the Y, U, V
        // and alpha planes, and the samples a row strides over)
        let mut widened: [Vec<u16>; 4] = Default::default();
        let sample_bytes = if image.depth > 8 { 2 } else { 1 };
        if image.depth <= 8 {
            let planes = [
                image.yuv_planes[0].as_ref(),
                image.yuv_planes[1].as_ref(),
                image.yuv_planes[2].as_ref(),
                image.alpha_plane.as_ref(),
            ];
            for (w, plane) in widened.iter_mut().zip(planes) {
                if let Some(plane) = plane {
                    let src = plane.u8s();
                    if w.try_reserve_exact(src.len()).is_err() {
                        return AvifResult::OutOfMemory;
                    }
                    w.extend(src.iter().map(|&b| b as u16));
                }
            }
        }
        let mut aom_image = AomImage {
            fmt: aom_format,
            cp: 0,
            tc: 0,
            mc: 0,
            monochrome: false,
            csp: 0,
            range: 0,
            w: image.width,
            h: image.height,
            bit_depth: if image.depth > 8 { 16 } else { 8 },
            d_w: image.width,
            d_h: image.height,
            x_chroma_shift: 0,
            y_chroma_shift: 0,
            planes: [&[], &[], &[]],
            stride: [0; 3],
        };
        if aom_image_allocated {
            // (aom_img_alloc(&aomImage, aomFormat, width, height, 16): the
            // strides aligned to 16 bytes, in samples here)
            let (xcs, ycs) = match aom_format {
                AOM_IMG_FMT_I420 | AOM_IMG_FMT_I42016 => (1, 1),
                AOM_IMG_FMT_I422 | AOM_IMG_FMT_I42216 => (1, 0),
                _ => (0, 0),
            };
            let w = (image.width + ((1 << xcs) - 1)) & !((1 << xcs) - 1);
            let h = (image.height + ((1 << ycs) - 1)) & !((1 << ycs) - 1);
            let stride_in_bytes = (w as usize * sample_bytes + 16 - 1) & !(16 - 1);
            aom_image.w = w;
            aom_image.h = h;
            aom_image.x_chroma_shift = xcs;
            aom_image.y_chroma_shift = ycs;
            aom_image.stride[0] = (stride_in_bytes / sample_bytes) as i32;
            aom_image.stride[1] = ((stride_in_bytes >> xcs) / sample_bytes) as i32;
            aom_image.stride[2] = aom_image.stride[1];
            let sizes = [
                h as usize * (stride_in_bytes / sample_bytes),
                (h >> ycs) as usize * ((stride_in_bytes >> xcs) / sample_bytes),
                (h >> ycs) as usize * ((stride_in_bytes >> xcs) / sample_bytes),
            ];
            for (plane, size) in allocated_planes.iter_mut().zip(sizes) {
                if plane.try_reserve_exact(size).is_err() {
                    return AvifResult::OutOfMemory;
                }
                plane.resize(size, 0);
            }
        } else {
            // Get sample size for this format.
            // (aomImage.bps: av1_get_image_bps() of the format)
            // See avifImageCalcAOMFmt(). libaom doesn't have AOM_IMG_FMT_I400, so we use AOM_IMG_FMT_I420 as a substitute for monochrome.
            aom_image.x_chroma_shift = if alpha || self.internal.format_info.monochrome {
                1
            } else {
                self.internal.format_info.chroma_shift_x as u32
            };
            aom_image.y_chroma_shift = if alpha || self.internal.format_info.monochrome {
                1
            } else {
                self.internal.format_info.chroma_shift_y as u32
            };
        }

        let mut monochrome_requested = false;

        if alpha {
            aom_image.range = AOM_CR_FULL_RANGE;
            monochrome_requested = true;
            let Some(alpha_plane) = &image.alpha_plane else {
                return AvifResult::UnknownError;
            };
            let src: &[u16] = if sample_bytes == 2 {
                alpha_plane.u16s()
            } else {
                &widened[3]
            };
            let src_stride = image.alpha_row_bytes as usize / sample_bytes;
            if aom_image_allocated {
                let samples_per_row = image.width as usize;
                let dst_stride = aom_image.stride[0] as usize;
                for j in 0..image.height as usize {
                    allocated_planes[0][j * dst_stride..j * dst_stride + samples_per_row]
                        .copy_from_slice(&src[j * src_stride..j * src_stride + samples_per_row]);
                }
            } else {
                aom_image.planes[0] = src;
                aom_image.stride[0] = src_stride as i32;
            }

            // Ignore UV planes when monochrome
        } else {
            let mut yuv_plane_count = 3;
            if image.yuv_format == AvifPixelFormat::Yuv400 {
                yuv_plane_count = 1; // Ignore UV planes when monochrome
                monochrome_requested = true;
            }
            if aom_image_allocated {
                for yuv_plane in 0..yuv_plane_count {
                    let plane_width = avif_image_plane_width(image, yuv_plane) as usize;
                    let plane_height = avif_image_plane_height(image, yuv_plane) as usize;
                    let Some(src_plane) = &image.yuv_planes[yuv_plane] else {
                        return AvifResult::UnknownError;
                    };
                    let src: &[u16] = if sample_bytes == 2 {
                        src_plane.u16s()
                    } else {
                        &widened[yuv_plane]
                    };
                    let src_stride = image.yuv_row_bytes[yuv_plane] as usize / sample_bytes;
                    let dst_stride = aom_image.stride[yuv_plane] as usize;

                    for j in 0..plane_height {
                        allocated_planes[yuv_plane][j * dst_stride..j * dst_stride + plane_width]
                            .copy_from_slice(&src[j * src_stride..j * src_stride + plane_width]);
                    }
                }
            } else {
                for yuv_plane in 0..yuv_plane_count {
                    let Some(src_plane) = &image.yuv_planes[yuv_plane] else {
                        return AvifResult::UnknownError;
                    };
                    aom_image.planes[yuv_plane] = if sample_bytes == 2 {
                        src_plane.u16s()
                    } else {
                        &widened[yuv_plane]
                    };
                    aom_image.stride[yuv_plane] =
                        (image.yuv_row_bytes[yuv_plane] as usize / sample_bytes) as i32;
                }
            }

            aom_image.cp = image.color_primaries as i32;
            aom_image.tc = image.transfer_characteristics as i32;
            aom_image.mc = image.matrix_coefficients as i32;
            aom_image.csp = image.yuv_chroma_sample_position as i32;
            aom_image.range = image.yuv_range as i32;
        }
        if aom_image_allocated {
            for (i, plane) in allocated_planes.iter().enumerate() {
                aom_image.planes[i] = plane;
            }
        }

        if monochrome_requested {
            if self.internal.monochrome_enabled {
                aom_image.monochrome = true;
            } else {
                // The user requested monochrome (via alpha or YUV400) but libaom cannot currently support
                // monochrome (see chroma_check comment above). Manually set UV planes to 0.5.
                // (libaom 3.6.1 is later than 2.0.0: monochromeEnabled is always
                // set when monochrome is requested)
                return AvifResult::UnknownError;
            }
        }

        let mut encode_flags: u32 = 0;
        if add_image_flags & AVIF_ADD_IMAGE_FLAG_FORCE_KEYFRAME != 0 {
            // (AOM_EFLAG_FORCE_KF)
            encode_flags |= 1 << 0;
        }
        // (currentLayer > 0 needs spatial layers, which failed above)
        let Some(ctx) = self.internal.encoder.as_mut() else {
            return AvifResult::UnknownError;
        };
        let encode_result = encoder_encode(ctx, Some(&aom_image), 0, 1, encode_flags);
        let Ok(pkts) = encode_result else {
            printf(
                diag,
                format_args!(
                    "aom_codec_encode() failed: {}: {}",
                    AOM_CODEC_ERROR, "(null)"
                ),
            );
            return AvifResult::UnknownError;
        };

        avif_checkres!(add_frame_packets(output, pkts));

        if (add_image_flags & AVIF_ADD_IMAGE_FLAG_SINGLE != 0)
            || ((encoder.extra_layer_count > 0)
                && (encoder.extra_layer_count == self.internal.current_layer))
        {
            // Flush and clean up encoder resources early to save on overhead when encoding alpha or grid images,
            // as encoding is finished now. For layered image, encoding finishes when the last layer is encoded.

            if !self.encode_finish(&encoder.diag, output) {
                return AvifResult::UnknownError;
            }
            self.internal.encoder = None;
            self.internal.encoder_initialized = false;
        }
        if encoder.extra_layer_count > 0 {
            self.internal.current_layer += 1;
        }
        AvifResult::Ok
    }

    /// Translation of `aomCodecEncodeFinish()`.
    pub(crate) fn encode_finish(
        &mut self,
        diag: &AvifDiagnostics,
        output: &mut AvifCodecEncodeOutput,
    ) -> bool {
        if !self.internal.encoder_initialized {
            return true;
        }
        let Some(ctx) = self.internal.encoder.as_mut() else {
            return false;
        };
        loop {
            // flush encoder
            let Ok(pkts) = encoder_encode(ctx, None, 0, 1, 0) else {
                printf(
                    Some(diag),
                    format_args!(
                        "aom_codec_encode() with img=NULL failed: {}: {}",
                        AOM_CODEC_ERROR, "(null)"
                    ),
                );
                return false;
            };

            let mut got_packet = false;
            for pkt in pkts {
                got_packet = true;
                let result = avif_codec_encode_output_add_sample(
                    output,
                    &pkt.buf,
                    (pkt.flags & AOM_FRAME_IS_KEY) != 0,
                );
                if result != AvifResult::Ok {
                    printf(
                        Some(diag),
                        format_args!(
                            "avifCodecEncodeOutputAddSample() failed: {}",
                            avif_result_to_string(result)
                        ),
                    );
                    return false;
                }
            }

            if !got_packet {
                break;
            }
        }
        true
    }
}

/// Translation of `avifCodecVersionAOM()` (`aom_codec_version_str()` of
/// the libaom SDL_image builds).
pub(crate) fn avif_codec_version_aom() -> &'static str {
    "v3.6.1"
}

/// Translation of `avifCodecCreateAOM()`.
pub(crate) fn avif_codec_create_aom() -> Box<AvifCodec> {
    Box::<AvifCodec>::default()
}
