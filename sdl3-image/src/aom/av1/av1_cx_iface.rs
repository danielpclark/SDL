// Rust translation of the parts of av1/av1_cx_iface.c, av1/av1_iface_common.h
// and aom/src/aom_encoder.c from aom (https://aomedia.googlesource.com/aom,
// at the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches) the saver reaches: the all intra encoder interface, its default
// configuration and the controls libavif sets.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The encoder interface. The generic `aom_codec_*()` layer of
//! aom_encoder.c and aom_codec.c only dispatches to these functions
//! through the interface table and keeps an error state; the saver calls
//! them directly (`aom_codec_enc_init()` is `encoder_init()`,
//! `aom_codec_control()` the `ctrl_*()` functions, `aom_codec_encode()`
//! `encoder_encode()`, `aom_codec_get_cx_data()` the returned packets and
//! `aom_codec_destroy()` the drop of the context).
//!
//! `validate_config()` and `validate_img()` are left out: they only reject
//! configurations outside the ranges the saver ever sets.

use super::common::av1_common_int::MAX_NUM_OPERATING_POINTS;
use super::common::enums::*;
use super::encoder::av1_quantize::av1_quantizer_to_qindex;
use super::encoder::bitstream::{
    av1_write_obu_header, av1_write_uleb_obu_size, OBU_TEMPORAL_DELIMITER,
};
use super::encoder::encode_strategy::AOM_EFLAG_FORCE_KF;
use super::encoder::encode_strategy::{timebase_units_to_ticks, TICKS_PER_SEC};
use super::encoder::encoder::*;
use super::encoder::encoder_utils::av1_select_sb_size;
use super::encoder::ethread::*;
use super::encoder::lookahead::*;
use super::encoder::ratectrl::MAX_STATIC_GF_GROUP_LENGTH;
use crate::aom::aom_integer::aom_uleb_size_in_bytes;
use crate::aom::aom_scale::yv12config::*;

/// `aom_codec_flags_t` `AOM_CODEC_USE_HIGHBITDEPTH`
pub(crate) const AOM_CODEC_USE_HIGHBITDEPTH: u32 = 0x40000;

/// `AOM_USAGE_REALTIME`
pub(crate) const AOM_USAGE_REALTIME: u32 = 1;
/// `AOM_USAGE_ALL_INTRA`
pub(crate) const AOM_USAGE_ALL_INTRA: u32 = 2;

/// `aom_kf_mode`
pub(crate) const AOM_KF_DISABLED: i32 = 0;
pub(crate) const AOM_KF_AUTO: i32 = 1;

/// `aom_img_fmt_t`
pub(crate) const AOM_IMG_FMT_HIGHBITDEPTH: u32 = 0x800;
pub(crate) const AOM_IMG_FMT_I420: u32 = 0x100 | 2;
pub(crate) const AOM_IMG_FMT_I422: u32 = 0x100 | 5;
pub(crate) const AOM_IMG_FMT_I444: u32 = 0x100 | 6;
pub(crate) const AOM_IMG_FMT_I42016: u32 = AOM_IMG_FMT_I420 | AOM_IMG_FMT_HIGHBITDEPTH;
pub(crate) const AOM_IMG_FMT_I42216: u32 = AOM_IMG_FMT_I422 | AOM_IMG_FMT_HIGHBITDEPTH;
pub(crate) const AOM_IMG_FMT_I44416: u32 = AOM_IMG_FMT_I444 | AOM_IMG_FMT_HIGHBITDEPTH;

/// `aom_color_range`
pub(crate) const AOM_CR_STUDIO_RANGE: i32 = 0;
pub(crate) const AOM_CR_FULL_RANGE: i32 = 1;

/// `AOM_CODEC_CX_FRAME_PKT`'s frame flag `AOM_FRAME_IS_KEY`
pub(crate) const AOM_FRAME_IS_KEY: u32 = 0x1;
const AOM_FRAME_IS_DROPPABLE: u32 = 0x2;
const AOM_FRAME_IS_INTRAONLY: u32 = 0x10;
const AOM_FRAME_IS_SWITCH: u32 = 0x20;
const AOM_FRAME_IS_ERROR_RESILIENT: u32 = 0x40;

/// The reached members of `aom_codec_enc_cfg_t`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AomCodecEncCfg {
    pub(crate) g_usage: u32,
    pub(crate) g_threads: u32,
    pub(crate) g_profile: u32,
    pub(crate) g_w: u32,
    pub(crate) g_h: u32,
    pub(crate) g_limit: u32,
    pub(crate) g_forced_max_frame_width: u32,
    pub(crate) g_forced_max_frame_height: u32,
    pub(crate) g_bit_depth: i32,
    pub(crate) g_input_bit_depth: u32,
    pub(crate) g_timebase: (i32, i32),
    pub(crate) g_error_resilient: u32,
    pub(crate) g_pass: i32,
    pub(crate) g_lag_in_frames: u32,
    pub(crate) rc_dropframe_thresh: u32,
    pub(crate) rc_resize_mode: u32,
    pub(crate) rc_resize_denominator: u32,
    pub(crate) rc_resize_kf_denominator: u32,
    pub(crate) rc_superres_mode: u32,
    pub(crate) rc_superres_denominator: u32,
    pub(crate) rc_superres_kf_denominator: u32,
    pub(crate) rc_superres_qthresh: u32,
    pub(crate) rc_superres_kf_qthresh: u32,
    pub(crate) rc_end_usage: i32,
    pub(crate) rc_target_bitrate: u32,
    pub(crate) rc_min_quantizer: u32,
    pub(crate) rc_max_quantizer: u32,
    pub(crate) rc_undershoot_pct: u32,
    pub(crate) rc_overshoot_pct: u32,
    pub(crate) rc_buf_sz: u32,
    pub(crate) rc_buf_initial_sz: u32,
    pub(crate) rc_buf_optimal_sz: u32,
    pub(crate) rc_2pass_vbr_bias_pct: u32,
    pub(crate) rc_2pass_vbr_minsection_pct: u32,
    pub(crate) rc_2pass_vbr_maxsection_pct: u32,
    pub(crate) fwd_kf_enabled: i32,
    pub(crate) kf_mode: i32,
    pub(crate) kf_min_dist: u32,
    pub(crate) kf_max_dist: u32,
    pub(crate) sframe_dist: u32,
    pub(crate) sframe_mode: u32,
    pub(crate) large_scale_tile: u32,
    pub(crate) monochrome: u32,
    pub(crate) full_still_picture_hdr: u32,
    pub(crate) save_as_annexb: u32,
    pub(crate) tile_width_count: i32,
    pub(crate) tile_height_count: i32,
    pub(crate) use_fixed_qp_offsets: u32,
}

/// The `encoder_usage_cfg[]` entry of `AOM_USAGE_REALTIME`.
fn realtime_usage_cfg() -> AomCodecEncCfg {
    AomCodecEncCfg {
        g_usage: AOM_USAGE_REALTIME, // g_usage - real-time usage
        g_threads: 0,                // g_threads
        g_profile: 0,                // g_profile

        g_w: 320,                     // g_w
        g_h: 240,                     // g_h
        g_limit: 0,                   // g_limit
        g_forced_max_frame_width: 0,  // g_forced_max_frame_width
        g_forced_max_frame_height: 0, // g_forced_max_frame_height
        g_bit_depth: AOM_BITS_8,      // g_bit_depth
        g_input_bit_depth: 8,         // g_input_bit_depth

        g_timebase: (1, 30), // g_timebase

        g_error_resilient: 0, // g_error_resilient

        g_pass: AOM_RC_ONE_PASS, // g_pass

        g_lag_in_frames: 0, // g_lag_in_frames

        rc_dropframe_thresh: 0,                        // rc_dropframe_thresh
        rc_resize_mode: RESIZE_NONE as u32,            // rc_resize_mode
        rc_resize_denominator: SCALE_NUMERATOR as u32, // rc_resize_denominator
        rc_resize_kf_denominator: SCALE_NUMERATOR as u32, // rc_resize_kf_denominator

        rc_superres_mode: AOM_SUPERRES_NONE as u32, // rc_superres_mode
        rc_superres_denominator: SCALE_NUMERATOR as u32, // rc_superres_denominator
        rc_superres_kf_denominator: SCALE_NUMERATOR as u32, // rc_superres_kf_denominator
        rc_superres_qthresh: 63,                    // rc_superres_qthresh
        rc_superres_kf_qthresh: 32,                 // rc_superres_kf_qthresh

        rc_end_usage: AOM_CBR,  // rc_end_usage
        rc_target_bitrate: 256, // rc_target_bitrate
        rc_min_quantizer: 0,    // rc_min_quantizer
        rc_max_quantizer: 63,   // rc_max_quantizer
        rc_undershoot_pct: 25,  // rc_undershoot_pct
        rc_overshoot_pct: 25,   // rc_overshoot_pct

        rc_buf_sz: 6000,         // rc_max_buffer_size
        rc_buf_initial_sz: 4000, // rc_buffer_initial_size
        rc_buf_optimal_sz: 5000, // rc_buffer_optimal_size

        rc_2pass_vbr_bias_pct: 50,         // rc_two_pass_vbrbias
        rc_2pass_vbr_minsection_pct: 0,    // rc_two_pass_vbrmin_section
        rc_2pass_vbr_maxsection_pct: 2000, // rc_two_pass_vbrmax_section

        // keyframing settings (kf)
        fwd_kf_enabled: 0,         // fwd_kf_enabled
        kf_mode: AOM_KF_AUTO,      // kf_mode
        kf_min_dist: 0,            // kf_min_dist
        kf_max_dist: 9999,         // kf_max_dist
        sframe_dist: 0,            // sframe_dist
        sframe_mode: 1,            // sframe_mode
        large_scale_tile: 0,       // large_scale_tile
        monochrome: 0,             // monochrome
        full_still_picture_hdr: 0, // full_still_picture_hdr
        save_as_annexb: 0,         // save_as_annexb
        tile_width_count: 0,       // tile_width_count
        tile_height_count: 0,      // tile_height_count
        use_fixed_qp_offsets: 0,   // use_fixed_qp_offsets
    }
}

/// The `encoder_usage_cfg[]` entry of `AOM_USAGE_ALL_INTRA`.
fn all_intra_usage_cfg() -> AomCodecEncCfg {
    AomCodecEncCfg {
        g_usage: AOM_USAGE_ALL_INTRA, // g_usage - all intra usage
        g_threads: 0,                 // g_threads
        g_profile: 0,                 // g_profile

        g_w: 320,                     // g_w
        g_h: 240,                     // g_h
        g_limit: 0,                   // g_limit
        g_forced_max_frame_width: 0,  // g_forced_max_frame_width
        g_forced_max_frame_height: 0, // g_forced_max_frame_height
        g_bit_depth: AOM_BITS_8,      // g_bit_depth
        g_input_bit_depth: 8,         // g_input_bit_depth

        g_timebase: (1, 30), // g_timebase

        g_error_resilient: 0, // g_error_resilient

        g_pass: AOM_RC_ONE_PASS, // g_pass

        g_lag_in_frames: 0, // g_lag_in_frames

        rc_dropframe_thresh: 0,                        // rc_dropframe_thresh
        rc_resize_mode: RESIZE_NONE as u32,            // rc_resize_mode
        rc_resize_denominator: SCALE_NUMERATOR as u32, // rc_resize_denominator
        rc_resize_kf_denominator: SCALE_NUMERATOR as u32, // rc_resize_kf_denominator

        rc_superres_mode: AOM_SUPERRES_NONE as u32, // rc_superres_mode
        rc_superres_denominator: SCALE_NUMERATOR as u32, // rc_superres_denominator
        rc_superres_kf_denominator: SCALE_NUMERATOR as u32, // rc_superres_kf_denominator
        rc_superres_qthresh: 63,                    // rc_superres_qthresh
        rc_superres_kf_qthresh: 32,                 // rc_superres_kf_qthresh

        rc_end_usage: AOM_Q,    // rc_end_usage
        rc_target_bitrate: 256, // rc_target_bitrate
        rc_min_quantizer: 0,    // rc_min_quantizer
        rc_max_quantizer: 63,   // rc_max_quantizer
        rc_undershoot_pct: 25,  // rc_undershoot_pct
        rc_overshoot_pct: 25,   // rc_overshoot_pct

        rc_buf_sz: 6000,         // rc_max_buffer_size
        rc_buf_initial_sz: 4000, // rc_buffer_initial_size
        rc_buf_optimal_sz: 5000, // rc_buffer_optimal_size

        rc_2pass_vbr_bias_pct: 50,         // rc_two_pass_vbrbias
        rc_2pass_vbr_minsection_pct: 0,    // rc_two_pass_vbrmin_section
        rc_2pass_vbr_maxsection_pct: 2000, // rc_two_pass_vbrmax_section

        // keyframing settings (kf)
        fwd_kf_enabled: 0,         // fwd_kf_enabled
        kf_mode: AOM_KF_DISABLED,  // kf_mode
        kf_min_dist: 0,            // kf_min_dist
        kf_max_dist: 0,            // kf_max_dist
        sframe_dist: 0,            // sframe_dist
        sframe_mode: 1,            // sframe_mode
        large_scale_tile: 0,       // large_scale_tile
        monochrome: 0,             // monochrome
        full_still_picture_hdr: 0, // full_still_picture_hdr
        save_as_annexb: 0,         // save_as_annexb
        tile_width_count: 0,       // tile_width_count
        tile_height_count: 0,      // tile_height_count
        use_fixed_qp_offsets: 0,   // use_fixed_qp_offsets
    }
}

/// Translation of `aom_codec_enc_config_default()` (aom_encoder.c) for the
/// real-time and all intra usages (`None` for the good quality usage,
/// which libavif never asks for SDL_image).
pub(crate) fn aom_codec_enc_config_default(usage: u32) -> Option<AomCodecEncCfg> {
    match usage {
        AOM_USAGE_REALTIME => Some(realtime_usage_cfg()),
        AOM_USAGE_ALL_INTRA => Some(all_intra_usage_cfg()),
        // (the good quality usage is not translated)
        _ => None,
    }
}

/// The reached members of `struct av1_extracfg`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Av1ExtraCfg {
    pub(crate) cpu_used: i32,
    pub(crate) enable_auto_alt_ref: u32,
    pub(crate) enable_auto_bwd_ref: u32,
    pub(crate) sharpness: u32,
    pub(crate) row_mt: u32,
    pub(crate) fp_mt: u32,
    pub(crate) tile_columns: u32,
    pub(crate) tile_rows: u32,
    pub(crate) enable_tpl_model: u32,
    pub(crate) enable_keyframe_filtering: u32,
    pub(crate) arnr_max_frames: u32,
    pub(crate) arnr_strength: u32,
    pub(crate) min_gf_interval: u32,
    pub(crate) max_gf_interval: u32,
    pub(crate) gf_min_pyr_height: u32,
    pub(crate) gf_max_pyr_height: u32,
    pub(crate) tuning: i32,
    pub(crate) dist_metric: i32,
    pub(crate) cq_level: u32, // constrained quality level
    pub(crate) rc_max_intra_bitrate_pct: u32,
    pub(crate) rc_max_inter_bitrate_pct: u32,
    pub(crate) gf_cbr_boost_pct: u32,
    pub(crate) lossless: u32,
    pub(crate) enable_cdef: u32,
    pub(crate) enable_restoration: u32,
    pub(crate) force_video_mode: u32,
    pub(crate) enable_obmc: u32,
    pub(crate) disable_trellis_quant: u32,
    pub(crate) enable_qm: u32,
    pub(crate) qm_min: u32,
    pub(crate) qm_max: u32,
    pub(crate) num_tg: u32,
    pub(crate) mtu_size: u32,
    pub(crate) timing_info_type: i32,
    pub(crate) frame_parallel_decoding_mode: u32,
    pub(crate) enable_dual_filter: i32,
    pub(crate) enable_chroma_deltaq: u32,
    pub(crate) aq_mode: i32,
    pub(crate) deltaq_mode: i32,
    pub(crate) deltaq_strength: i32,
    pub(crate) deltalf_mode: i32,
    pub(crate) content: i32,
    pub(crate) color_primaries: i32,
    pub(crate) transfer_characteristics: i32,
    pub(crate) matrix_coefficients: i32,
    pub(crate) chroma_sample_position: i32,
    pub(crate) color_range: i32,
    pub(crate) render_width: i32,
    pub(crate) render_height: i32,
    pub(crate) superblock_size: i32,
    pub(crate) single_tile_decoding: u32,
    pub(crate) error_resilient_mode: i32,
    pub(crate) s_frame_mode: i32,
    pub(crate) film_grain_test_vector: i32,
    pub(crate) cdf_update_mode: u32,
    pub(crate) enable_rect_partitions: i32,
    pub(crate) enable_ab_partitions: i32,
    pub(crate) enable_1to4_partitions: i32,
    pub(crate) min_partition_size: i32,
    pub(crate) max_partition_size: i32,
    pub(crate) enable_intra_edge_filter: i32,
    pub(crate) enable_order_hint: i32,
    pub(crate) enable_tx64: i32,
    pub(crate) enable_flip_idtx: i32,
    pub(crate) enable_rect_tx: i32,
    pub(crate) enable_dist_wtd_comp: i32,
    pub(crate) max_reference_frames: i32,
    pub(crate) enable_reduced_reference_set: i32,
    pub(crate) enable_ref_frame_mvs: i32,
    pub(crate) allow_ref_frame_mvs: i32,
    pub(crate) enable_masked_comp: i32,
    pub(crate) enable_onesided_comp: i32,
    pub(crate) enable_interintra_comp: i32,
    pub(crate) enable_smooth_interintra: i32,
    pub(crate) enable_diff_wtd_comp: i32,
    pub(crate) enable_interinter_wedge: i32,
    pub(crate) enable_interintra_wedge: i32,
    pub(crate) enable_global_motion: i32,
    pub(crate) enable_warped_motion: i32,
    pub(crate) allow_warped_motion: i32,
    pub(crate) enable_filter_intra: i32,
    pub(crate) enable_smooth_intra: i32,
    pub(crate) enable_paeth_intra: i32,
    pub(crate) enable_cfl_intra: i32,
    pub(crate) enable_directional_intra: i32,
    pub(crate) enable_diagonal_intra: i32,
    pub(crate) enable_superres: i32,
    pub(crate) enable_overlay: i32,
    pub(crate) enable_palette: i32,
    pub(crate) enable_intrabc: i32,
    pub(crate) enable_angle_delta: i32,
    pub(crate) noise_level: f32,
    pub(crate) chroma_subsampling_x: u32,
    pub(crate) chroma_subsampling_y: u32,
    pub(crate) reduced_tx_type_set: i32,
    pub(crate) use_intra_dct_only: i32,
    pub(crate) use_inter_dct_only: i32,
    pub(crate) use_intra_default_tx_only: i32,
    pub(crate) enable_tx_size_search: i32,
    pub(crate) quant_b_adapt: i32,
    pub(crate) vbr_corpus_complexity_lap: u32,
    pub(crate) target_seq_level_idx: [Av1Level; MAX_NUM_OPERATING_POINTS],
    pub(crate) tier_mask: u32,
    pub(crate) min_cr: u32,
    pub(crate) coeff_cost_upd_freq: i32,
    pub(crate) mode_cost_upd_freq: i32,
    pub(crate) mv_cost_upd_freq: i32,
    pub(crate) dv_cost_upd_freq: i32,
    pub(crate) ext_tile_debug: u32,
    pub(crate) passes: i32,
    pub(crate) fwd_kf_dist: i32,
    pub(crate) loopfilter_control: i32,
    pub(crate) skip_postproc_filtering: u32,
    pub(crate) auto_intra_tools_off: u32,
    pub(crate) strict_level_conformance: u32,
    pub(crate) kf_max_pyr_height: i32,
    pub(crate) sb_qp_sweep: i32,
}

/// `AOM_TIMING_UNSPECIFIED`
const AOM_TIMING_UNSPECIFIED: i32 = 0;
const AOM_TIMING_EQUAL: i32 = 1;
const AOM_TIMING_DEC_MODEL: i32 = 2;
/// `AOM_CSP_UNKNOWN`
const AOM_CSP_UNKNOWN: i32 = 0;

/// `default_extra_cfg` (the build is not real-time only).
const DEFAULT_EXTRA_CFG: Av1ExtraCfg = Av1ExtraCfg {
    cpu_used: 0,                  // cpu_used
    enable_auto_alt_ref: 1,       // enable_auto_alt_ref
    enable_auto_bwd_ref: 0,       // enable_auto_bwd_ref
    sharpness: 0,                 // sharpness
    row_mt: 1,                    // row_mt
    fp_mt: 0,                     // fp_mt
    tile_columns: 0,              // tile_columns
    tile_rows: 0,                 // tile_rows
    enable_tpl_model: 1,          // enable_tpl_model
    enable_keyframe_filtering: 1, // enable_keyframe_filtering
    arnr_max_frames: 7,           // arnr_max_frames
    arnr_strength: 5,             // arnr_strength
    min_gf_interval: 0,           // min_gf_interval; 0 -> default decision
    max_gf_interval: 0,           // max_gf_interval; 0 -> default decision
    gf_min_pyr_height: 0,         // gf_min_pyr_height
    gf_max_pyr_height: 5,         // gf_max_pyr_height
    tuning: AOM_TUNE_PSNR,        // tuning
    // (VMAF model path and partition info path: unused)
    dist_metric: AOM_DIST_METRIC_PSNR, // dist_metric
    cq_level: 10,                      // cq_level
    rc_max_intra_bitrate_pct: 0,       // rc_max_intra_bitrate_pct
    rc_max_inter_bitrate_pct: 0,       // rc_max_inter_bitrate_pct
    gf_cbr_boost_pct: 0,               // gf_cbr_boost_pct
    lossless: 0,                       // lossless
    enable_cdef: 1,                    // enable_cdef
    enable_restoration: 1,             // enable_restoration
    force_video_mode: 0,               // force_video_mode
    enable_obmc: 1,                    // enable_obmc
    disable_trellis_quant: 3,          // disable_trellis_quant
    enable_qm: 0,                      // enable_qm
    // (qm_y, qm_u, qm_v: DEFAULT_QM_Y, DEFAULT_QM_U, DEFAULT_QM_V)
    qm_min: 5,                                // qm_min (DEFAULT_QM_FIRST)
    qm_max: 9,                                // qm_max (DEFAULT_QM_LAST)
    num_tg: 1,                                // max number of tile groups
    mtu_size: 0,                              // mtu_size
    timing_info_type: AOM_TIMING_UNSPECIFIED, // No picture timing signaling in bitstream
    frame_parallel_decoding_mode: 0,          // frame_parallel_decoding_mode
    enable_dual_filter: 1,                    // enable dual filter
    enable_chroma_deltaq: 0,                  // enable delta quant in chroma planes
    aq_mode: NO_AQ,                           // aq_mode
    deltaq_mode: DELTA_Q_OBJECTIVE,           // deltaq_mode
    deltaq_strength: 100,                     // deltaq_strength
    deltalf_mode: 0,                          // delta lf mode
    // (frame_periodic_boost: 0; the bit depth: AOM_BITS_8)
    content: AOM_CONTENT_DEFAULT,                      // content
    color_primaries: AOM_CICP_CP_UNSPECIFIED,          // CICP color primaries
    transfer_characteristics: AOM_CICP_TC_UNSPECIFIED, // CICP transfer characteristics
    matrix_coefficients: AOM_CICP_MC_UNSPECIFIED,      // CICP matrix coefficients
    chroma_sample_position: AOM_CSP_UNKNOWN,           // chroma sample position
    color_range: 0,                                    // color range
    render_width: 0,                                   // render width
    render_height: 0,                                  // render height
    superblock_size: AOM_SUPERBLOCK_SIZE_DYNAMIC,      // superblock_size
    single_tile_decoding: 1,                           // this depends on large_scale_tile.
    error_resilient_mode: 0,                           // error_resilient_mode off by default.
    s_frame_mode: 0,                                   // s_frame_mode off by default.
    film_grain_test_vector: 0,                         // film_grain_test_vector
    // (film_grain_table_filename: NULL; motion_vector_unit_test: 0)
    cdf_update_mode: 1,              // CDF update mode
    enable_rect_partitions: 1,       // enable rectangular partitions
    enable_ab_partitions: 1,         // enable ab shape partitions
    enable_1to4_partitions: 1,       // enable 1:4 and 4:1 partitions
    min_partition_size: 4,           // min_partition_size
    max_partition_size: 128,         // max_partition_size
    enable_intra_edge_filter: 1,     // enable intra edge filter
    enable_order_hint: 1,            // frame order hint
    enable_tx64: 1,                  // enable 64-pt transform usage
    enable_flip_idtx: 1,             // enable flip and identity transform
    enable_rect_tx: 1,               // enable rectangular transform usage
    enable_dist_wtd_comp: 1,         // dist-wtd compound
    max_reference_frames: 7,         // max_reference_frames
    enable_reduced_reference_set: 0, // enable_reduced_reference_set
    enable_ref_frame_mvs: 1,         // enable_ref_frame_mvs sequence level
    allow_ref_frame_mvs: 1,          // allow ref_frame_mvs frame level
    enable_masked_comp: 1,           // enable masked compound at sequence level
    enable_onesided_comp: 1,         // enable one sided compound at sequence level
    enable_interintra_comp: 1,       // enable interintra compound at sequence level
    enable_smooth_interintra: 1,     // enable smooth interintra mode
    enable_diff_wtd_comp: 1,         // enable difference-weighted compound
    enable_interinter_wedge: 1,      // enable interinter wedge compound
    enable_interintra_wedge: 1,      // enable interintra wedge compound
    enable_global_motion: 1,         // enable_global_motion usage
    enable_warped_motion: 1,         // enable_warped_motion at sequence level
    allow_warped_motion: 1,          // allow_warped_motion at frame level
    enable_filter_intra: 1,          // enable filter intra at sequence level
    enable_smooth_intra: 1,          // enable smooth intra modes usage for sequence
    enable_paeth_intra: 1,           // enable Paeth intra mode usage for sequence
    enable_cfl_intra: 1,             // enable CFL uv intra mode usage for sequence
    enable_directional_intra: 1,     // enable directional intra mode usage for sequence
    enable_diagonal_intra: 1,        // enable D45 to D203 intra mode usage for sequence
    enable_superres: 1,              // superres
    enable_overlay: 1,               // enable overlay
    enable_palette: 1,               // enable palette
    enable_intrabc: 1,               // enable intrabc
    enable_angle_delta: 1,           // enable angle delta
    noise_level: 0.0,                // noise_level (CONFIG_DENOISE)
    // (noise_block_size: 32; enable_dnl_denoising: 1)
    chroma_subsampling_x: 0,      // chroma_subsampling_x
    chroma_subsampling_y: 0,      // chroma_subsampling_y
    reduced_tx_type_set: 0,       // reduced_tx_type_set
    use_intra_dct_only: 0,        // use_intra_dct_only
    use_inter_dct_only: 0,        // use_inter_dct_only
    use_intra_default_tx_only: 0, // use_intra_default_tx_only
    enable_tx_size_search: 1,     // enable_tx_size_search
    quant_b_adapt: 0,             // quant_b_adapt
    vbr_corpus_complexity_lap: 0, // vbr_corpus_complexity_lap
    target_seq_level_idx: [SEQ_LEVEL_MAX; MAX_NUM_OPERATING_POINTS], // target_seq_level_idx
    tier_mask: 0,                 // tier_mask
    min_cr: 0,                    // min_cr
    coeff_cost_upd_freq: COST_UPD_SB, // coeff_cost_upd_freq
    mode_cost_upd_freq: COST_UPD_SB, // mode_cost_upd_freq
    mv_cost_upd_freq: COST_UPD_SB, // mv_cost_upd_freq
    dv_cost_upd_freq: COST_UPD_SB, // dv_cost_upd_freq
    ext_tile_debug: 0,            // ext_tile_debug
    // (sb_multipass_unit_test: 0)
    passes: -1,                         // passes
    fwd_kf_dist: -1,                    // fwd_kf_dist
    loopfilter_control: LOOPFILTER_ALL, // loopfilter_control
    skip_postproc_filtering: 0,         // skip_postproc_filtering
    // (two_pass_output, second_pass_log: NULL)
    auto_intra_tools_off: 0,     // auto_intra_tools_off
    strict_level_conformance: 0, // strict_level_conformance
    kf_max_pyr_height: -1,       // kf_max_pyr_height
    sb_qp_sweep: 0,              // sb_qp_sweep
};

/// `struct aom_codec_alg_priv`
#[derive(Debug)]
pub(crate) struct AomCodecAlgPriv {
    pub(crate) cfg: AomCodecEncCfg,
    pub(crate) extra_cfg: Av1ExtraCfg,
    /// `{ num, den }`
    pub(crate) timestamp_ratio: (i64, i64),
    pub(crate) pts_offset: i64,
    pub(crate) pts_offset_initialized: bool,
    pub(crate) oxcf: Av1EncoderConfig,
    /// `ppi->cpi` (with `ppi` in it)
    pub(crate) cpi: Box<Av1Comp>,
    pub(crate) cx_data: Vec<u8>,
    pub(crate) pending_cx_data_sz: usize,
    pub(crate) next_frame_flags: u32,
    pub(crate) init_flags: u32,
    pub(crate) fixed_kf_cntr: u32,
}

/// One `AOM_CODEC_CX_FRAME_PKT` packet.
#[derive(Debug)]
pub(crate) struct AomCodecCxFramePkt {
    pub(crate) buf: Vec<u8>,
    pub(crate) pts: i64,
    pub(crate) flags: u32,
}

/// Translation of `gcd()` and `reduce_ratio()`.
fn reduce_ratio(ratio: &mut (i64, i64)) {
    fn gcd(mut a: i64, mut b: i64) -> i64 {
        while b > 0 {
            let r = a % b;
            a = b;
            b = r;
        }
        a
    }
    let denom = gcd(ratio.0, ratio.1);
    ratio.0 /= denom;
    ratio.1 /= denom;
}

/// Translation of `av1_get_image_bps()`.
pub(crate) fn av1_get_image_bps(img: &AomImage) -> u32 {
    match img.fmt {
        AOM_IMG_FMT_I420 => 12,
        AOM_IMG_FMT_I422 => 16,
        AOM_IMG_FMT_I444 => 24,
        AOM_IMG_FMT_I42016 => 24,
        AOM_IMG_FMT_I42216 => 32,
        AOM_IMG_FMT_I44416 => 48,
        _ => {
            debug_assert!(false, "Invalid image format");
            0
        }
    }
}

/// Translation of `disable_superres()`.
fn disable_superres(superres_cfg: &mut SuperResCfg) {
    superres_cfg.superres_mode = AOM_SUPERRES_NONE;
    superres_cfg.superres_scale_denominator = SCALE_NUMERATOR as u8;
    superres_cfg.superres_kf_scale_denominator = SCALE_NUMERATOR as u8;
    superres_cfg.superres_qthresh = 255;
    superres_cfg.superres_kf_qthresh = 255;
}

/// `MAX_TILE_COLS`, `MAX_TILE_ROWS`
const MAX_TILE_COLS_: i32 = 64;
const MAX_TILE_ROWS_: i32 = 64;

/// Translation of `set_encoder_config()` (no configuration file, one pass,
/// no tile sizes).
fn set_encoder_config(oxcf: &mut Av1EncoderConfig, cfg: &AomCodecEncCfg, extra_cfg: &Av1ExtraCfg) {
    let is_vbr = cfg.rc_end_usage == AOM_VBR;
    oxcf.profile = cfg.g_profile as i32;
    oxcf.max_threads = cfg.g_threads as i32;

    oxcf.mode = match cfg.g_usage {
        1 => REALTIME, // AOM_USAGE_REALTIME
        AOM_USAGE_ALL_INTRA => ALLINTRA,
        _ => GOOD,
    };

    // Set frame-dimension related configuration.
    let frm_dim_cfg = &mut oxcf.frm_dim_cfg;
    frm_dim_cfg.width = cfg.g_w as i32;
    frm_dim_cfg.height = cfg.g_h as i32;
    frm_dim_cfg.forced_max_frame_width = cfg.g_forced_max_frame_width as i32;
    frm_dim_cfg.forced_max_frame_height = cfg.g_forced_max_frame_height as i32;
    frm_dim_cfg.render_width = extra_cfg.render_width;
    frm_dim_cfg.render_height = extra_cfg.render_height;

    // Set input video related configuration.
    let input_cfg = &mut oxcf.input_cfg;
    input_cfg.input_bit_depth = cfg.g_input_bit_depth as i32;
    // guess a frame rate if out of whack, use 30
    input_cfg.init_framerate = cfg.g_timebase.1 as f64 / cfg.g_timebase.0 as f64;
    debug_assert!(cfg.g_pass < AOM_RC_SECOND_PASS);
    input_cfg.limit = cfg.g_limit;
    input_cfg.chroma_subsampling_x = extra_cfg.chroma_subsampling_x;
    input_cfg.chroma_subsampling_y = extra_cfg.chroma_subsampling_y;
    if input_cfg.init_framerate > 180.0 {
        input_cfg.init_framerate = 30.0;
        oxcf.dec_model_cfg.timing_info_present = false;
    }

    // Set Decoder model configuration.
    debug_assert!(
        extra_cfg.timing_info_type != AOM_TIMING_EQUAL
            && extra_cfg.timing_info_type != AOM_TIMING_DEC_MODEL
    );
    oxcf.dec_model_cfg.timing_info_present = false;

    oxcf.pass = cfg.g_pass;
    // For backward compatibility, assume that if extra_cfg->passes==-1, then
    // passes = 1 or 2.
    if extra_cfg.passes == -1 {
        if cfg.g_pass == AOM_RC_ONE_PASS {
            oxcf.passes = 1;
        } else {
            oxcf.passes = 2;
        }
    } else {
        oxcf.passes = extra_cfg.passes;
    }

    // Set Rate Control configuration.
    let rc_cfg = &mut oxcf.rc_cfg;
    rc_cfg.max_intra_bitrate_pct = extra_cfg.rc_max_intra_bitrate_pct;
    rc_cfg.max_inter_bitrate_pct = extra_cfg.rc_max_inter_bitrate_pct;
    rc_cfg.gf_cbr_boost_pct = extra_cfg.gf_cbr_boost_pct;
    rc_cfg.mode = cfg.rc_end_usage;
    rc_cfg.min_cr = extra_cfg.min_cr;
    rc_cfg.best_allowed_q = if extra_cfg.lossless != 0 {
        0
    } else {
        av1_quantizer_to_qindex(cfg.rc_min_quantizer as i32)
    };
    rc_cfg.worst_allowed_q = if extra_cfg.lossless != 0 {
        0
    } else {
        av1_quantizer_to_qindex(cfg.rc_max_quantizer as i32)
    };
    rc_cfg.cq_level = av1_quantizer_to_qindex(extra_cfg.cq_level as i32);
    rc_cfg.under_shoot_pct = cfg.rc_undershoot_pct as i32;
    rc_cfg.over_shoot_pct = cfg.rc_overshoot_pct as i32;
    rc_cfg.maximum_buffer_size_ms = if is_vbr { 240000 } else { cfg.rc_buf_sz as i64 };
    rc_cfg.starting_buffer_level_ms = if is_vbr {
        60000
    } else {
        cfg.rc_buf_initial_sz as i64
    };
    rc_cfg.optimal_buffer_level_ms = if is_vbr {
        60000
    } else {
        cfg.rc_buf_optimal_sz as i64
    };
    // Convert target bandwidth from Kbit/s to Bit/s
    rc_cfg.target_bandwidth = 1000 * cfg.rc_target_bitrate as i64;
    rc_cfg.drop_frames_water_mark = cfg.rc_dropframe_thresh as i32;
    rc_cfg.vbr_corpus_complexity_lap = extra_cfg.vbr_corpus_complexity_lap;
    rc_cfg.vbrbias = cfg.rc_2pass_vbr_bias_pct as i32;
    rc_cfg.vbrmin_section = cfg.rc_2pass_vbr_minsection_pct as i32;
    rc_cfg.vbrmax_section = cfg.rc_2pass_vbr_maxsection_pct as i32;

    // Set Toolset related configuration.
    let tool_cfg = &mut oxcf.tool_cfg;
    tool_cfg.bit_depth = cfg.g_bit_depth;
    tool_cfg.cdef_control = extra_cfg.enable_cdef as i32;
    tool_cfg.enable_restoration = if cfg.g_usage == 1 {
        false
    } else {
        extra_cfg.enable_restoration != 0
    };
    tool_cfg.force_video_mode = extra_cfg.force_video_mode != 0;
    tool_cfg.enable_palette = extra_cfg.enable_palette != 0;
    // FIXME(debargha): Should this be:
    // tool_cfg->enable_ref_frame_mvs  = extra_cfg->allow_ref_frame_mvs &
    //                                         extra_cfg->enable_order_hint ?
    // Disallow using temporal MVs while large_scale_tile = 1.
    tool_cfg.enable_ref_frame_mvs = extra_cfg.allow_ref_frame_mvs != 0 && cfg.large_scale_tile == 0;
    tool_cfg.superblock_size = extra_cfg.superblock_size;
    tool_cfg.enable_monochrome = cfg.monochrome != 0;
    tool_cfg.full_still_picture_hdr = cfg.full_still_picture_hdr != 0;
    tool_cfg.enable_dual_filter = extra_cfg.enable_dual_filter != 0;
    tool_cfg.enable_order_hint = extra_cfg.enable_order_hint != 0;
    tool_cfg.enable_interintra_comp = extra_cfg.enable_interintra_comp != 0;
    tool_cfg.ref_frame_mvs_present =
        (extra_cfg.enable_ref_frame_mvs & extra_cfg.enable_order_hint) != 0;
    tool_cfg.enable_global_motion = extra_cfg.enable_global_motion != 0;
    tool_cfg.error_resilient_mode =
        cfg.g_error_resilient != 0 || extra_cfg.error_resilient_mode != 0;
    tool_cfg.frame_parallel_decoding_mode = extra_cfg.frame_parallel_decoding_mode != 0;

    // Set Quantization related configuration.
    let q_cfg = &mut oxcf.q_cfg;
    q_cfg.using_qm = extra_cfg.enable_qm != 0;
    q_cfg.qm_minlevel = extra_cfg.qm_min as i32;
    q_cfg.qm_maxlevel = extra_cfg.qm_max as i32;
    q_cfg.quant_b_adapt = extra_cfg.quant_b_adapt != 0;
    q_cfg.enable_chroma_deltaq = extra_cfg.enable_chroma_deltaq as i32;
    q_cfg.aq_mode = extra_cfg.aq_mode;
    q_cfg.deltaq_mode = extra_cfg.deltaq_mode;
    q_cfg.deltaq_strength = extra_cfg.deltaq_strength;
    q_cfg.use_fixed_qp_offsets = (cfg.use_fixed_qp_offsets != 0 && rc_cfg.mode == AOM_Q) as i32;
    q_cfg.enable_hdr_deltaq = (q_cfg.deltaq_mode == DELTA_Q_HDR
        && cfg.g_bit_depth == AOM_BITS_10
        && extra_cfg.color_primaries == AOM_CICP_CP_BT_2020) as i32;

    tool_cfg.enable_deltalf_mode = q_cfg.deltaq_mode != NO_DELTA_Q && extra_cfg.deltalf_mode != 0;

    // Set cost update frequency configuration.
    oxcf.cost_upd_freq.coeff = extra_cfg.coeff_cost_upd_freq;
    oxcf.cost_upd_freq.mode = extra_cfg.mode_cost_upd_freq;
    // Avoid MV cost update for allintra encoding mode.
    oxcf.cost_upd_freq.mv = if cfg.kf_max_dist != 0 {
        extra_cfg.mv_cost_upd_freq
    } else {
        COST_UPD_OFF
    };
    oxcf.cost_upd_freq.dv = extra_cfg.dv_cost_upd_freq;

    // Set frame resize mode configuration.
    let resize_cfg = &mut oxcf.resize_cfg;
    resize_cfg.resize_mode = cfg.rc_resize_mode as i32;
    resize_cfg.resize_scale_denominator = cfg.rc_resize_denominator as u8;
    resize_cfg.resize_kf_scale_denominator = cfg.rc_resize_kf_denominator as u8;
    debug_assert!(resize_cfg.resize_mode == RESIZE_NONE);

    // Set encoder algorithm related configuration.
    let algo_cfg = &mut oxcf.algo_cfg;
    algo_cfg.enable_overlay = extra_cfg.enable_overlay != 0;
    algo_cfg.disable_trellis_quant = extra_cfg.disable_trellis_quant as i32;
    algo_cfg.sharpness = extra_cfg.sharpness as i32;
    algo_cfg.arnr_max_frames = extra_cfg.arnr_max_frames as i32;
    algo_cfg.arnr_strength = extra_cfg.arnr_strength as i32;
    algo_cfg.cdf_update_mode = extra_cfg.cdf_update_mode as u8;
    // TODO(any): Fix and Enable TPL for resize-mode > 0
    algo_cfg.enable_tpl_model = if resize_cfg.resize_mode != 0 {
        false
    } else {
        extra_cfg.enable_tpl_model != 0
    };
    algo_cfg.loopfilter_control = extra_cfg.loopfilter_control;
    algo_cfg.skip_postproc_filtering = extra_cfg.skip_postproc_filtering != 0;

    // Set two-pass stats configuration.
    // (no stats)

    // Set Key frame configuration.
    let kf_cfg = &mut oxcf.kf_cfg;
    kf_cfg.fwd_kf_enabled = cfg.fwd_kf_enabled != 0;
    kf_cfg.auto_key = cfg.kf_mode == AOM_KF_AUTO && cfg.kf_min_dist != cfg.kf_max_dist;
    kf_cfg.key_freq_min = cfg.kf_min_dist as i32;
    kf_cfg.key_freq_max = cfg.kf_max_dist as i32;
    kf_cfg.sframe_dist = cfg.sframe_dist as i32;
    kf_cfg.sframe_mode = cfg.sframe_mode as i32;
    kf_cfg.enable_sframe = extra_cfg.s_frame_mode != 0;
    kf_cfg.enable_keyframe_filtering = extra_cfg.enable_keyframe_filtering as i32;
    kf_cfg.fwd_kf_dist = extra_cfg.fwd_kf_dist;
    // Disable key frame filtering in all intra mode.
    if cfg.kf_max_dist == 0 {
        kf_cfg.enable_keyframe_filtering = 0;
    }
    kf_cfg.enable_intrabc = extra_cfg.enable_intrabc != 0;

    oxcf.speed = extra_cfg.cpu_used;
    // TODO(yunqingwang, any) In REALTIME mode, 1080p performance at speed 5 & 6
    // is quite bad. Force to use speed 7 for now. Will investigate it when we
    // work on rd path optimization later.
    if oxcf.mode == REALTIME && cfg.g_w.min(cfg.g_h) >= 1080 && oxcf.speed < 7 {
        oxcf.speed = 7;
    }

    // Set Color related configuration.
    let color_cfg = &mut oxcf.color_cfg;
    color_cfg.color_primaries = extra_cfg.color_primaries;
    color_cfg.transfer_characteristics = extra_cfg.transfer_characteristics;
    color_cfg.matrix_coefficients = extra_cfg.matrix_coefficients;
    color_cfg.color_range = extra_cfg.color_range;
    color_cfg.chroma_sample_position = extra_cfg.chroma_sample_position;

    // Set Group of frames configuration.
    // Force lag_in_frames to 0 for REALTIME mode
    let gf_cfg = &mut oxcf.gf_cfg;
    gf_cfg.lag_in_frames = if oxcf.mode == REALTIME {
        0
    } else {
        (cfg.g_lag_in_frames as i32).clamp(0, MAX_LAG_BUFFERS)
    };
    gf_cfg.enable_auto_arf = extra_cfg.enable_auto_alt_ref != 0;
    gf_cfg.enable_auto_brf = extra_cfg.enable_auto_bwd_ref != 0;
    gf_cfg.min_gf_interval = extra_cfg.min_gf_interval as i32;
    gf_cfg.max_gf_interval = extra_cfg.max_gf_interval as i32;
    gf_cfg.gf_min_pyr_height = extra_cfg.gf_min_pyr_height as i32;
    gf_cfg.gf_max_pyr_height = extra_cfg.gf_max_pyr_height as i32;

    // Set tune related configuration.
    let tune_cfg = &mut oxcf.tune_cfg;
    tune_cfg.tuning = extra_cfg.tuning;
    tune_cfg.content = extra_cfg.content;
    if cfg.large_scale_tile != 0 {
        tune_cfg.film_grain_test_vector = 0;
    } else {
        tune_cfg.film_grain_test_vector = extra_cfg.film_grain_test_vector;
    }
    tune_cfg.dist_metric = extra_cfg.dist_metric;
    // (CONFIG_DENOISE)
    oxcf.noise_level = extra_cfg.noise_level;

    // Set Tile related configuration.
    let tile_cfg = &mut oxcf.tile_cfg;
    tile_cfg.num_tile_groups = extra_cfg.num_tg as i32;
    // In large-scale tile encoding mode, num_tile_groups is always 1.
    if cfg.large_scale_tile != 0 {
        tile_cfg.num_tile_groups = 1;
    }
    tile_cfg.mtu = extra_cfg.mtu_size;
    tile_cfg.enable_large_scale_tile = cfg.large_scale_tile != 0;
    tile_cfg.enable_single_tile_decoding = if tile_cfg.enable_large_scale_tile {
        extra_cfg.single_tile_decoding != 0
    } else {
        false
    };
    tile_cfg.tile_columns = extra_cfg.tile_columns as i32;
    tile_cfg.tile_rows = extra_cfg.tile_rows as i32;
    tile_cfg.tile_width_count = cfg.tile_width_count.min(MAX_TILE_COLS_);
    tile_cfg.tile_height_count = cfg.tile_height_count.min(MAX_TILE_ROWS_);
    debug_assert!(tile_cfg.tile_width_count == 0 && tile_cfg.tile_height_count == 0);
    tile_cfg.enable_ext_tile_debug = extra_cfg.ext_tile_debug != 0;
    debug_assert!(!tile_cfg.enable_large_scale_tile);

    // Set reference frame related configuration.
    oxcf.ref_frm_cfg.max_reference_frames = extra_cfg.max_reference_frames;
    oxcf.ref_frm_cfg.enable_reduced_reference_set = extra_cfg.enable_reduced_reference_set != 0;
    oxcf.ref_frm_cfg.enable_onesided_comp = extra_cfg.enable_onesided_comp != 0;

    oxcf.row_mt = extra_cfg.row_mt != 0;
    oxcf.fp_mt = extra_cfg.fp_mt != 0;

    // Set motion mode related configuration.
    oxcf.motion_mode_cfg.enable_obmc = extra_cfg.enable_obmc != 0;
    oxcf.motion_mode_cfg.enable_warped_motion = extra_cfg.enable_warped_motion != 0;
    if cfg.g_usage == 1 && oxcf.speed >= 7 && oxcf.tune_cfg.content == AOM_CONTENT_SCREEN {
        // TODO(marpan): warped motion is causing a crash for RT mode with screen
        // in nonrd (speed >= 7), for non-realtime build.
        // Re-enable/allow when the issue is fixed.
        oxcf.motion_mode_cfg.enable_warped_motion = false;
        oxcf.motion_mode_cfg.allow_warped_motion = false;
    } else {
        oxcf.motion_mode_cfg.allow_warped_motion =
            (extra_cfg.allow_warped_motion & extra_cfg.enable_warped_motion) != 0;
    }

    // Set partition related configuration.
    let part_cfg = &mut oxcf.part_cfg;
    part_cfg.enable_rect_partitions = extra_cfg.enable_rect_partitions != 0;
    part_cfg.enable_ab_partitions = extra_cfg.enable_ab_partitions != 0;
    part_cfg.enable_1to4_partitions = extra_cfg.enable_1to4_partitions != 0;
    part_cfg.min_partition_size = extra_cfg.min_partition_size;
    part_cfg.max_partition_size = extra_cfg.max_partition_size;

    // Set intra mode configuration.
    let intra_mode_cfg = &mut oxcf.intra_mode_cfg;
    intra_mode_cfg.enable_angle_delta = extra_cfg.enable_angle_delta != 0;
    intra_mode_cfg.enable_intra_edge_filter = extra_cfg.enable_intra_edge_filter != 0;
    intra_mode_cfg.enable_filter_intra = extra_cfg.enable_filter_intra != 0;
    intra_mode_cfg.enable_smooth_intra = extra_cfg.enable_smooth_intra != 0;
    intra_mode_cfg.enable_paeth_intra = extra_cfg.enable_paeth_intra != 0;
    intra_mode_cfg.enable_cfl_intra = extra_cfg.enable_cfl_intra != 0;
    intra_mode_cfg.enable_directional_intra = extra_cfg.enable_directional_intra != 0;
    intra_mode_cfg.enable_diagonal_intra = extra_cfg.enable_diagonal_intra != 0;
    intra_mode_cfg.auto_intra_tools_off = extra_cfg.auto_intra_tools_off != 0;

    // Set transform size/type configuration.
    let txfm_cfg = &mut oxcf.txfm_cfg;
    txfm_cfg.enable_tx64 = extra_cfg.enable_tx64 != 0;
    txfm_cfg.enable_flip_idtx = extra_cfg.enable_flip_idtx != 0;
    txfm_cfg.enable_rect_tx = extra_cfg.enable_rect_tx != 0;
    txfm_cfg.reduced_tx_type_set = extra_cfg.reduced_tx_type_set != 0;
    txfm_cfg.use_intra_dct_only = extra_cfg.use_intra_dct_only != 0;
    txfm_cfg.use_inter_dct_only = extra_cfg.use_inter_dct_only != 0;
    txfm_cfg.use_intra_default_tx_only = extra_cfg.use_intra_default_tx_only != 0;
    txfm_cfg.enable_tx_size_search = extra_cfg.enable_tx_size_search != 0;

    // Set compound type configuration.
    let comp_type_cfg = &mut oxcf.comp_type_cfg;
    comp_type_cfg.enable_dist_wtd_comp =
        (extra_cfg.enable_dist_wtd_comp & extra_cfg.enable_order_hint) != 0;
    comp_type_cfg.enable_masked_comp = extra_cfg.enable_masked_comp != 0;
    comp_type_cfg.enable_diff_wtd_comp =
        (extra_cfg.enable_masked_comp & extra_cfg.enable_diff_wtd_comp) != 0;
    comp_type_cfg.enable_interinter_wedge =
        (extra_cfg.enable_masked_comp & extra_cfg.enable_interinter_wedge) != 0;
    comp_type_cfg.enable_smooth_interintra =
        extra_cfg.enable_interintra_comp != 0 && extra_cfg.enable_smooth_interintra != 0;
    comp_type_cfg.enable_interintra_wedge =
        (extra_cfg.enable_interintra_comp & extra_cfg.enable_interintra_wedge) != 0;

    // Set Super-resolution mode configuration.
    let superres_cfg = &mut oxcf.superres_cfg;
    if extra_cfg.lossless != 0 || cfg.large_scale_tile != 0 {
        disable_superres(superres_cfg);
    } else {
        superres_cfg.superres_mode = cfg.rc_superres_mode as i32;
        superres_cfg.superres_scale_denominator = cfg.rc_superres_denominator as u8;
        superres_cfg.superres_kf_scale_denominator = cfg.rc_superres_kf_denominator as u8;
        superres_cfg.superres_qthresh = av1_quantizer_to_qindex(cfg.rc_superres_qthresh as i32);
        superres_cfg.superres_kf_qthresh =
            av1_quantizer_to_qindex(cfg.rc_superres_kf_qthresh as i32);
        debug_assert!(superres_cfg.superres_mode == AOM_SUPERRES_NONE);
    }

    superres_cfg.enable_superres =
        superres_cfg.superres_mode != AOM_SUPERRES_NONE && extra_cfg.enable_superres != 0;
    if !superres_cfg.enable_superres {
        disable_superres(superres_cfg);
    }

    if input_cfg_limit(oxcf) == 1 {
        // still picture mode, display model and timing is meaningless
        oxcf.dec_model_cfg.display_model_info_present_flag = false;
        oxcf.dec_model_cfg.timing_info_present = false;
    }

    oxcf.save_as_annexb = cfg.save_as_annexb != 0;

    // Set unit test related configuration.
    // (no unit tests)

    oxcf.border_in_pixels = av1_get_enc_border_size(
        av1_is_resize_needed(oxcf),
        oxcf.kf_cfg.key_freq_max == 0,
        BLOCK_128X128,
    );
    oxcf.target_seq_level_idx = extra_cfg.target_seq_level_idx;
    oxcf.tier_mask = extra_cfg.tier_mask;

    oxcf.strict_level_conformance = extra_cfg.strict_level_conformance != 0;

    oxcf.kf_max_pyr_height = extra_cfg.kf_max_pyr_height;

    oxcf.sb_qp_sweep = extra_cfg.sb_qp_sweep != 0;
}

#[inline]
fn input_cfg_limit(oxcf: &Av1EncoderConfig) -> u32 {
    oxcf.input_cfg.limit
}

/// Translation of `update_extra_cfg()` (one compressor, no lookahead
/// compressor).
fn update_extra_cfg(ctx: &mut AomCodecAlgPriv, extra_cfg: &Av1ExtraCfg) -> Result<(), ()> {
    // (validate_config(): see the module documentation)
    ctx.extra_cfg = *extra_cfg;
    set_encoder_config(&mut ctx.oxcf, &ctx.cfg, &ctx.extra_cfg);
    // (av1_check_fpmt_config(): one thread)
    let mut is_sb_size_changed = false;
    let cpi = &mut *ctx.cpi;
    av1_change_config_seq(
        &mut cpi.ppi,
        &mut cpi.common.seq_params,
        &ctx.oxcf,
        Some(&mut is_sb_size_changed),
    )?;
    av1_change_config(cpi, &ctx.oxcf, is_sb_size_changed)
}

/// Translation of `ctrl_set_cpuused()`.
pub(crate) fn ctrl_set_cpuused(ctx: &mut AomCodecAlgPriv, cpu_used: i32) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.cpu_used = cpu_used;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_tuning()`.
pub(crate) fn ctrl_set_tuning(ctx: &mut AomCodecAlgPriv, tuning: i32) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.tuning = tuning;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_cq_level()`.
pub(crate) fn ctrl_set_cq_level(ctx: &mut AomCodecAlgPriv, cq_level: u32) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.cq_level = cq_level;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_lossless()`.
pub(crate) fn ctrl_set_lossless(ctx: &mut AomCodecAlgPriv, lossless: u32) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.lossless = lossless;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_tile_columns()`.
pub(crate) fn ctrl_set_tile_columns(
    ctx: &mut AomCodecAlgPriv,
    tile_columns: u32,
) -> Result<(), ()> {
    // If the encoder has already been initialized, do not accept changes to
    // the number of tile columns.
    // (the saver sets it before the first frame)
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.tile_columns = tile_columns;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_tile_rows()`.
pub(crate) fn ctrl_set_tile_rows(ctx: &mut AomCodecAlgPriv, tile_rows: u32) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.tile_rows = tile_rows;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_skip_postproc_filtering()`.
pub(crate) fn ctrl_set_skip_postproc_filtering(
    ctx: &mut AomCodecAlgPriv,
    skip: u32,
) -> Result<(), ()> {
    // Skipping the application of post-processing filters is allowed only
    // for ALLINTRA mode.
    if ctx.cfg.g_usage != AOM_USAGE_ALL_INTRA {
        return Err(()); // AOM_CODEC_INCAPABLE
    }
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.skip_postproc_filtering = skip;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_color_primaries()`.
pub(crate) fn ctrl_set_color_primaries(ctx: &mut AomCodecAlgPriv, v: i32) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.color_primaries = v;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_transfer_characteristics()`.
pub(crate) fn ctrl_set_transfer_characteristics(
    ctx: &mut AomCodecAlgPriv,
    v: i32,
) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.transfer_characteristics = v;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_matrix_coefficients()`.
pub(crate) fn ctrl_set_matrix_coefficients(ctx: &mut AomCodecAlgPriv, v: i32) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.matrix_coefficients = v;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_chroma_sample_position()`.
pub(crate) fn ctrl_set_chroma_sample_position(ctx: &mut AomCodecAlgPriv, v: i32) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.chroma_sample_position = v;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `ctrl_set_color_range()`.
pub(crate) fn ctrl_set_color_range(ctx: &mut AomCodecAlgPriv, v: i32) -> Result<(), ()> {
    let mut extra_cfg = ctx.extra_cfg;
    extra_cfg.color_range = v;
    update_extra_cfg(ctx, &extra_cfg)
}

/// Translation of `encoder_init()` (`aom_codec_enc_init()`).
pub(crate) fn encoder_init(cfg: &AomCodecEncCfg, init_flags: u32) -> Result<AomCodecAlgPriv, ()> {
    let mut extra_cfg = DEFAULT_EXTRA_CFG;
    // Special handling:
    // By default, if omitted, --enable-cdef = 1.
    // Here we set its default value to 0 when --allintra is turned on.
    // However, if users set --enable-cdef = 1 from command line,
    // The encoder still respects it.
    if cfg.g_usage == AOM_USAGE_ALL_INTRA {
        extra_cfg.enable_cdef = 0;
    }
    // (av1_initialize_enc(): the function tables are direct calls, and the
    // all intra AOM_Q coding builds no minimum q tables)

    // (validate_config(): see the module documentation)
    let num_lap_buffers = 0;
    // (timestamp_ratio.den = g_timebase.den; .num = g_timebase.num *
    // TICKS_PER_SEC)
    let mut timestamp_ratio = (
        cfg.g_timebase.0 as i64 * TICKS_PER_SEC,
        cfg.g_timebase.1 as i64,
    );
    reduce_ratio(&mut timestamp_ratio);

    let mut oxcf = Av1EncoderConfig::default();
    set_encoder_config(&mut oxcf, cfg, &extra_cfg);
    // (no lookahead processing: AOM_Q, one pass, all intra)
    debug_assert!(
        !(oxcf.rc_cfg.mode != AOM_CBR && oxcf.pass == AOM_RC_ONE_PASS && oxcf.mode == GOOD)
    );
    oxcf.use_highbitdepth = (init_flags & AOM_CODEC_USE_HIGHBITDEPTH) != 0;

    let mut cpi = Box::<Av1Comp>::default();
    {
        let cpi = &mut *cpi;
        av1_create_primary_compressor(
            &mut cpi.ppi,
            &mut cpi.common.seq_params,
            num_lap_buffers,
            &oxcf,
        )?;
        // (av1_create_stats_buffer(): one pass)
        av1_create_compressor(cpi, &oxcf, ENCODE_STAGE)?;
    }

    Ok(AomCodecAlgPriv {
        cfg: *cfg,
        extra_cfg,
        timestamp_ratio,
        pts_offset: 0,
        pts_offset_initialized: false,
        oxcf,
        cpi,
        cx_data: Vec::new(),
        pending_cx_data_sz: 0,
        next_frame_flags: 0,
        init_flags,
        fixed_kf_cntr: 0,
    })
}

/// Translation of `get_frame_pkt_flags()`.
fn get_frame_pkt_flags(cpi: &Av1Comp, lib_flags: u32) -> u32 {
    let mut flags = lib_flags << 16;

    if lib_flags & FRAMEFLAGS_KEY != 0 {
        flags |= AOM_FRAME_IS_KEY;
    }
    if lib_flags & FRAMEFLAGS_INTRAONLY != 0 {
        flags |= AOM_FRAME_IS_INTRAONLY;
    }
    if lib_flags & FRAMEFLAGS_SWITCH != 0 {
        flags |= AOM_FRAME_IS_SWITCH;
    }
    if lib_flags & FRAMEFLAGS_ERROR_RESILIENT != 0 {
        flags |= AOM_FRAME_IS_ERROR_RESILIENT;
    }
    if cpi.droppable {
        flags |= AOM_FRAME_IS_DROPPABLE;
    }

    flags
}

/// Translation of `get_src_border_in_pixels()`.
#[inline]
fn get_src_border_in_pixels(cpi: &Av1Comp, sb_size: BlockSize) -> i32 {
    if cpi.oxcf.mode != REALTIME || av1_is_resize_needed(&cpi.oxcf) {
        return cpi.oxcf.border_in_pixels;
    }

    let sb_size_in_pixels_log2 = MI_SIZE_WIDE_LOG2[sb_size as usize] as i32 + MI_SIZE_LOG2;
    let align = |v: i32, n: i32| (v + (1 << n) - 1) & !((1 << n) - 1);
    let sb_aligned_width = align(cpi.oxcf.frm_dim_cfg.width, sb_size_in_pixels_log2);
    let sb_aligned_height = align(cpi.oxcf.frm_dim_cfg.height, sb_size_in_pixels_log2);
    // Align the border pixels to a multiple of 32.
    let border_pixels_width = align(sb_aligned_width - cpi.oxcf.frm_dim_cfg.width, 5);
    let border_pixels_height = align(sb_aligned_height - cpi.oxcf.frm_dim_cfg.height, 5);
    border_pixels_width.max(border_pixels_height).max(32)
}

/// `aom_image_t` as libavif fills it, with the planes as 16-bit samples
/// at either bit depth and the strides in samples (upstream's are in bytes,
/// the same number for 8-bit images).
#[derive(Debug, Clone, Copy)]
pub(crate) struct AomImage<'a> {
    pub(crate) fmt: u32,
    pub(crate) cp: i32,
    pub(crate) tc: i32,
    pub(crate) mc: i32,
    pub(crate) monochrome: bool,
    pub(crate) csp: i32,
    pub(crate) range: i32,
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) bit_depth: u32,
    pub(crate) d_w: u32,
    pub(crate) d_h: u32,
    pub(crate) x_chroma_shift: u32,
    pub(crate) y_chroma_shift: u32,
    pub(crate) planes: [&'a [u16]; 3],
    pub(crate) stride: [i32; 3],
}

/// Translation of `image2yuvconfig()`: the planes are passed on beside the
/// returned frame description.
fn image2yuvconfig(img: &AomImage) -> Yv12BufferConfig {
    let mut yv12 = Yv12BufferConfig::default();
    yv12.crop_widths[0] = img.d_w as i32;
    yv12.crop_heights[0] = img.d_h as i32;
    yv12.render_width = 0;
    yv12.render_height = 0;
    yv12.widths[0] = img.w as i32;
    yv12.heights[0] = img.h as i32;

    yv12.widths[1] = (yv12.widths[0] + img.x_chroma_shift as i32) >> img.x_chroma_shift;
    yv12.heights[1] = (yv12.heights[0] + img.y_chroma_shift as i32) >> img.y_chroma_shift;
    yv12.crop_widths[1] = (yv12.crop_widths[0] + img.x_chroma_shift as i32) >> img.x_chroma_shift;
    yv12.crop_heights[1] = (yv12.crop_heights[0] + img.y_chroma_shift as i32) >> img.y_chroma_shift;

    yv12.strides[0] = img.stride[0];
    yv12.strides[1] = img.stride[1];
    yv12.color_primaries = img.cp;
    yv12.transfer_characteristics = img.tc;
    yv12.matrix_coefficients = img.mc;
    yv12.monochrome = img.monochrome as u8;
    yv12.chroma_sample_position = img.csp;
    yv12.color_range = img.range;

    if img.fmt & AOM_IMG_FMT_HIGHBITDEPTH != 0 {
        // (the strides are already in samples)
        yv12.flags = YV12_FLAG_HIGHBITDEPTH;
    } else {
        yv12.flags = 0;
    }

    // Note(yunqing): if img is allocated the same as the frame buffer, y_stride
    // is 32-byte aligned. Also, handle the cases while allocating img without a
    // border or stride_align is less than 32.
    let border = (yv12.strides[0] - ((img.w + 31) & !31) as i32) / 2;
    yv12.border = if border < 0 { 0 } else { border };
    yv12.subsampling_x = img.x_chroma_shift as i32;
    yv12.subsampling_y = img.y_chroma_shift as i32;
    yv12
}

/// Translation of `encoder_encode()` (`aom_codec_encode()`); `img` `None`
/// flushes. The returned packets are those `aom_codec_get_cx_data()`
/// returns after the call.
pub(crate) fn encoder_encode(
    ctx: &mut AomCodecAlgPriv,
    img: Option<&AomImage>,
    pts: i64,
    duration: u64,
    enc_flags: u32,
) -> Result<Vec<AomCodecCxFramePkt>, ()> {
    const K_MIN_COMPRESSED_SIZE: usize = 8192;
    let mut pkts = Vec::new();
    let mut cpi_data = Av1CompData {
        timestamp_ratio: ctx.timestamp_ratio,
        flush: img.is_none(),
        ..Default::default()
    };

    if let Some(img) = img {
        // (validate_img(): see the module documentation)
        let uncompressed_frame_sz = ((ctx.cfg.g_w as usize + 31) & !31)
            * ((ctx.cfg.g_h as usize + 31) & !31)
            * av1_get_image_bps(img) as usize
            / 8;

        // Due to the presence of no-show frames, the ctx->cx_data buffer holds
        // compressed data corresponding to multiple frames. As no-show frames are
        // not possible for all intra frame encoding with no forward key frames,
        // the buffer is allocated with a smaller size in this case.
        //
        // For pseudo random input, the compressed frame size is seen to exceed
        // the uncompressed frame size, but is less than 2 times the uncompressed
        // frame size. Hence the size of the buffer is chosen as 2 times the
        // uncompressed frame size.
        let mut multiplier = 8;
        if ctx.cpi.oxcf.kf_cfg.key_freq_max == 0 && !ctx.cpi.oxcf.kf_cfg.fwd_kf_enabled {
            multiplier = 2;
        }
        let mut data_sz = uncompressed_frame_sz * multiplier;
        if data_sz < K_MIN_COMPRESSED_SIZE {
            data_sz = K_MIN_COMPRESSED_SIZE;
        }
        if ctx.cx_data.len() < data_sz {
            let mut cx_data = Vec::new();
            cx_data.try_reserve_exact(data_sz).map_err(|_| ())?;
            cx_data.resize(data_sz, 0);
            ctx.cx_data = cx_data;
        }
    }

    let mut flags = enc_flags;
    let cpi = &mut *ctx.cpi;

    // (the scalable video coding fixed mode: no SVC)
    debug_assert!(!cpi.ppi.use_svc);

    // Note(yunqing): While applying encoding flags, always start from enabling
    // all, and then modifying according to the flags. Previous frame's flags are
    // overwritten.
    av1_apply_encoding_flags(cpi, flags);

    // Handle fixed keyframe intervals
    if (is_stat_generation_stage(cpi) || is_one_pass_rt_params(cpi))
        && ctx.cfg.kf_mode == AOM_KF_AUTO
        && ctx.cfg.kf_min_dist == ctx.cfg.kf_max_dist
    {
        if cpi.common.spatial_layer_id == 0 && {
            ctx.fixed_kf_cntr += 1;
            ctx.fixed_kf_cntr > ctx.cfg.kf_min_dist
        } {
            flags |= AOM_EFLAG_FORCE_KF;
            ctx.fixed_kf_cntr = 1;
        }
    }

    // (av1_alloc_layer_context(): one spatial and one temporal layer, whose
    // context the non-SVC coding never reads)

    // Set up internal flags
    // (no PSNR)

    if let Some(img) = img {
        let mut ptsvol = pts;
        if !ctx.pts_offset_initialized {
            ctx.pts_offset = ptsvol;
            ctx.pts_offset_initialized = true;
        }
        ptsvol -= ctx.pts_offset;
        let src_time_stamp = timebase_units_to_ticks(cpi_data.timestamp_ratio, ptsvol);
        let src_end_time_stamp =
            timebase_units_to_ticks(cpi_data.timestamp_ratio, ptsvol + duration as i64);

        let mut sd = image2yuvconfig(img);
        let mut planes = img.planes;
        // When generating a monochrome stream, make |sd| a monochrome image.
        if ctx.cfg.monochrome != 0 {
            planes[1] = &[];
            planes[2] = &[];
            sd.strides[1] = 0;
            sd.monochrome = 1;
        }
        let use_highbitdepth = (sd.flags & YV12_FLAG_HIGHBITDEPTH) != 0;
        let subsampling_x = sd.subsampling_x;
        let subsampling_y = sd.subsampling_y;

        if cpi.ppi.lookahead.is_none() {
            let lag_in_frames = cpi.oxcf.gf_cfg.lag_in_frames;
            let oxcf = &mut cpi.oxcf;
            let sb_size = av1_select_sb_size(
                oxcf,
                oxcf.frm_dim_cfg.width,
                oxcf.frm_dim_cfg.height,
                cpi.ppi.number_spatial_layers,
            );
            oxcf.border_in_pixels = av1_get_enc_border_size(
                av1_is_resize_needed(oxcf),
                oxcf.kf_cfg.key_freq_max == 0,
                sb_size,
            );
            // (one compressor)

            let src_border_in_pixels = get_src_border_in_pixels(cpi, sb_size);
            let lookahead = av1_lookahead_init(
                cpi.oxcf.frm_dim_cfg.width as u32,
                cpi.oxcf.frm_dim_cfg.height as u32,
                subsampling_x as u32,
                subsampling_y as u32,
                use_highbitdepth,
                lag_in_frames as u32,
                src_border_in_pixels,
                cpi.common.features.byte_alignment,
                0,
                cpi.oxcf.kf_cfg.key_freq_max == 0,
                cpi.oxcf.tool_cfg.enable_global_motion,
            )?;
            cpi.ppi.lookahead = Some(lookahead);
        }
        av1_check_initial_width(cpi, use_highbitdepth, subsampling_x, subsampling_y);

        // Store the original flags in to the frame buffer. Will extract the
        // key frame flag when we actually encode this frame.
        av1_receive_raw_frame(
            cpi,
            flags | ctx.next_frame_flags,
            &sd,
            planes,
            src_time_stamp,
            src_end_time_stamp,
        )?;
        ctx.next_frame_flags = 0;
    }

    // (no pending invisible frames: every frame is shown)
    debug_assert!(ctx.pending_cx_data_sz == 0);

    let mut is_frame_visible = false;
    let mut cx_off = 0usize;

    debug_assert!(cpi.oxcf.pass != AOM_RC_FIRST_PASS);
    av1_compute_num_workers_for_mt(cpi);
    let num_workers = av1_get_max_num_workers(cpi);
    if num_workers > 1 && cpi.ppi.p_mt_info.num_workers == 0 {
        // (one frame at a time: no frame parallel contexts)
        av1_create_workers(cpi, num_workers);
        // (av1_init_tile_thread_data() and av1_init_mt_sync(): see ethread.rs)
        cpi.ppi.p_mt_info.prev_num_enc_workers =
            av1_get_num_mod_workers_for_alloc(&cpi.ppi.p_mt_info, MOD_ENC);
    }

    // Re-allocate thread data if workers for encoder multi-threading stage
    // exceeds prev_num_enc_workers.
    let num_enc_workers = av1_get_num_mod_workers_for_alloc(&cpi.ppi.p_mt_info, MOD_ENC);
    if cpi.ppi.p_mt_info.prev_num_enc_workers < num_enc_workers
        && num_enc_workers <= cpi.ppi.p_mt_info.num_workers
    {
        // (free_thread_data() and av1_init_tile_thread_data(): no thread data)
        cpi.ppi.p_mt_info.prev_num_enc_workers = num_enc_workers;
    }

    av1_init_frame_mt(cpi);

    // Reset gf_frame_index in case it reaches MAX_STATIC_GF_GROUP_LENGTH for
    // real time encoding.
    if is_one_pass_rt_params(cpi) && cpi.gf_frame_index == MAX_STATIC_GF_GROUP_LENGTH as u8 {
        cpi.gf_frame_index = 0;
    }

    // Call for LAP stage
    // (no lookahead processing)

    // Get the next visible frame. Invisible frames get packed with the next
    // visible frame.
    while ctx.cx_data.len() - cx_off >= ctx.cx_data.len() / 2 && !is_frame_visible {
        cpi.do_frame_data_update = true;
        // (ref_idx_to_skip and ref_refresh_index: frame parallel encoding
        // only)
        cpi.refresh_idx_available = false;
        // (one frame at a time: no parallel frames)
        let status = av1_get_compressed_data(cpi, &mut cpi_data, &mut ctx.cx_data[cx_off..])?;
        if status.is_none() {
            break;
        }
        // (av1_init_sc_decisions(): one compressor)

        cpi.ppi.seq_params_locked = true;
        av1_post_encode_updates(cpi, &cpi_data);

        if cpi_data.frame_size == 0 {
            continue;
        }
        let write_temporal_delimiter =
            cpi.common.spatial_layer_id == 0 && ctx.pending_cx_data_sz == 0;

        if write_temporal_delimiter {
            let mut obu_header_size: usize = 1;
            let obu_payload_size: usize = 0;
            let length_field_size = aom_uleb_size_in_bytes(obu_payload_size as u64);

            let move_offset = obu_header_size + length_field_size;
            let frame = cx_off..cx_off + cpi_data.frame_size;
            if cx_off + move_offset + cpi_data.frame_size > ctx.cx_data.len() {
                return Err(());
            }
            ctx.cx_data.copy_within(frame, cx_off + move_offset);
            obu_header_size =
                av1_write_obu_header(OBU_TEMPORAL_DELIMITER, 0, &mut ctx.cx_data[cx_off..])
                    as usize;

            // OBUs are preceded/succeeded by an unsigned leb128 coded integer.
            av1_write_uleb_obu_size(
                obu_header_size,
                obu_payload_size,
                &mut ctx.cx_data[cx_off..],
            )?;

            cpi_data.frame_size += obu_header_size + obu_payload_size + length_field_size;
        }

        debug_assert!(!ctx.oxcf.save_as_annexb);

        ctx.pending_cx_data_sz += cpi_data.frame_size;

        cx_off += cpi_data.frame_size;

        is_frame_visible = cpi.common.show_frame != 0;
    }
    if is_frame_visible {
        // Add the frame packet to the list of returned packets.
        // decrement frames_left counter
        cpi.ppi.frames_left = cpi.ppi.frames_left.saturating_sub(1);

        let pts = crate::aom::av1::encoder::encode_strategy::ticks_to_timebase_units(
            cpi_data.timestamp_ratio,
            cpi_data.ts_frame_start,
        ) + ctx.pts_offset;
        let flags = get_frame_pkt_flags(cpi, cpi_data.lib_flags);
        pkts.push(AomCodecCxFramePkt {
            buf: ctx.cx_data[..ctx.pending_cx_data_sz].to_vec(),
            pts,
            flags,
        });

        ctx.pending_cx_data_sz = 0;
    }

    Ok(pkts)
}
