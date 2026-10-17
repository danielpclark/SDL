// Rust translation of the parts of av1/encoder/encoder.c, encoder.h and
// encoder_alloc.h from aom (https://aomedia.googlesource.com/aom, at the
// revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches)
// the saver reaches: one still picture, coded as a single shown key frame
// with the all intra usage, one pass, one thread.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The compressor (`AV1_COMP` and `AV1_PRIMARY`), its configuration, and the
//! frame level coding pipeline.
//!
//! Only the members the reached paths read are kept. The compressor codes
//! one frame in its life: the state that only carries over to later frames
//! (rate control history, reference frame bookkeeping, level statistics)
//! is updated where it can change the coded frame and otherwise left out,
//! with a note at each place.

use super::av1_quantize::*;
use super::bitstream::av1_pack_bitstream;
use super::block::*;
use super::enc_enums::MAX_MODES;
use super::encode_strategy::{
    av1_encode_strategy, get_current_frame_ref_type, is_frame_droppable, EncodeFrameInput,
};
use super::encodeframe::av1_encode_frame;
use super::encodeframe_utils::MAX_TPL_FRAME_IDX;
use super::encoder_utils::*;
use super::encodetxb::av1_alloc_txb_buf;
use super::lookahead::*;
use super::mcomp::*;
use super::pickcdef::av1_cdef_search;
use super::picklpf::av1_pick_filter_level;
use super::ratectrl::*;
use super::speed_features::*;
use super::superres_scale::{av1_setup_frame_size, av1_superres_in_recode_allowed};
use super::var_based_part::{av1_set_variance_partition_thresholds, VP64x64};
use crate::aom::aom_dsp::aom_dsp_common::get_msb;
use crate::aom::aom_dsp::variance::AomVarianceFnPtr;
use crate::aom::aom_scale::yv12config::*;
use crate::aom::aom_scale::yv12extend::aom_extend_frame_borders_plane_row;
use crate::aom::av1::common::alloccommon::*;
use crate::aom::av1::common::av1_common_int::*;
use crate::aom::av1::common::av1_loopfilter::{av1_loop_filter_frame, av1_loop_filter_init};
use crate::aom::av1::common::blockd::*;
use crate::aom::av1::common::cdef::av1_cdef_frame;
use crate::aom::av1::common::entropy::av1_reset_cdf_symbol_counters;
use crate::aom::av1::common::entropymode::FrameContext;
use crate::aom::av1::common::enums::*;
use crate::aom::av1::common::filter::{SWITCHABLE_FILTERS, SWITCHABLE_FILTER_CONTEXTS};
use crate::aom::av1::common::mv::IDENTITY;
use crate::aom::av1::common::quant_common::QINDEX_RANGE;
use crate::aom::av1::common::scale::{av1_is_scaled, av1_setup_scale_factors_for_frame};
use crate::aom::av1::common::seg_common::*;
use crate::aom::av1::common::tile_common::*;
use crate::aom::av1::common::timing::av1_max_level_bitrate;

// ---------------------------------------------------------------------------
// Enumerations (encoder.h, aomcx.h, aom_encoder.h)
// ---------------------------------------------------------------------------

/// `MODE`: good quality, real time and all intra coding.
pub(crate) const GOOD: i32 = 0;
pub(crate) const REALTIME: i32 = 1;
pub(crate) const ALLINTRA: i32 = 2;

/// `enum aom_enc_pass`
pub(crate) const AOM_RC_ONE_PASS: i32 = 0;
pub(crate) const AOM_RC_FIRST_PASS: i32 = 1;
pub(crate) const AOM_RC_SECOND_PASS: i32 = 2;
pub(crate) const AOM_RC_THIRD_PASS: i32 = 3;

/// `enum aom_rc_mode`
pub(crate) const AOM_VBR: i32 = 0;
pub(crate) const AOM_CBR: i32 = 1;
pub(crate) const AOM_CQ: i32 = 2;
pub(crate) const AOM_Q: i32 = 3;

/// `AQ_MODE`
pub(crate) const NO_AQ: i32 = 0;
pub(crate) const VARIANCE_AQ: i32 = 1;
pub(crate) const COMPLEXITY_AQ: i32 = 2;
pub(crate) const CYCLIC_REFRESH_AQ: i32 = 3;

/// `DELTAQ_MODE`
pub(crate) const NO_DELTA_Q: i32 = 0;
pub(crate) const DELTA_Q_OBJECTIVE: i32 = 1;
pub(crate) const DELTA_Q_PERCEPTUAL: i32 = 2;
pub(crate) const DELTA_Q_PERCEPTUAL_AI: i32 = 3;
pub(crate) const DELTA_Q_USER_RATING_BASED: i32 = 4;
pub(crate) const DELTA_Q_HDR: i32 = 5;

/// `COST_UPDATE_TYPE`
pub(crate) const COST_UPD_SB: i32 = 0;
pub(crate) const COST_UPD_SBROW: i32 = 1;
pub(crate) const COST_UPD_TILE: i32 = 2;
pub(crate) const COST_UPD_OFF: i32 = 3;

/// `LOOPFILTER_CONTROL`
pub(crate) const LOOPFILTER_NONE: i32 = 0;
pub(crate) const LOOPFILTER_ALL: i32 = 1;
pub(crate) const LOOPFILTER_REFERENCE: i32 = 2;
pub(crate) const LOOPFILTER_SELECTIVELY: i32 = 3;

/// `aom_tune_metric`
pub(crate) const AOM_TUNE_PSNR: i32 = 0;
pub(crate) const AOM_TUNE_SSIM: i32 = 1;

/// `aom_dist_metric`
pub(crate) const AOM_DIST_METRIC_PSNR: i32 = 0;
pub(crate) const AOM_DIST_METRIC_QM_PSNR: i32 = 1;

/// `aom_superblock_size_t`
pub(crate) const AOM_SUPERBLOCK_SIZE_64X64: i32 = 0;
pub(crate) const AOM_SUPERBLOCK_SIZE_128X128: i32 = 1;
pub(crate) const AOM_SUPERBLOCK_SIZE_DYNAMIC: i32 = 2;

/// `aom_superres_mode`
pub(crate) const AOM_SUPERRES_NONE: i32 = 0;
pub(crate) const AOM_SUPERRES_FIXED: i32 = 1;
pub(crate) const AOM_SUPERRES_RANDOM: i32 = 2;
pub(crate) const AOM_SUPERRES_QTHRESH: i32 = 3;
pub(crate) const AOM_SUPERRES_AUTO: i32 = 4;

/// `RESIZE_MODE`
pub(crate) const RESIZE_NONE: i32 = 0;

/// `aom_tune_content`
pub(crate) const AOM_CONTENT_DEFAULT: i32 = 0;
pub(crate) const AOM_CONTENT_SCREEN: i32 = 1;
pub(crate) const AOM_CONTENT_FILM: i32 = 2;

/// `CDEF_CONTROL`
pub(crate) const CDEF_NONE: i32 = 0;

/// `REFRESH_FRAME_CONTEXT_MODE`
pub(crate) const REFRESH_FRAME_CONTEXT_DISABLED: i32 = 0;
pub(crate) const REFRESH_FRAME_CONTEXT_BACKWARD: i32 = 1;

/// The frame flags of `av1_get_compressed_data()`.
pub(crate) const FRAMEFLAGS_KEY: u32 = 1 << 0;
pub(crate) const FRAMEFLAGS_GOLDEN: u32 = 1 << 1;
pub(crate) const FRAMEFLAGS_BWDREF: u32 = 1 << 2;
pub(crate) const FRAMEFLAGS_ALTREF: u32 = 1 << 3;
pub(crate) const FRAMEFLAGS_INTRAONLY: u32 = 1 << 4;
pub(crate) const FRAMEFLAGS_SWITCH: u32 = 1 << 5;
pub(crate) const FRAMEFLAGS_ERROR_RESILIENT: u32 = 1 << 6;

/// `av1_ref_frame_flag_list[]`
pub(crate) const AV1_REF_FRAME_FLAG_LIST: [i32; REF_FRAMES as usize] = [0, 1, 2, 4, 8, 16, 32, 64];
// ---------------------------------------------------------------------------
// Configuration (AV1EncoderConfig and its parts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FrameDimensionCfg {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) forced_max_frame_width: i32,
    pub(crate) forced_max_frame_height: i32,
    pub(crate) render_width: i32,
    pub(crate) render_height: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InputCfg {
    pub(crate) init_framerate: f64,
    pub(crate) input_bit_depth: i32,
    pub(crate) limit: u32,
    pub(crate) chroma_subsampling_x: u32,
    pub(crate) chroma_subsampling_y: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RateControlCfg {
    pub(crate) starting_buffer_level_ms: i64,
    pub(crate) optimal_buffer_level_ms: i64,
    pub(crate) maximum_buffer_size_ms: i64,
    pub(crate) target_bandwidth: i64,
    pub(crate) vbr_corpus_complexity_lap: u32,
    pub(crate) max_intra_bitrate_pct: u32,
    pub(crate) max_inter_bitrate_pct: u32,
    pub(crate) gf_cbr_boost_pct: u32,
    pub(crate) min_cr: u32,
    pub(crate) drop_frames_water_mark: i32,
    pub(crate) under_shoot_pct: i32,
    pub(crate) over_shoot_pct: i32,
    pub(crate) worst_allowed_q: i32,
    pub(crate) best_allowed_q: i32,
    pub(crate) cq_level: i32,
    pub(crate) mode: i32,
    pub(crate) vbrbias: i32,
    pub(crate) vbrmin_section: i32,
    pub(crate) vbrmax_section: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ToolCfg {
    pub(crate) bit_depth: i32,
    pub(crate) cdef_control: i32,
    pub(crate) enable_restoration: bool,
    pub(crate) force_video_mode: bool,
    pub(crate) enable_palette: bool,
    pub(crate) enable_ref_frame_mvs: bool,
    pub(crate) superblock_size: i32,
    pub(crate) enable_monochrome: bool,
    pub(crate) full_still_picture_hdr: bool,
    pub(crate) enable_dual_filter: bool,
    pub(crate) enable_order_hint: bool,
    pub(crate) enable_interintra_comp: bool,
    pub(crate) ref_frame_mvs_present: bool,
    pub(crate) enable_global_motion: bool,
    pub(crate) error_resilient_mode: bool,
    pub(crate) frame_parallel_decoding_mode: bool,
    pub(crate) enable_deltalf_mode: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct QuantizationCfg {
    pub(crate) using_qm: bool,
    pub(crate) qm_minlevel: i32,
    pub(crate) qm_maxlevel: i32,
    pub(crate) quant_b_adapt: bool,
    pub(crate) enable_chroma_deltaq: i32,
    pub(crate) aq_mode: i32,
    pub(crate) deltaq_mode: i32,
    pub(crate) deltaq_strength: i32,
    pub(crate) use_fixed_qp_offsets: i32,
    pub(crate) enable_hdr_deltaq: i32,
}

/// `CostUpdateFreq`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CostUpdateFreq {
    pub(crate) coeff: i32,
    pub(crate) mode: i32,
    pub(crate) mv: i32,
    pub(crate) dv: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResizeCfg {
    pub(crate) resize_mode: i32,
    pub(crate) resize_scale_denominator: u8,
    pub(crate) resize_kf_scale_denominator: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AlgoCfg {
    pub(crate) sharpness: i32,
    pub(crate) disable_trellis_quant: i32,
    pub(crate) arnr_max_frames: i32,
    pub(crate) arnr_strength: i32,
    pub(crate) cdf_update_mode: u8,
    pub(crate) enable_tpl_model: bool,
    pub(crate) enable_overlay: bool,
    pub(crate) loopfilter_control: i32,
    pub(crate) skip_postproc_filtering: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct KeyFrameCfg {
    pub(crate) key_freq_min: i32,
    pub(crate) key_freq_max: i32,
    pub(crate) enable_keyframe_filtering: i32,
    pub(crate) sframe_dist: i32,
    pub(crate) sframe_mode: i32,
    pub(crate) auto_key: bool,
    pub(crate) fwd_kf_dist: i32,
    pub(crate) fwd_kf_enabled: bool,
    pub(crate) enable_sframe: bool,
    pub(crate) enable_intrabc: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ColorCfg {
    pub(crate) color_primaries: i32,
    pub(crate) transfer_characteristics: i32,
    pub(crate) matrix_coefficients: i32,
    pub(crate) color_range: i32,
    pub(crate) chroma_sample_position: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GfConfig {
    pub(crate) min_gf_interval: i32,
    pub(crate) max_gf_interval: i32,
    pub(crate) gf_min_pyr_height: i32,
    pub(crate) gf_max_pyr_height: i32,
    pub(crate) enable_auto_arf: bool,
    pub(crate) enable_auto_brf: bool,
    pub(crate) lag_in_frames: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TuneCfg {
    pub(crate) tuning: i32,
    pub(crate) content: i32,
    pub(crate) film_grain_test_vector: i32,
    pub(crate) dist_metric: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TileConfig {
    pub(crate) enable_large_scale_tile: bool,
    pub(crate) enable_single_tile_decoding: bool,
    pub(crate) num_tile_groups: i32,
    pub(crate) mtu: u32,
    pub(crate) tile_columns: i32,
    pub(crate) tile_rows: i32,
    pub(crate) tile_width_count: i32,
    pub(crate) tile_height_count: i32,
    pub(crate) enable_ext_tile_debug: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MotionModeCfg {
    pub(crate) enable_obmc: bool,
    pub(crate) enable_warped_motion: bool,
    pub(crate) allow_warped_motion: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct IntraModeCfg {
    pub(crate) enable_intra_edge_filter: bool,
    pub(crate) enable_filter_intra: bool,
    pub(crate) enable_smooth_intra: bool,
    pub(crate) enable_paeth_intra: bool,
    pub(crate) enable_cfl_intra: bool,
    pub(crate) enable_directional_intra: bool,
    pub(crate) enable_diagonal_intra: bool,
    pub(crate) enable_angle_delta: bool,
    pub(crate) auto_intra_tools_off: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TxfmSizeTypeCfg {
    pub(crate) enable_tx64: bool,
    pub(crate) enable_flip_idtx: bool,
    pub(crate) enable_rect_tx: bool,
    pub(crate) reduced_tx_type_set: bool,
    pub(crate) use_intra_dct_only: bool,
    pub(crate) use_inter_dct_only: bool,
    pub(crate) use_intra_default_tx_only: bool,
    pub(crate) enable_tx_size_search: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CompoundTypeCfg {
    pub(crate) enable_dist_wtd_comp: bool,
    pub(crate) enable_masked_comp: bool,
    pub(crate) enable_smooth_interintra: bool,
    pub(crate) enable_diff_wtd_comp: bool,
    pub(crate) enable_interinter_wedge: bool,
    pub(crate) enable_interintra_wedge: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SuperResCfg {
    pub(crate) superres_qthresh: i32,
    pub(crate) superres_kf_qthresh: i32,
    pub(crate) superres_scale_denominator: u8,
    pub(crate) superres_kf_scale_denominator: u8,
    pub(crate) superres_mode: i32,
    pub(crate) enable_superres: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DecoderModelCfg {
    pub(crate) timing_info_present: bool,
    pub(crate) decoder_model_info_present_flag: bool,
    pub(crate) display_model_info_present_flag: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PartitionCfg {
    pub(crate) enable_rect_partitions: bool,
    pub(crate) enable_ab_partitions: bool,
    pub(crate) enable_1to4_partitions: bool,
    pub(crate) min_partition_size: i32,
    pub(crate) max_partition_size: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RefFrameCfg {
    pub(crate) max_reference_frames: i32,
    pub(crate) enable_reduced_reference_set: bool,
    pub(crate) enable_onesided_comp: bool,
}

/// `AV1EncoderConfig`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Av1EncoderConfig {
    pub(crate) profile: i32,
    pub(crate) max_threads: i32,
    pub(crate) mode: i32,
    pub(crate) frm_dim_cfg: FrameDimensionCfg,
    pub(crate) input_cfg: InputCfg,
    pub(crate) pass: i32,
    pub(crate) passes: i32,
    pub(crate) rc_cfg: RateControlCfg,
    pub(crate) tool_cfg: ToolCfg,
    pub(crate) q_cfg: QuantizationCfg,
    pub(crate) cost_upd_freq: CostUpdateFreq,
    pub(crate) resize_cfg: ResizeCfg,
    pub(crate) algo_cfg: AlgoCfg,
    pub(crate) kf_cfg: KeyFrameCfg,
    pub(crate) speed: i32,
    pub(crate) color_cfg: ColorCfg,
    pub(crate) gf_cfg: GfConfig,
    pub(crate) tune_cfg: TuneCfg,
    pub(crate) tile_cfg: TileConfig,
    pub(crate) ref_frm_cfg: RefFrameCfg,
    pub(crate) motion_mode_cfg: MotionModeCfg,
    pub(crate) part_cfg: PartitionCfg,
    pub(crate) intra_mode_cfg: IntraModeCfg,
    pub(crate) txfm_cfg: TxfmSizeTypeCfg,
    pub(crate) comp_type_cfg: CompoundTypeCfg,
    pub(crate) superres_cfg: SuperResCfg,
    pub(crate) dec_model_cfg: DecoderModelCfg,
    pub(crate) use_highbitdepth: bool,
    pub(crate) border_in_pixels: i32,
    pub(crate) target_seq_level_idx: [Av1Level; MAX_NUM_OPERATING_POINTS],
    pub(crate) tier_mask: u32,
    pub(crate) row_mt: bool,
    pub(crate) fp_mt: bool,
    pub(crate) save_as_annexb: bool,
    pub(crate) noise_level: f32,
    pub(crate) strict_level_conformance: bool,
    pub(crate) kf_max_pyr_height: i32,
    pub(crate) sb_qp_sweep: bool,
}

/// Translation of `is_lossless_requested()`.
#[inline]
pub(crate) fn is_lossless_requested(rc_cfg: &RateControlCfg) -> bool {
    rc_cfg.best_allowed_q == 0 && rc_cfg.worst_allowed_q == 0
}

// ---------------------------------------------------------------------------
// Compressor state
// ---------------------------------------------------------------------------

/// `COMPRESSOR_STAGE`
pub(crate) const ENCODE_STAGE: i32 = 0;
pub(crate) const LAP_STAGE: i32 = 1;

/// The reached members of `GF_GROUP`: the group of one key frame.
#[derive(Debug, Clone)]
pub(crate) struct GfGroup {
    pub(crate) update_type: [u8; MAX_STATIC_GF_GROUP_LENGTH],
    pub(crate) arf_src_offset: [u8; MAX_STATIC_GF_GROUP_LENGTH],
    pub(crate) src_offset: [u8; MAX_STATIC_GF_GROUP_LENGTH],
    pub(crate) layer_depth: [u8; MAX_STATIC_GF_GROUP_LENGTH],
    pub(crate) frame_parallel_level: [i32; MAX_STATIC_GF_GROUP_LENGTH],
    pub(crate) refbuf_state: [i32; MAX_STATIC_GF_GROUP_LENGTH],
    pub(crate) is_frame_non_ref: [bool; MAX_STATIC_GF_GROUP_LENGTH],
    pub(crate) frame_type: [FrameType; MAX_STATIC_GF_GROUP_LENGTH],
    pub(crate) max_layer_depth: i32,
    pub(crate) size: i32,
}

impl Default for GfGroup {
    fn default() -> Self {
        GfGroup {
            update_type: [0; MAX_STATIC_GF_GROUP_LENGTH],
            arf_src_offset: [0; MAX_STATIC_GF_GROUP_LENGTH],
            src_offset: [0; MAX_STATIC_GF_GROUP_LENGTH],
            layer_depth: [0; MAX_STATIC_GF_GROUP_LENGTH],
            frame_parallel_level: [0; MAX_STATIC_GF_GROUP_LENGTH],
            refbuf_state: [0; MAX_STATIC_GF_GROUP_LENGTH],
            is_frame_non_ref: [false; MAX_STATIC_GF_GROUP_LENGTH],
            frame_type: [0; MAX_STATIC_GF_GROUP_LENGTH],
            max_layer_depth: 0,
            size: 0,
        }
    }
}

/// The reached members of `TplParams`: no TPL statistics are ever made
/// (the TPL model is off with no lag).
#[derive(Debug, Clone)]
pub(crate) struct TplParams {
    pub(crate) tpl_frame_is_valid: [bool; MAX_TPL_FRAME_IDX as usize],
}

impl Default for TplParams {
    fn default() -> Self {
        TplParams {
            tpl_frame_is_valid: [false; MAX_TPL_FRAME_IDX as usize],
        }
    }
}

/// `AV1_PRIMARY`: the top level encoder state, with the sequence parameters
/// (`ppi->seq_params`) kept in `Av1Comp::common`.
#[derive(Debug, Default)]
pub(crate) struct Av1Primary {
    pub(crate) seq_params_locked: bool,
    pub(crate) lap_enabled: bool,
    pub(crate) frames_left: u32,
    pub(crate) num_fp_contexts: i32,
    pub(crate) use_svc: bool,
    pub(crate) number_spatial_layers: i32,
    pub(crate) number_temporal_layers: i32,
    pub(crate) gf_group: GfGroup,
    pub(crate) p_rc: PrimaryRateControl,
    pub(crate) tpl_data: TplParams,
    pub(crate) lookahead: Option<LookaheadCtx>,
    pub(crate) filter_level: [i32; 2],
    pub(crate) filter_level_u: i32,
    pub(crate) filter_level_v: i32,
    /// `ppi->rtc_ref`
    pub(crate) rtc_ref: RtcRef,
    /// `ppi->fn_ptr[]`
    pub(crate) fn_ptr: AomVarianceFnPtr,
    /// `ppi->fb_of_context_type[]`
    pub(crate) fb_of_context_type: [i32; REF_FRAMES as usize],
    pub(crate) mv_stats: MvStats,
    pub(crate) valid_gm_model_found: [i32; FRAME_UPDATE_TYPES],
    pub(crate) frame_probs: FrameProbInfo,
    pub(crate) ts_start_last_show_frame: i64,
    pub(crate) ts_end_last_show_frame: i64,
}

/// `RD_COUNTS`
#[derive(Debug, Clone, Default)]
pub(crate) struct RdCounts {
    pub(crate) compound_ref_used_flag: i32,
    pub(crate) skip_mode_used_flag: i32,
    pub(crate) tx_type_used: [[i32; TX_TYPES as usize]; TX_SIZES_ALL as usize],
    pub(crate) seg_tmp_pred_cost: [i32; 2],
    pub(crate) newmv_or_intra_blocks: i32,
}

/// `ThreadData`, for the one thread.
#[derive(Debug, Default)]
pub(crate) struct ThreadData {
    pub(crate) mb: Macroblock,
    pub(crate) rd_counts: RdCounts,
    pub(crate) vt64x64: Box<[VP64x64; 4]>,
    pub(crate) intrabc_used: i32,
    pub(crate) deltaq_used: i32,
    pub(crate) abs_sum_level: i32,
    pub(crate) coefficient_size: i64,
    pub(crate) max_mv_magnitude: i32,
    pub(crate) interp_filter_selected: [i32; SWITCHABLE as usize],
    /// `*td->counts`
    pub(crate) counts: FrameCounts,
}

/// `TileDataEnc`
#[derive(Debug, Default)]
pub(crate) struct TileDataEnc {
    pub(crate) tile_info: TileInfo,
    pub(crate) allow_update_cdf: bool,
    pub(crate) tctx: Box<FrameContext>,
    pub(crate) abs_sum_level: i32,
}

/// `WinnerModeParams`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WinnerModeParams {
    pub(crate) coeff_opt_thresholds: [[u32; 2]; MODE_EVAL_TYPES as usize],
    pub(crate) tx_domain_dist_threshold: [u32; MODE_EVAL_TYPES as usize],
    pub(crate) use_transform_domain_distortion: [u32; MODE_EVAL_TYPES as usize],
    pub(crate) skip_txfm_level: [u32; MODE_EVAL_TYPES as usize],
    pub(crate) tx_size_search_methods: [i32; MODE_EVAL_TYPES as usize],
    pub(crate) use_transform_domain_distortion_flag: i32,
    pub(crate) predict_dc_level: [u32; MODE_EVAL_TYPES as usize],
}

/// `MBMIExtFrameBufferInfo`: the frame base is moved into the macroblock
/// while the frame is coded or packed (see `enc_move_frame_state_in()`).
#[derive(Debug, Default)]
pub(crate) struct MbmiExtFrameBufferInfo {
    pub(crate) frame_base: Vec<MbModeInfoExtFrame>,
    pub(crate) alloc_size: i32,
    pub(crate) stride: i32,
}

/// The reached members of `MultiThreadInfo` (one thread).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MultiThreadInfo {
    pub(crate) num_workers: i32,
    pub(crate) row_mt_enabled: bool,
    pub(crate) pack_bs_mt_enabled: bool,
    pub(crate) pipeline_lpf_mt_with_enc: bool,
}

/// `EncSegmentationInfo`
#[derive(Debug, Clone, Default)]
pub(crate) struct EncSegmentationInfo {
    pub(crate) map: Vec<u8>,
    pub(crate) has_lossless_segment: i32,
}

/// `VarBasedPartitionInfo`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct VarBasedPartitionInfo {
    pub(crate) thresholds: [i64; 5],
    pub(crate) threshold_minmax: i32,
}

/// `RefFrameDistanceInfo`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RefFrameDistanceInfo {
    pub(crate) ref_relative_dist: [i32; INTER_REFS_PER_FRAME as usize],
    pub(crate) nearest_past_ref: i8,
    pub(crate) nearest_future_ref: i8,
}

/// The reached members of `MotionVectorSearchParams`.
#[derive(Debug, Clone, Default)]
pub(crate) struct MotionVectorSearchParams {
    pub(crate) max_mv_magnitude: i32,
    pub(crate) mv_step_param: i32,
    /// `find_fractional_mv_step`: the fractional search, a
    /// `SUBPEL_SEARCH_METHODS` value.
    pub(crate) find_fractional_mv_step: i32,
    /// `search_site_cfg[SS_CFG_TOTAL][NUM_DISTINCT_SEARCH_METHODS]`
    pub(crate) search_site_cfg:
        Box<[[SearchSiteConfig; NUM_DISTINCT_SEARCH_METHODS as usize]; SS_CFG_TOTAL]>,
}

/// `FrameProbInfo` (the probabilities the real-time speed features use)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FrameProbInfo {
    pub(crate) warped_probs: [i32; FRAME_UPDATE_TYPES],
}

/// `MV_STATS` (never valid: the real-time speed features use quarter pel
/// motion vectors only, `QTR_ONLY`, or the q index threshold)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MvStats {
    pub(crate) valid: bool,
}

/// `ActiveMap` (never enabled)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ActiveMap {
    pub(crate) enabled: bool,
    pub(crate) update: bool,
}

/// The reached member of `GlobalMotionInfo`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GlobalMotionInfo {
    pub(crate) search_done: bool,
}

/// The reached members of `FRAME_COUNTS`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FrameCounts {
    pub(crate) switchable_interp: [[u32; SWITCHABLE_FILTERS as usize]; SWITCHABLE_FILTER_CONTEXTS],
}

/// `RD_OPT`
#[derive(Debug, Clone)]
pub(crate) struct RdOpt {
    // Thresh_mult is used to set a threshold for the rd score. A higher value
    // means that we will accept the best mode so far more often. This number
    // is used in combination with the current block size, and thresh_freq_fact
    // to pick a threshold.
    pub(crate) thresh_mult: [i32; MAX_MODES as usize],

    pub(crate) threshes: Vec<[[i32; MAX_MODES as usize]; BLOCK_SIZES_ALL as usize]>,

    pub(crate) rdmult: i32,

    pub(crate) r0: f64,
}

impl Default for RdOpt {
    fn default() -> Self {
        RdOpt {
            thresh_mult: [0; MAX_MODES as usize],
            threshes: vec![[[0; MAX_MODES as usize]; BLOCK_SIZES_ALL as usize]; MAX_SEGMENTS],
            rdmult: 0,
            r0: 0.0,
        }
    }
}

/// `InitialDimensions`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InitialDimensions {
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// `RefreshFrameInfo`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RefreshFrameInfo {
    pub(crate) golden_frame: bool,
    pub(crate) bwd_ref_frame: bool,
    pub(crate) alt_ref_frame: bool,
}

/// `ExtRefreshFrameFlagsInfo`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExtRefreshFrameFlagsInfo {
    pub(crate) update_pending: bool,
    pub(crate) last_frame: bool,
    pub(crate) golden_frame: bool,
    pub(crate) bwd_ref_frame: bool,
    pub(crate) alt2_ref_frame: bool,
    pub(crate) alt_ref_frame: bool,
}

/// The reached members of `ExternalFlags`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ExternalFlags {
    pub(crate) ref_frame_flags: i32,
    pub(crate) refresh_frame: ExtRefreshFrameFlagsInfo,
    pub(crate) refresh_frame_context: bool,
    pub(crate) refresh_frame_context_pending: bool,
    pub(crate) use_ref_frame_mvs: bool,
    pub(crate) use_error_resilient: bool,
    pub(crate) use_s_frame: bool,
    pub(crate) use_primary_ref_none: bool,
}

/// The reached members of `SVC`: one spatial and one temporal layer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Svc {
    pub(crate) spatial_layer_id: i32,
    pub(crate) temporal_layer_id: i32,
    pub(crate) number_spatial_layers: i32,
    pub(crate) number_temporal_layers: i32,
    pub(crate) num_encoded_top_layer: i32,
}

impl Default for Svc {
    fn default() -> Self {
        // (scalable coding is off: av1_change_config() copies
        // ppi->number_spatial_layers and number_temporal_layers, 1)
        Svc {
            spatial_layer_id: 0,
            temporal_layer_id: 0,
            number_spatial_layers: 1,
            number_temporal_layers: 1,
            num_encoded_top_layer: 0,
        }
    }
}

/// `RTC_REF`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RtcRef {
    /*
     * LAST_FRAME (0), LAST2_FRAME(1), LAST3_FRAME(2), GOLDEN_FRAME(3),
     * BWDREF_FRAME(4), ALTREF2_FRAME(5), ALTREF_FRAME(6).
     */
    pub(crate) reference: [i32; INTER_REFS_PER_FRAME as usize],
    pub(crate) ref_idx: [i32; INTER_REFS_PER_FRAME as usize],
    pub(crate) refresh: [i32; REF_FRAMES as usize],
    pub(crate) set_ref_frame_config: bool,
    pub(crate) non_reference_frame: bool,
    pub(crate) ref_frame_comp: [i32; 3],
    pub(crate) gld_idx_1layer: i32,
}

/// `ResizePendingParams`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResizePendingParams {
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// `TimeStamps`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TimeStamps {
    pub(crate) first_ts_start: i64,
    pub(crate) prev_ts_start: i64,
    pub(crate) prev_ts_end: i64,
}

/// `AV1_COMP`
#[derive(Debug, Default)]
pub(crate) struct Av1Comp {
    pub(crate) ppi: Av1Primary,
    pub(crate) common: Av1Common,
    pub(crate) oxcf: Av1EncoderConfig,
    pub(crate) td: ThreadData,
    pub(crate) sf: SpeedFeatures,
    pub(crate) rd: RdOpt,
    pub(crate) rc: RateControl,
    pub(crate) winner_mode_params: WinnerModeParams,
    pub(crate) mbmi_ext_info: MbmiExtFrameBufferInfo,
    pub(crate) coeff_buffer_base: Vec<CbCoeffBuffer>,
    pub(crate) tile_data: Vec<TileDataEnc>,
    pub(crate) allocated_tiles: i32,
    pub(crate) mt_info: MultiThreadInfo,
    pub(crate) enc_seg: EncSegmentationInfo,
    pub(crate) optimize_seg_arr: [i32; MAX_SEGMENTS],
    pub(crate) ssim_rdmult_scaling_factors: Vec<f64>,
    pub(crate) vbp_info: VarBasedPartitionInfo,
    pub(crate) enc_quant_dequant_params: Box<EncQuantDequantParams>,
    pub(crate) mv_search_params: MotionVectorSearchParams,
    pub(crate) ref_frame_dist_info: RefFrameDistanceInfo,
    pub(crate) initial_dimensions: InitialDimensions,
    pub(crate) refresh_frame: RefreshFrameInfo,
    pub(crate) ext_flags: ExternalFlags,
    pub(crate) time_stamps: TimeStamps,
    pub(crate) compressor_stage: i32,
    pub(crate) gf_frame_index: u8,
    pub(crate) num_tg: i32,
    pub(crate) frame_header_count: i32,
    pub(crate) intrabc_used: i32,
    pub(crate) deltaq_used: i32,
    pub(crate) ref_frame_flags: i32,
    pub(crate) all_one_sided_refs: i32,
    pub(crate) prune_ref_frame_mask: i32,
    pub(crate) use_ducky_encode: bool,
    pub(crate) do_frame_data_update: bool,
    pub(crate) framerate: f64,
    pub(crate) new_framerate: f64,
    pub(crate) speed: i32,
    pub(crate) superres_mode: i32,
    pub(crate) is_screen_content_type: i32,
    pub(crate) frames_since_last_update: i32,
    pub(crate) last_frame_type: FrameType,
    pub(crate) droppable: bool,
    pub(crate) initial_mbs: i32,
    /// `cpi->source`: the frame being coded, moved out of the lookahead
    /// queue while it is coded.
    pub(crate) source_buf: Yv12BufferConfig,
    /// `cpi->last_source`: the previous source frame (the lookahead
    /// queue's entry before the coded one), moved out of the queue while
    /// the frame is coded; `has_last_source` tells whether there is one.
    pub(crate) last_source_buf: Yv12BufferConfig,
    pub(crate) has_last_source: bool,
    pub(crate) svc: Svc,
    pub(crate) use_screen_content_tools: i32,
    pub(crate) rec_sse: u64,
    pub(crate) rt_reduce_num_ref_buffers: bool,
    pub(crate) src_sad_blk_64x64: Vec<u64>,
    pub(crate) resize_pending_params: ResizePendingParams,
    pub(crate) refresh_idx_available: bool,
    /// `cpi->consec_zero_mv`
    pub(crate) consec_zero_mv: Vec<u8>,
    pub(crate) last_show_frame_buf: Option<usize>,
    pub(crate) existing_fb_idx_to_show: i32,
    pub(crate) mv_stats: MvStats,
    pub(crate) active_map: ActiveMap,
    pub(crate) gm_info: GlobalMotionInfo,
    /// `cpi->scaled_ref_buf[]`: buffer pool indices
    pub(crate) scaled_ref_buf: [Option<usize>; INTER_REFS_PER_FRAME as usize],
    pub(crate) is_dropped_frame: bool,
}

impl Av1Comp {
    /// `cpi->source`
    #[inline]
    pub(crate) fn source(&self) -> &Yv12BufferConfig {
        &self.source_buf
    }
}

/// Moves the frame-wide state the blocks read and write into the thread
/// data while a frame is coded or packed: `cm->mi_params` arrays,
/// `cm->above_contexts`, the reconstruction `cm->cur_frame->buf`,
/// `cpi->mbmi_ext_info.frame_base` and `cpi->coeff_buffer_base`.
pub(crate) fn enc_move_frame_state_in(cpi: &mut Av1Comp, td: &mut ThreadData) {
    let cm = &mut cpi.common;
    let xd = &mut td.mb.e_mbd;
    xd.mi_alloc = std::mem::take(&mut cm.mi_params.mi_alloc);
    xd.mi_grid_base = std::mem::take(&mut cm.mi_params.mi_grid_base);
    xd.tx_type_map_frame = std::mem::take(&mut cm.mi_params.tx_type_map);
    xd.above_contexts = std::mem::take(&mut cm.above_contexts);
    if cm.cur_frame.is_some() {
        xd.recon = std::mem::take(&mut cm.cur_frame_mut().buf);
        xd.cur_frame_mvs = std::mem::take(&mut cm.cur_frame_mut().mvs);
    }
    td.mb.mbmi_ext_frame_base = std::mem::take(&mut cpi.mbmi_ext_info.frame_base);
    td.mb.cb_coeff_buffers = std::mem::take(&mut cpi.coeff_buffer_base);
}

/// The reverse of `enc_move_frame_state_in()`.
pub(crate) fn enc_move_frame_state_out(cpi: &mut Av1Comp, td: &mut ThreadData) {
    let cm = &mut cpi.common;
    let xd = &mut td.mb.e_mbd;
    cm.mi_params.mi_alloc = std::mem::take(&mut xd.mi_alloc);
    cm.mi_params.mi_grid_base = std::mem::take(&mut xd.mi_grid_base);
    cm.mi_params.tx_type_map = std::mem::take(&mut xd.tx_type_map_frame);
    cm.above_contexts = std::mem::take(&mut xd.above_contexts);
    if cm.cur_frame.is_some() {
        cm.cur_frame_mut().buf = std::mem::take(&mut xd.recon);
        cm.cur_frame_mut().mvs = std::mem::take(&mut xd.cur_frame_mvs);
    }
    cpi.mbmi_ext_info.frame_base = std::mem::take(&mut td.mb.mbmi_ext_frame_base);
    cpi.coeff_buffer_base = std::mem::take(&mut td.mb.cb_coeff_buffers);
}

// ---------------------------------------------------------------------------
// encoder.h helpers
// ---------------------------------------------------------------------------

/// Translation of `get_mi_ext_idx()`.
#[inline]
pub(crate) fn get_mi_ext_idx(
    mi_row: i32,
    mi_col: i32,
    mi_alloc_bsize: BlockSize,
    mbmi_ext_stride: i32,
) -> i32 {
    let mi_ext_size_1d = MI_SIZE_WIDE[mi_alloc_bsize as usize] as i32;
    let mi_ext_row = mi_row / mi_ext_size_1d;
    let mi_ext_col = mi_col / mi_ext_size_1d;
    mi_ext_row * mbmi_ext_stride + mi_ext_col
}

/// Translation of `set_mode_info_offsets()`.
#[inline]
pub(crate) fn set_mode_info_offsets(
    mi_params: &CommonModeInfoParams,
    mbmi_ext_info: &MbmiExtFrameBufferInfo,
    x: &mut Macroblock,
    mi_row: i32,
    mi_col: i32,
) {
    set_mi_offsets(mi_params, &mut x.e_mbd, mi_row, mi_col);
    let ext_idx = get_mi_ext_idx(
        mi_row,
        mi_col,
        mi_params.mi_alloc_bsize,
        mbmi_ext_info.stride,
    );
    x.mbmi_ext_frame = ext_idx as usize;
}

/// Translation of `frame_is_kf_gf_arf()`.
#[inline]
pub(crate) fn frame_is_kf_gf_arf(cpi: &Av1Comp) -> bool {
    let cm = &cpi.common;
    let update_type = cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize];

    frame_is_intra_only(cm) || update_type == ARF_UPDATE || update_type == GF_UPDATE
}

/// Translation of `is_stat_generation_stage()`.
#[inline]
pub(crate) fn is_stat_generation_stage(cpi: &Av1Comp) -> bool {
    cpi.oxcf.pass == AOM_RC_FIRST_PASS || cpi.compressor_stage == LAP_STAGE
}

/// Translation of `is_stat_consumption_stage_twopass()`.
#[inline]
pub(crate) fn is_stat_consumption_stage_twopass(cpi: &Av1Comp) -> bool {
    cpi.oxcf.pass >= AOM_RC_SECOND_PASS
}

/// Translation of `is_stat_consumption_stage()`.
#[inline]
pub(crate) fn is_stat_consumption_stage(cpi: &Av1Comp) -> bool {
    is_stat_consumption_stage_twopass(cpi)
        || (cpi.oxcf.pass == AOM_RC_ONE_PASS
            && cpi.compressor_stage == ENCODE_STAGE
            && cpi.ppi.lap_enabled)
}

/// Translation of `has_no_stats_stage()`.
#[inline]
pub(crate) fn has_no_stats_stage(cpi: &Av1Comp) -> bool {
    debug_assert!(cpi.ppi.lap_enabled || cpi.compressor_stage == ENCODE_STAGE);
    cpi.oxcf.pass == AOM_RC_ONE_PASS && !cpi.ppi.lap_enabled
}

/// Translation of `is_one_pass_rt_params()`.
#[inline]
pub(crate) fn is_one_pass_rt_params(cpi: &Av1Comp) -> bool {
    has_no_stats_stage(cpi) && cpi.oxcf.mode == REALTIME && cpi.oxcf.gf_cfg.lag_in_frames == 0
}

// Use default/internal reference structure for single-layer RTC.
/// Translation of `use_rtc_reference_structure_one_layer()`.
#[inline]
pub(crate) fn use_rtc_reference_structure_one_layer(cpi: &Av1Comp) -> bool {
    is_one_pass_rt_params(cpi)
        && cpi.ppi.number_spatial_layers == 1
        && cpi.ppi.number_temporal_layers == 1
        && !cpi.ppi.rtc_ref.set_ref_frame_config
}

/// Translation of `av1_apply_encoding_flags()`: libavif passes no flags
/// but `AOM_EFLAG_FORCE_KF`, so no reference or update is disabled.
pub(crate) fn av1_apply_encoding_flags(cpi: &mut Av1Comp, flags: u32) {
    // TODO(yunqingwang): For what references to use, external encoding flags
    // should be consistent with internal reference frame selection. Need to
    // ensure that there is not conflict between the two. In AV1 encoder, the
    // priority rank for 7 reference frames are: LAST, ALTREF, LAST2, LAST3,
    // GOLDEN, BWDREF, ALTREF2.

    debug_assert!(flags & !super::encode_strategy::AOM_EFLAG_FORCE_KF == 0);
    let ext_flags = &mut cpi.ext_flags;
    ext_flags.ref_frame_flags = AOM_REFFRAME_ALL;
    // (no AOM_EFLAG_NO_REF_* flags; no external reference configuration)
    debug_assert!(!cpi.ppi.rtc_ref.set_ref_frame_config);

    // (no AOM_EFLAG_NO_UPD_* flags)
    ext_flags.refresh_frame.update_pending = false;

    ext_flags.use_ref_frame_mvs = cpi.oxcf.tool_cfg.enable_ref_frame_mvs;
    ext_flags.use_error_resilient = cpi.oxcf.tool_cfg.error_resilient_mode;
    ext_flags.use_s_frame = cpi.oxcf.kf_cfg.enable_sframe;
    ext_flags.use_primary_ref_none = false;

    // (no AOM_EFLAG_NO_UPD_ENTROPY)
}

/// `HIGH_PRECISION_MV_QTHRESH` (mv_prec.h)
const HIGH_PRECISION_MV_QTHRESH: i32 = 128;

/// Translation of `av1_pick_and_set_high_precision_mv()` (mv_prec.c).
pub(crate) fn av1_pick_and_set_high_precision_mv(cpi: &mut Av1Comp, qindex: i32) {
    let mut use_hp = qindex < HIGH_PRECISION_MV_QTHRESH;

    if cpi.sf.hl_sf.high_precision_mv_usage == QTR_ONLY {
        use_hp = false;
    } else if cpi.sf.hl_sf.high_precision_mv_usage == LAST_MV_DATA && cpi.mv_stats.valid {
        // (get_smart_mv_prec(): the motion vector statistics are never
        // collected, see MvStats)
        debug_assert!(false);
    }

    let force_integer_mv = cpi.common.features.cur_frame_force_integer_mv;
    av1_set_high_precision_mv(cpi, use_hp, force_integer_mv);
}

/// Translation of `av1_set_high_precision_mv()` (mv_prec.h).
#[inline]
pub(crate) fn av1_set_high_precision_mv(
    cpi: &mut Av1Comp,
    allow_high_precision_mv: bool,
    cur_frame_force_integer_mv: bool,
) {
    // Avoid accessing 'mv_costs' when it is not allocated.
    let Some(mv_costs) = cpi.td.mb.mv_costs.as_mut() else {
        return;
    };

    let copy_hp = allow_high_precision_mv && !cur_frame_force_integer_mv;
    cpi.common.features.allow_high_precision_mv = copy_hp;

    mv_costs.mv_cost_stack_hp = copy_hp;
}

/// Translation of `av1_encoder_get_relative_dist()`.
#[inline]
pub(crate) fn av1_encoder_get_relative_dist(a: i32, b: i32) -> i32 {
    debug_assert!(a >= 0 && b >= 0);
    a - b
}

/// Translation of `encode_show_existing_frame()`.
#[inline]
pub(crate) fn encode_show_existing_frame(cm: &Av1Common) -> bool {
    cm.show_existing_frame != 0
        && (!cm.features.error_resilient_mode || cm.current_frame.frame_type == KEY_FRAME)
}

/// `disable_order`
pub(crate) static DISABLE_ORDER: [MvReferenceFrame; 4] =
    [LAST3_FRAME, LAST2_FRAME, ALTREF2_FRAME, BWDREF_FRAME];

/// Translation of `is_loopfilter_used()`.
#[inline]
pub(crate) fn is_loopfilter_used(cm: &Av1Common) -> bool {
    !cm.features.coded_lossless && cm.tiles.large_scale == 0
}

/// Translation of `is_cdef_used()`.
#[inline]
pub(crate) fn is_cdef_used(cm: &Av1Common) -> bool {
    cm.seq_params.enable_cdef != 0 && !cm.features.coded_lossless && cm.tiles.large_scale == 0
}

/// Translation of `is_restoration_used()`.
#[inline]
pub(crate) fn is_restoration_used(cm: &Av1Common) -> bool {
    cm.seq_params.enable_restoration != 0 && !cm.features.all_lossless && cm.tiles.large_scale == 0
}

/// `SKIP_APPLY_*`
pub(crate) const SKIP_APPLY_RESTORATION: u32 = 1 << 1;
pub(crate) const SKIP_APPLY_SUPERRES: u32 = 1 << 2;
pub(crate) const SKIP_APPLY_CDEF: u32 = 1 << 3;
pub(crate) const SKIP_APPLY_LOOPFILTER: u32 = 1 << 4;

/// Translation of `derive_skip_apply_postproc_filters()` (no real-time
/// reference structure, no PSNR statistics).
#[inline]
pub(crate) fn derive_skip_apply_postproc_filters(
    cpi: &Av1Comp,
    use_loopfilter: bool,
    use_cdef: bool,
    use_superres: bool,
    use_restoration: bool,
) -> u32 {
    if !cpi.oxcf.algo_cfg.skip_postproc_filtering {
        return 0;
    }
    debug_assert!(cpi.oxcf.mode == ALLINTRA);

    // The post-processing filters are applied one after the other in the
    // following order: deblocking->cdef->superres->restoration. In case of
    // ALLINTRA encoding, the reconstructed frame is not used as a reference
    // frame. Hence, the application of these filters can be skipped when
    // 1. filter parameters of the subsequent stages are not dependent on the
    // filtered output of the current stage or
    // 2. subsequent filtering stages are disabled
    if use_restoration {
        return SKIP_APPLY_RESTORATION;
    }
    if use_superres {
        return SKIP_APPLY_SUPERRES;
    }
    if use_cdef {
        // CDEF parameter selection is not dependent on the deblocked frame if
        // cdef_pick_method is CDEF_PICK_FROM_Q. Hence the application of deblocking
        // filters and cdef filters can be skipped in this case.
        // (CDEF is off for all intra coding)
        debug_assert!(false);
        return SKIP_APPLY_CDEF;
    }
    if use_loopfilter {
        return SKIP_APPLY_LOOPFILTER;
    }

    0 // All post-processing stages disabled.
}

/// Translation of `set_postproc_filter_default_params()`.
#[inline]
pub(crate) fn set_postproc_filter_default_params(cm: &mut Av1Common) {
    let lf = &mut cm.lf;
    lf.filter_level[0] = 0;
    lf.filter_level[1] = 0;
    let cdef_info = &mut cm.cdef_info;
    cdef_info.cdef_bits = 0;
    cdef_info.cdef_strengths[0] = 0;
    cdef_info.nb_cdef_strengths = 1;
    cdef_info.cdef_uv_strengths[0] = 0;
    let rst_info = &mut cm.rst_info;
    rst_info[0].frame_restoration_type = RESTORE_NONE;
    rst_info[1].frame_restoration_type = RESTORE_NONE;
    rst_info[2].frame_restoration_type = RESTORE_NONE;
}

/// Translation of `av1_get_enc_border_size()`.
#[inline]
pub(crate) fn av1_get_enc_border_size(resize: bool, all_intra: bool, sb_size: BlockSize) -> i32 {
    // For allintra encoding mode, inter-frame motion search is not applicable and
    // the intraBC motion vectors are restricted within the tile boundaries. Hence
    // a smaller frame border size (AOM_ENC_ALLINTRA_BORDER) is used in this case.
    if resize {
        return AOM_BORDER_IN_PIXELS;
    }
    if all_intra {
        return AOM_ENC_ALLINTRA_BORDER;
    }
    BLOCK_SIZE_WIDE[sb_size as usize] as i32 + 32
}

/// Translation of `av1_is_resize_needed()`.
#[inline]
pub(crate) fn av1_is_resize_needed(oxcf: &Av1EncoderConfig) -> bool {
    oxcf.resize_cfg.resize_mode != RESIZE_NONE
        || oxcf.superres_cfg.superres_mode != AOM_SUPERRES_NONE
}

// ---------------------------------------------------------------------------
// encoder_alloc.h
// ---------------------------------------------------------------------------

/// Translation of `alloc_context_buffers_ext()`.
fn alloc_context_buffers_ext(
    cm: &Av1Common,
    mbmi_ext_info: &mut MbmiExtFrameBufferInfo,
) -> Result<(), ()> {
    let mi_params = &cm.mi_params;

    let mi_alloc_size_1d = MI_SIZE_WIDE[mi_params.mi_alloc_bsize as usize] as i32;
    let mi_alloc_rows = (mi_params.mi_rows + mi_alloc_size_1d - 1) / mi_alloc_size_1d;
    let mi_alloc_cols = (mi_params.mi_cols + mi_alloc_size_1d - 1) / mi_alloc_size_1d;
    let new_ext_mi_size = mi_alloc_rows * mi_alloc_cols;

    if new_ext_mi_size > mbmi_ext_info.alloc_size {
        mbmi_ext_info.frame_base =
            try_vec(new_ext_mi_size as usize, MbModeInfoExtFrame::default())?;
        mbmi_ext_info.alloc_size = new_ext_mi_size;
    }
    // The stride needs to be updated regardless of whether new allocation
    // happened or not.
    mbmi_ext_info.stride = mi_alloc_cols;
    Ok(())
}

/// Translation of `alloc_compressor_data()` (no motion vector costs, no
/// first pass context; the shared coefficient buffers belong to the
/// macroblock planes, see `av1_alloc_mb_data()`).
fn alloc_compressor_data(cpi: &mut Av1Comp) -> Result<(), ()> {
    let default_min_partition_size = cpi.sf.part_sf.default_min_partition_size;
    let (width, height) = (cpi.common.width, cpi.common.height);

    // Setup mi_params
    enc_set_mb_mi(
        &mut cpi.common.mi_params,
        width,
        height,
        default_min_partition_size,
    );

    if !is_stat_generation_stage(cpi) {
        av1_alloc_txb_buf(cpi)?;
    }

    cpi.td.mb.mv_costs = None;
    // Avoid the memory allocation of 'mv_costs' for allintra encoding mode.
    if cpi.oxcf.kf_cfg.key_freq_max != 0 {
        cpi.td.mb.mv_costs = Some(MvCosts::new()?);
    }
    Ok(())
}

/// Translation of `alloc_mb_mode_info_buffers()`.
pub(crate) fn alloc_mb_mode_info_buffers(cpi: &mut Av1Comp) -> Result<(), ()> {
    let default_min_partition_size = cpi.sf.part_sf.default_min_partition_size;
    let (width, height) = (cpi.common.width, cpi.common.height);
    av1_alloc_context_buffers(&mut cpi.common, width, height, default_min_partition_size)?;

    if !is_stat_generation_stage(cpi) {
        alloc_context_buffers_ext(&cpi.common, &mut cpi.mbmi_ext_info)?;
    }
    Ok(())
}

/// Translation of `realloc_segmentation_maps()` (no cyclic refresh or active
/// map: neither is used).
fn realloc_segmentation_maps(cpi: &mut Av1Comp) -> Result<(), ()> {
    let mi_params = &cpi.common.mi_params;

    // Create the encoder segmentation map and set all entries to 0
    cpi.enc_seg.map = try_vec((mi_params.mi_rows * mi_params.mi_cols) as usize, 0)?;
    Ok(())
}

/// Translation of `variance_partition_alloc()`: `td->vt64x64` is allocated
/// with the thread data.
#[inline]
fn variance_partition_alloc(_cpi: &mut Av1Comp) {}

// ---------------------------------------------------------------------------
// encoder.c
// ---------------------------------------------------------------------------

/// Translation of `av1_new_framerate()`.
pub(crate) fn av1_new_framerate(cpi: &mut Av1Comp, framerate: f64) {
    cpi.framerate = if framerate < 0.1 { 30.0 } else { framerate };
    let (w, h) = (cpi.common.width, cpi.common.height);
    av1_rc_update_framerate(cpi, w, h);
}

/// Translation of `set_tile_info()` (uniform spacing: the saver sets no tile
/// widths or heights).
fn set_tile_info(cm: &mut Av1Common, tile_cfg: &TileConfig) {
    av1_get_tile_limits(cm);

    let mi_params = &cm.mi_params;
    let seq_params = &cm.seq_params;
    let tiles = &mut cm.tiles;
    let mut sb_cols = crate::aom::aom_dsp::aom_dsp_common::ceil_power_of_two(
        mi_params.mi_cols,
        seq_params.mib_size_log2,
    );
    // configure tile columns
    debug_assert!(tile_cfg.tile_width_count == 0 || tile_cfg.tile_height_count == 0);
    tiles.uniform_spacing = 1;
    tiles.log2_cols = tile_cfg.tile_columns.max(tiles.min_log2_cols);
    // Add a special case to handle super resolution
    sb_cols = coded_to_superres_mi(sb_cols, cm.superres_scale_denominator as i32);
    let mut min_log2_cols = 0;
    while (tiles.max_width_sb << min_log2_cols) <= sb_cols {
        min_log2_cols += 1;
    }
    tiles.log2_cols = tiles.log2_cols.max(min_log2_cols);

    tiles.log2_cols = tiles.log2_cols.min(tiles.max_log2_cols);
    av1_calculate_tile_cols(seq_params, mi_params.mi_rows, mi_params.mi_cols, tiles);

    // configure tile rows
    if tiles.uniform_spacing != 0 {
        tiles.log2_rows = tile_cfg.tile_rows.max(tiles.min_log2_rows);
        tiles.log2_rows = tiles.log2_rows.min(tiles.max_log2_rows);
    }
    av1_calculate_tile_rows(seq_params, mi_params.mi_rows, tiles);
}

/// Translation of `av1_update_frame_size()`.
pub(crate) fn av1_update_frame_size(cpi: &mut Av1Comp) {
    let default_min_partition_size = cpi.sf.part_sf.default_min_partition_size;
    let cm = &mut cpi.common;

    // Setup mi_params here in case we need more mi's.
    let (width, height) = (cm.width, cm.height);
    enc_set_mb_mi(&mut cm.mi_params, width, height, default_min_partition_size);

    av1_init_macroblockd(cm, &mut cpi.td.mb.e_mbd);

    if !cpi.ppi.seq_params_locked {
        let sb_size = av1_select_sb_size(
            &cpi.oxcf,
            cm.width,
            cm.height,
            cpi.ppi.number_spatial_layers,
        );
        set_sb_size(&mut cm.seq_params, sb_size);
    }

    let tile_cfg = cpi.oxcf.tile_cfg;
    set_tile_info(cm, &tile_cfg);
}

/// Translation of `does_level_match()`.
#[inline]
fn does_level_match(
    width: i32,
    height: i32,
    fps: f64,
    lvl_width: i32,
    lvl_height: i32,
    lvl_fps: f64,
    lvl_dim_mult: i32,
) -> bool {
    let lvl_luma_pels = lvl_width as i64 * lvl_height as i64;
    let lvl_display_sample_rate = lvl_luma_pels as f64 * lvl_fps;
    let luma_pels = width as i64 * height as i64;
    let display_sample_rate = luma_pels as f64 * fps;
    luma_pels <= lvl_luma_pels
        && display_sample_rate <= lvl_display_sample_rate
        && width <= lvl_width * lvl_dim_mult
        && height <= lvl_height * lvl_dim_mult
}

/// Translation of `set_bitstream_level_tier()`.
fn set_bitstream_level_tier(
    seq_params: &mut SequenceHeader,
    width: i32,
    height: i32,
    init_framerate: f64,
) -> Result<(), ()> {
    // TODO(any): This is a placeholder function that only addresses dimensions
    // and max display sample rates.
    // Need to add checks for max bit rate, max decoded luma sample rate, header
    // rate, etc. that are not covered by this function.
    let mut level = SEQ_LEVEL_MAX;
    if does_level_match(width, height, init_framerate, 512, 288, 30.0, 4) {
        level = SEQ_LEVEL_2_0;
    } else if does_level_match(width, height, init_framerate, 704, 396, 30.0, 4) {
        level = SEQ_LEVEL_2_1;
    } else if does_level_match(width, height, init_framerate, 1088, 612, 30.0, 4) {
        level = SEQ_LEVEL_3_0;
    } else if does_level_match(width, height, init_framerate, 1376, 774, 30.0, 4) {
        level = SEQ_LEVEL_3_1;
    } else if does_level_match(width, height, init_framerate, 2048, 1152, 30.0, 3) {
        level = SEQ_LEVEL_4_0;
    } else if does_level_match(width, height, init_framerate, 2048, 1152, 60.0, 3) {
        level = SEQ_LEVEL_4_1;
    } else if does_level_match(width, height, init_framerate, 4096, 2176, 30.0, 2) {
        level = SEQ_LEVEL_5_0;
    } else if does_level_match(width, height, init_framerate, 4096, 2176, 60.0, 2) {
        level = SEQ_LEVEL_5_1;
    } else if does_level_match(width, height, init_framerate, 4096, 2176, 120.0, 2) {
        level = SEQ_LEVEL_5_2;
    } else if does_level_match(width, height, init_framerate, 8192, 4352, 30.0, 2) {
        level = SEQ_LEVEL_6_0;
    } else if does_level_match(width, height, init_framerate, 8192, 4352, 60.0, 2) {
        level = SEQ_LEVEL_6_1;
    } else if does_level_match(width, height, init_framerate, 8192, 4352, 120.0, 2) {
        level = SEQ_LEVEL_6_2;
    }
    // (the CONFIG_CWG_C013 levels 7.x and 8.x are not built)

    for i in 0..MAX_NUM_OPERATING_POINTS {
        seq_params.seq_level_idx[i] = level;
        // Set the maximum parameters for bitrate and buffer size for this profile,
        // level, and tier
        seq_params.op_params[i].bitrate = av1_max_level_bitrate(
            seq_params.profile,
            seq_params.seq_level_idx[i],
            seq_params.tier[i] as i32,
        );
        // Level with seq_level_idx = 31 returns a high "dummy" bitrate to pass the
        // check
        if seq_params.op_params[i].bitrate == 0 {
            // aom_internal_error(): "AV1 does not support this combination of
            // profile, level, and tier."
            return Err(());
        }
        // Buffer size in bits/s is bitrate in bits/s * 1 s
        seq_params.op_params[i].buffer_size = seq_params.op_params[i].bitrate;
    }
    Ok(())
}

/// `FRAME_ID_LENGTH`, `DELTA_FRAME_ID_LENGTH` and
/// `DEFAULT_EXPLICIT_ORDER_HINT_BITS`.
const FRAME_ID_LENGTH: i32 = 15;
const DELTA_FRAME_ID_LENGTH: i32 = 14;
const DEFAULT_EXPLICIT_ORDER_HINT_BITS: i32 = 7;

/// Translation of `av1_init_seq_coding_tools()`.
pub(crate) fn av1_init_seq_coding_tools(
    ppi: &Av1Primary,
    seq: &mut SequenceHeader,
    oxcf: &Av1EncoderConfig,
    disable_frame_id_numbers: bool,
) -> Result<(), ()> {
    let _ = ppi;
    let frm_dim_cfg = &oxcf.frm_dim_cfg;
    let tool_cfg = &oxcf.tool_cfg;

    seq.still_picture = (!tool_cfg.force_video_mode && oxcf.input_cfg.limit == 1) as u8;
    seq.reduced_still_picture_hdr =
        (seq.still_picture != 0 && !tool_cfg.full_still_picture_hdr) as u8;
    seq.force_screen_content_tools = 2;
    seq.force_integer_mv = 2;
    seq.order_hint_info.enable_order_hint = tool_cfg.enable_order_hint as i32;
    seq.frame_id_numbers_present_flag = (seq.reduced_still_picture_hdr == 0
        && !oxcf.tile_cfg.enable_large_scale_tile
        && tool_cfg.error_resilient_mode
        && !disable_frame_id_numbers) as u8;
    if seq.reduced_still_picture_hdr != 0 {
        seq.order_hint_info.enable_order_hint = 0;
        seq.force_screen_content_tools = 2;
        seq.force_integer_mv = 2;
    }
    seq.order_hint_info.order_hint_bits_minus_1 = if seq.order_hint_info.enable_order_hint != 0 {
        DEFAULT_EXPLICIT_ORDER_HINT_BITS - 1
    } else {
        -1
    };

    seq.max_frame_width = if frm_dim_cfg.forced_max_frame_width != 0 {
        frm_dim_cfg.forced_max_frame_width
    } else {
        frm_dim_cfg.width
    };
    seq.max_frame_height = if frm_dim_cfg.forced_max_frame_height != 0 {
        frm_dim_cfg.forced_max_frame_height
    } else {
        frm_dim_cfg.height
    };
    seq.num_bits_width = if seq.max_frame_width > 1 {
        get_msb((seq.max_frame_width - 1) as u32) + 1
    } else {
        1
    };
    seq.num_bits_height = if seq.max_frame_height > 1 {
        get_msb((seq.max_frame_height - 1) as u32) + 1
    } else {
        1
    };
    debug_assert!(seq.num_bits_width <= 16);
    debug_assert!(seq.num_bits_height <= 16);

    seq.frame_id_length = FRAME_ID_LENGTH;
    seq.delta_frame_id_length = DELTA_FRAME_ID_LENGTH;

    seq.enable_dual_filter = tool_cfg.enable_dual_filter as u8;
    seq.order_hint_info.enable_dist_wtd_comp = oxcf.comp_type_cfg.enable_dist_wtd_comp as i32;
    seq.order_hint_info.enable_dist_wtd_comp &= seq.order_hint_info.enable_order_hint;
    seq.order_hint_info.enable_ref_frame_mvs = tool_cfg.ref_frame_mvs_present as i32;
    seq.order_hint_info.enable_ref_frame_mvs &= seq.order_hint_info.enable_order_hint;
    seq.enable_superres = oxcf.superres_cfg.enable_superres as u8;
    seq.enable_cdef = (tool_cfg.cdef_control != CDEF_NONE) as u8;
    seq.enable_restoration = tool_cfg.enable_restoration as u8;
    seq.enable_warped_motion = oxcf.motion_mode_cfg.enable_warped_motion as u8;
    seq.enable_interintra_compound = tool_cfg.enable_interintra_comp as u8;
    seq.enable_masked_compound = oxcf.comp_type_cfg.enable_masked_comp as u8;
    seq.enable_intra_edge_filter = oxcf.intra_mode_cfg.enable_intra_edge_filter as u8;
    seq.enable_filter_intra = oxcf.intra_mode_cfg.enable_filter_intra as u8;

    set_bitstream_level_tier(
        seq,
        frm_dim_cfg.width,
        frm_dim_cfg.height,
        oxcf.input_cfg.init_framerate,
    )?;

    // (one operating point)
    debug_assert!(seq.operating_points_cnt_minus_1 == 0);
    seq.operating_point_idc[0] = 0;
    Ok(())
}

/// Translation of `init_config_sequence()`.
fn init_config_sequence(
    ppi: &mut Av1Primary,
    seq_params: &mut SequenceHeader,
    oxcf: &Av1EncoderConfig,
) -> Result<(), ()> {
    let dec_model_cfg = &oxcf.dec_model_cfg;
    let color_cfg = &oxcf.color_cfg;

    ppi.use_svc = false;
    ppi.number_spatial_layers = 1;
    ppi.number_temporal_layers = 1;

    seq_params.profile = oxcf.profile;
    seq_params.bit_depth = oxcf.tool_cfg.bit_depth;
    seq_params.use_highbitdepth = oxcf.use_highbitdepth as u8;
    seq_params.color_primaries = color_cfg.color_primaries;
    seq_params.transfer_characteristics = color_cfg.transfer_characteristics;
    seq_params.matrix_coefficients = color_cfg.matrix_coefficients;
    seq_params.monochrome = oxcf.tool_cfg.enable_monochrome as u8;
    seq_params.chroma_sample_position = color_cfg.chroma_sample_position;
    seq_params.color_range = color_cfg.color_range;
    seq_params.timing_info_present = dec_model_cfg.timing_info_present as i32;
    // (the timing info fields: the timing info is never present)
    seq_params.display_model_info_present_flag =
        dec_model_cfg.display_model_info_present_flag as u8;
    seq_params.decoder_model_info_present_flag =
        dec_model_cfg.decoder_model_info_present_flag as u8;
    debug_assert!(
        !dec_model_cfg.decoder_model_info_present_flag && !dec_model_cfg.timing_info_present
    );
    seq_params.op_params[0].initial_display_delay = 10; // Default value (not signaled)

    if seq_params.monochrome != 0 {
        seq_params.subsampling_x = 1;
        seq_params.subsampling_y = 1;
    } else if seq_params.color_primaries == AOM_CICP_CP_BT_709
        && seq_params.transfer_characteristics == AOM_CICP_TC_SRGB
        && seq_params.matrix_coefficients == AOM_CICP_MC_IDENTITY
    {
        seq_params.subsampling_x = 0;
        seq_params.subsampling_y = 0;
    } else if seq_params.profile == 0 {
        seq_params.subsampling_x = 1;
        seq_params.subsampling_y = 1;
    } else if seq_params.profile == 1 {
        seq_params.subsampling_x = 0;
        seq_params.subsampling_y = 0;
    } else if seq_params.bit_depth == AOM_BITS_12 {
        seq_params.subsampling_x = oxcf.input_cfg.chroma_subsampling_x as i32;
        seq_params.subsampling_y = oxcf.input_cfg.chroma_subsampling_y as i32;
    } else {
        seq_params.subsampling_x = 1;
        seq_params.subsampling_y = 0;
    }
    av1_change_config_seq(ppi, seq_params, oxcf, None)
}

/// Translation of `init_config()`.
fn init_config(cpi: &mut Av1Comp, oxcf: &Av1EncoderConfig) -> Result<(), ()> {
    cpi.oxcf = *oxcf;
    cpi.framerate = oxcf.input_cfg.init_framerate;

    cpi.common.width = oxcf.frm_dim_cfg.width;
    cpi.common.height = oxcf.frm_dim_cfg.height;

    alloc_compressor_data(cpi)?;

    // Init SVC parameters.
    cpi.common.spatial_layer_id = 0;
    cpi.common.temporal_layer_id = 0;

    // change includes all joint functionality
    av1_change_config(cpi, oxcf, false)?;

    cpi.ref_frame_flags = 0;

    // (init_buffer_indices())
    for fb_idx in 0..REF_FRAMES as usize {
        cpi.common.remapped_ref_idx[fb_idx] = fb_idx as i32;
    }

    // (av1_noise_estimate_init(): the noise estimate is a real-time feature)
    Ok(())
}

/// Translation of `av1_change_config_seq()`.
pub(crate) fn av1_change_config_seq(
    ppi: &mut Av1Primary,
    seq_params: &mut SequenceHeader,
    oxcf: &Av1EncoderConfig,
    is_sb_size_changed: Option<&mut bool>,
) -> Result<(), ()> {
    let frm_dim_cfg = &oxcf.frm_dim_cfg;
    let dec_model_cfg = &oxcf.dec_model_cfg;
    let color_cfg = &oxcf.color_cfg;

    if seq_params.profile != oxcf.profile {
        seq_params.profile = oxcf.profile;
    }
    seq_params.bit_depth = oxcf.tool_cfg.bit_depth;
    seq_params.color_primaries = color_cfg.color_primaries;
    seq_params.transfer_characteristics = color_cfg.transfer_characteristics;
    seq_params.matrix_coefficients = color_cfg.matrix_coefficients;
    seq_params.monochrome = oxcf.tool_cfg.enable_monochrome as u8;
    seq_params.chroma_sample_position = color_cfg.chroma_sample_position;
    seq_params.color_range = color_cfg.color_range;

    debug_assert!(seq_params.profile > PROFILE_1 || seq_params.bit_depth <= AOM_BITS_10);

    seq_params.timing_info_present = dec_model_cfg.timing_info_present as i32;
    seq_params.display_model_info_present_flag =
        dec_model_cfg.display_model_info_present_flag as u8;
    seq_params.decoder_model_info_present_flag =
        dec_model_cfg.decoder_model_info_present_flag as u8;
    seq_params.op_params[0].initial_display_delay = 10; // Default value (not signaled)

    // (av1_update_film_grain_parameters_seq(): no film grain is configured,
    // and CONFIG_DENOISE's noise level is 0)
    seq_params.film_grain_params_present = (oxcf.noise_level > 0.0) as u8;

    let sb_size = seq_params.sb_size;
    // Superblock size should not be updated after the first key frame.
    if !ppi.seq_params_locked {
        set_sb_size(
            seq_params,
            av1_select_sb_size(
                oxcf,
                frm_dim_cfg.width,
                frm_dim_cfg.height,
                ppi.number_spatial_layers,
            ),
        );
        for i in 0..MAX_NUM_OPERATING_POINTS {
            seq_params.tier[i] = ((oxcf.tier_mask >> i) & 1) as u8;
        }
    }
    if let Some(changed) = is_sb_size_changed {
        if sb_size != seq_params.sb_size {
            *changed = true;
        }
    }

    // Init sequence level coding tools
    // This should not be called after the first key frame.
    if !ppi.seq_params_locked {
        seq_params.operating_points_cnt_minus_1 =
            if ppi.number_spatial_layers > 1 || ppi.number_temporal_layers > 1 {
                ppi.number_spatial_layers * ppi.number_temporal_layers - 1
            } else {
                0
            };
        av1_init_seq_coding_tools(ppi, seq_params, oxcf, ppi.use_svc)?;
    }
    seq_params.timing_info_present &= (seq_params.reduced_still_picture_hdr == 0) as i32;

    // (highbd_set_var_fns(): the variance functions are called directly)
    // (set_primary_rc_buffer_sizes(): the buffer model only steers CBR and
    // VBR rate control)
    Ok(())
}

/// Translation of `av1_change_config()`.
pub(crate) fn av1_change_config(
    cpi: &mut Av1Comp,
    oxcf: &Av1EncoderConfig,
    is_sb_size_changed: bool,
) -> Result<(), ()> {
    cpi.oxcf = *oxcf;

    // (av1_update_film_grain_parameters(): no film grain)

    // When user provides superres_mode = AOM_SUPERRES_AUTO, we still initialize
    // superres mode for current encoding = AOM_SUPERRES_NONE. This is to ensure
    // that any analysis (e.g. TPL) happening outside the main encoding loop still
    // happens at full resolution.
    // This value will later be set appropriately just before main encoding loop.
    cpi.superres_mode = if oxcf.superres_cfg.superres_mode == AOM_SUPERRES_AUTO {
        AOM_SUPERRES_NONE
    } else {
        oxcf.superres_cfg.superres_mode
    }; // default
    cpi.td.mb.e_mbd.bd = cpi.common.seq_params.bit_depth;

    // (level_params: no target level, so no level statistics are kept)
    debug_assert!(cpi.oxcf.target_seq_level_idx[0] >= SEQ_LEVELS);

    if has_no_stats_stage(cpi) && oxcf.rc_cfg.mode == AOM_Q {
        cpi.ppi.p_rc.baseline_gf_interval = FIXED_GF_INTERVAL;
    } else {
        cpi.ppi.p_rc.baseline_gf_interval = (MIN_GF_INTERVAL + MAX_GF_INTERVAL) / 2;
    }

    cpi.refresh_frame.golden_frame = false;
    cpi.refresh_frame.bwd_ref_frame = false;

    let features = &mut cpi.common.features;
    features.refresh_frame_context = if oxcf.tool_cfg.frame_parallel_decoding_mode {
        REFRESH_FRAME_CONTEXT_DISABLED
    } else {
        REFRESH_FRAME_CONTEXT_BACKWARD
    };
    if oxcf.tile_cfg.enable_large_scale_tile {
        features.refresh_frame_context = REFRESH_FRAME_CONTEXT_DISABLED;
    }

    // (the palette buffer and the convolution buffer: no palette, no inter
    // prediction)
    // The buffers 'tmp_pred_bufs[]' and 'comp_rd_buffer' are used in inter frames
    // to store intermediate inter mode prediction results and are not required
    // for allintra encoding mode. Hence, the memory allocations for these buffers
    // are avoided for allintra encoding mode.
    debug_assert!(cpi.oxcf.kf_cfg.key_freq_max == 0);

    av1_reset_segment_features(&mut cpi.common);

    // (av1_set_high_precision_mv(): motion vector precision is an inter
    // frame setting)

    // Under a configuration change, where maximum_buffer_size may change,
    // keep buffer level clipped to the maximum allowed buffer size.
    // (the buffer levels only steer CBR and VBR rate control)

    // Set up frame rate and related parameters rate control values.
    av1_new_framerate(cpi, cpi.framerate);

    // Set absolute upper and lower quality limits
    let rc_cfg = &oxcf.rc_cfg;
    cpi.rc.worst_quality = rc_cfg.worst_allowed_q;
    cpi.rc.best_quality = rc_cfg.best_allowed_q;

    let features = &mut cpi.common.features;
    features.interp_filter = if oxcf.tile_cfg.enable_large_scale_tile {
        EIGHTTAP_REGULAR
    } else {
        SWITCHABLE
    };
    features.switchable_motion_mode = is_switchable_motion_mode_allowed(
        features.allow_warped_motion,
        oxcf.motion_mode_cfg.enable_obmc,
    );

    let frm_dim_cfg = &cpi.oxcf.frm_dim_cfg;
    let cm = &mut cpi.common;
    if frm_dim_cfg.render_width > 0 && frm_dim_cfg.render_height > 0 {
        cm.render_width = frm_dim_cfg.render_width;
        cm.render_height = frm_dim_cfg.render_height;
    } else {
        cm.render_width = frm_dim_cfg.width;
        cm.render_height = frm_dim_cfg.height;
    }
    cm.width = frm_dim_cfg.width;
    cm.height = frm_dim_cfg.height;

    if (cpi.initial_dimensions.width != 0 || is_sb_size_changed)
        && (cm.width > cpi.initial_dimensions.width
            || cm.height > cpi.initial_dimensions.height
            || is_sb_size_changed)
    {
        av1_free_context_buffers(cm);
        alloc_compressor_data(cpi)?;
        realloc_segmentation_maps(cpi)?;
        cpi.initial_dimensions.width = 0;
        cpi.initial_dimensions.height = 0;
    }
    av1_update_frame_size(cpi);

    cpi.rc.is_src_frame_alt_ref = 0;

    let tile_cfg = cpi.oxcf.tile_cfg;
    set_tile_info(&mut cpi.common, &tile_cfg);

    cpi.ext_flags.refresh_frame.update_pending = false;
    cpi.ext_flags.refresh_frame_context_pending = false;

    // (check_reset_rc_flag(): a real-time rate control reset)
    Ok(())
}

/// Translation of `init_frame_info()`: the frame info only feeds the first
/// pass and the TPL model.
#[inline]
fn init_frame_info(_cpi: &mut Av1Comp) {}

/// Translation of `av1_create_primary_compressor()` (no lookahead
/// processing, no parallel frames; the function pointer tables are direct
/// calls; no TPL buffers).
pub(crate) fn av1_create_primary_compressor(
    ppi: &mut Av1Primary,
    seq_params: &mut SequenceHeader,
    num_lap_buffers: i32,
    oxcf: &Av1EncoderConfig,
) -> Result<(), ()> {
    ppi.seq_params_locked = false;
    ppi.lap_enabled = num_lap_buffers > 0;
    ppi.frames_left = oxcf.input_cfg.limit;
    ppi.num_fp_contexts = 1;

    init_config_sequence(ppi, seq_params, oxcf)?;

    av1_primary_rc_init(oxcf, &mut ppi.p_rc);

    // (scene cut detection: a two pass and lookahead feature)
    debug_assert!(!ppi.lap_enabled);
    Ok(())
}

/// Translation of `av1_create_compressor()`.
pub(crate) fn av1_create_compressor(
    cpi: &mut Av1Comp,
    oxcf: &Av1EncoderConfig,
    stage: i32,
) -> Result<(), ()> {
    cpi.compressor_stage = stage;

    cpi.do_frame_data_update = true;

    {
        let mi_params = &mut cpi.common.mi_params;
        debug_assert!(oxcf.pass != AOM_RC_FIRST_PASS && stage != LAP_STAGE);
        mi_params.mi_alloc_bsize = BLOCK_4X4;
    }

    // (cm->fc and cm->default_frame_context are zeroed boxes)
    *cpi.common.fc = FrameContext::default();
    *cpi.common.default_frame_context = FrameContext::default();

    init_config(cpi, oxcf)?;
    av1_rc_init(&cpi.oxcf, &mut cpi.rc);

    init_frame_info(cpi);
    // (init_frame_index_set(): the frame index set is a parallel frame
    // feature)

    cpi.common.current_frame.frame_number = 0;
    cpi.common.current_frame_id = -1;
    cpi.tile_data = Vec::new();
    realloc_segmentation_maps(cpi)?;

    cpi.refresh_frame.alt_ref_frame = false;

    cpi.time_stamps.first_ts_start = i64::MAX;

    debug_assert!(!is_stat_consumption_stage(cpi));
    // (obmc buffers: not for all intra coding)
    // (intra block copy hash buffers: intra block copy is never used with
    // the non-RD pick mode)

    av1_set_speed_features_framesize_independent(cpi, oxcf.speed);
    av1_set_speed_features_framesize_dependent(cpi, oxcf.speed);

    let mi_params = &cpi.common.mi_params;
    let mut max_mi_cols = mi_params.mi_cols;
    let mut max_mi_rows = mi_params.mi_rows;
    if oxcf.frm_dim_cfg.forced_max_frame_width != 0 {
        max_mi_cols = size_in_mi(oxcf.frm_dim_cfg.forced_max_frame_width);
    }
    if oxcf.frm_dim_cfg.forced_max_frame_height != 0 {
        max_mi_rows = size_in_mi(oxcf.frm_dim_cfg.forced_max_frame_height);
    }
    cpi.consec_zero_mv = try_vec(((max_mi_rows * max_mi_cols) >> 2) as usize, 0u8)?;

    {
        let bsize = BLOCK_16X16;
        let w = MI_SIZE_WIDE[bsize as usize] as i32;
        let h = MI_SIZE_HIGH[bsize as usize] as i32;
        let num_cols = (max_mi_cols + w - 1) / w;
        let num_rows = (max_mi_rows + h - 1) / h;
        cpi.ssim_rdmult_scaling_factors = try_vec((num_rows * num_cols) as usize, 0.0f64)?;
        // (tpl_rdmult_scaling_factors: no TPL model)
    }

    // av1_init_quantizer() is first called here. Add check in
    // av1_frame_init_quantizer() so that av1_init_quantizer is only
    // called later when needed. This will avoid unnecessary calls of
    // av1_init_quantizer() for every frame.
    av1_init_quantizer(
        &mut cpi.enc_quant_dequant_params,
        &cpi.common.quant_params,
        cpi.common.seq_params.bit_depth,
    );
    // (av1_qm_init(): quantization matrices are off)
    debug_assert!(!cpi.oxcf.q_cfg.using_qm);

    av1_loop_filter_init(&mut cpi.common.lf_info, &cpi.common.lf);
    cpi.common.superres_scale_denominator = SCALE_NUMERATOR as u8;
    cpi.common.superres_upscaled_width = oxcf.frm_dim_cfg.width;
    cpi.common.superres_upscaled_height = oxcf.frm_dim_cfg.height;
    // (av1_loop_restoration_precal(): loop restoration is off)

    cpi.use_ducky_encode = false;
    Ok(())
}

/// Translation of `av1_set_mv_search_params()`.
pub(crate) fn av1_set_mv_search_params(cpi: &mut Av1Comp) {
    let cm = &cpi.common;
    let mv_search_params = &mut cpi.mv_search_params;
    let max_mv_def = cm.width.max(cm.height);

    // Default based on max resolution.
    mv_search_params.mv_step_param = av1_init_search_range(max_mv_def);

    if cpi.sf.mv_sf.auto_mv_step_size != 0 {
        if frame_is_intra_only(cm) {
            // Initialize max_mv_magnitude for use in the first INTER frame
            // after a key/intra-only frame.
            mv_search_params.max_mv_magnitude = max_mv_def;
        } else {
            // Use adaptive mv steps based on previous frame stats for show frames and
            // internal arfs.
            let cur_update_type = cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize];
            let use_auto_mv_step = (cm.show_frame != 0 || cur_update_type == INTNL_ARF_UPDATE)
                && mv_search_params.max_mv_magnitude != -1
                && cpi.sf.mv_sf.auto_mv_step_size >= 2;
            if use_auto_mv_step {
                // Allow mv_steps to correspond to twice the max mv magnitude found
                // in the previous frame, capped by the default max_mv_magnitude based
                // on resolution.
                mv_search_params.mv_step_param =
                    av1_init_search_range(max_mv_def.min(2 * mv_search_params.max_mv_magnitude));
            }
            // Reset max_mv_magnitude based on update flag.
            if cpi.do_frame_data_update {
                mv_search_params.max_mv_magnitude = -1;
            }
        }
    }
}

/// Translation of `av1_init_search_range()` (mcomp.c).
fn av1_init_search_range(size: i32) -> i32 {
    // MAX_FULL_PEL_VAL / MAX_MVSEARCH_STEPS
    const MAX_MVSEARCH_STEPS: i32 = 11;
    const MAX_FULL_PEL_VAL: i32 = (1 << (MAX_MVSEARCH_STEPS - 1)) - 1;
    let mut sr = 0;
    // Minimum search size no matter what the passed in value.
    let size = size.max(16);

    while (size << sr) < MAX_FULL_PEL_VAL {
        sr += 1;
    }

    sr = sr.min(MAX_MVSEARCH_STEPS - 2);
    sr
}

/// Translation of `av1_set_screen_content_options()` (the non-RD pick mode
/// and the real-time mode turn the screen content tools off).
pub(crate) fn av1_set_screen_content_options(cpi: &Av1Comp, features: &mut FeatureFlags) {
    debug_assert!(cpi.common.seq_params.force_screen_content_tools == 2);
    debug_assert!(cpi.oxcf.tune_cfg.content != AOM_CONTENT_SCREEN);

    if cpi.oxcf.mode == REALTIME {
        features.allow_screen_content_tools = false;
        features.allow_intrabc = false;
        return;
    }

    if cpi.sf.rt_sf.use_nonrd_pick_mode != 0 && cpi.sf.rt_sf.hybrid_intra_pickmode == 0 {
        features.allow_screen_content_tools = false;
        features.allow_intrabc = false;
        return;
    }
    // (the source estimate of screen content is only reached with the RD
    // pick mode)
    debug_assert!(false);
}

/// Translation of `init_motion_estimation()`.
fn init_motion_estimation(cpi: &mut Av1Comp) {
    let cm = &cpi.common;
    let aligned_width = (cm.width + 7) & !7;
    let y_stride = aom_calc_y_stride(aligned_width, cpi.oxcf.border_in_pixels);
    let y_stride_src = if (cpi.oxcf.frm_dim_cfg.width != cm.width
        || cpi.oxcf.frm_dim_cfg.height != cm.height)
        || av1_superres_scaled(cm)
    {
        y_stride
    } else {
        match cpi.ppi.lookahead.as_ref() {
            Some(l) => l.buf[0].img.y_stride(),
            None => y_stride,
        }
    };
    let fpf_y_stride = if cm.cur_frame.is_some() {
        cm.cur_frame().buf.y_stride()
    } else {
        y_stride
    };

    let mv_search_params = &mut cpi.mv_search_params;
    // Update if search_site_cfg is uninitialized or the current frame has a new
    // stride
    let should_update = mv_search_params.search_site_cfg[SS_CFG_SRC][DIAMOND as usize].stride == 0
        || mv_search_params.search_site_cfg[SS_CFG_LOOKAHEAD][DIAMOND as usize].stride == 0
        || (y_stride != mv_search_params.search_site_cfg[SS_CFG_SRC][DIAMOND as usize].stride);

    if !should_update {
        return;
    }

    // Initialization of search_site_cfg for NUM_DISTINCT_SEARCH_METHODS.
    for i in DIAMOND..NUM_DISTINCT_SEARCH_METHODS {
        let level = if i == NSTEP_8PT || i == CLAMPED_DIAMOND {
            1
        } else {
            0
        };
        av1_init_motion_compensation(
            i,
            &mut mv_search_params.search_site_cfg[SS_CFG_SRC][i as usize],
            y_stride,
            level,
        );
        av1_init_motion_compensation(
            i,
            &mut mv_search_params.search_site_cfg[SS_CFG_LOOKAHEAD][i as usize],
            y_stride_src,
            level,
        );
    }

    // First pass search site config initialization.
    av1_init_motion_fpf(
        &mut mv_search_params.search_site_cfg[SS_CFG_FPF][DIAMOND as usize],
        fpf_y_stride,
    );
    for i in NSTEP..NUM_DISTINCT_SEARCH_METHODS {
        mv_search_params.search_site_cfg[SS_CFG_FPF][i as usize] =
            mv_search_params.search_site_cfg[SS_CFG_FPF][DIAMOND as usize].clone();
    }
}

/// Translation of `init_ref_frame_bufs()`.
fn init_ref_frame_bufs(cpi: &mut Av1Comp) {
    let cm = &mut cpi.common;
    if let Some(cur) = cm.cur_frame.take() {
        cm.buffer_pool.frame_bufs[cur].ref_count -= 1;
    }
    for i in 0..REF_FRAMES as usize {
        if let Some(r) = cm.ref_frame_map[i].take() {
            cm.buffer_pool.frame_bufs[r].ref_count -= 1;
        }
    }
    debug_assert!(cm.buffer_pool.frame_bufs.iter().all(|b| b.ref_count == 0));
}

/// Translation of `av1_check_initial_width()`.
pub(crate) fn av1_check_initial_width(
    cpi: &mut Av1Comp,
    use_highbitdepth: bool,
    subsampling_x: i32,
    subsampling_y: i32,
) {
    let seq_params = &cpi.common.seq_params;
    if cpi.initial_dimensions.width == 0
        || (seq_params.use_highbitdepth != 0) != use_highbitdepth
        || seq_params.subsampling_x != subsampling_x
        || seq_params.subsampling_y != subsampling_y
    {
        let seq_params = &mut cpi.common.seq_params;
        seq_params.subsampling_x = subsampling_x;
        seq_params.subsampling_y = subsampling_y;
        seq_params.use_highbitdepth = use_highbitdepth as u8;

        let speed = cpi.oxcf.speed;
        av1_set_speed_features_framesize_independent(cpi, speed);
        av1_set_speed_features_framesize_dependent(cpi, speed);

        // (av1_tf_info_alloc(): the temporal filter buffers, never used: the
        // temporal filter is off)
        init_ref_frame_bufs(cpi);

        init_motion_estimation(cpi); // TODO(agrange) This can be removed.

        cpi.initial_dimensions.width = cpi.common.width;
        cpi.initial_dimensions.height = cpi.common.height;
        cpi.initial_mbs = cpi.common.mi_params.mbs;
    }
}

/// Translation of `alloc_frame_mvs()` (encoder.h).
#[inline]
pub(crate) fn alloc_frame_mvs(cm: &mut Av1Common, buf: usize) -> Result<(), ()> {
    ensure_mv_buffer(buf, cm)?;
    let (w, h) = (cm.width, cm.height);
    let b = &mut cm.buffer_pool.frame_bufs[buf];
    b.width = w;
    b.height = h;
    Ok(())
}

/// Translation of `av1_set_frame_size()` (no size change, no restoration).
pub(crate) fn av1_set_frame_size(cpi: &mut Av1Comp, width: i32, height: i32) -> Result<(), ()> {
    let cm = &mut cpi.common;
    let num_planes = av1_num_planes(cm) as i32;

    // (a change of the coded frame size: libavif codes the configured size)
    debug_assert!(width == cm.width && height == cm.height);

    let cur = cm.cur_frame.ok_or(())?;
    alloc_frame_mvs(cm, cur)?;

    // Allocate above context buffers
    let above_contexts = &mut cm.above_contexts;
    if above_contexts.num_planes < num_planes
        || above_contexts.num_mi_cols < cm.mi_params.mi_cols
        || above_contexts.num_tile_rows < cm.tiles.rows
    {
        av1_free_above_context_buffers(above_contexts);
        av1_alloc_above_context_buffers(
            above_contexts,
            cm.tiles.rows,
            cm.mi_params.mi_cols,
            num_planes,
        )?;
    }

    let oxcf = &mut cpi.oxcf;
    oxcf.border_in_pixels = av1_get_enc_border_size(
        av1_is_resize_needed(oxcf),
        oxcf.kf_cfg.key_freq_max == 0,
        cm.seq_params.sb_size,
    );

    // Reset the frame pointers to the current frame size.
    let (w, h) = (cm.width, cm.height);
    let (ss_x, ss_y) = (cm.seq_params.subsampling_x, cm.seq_params.subsampling_y);
    let use_highbitdepth = cm.seq_params.use_highbitdepth as i32;
    let border = cpi.oxcf.border_in_pixels;
    let byte_alignment = cm.features.byte_alignment;
    aom_realloc_frame_buffer(
        &mut cm.cur_frame_mut().buf,
        w,
        h,
        ss_x,
        ss_y,
        use_highbitdepth,
        border,
        byte_alignment,
        0,
    )?;

    // (av1_init_cdef_worker(): the CDEF workers of multi-threaded CDEF,
    // which is never used, see cdef_restoration_frame())
    debug_assert!(!is_restoration_used(&cpi.common));

    init_motion_estimation(cpi);

    let cm = &mut cpi.common;
    for ref_frame in LAST_FRAME..=ALTREF_FRAME {
        if let Some(buf) = get_ref_frame_buf(cm, ref_frame) {
            let b = &cm.buffer_pool.frame_bufs[buf].buf;
            let (rw, rh) = (b.y_crop_width(), b.y_crop_height());
            let (cw, ch) = (cm.width, cm.height);
            let sf = &mut cm.ref_scale_factors[get_ref_frame_map_idx(cm, ref_frame) as usize];
            av1_setup_scale_factors_for_frame(sf, rw, rh, cw, ch);
            // (av1_is_scaled(): the references have the frame's size)
            debug_assert!(!av1_is_scaled(sf));
        }
    }

    av1_setup_scale_factors_for_frame(
        &mut cm.sf_identity,
        cm.width,
        cm.height,
        cm.width,
        cm.height,
    );

    // (set_ref_ptrs(): the block level code looks the scale factors up)
    Ok(())
}

/// Translation of `extend_frame_borders()` (no multi-threaded CDEF or loop
/// restoration).
fn extend_frame_borders(cpi: &mut Av1Comp) {
    let cm = &mut cpi.common;
    for plane in 0..av1_num_planes(cm) {
        let ybf = &mut cm.cur_frame_mut().buf;
        let h = ybf.crop_heights[(plane > 0) as usize];
        aom_extend_frame_borders_plane_row(ybf, plane, 0, h);
    }
}

/// Translation of `cdef_restoration_frame()`: CDEF is picked from the q
/// index and applied single-threaded (the CDEF workers only exist with
/// `num_mod_workers[MOD_CDEF] > 1`, see the module documentation of
/// ethread.rs); no superres, no loop restoration.
fn cdef_restoration_frame(
    cpi: &mut Av1Comp,
    use_cdef: bool,
    skip_apply_postproc_filters: u32,
) -> Result<(), ()> {
    if use_cdef {
        let use_screen_content_model = cpi.common.quant_params.base_qindex
            > cpi
                .sf
                .rt_sf
                .screen_content_cdef_filter_qindex_thresh
                .max(cpi.rc.best_quality + 5)
            && cpi.oxcf.tune_cfg.content == AOM_CONTENT_SCREEN;
        // Find CDEF parameters
        av1_cdef_search(cpi, use_screen_content_model)?;

        // Apply the filter
        if (skip_apply_postproc_filters & SKIP_APPLY_CDEF) == 0 {
            debug_assert!(!cpi.ppi.rtc_ref.non_reference_frame);
            av1_cdef_frame(&mut cpi.common)?;
        }
    }

    debug_assert!(!av1_superres_scaled(&cpi.common));
    Ok(())
}

/// Translation of `loopfilter_frame()`.
fn loopfilter_frame(cpi: &mut Av1Comp) -> Result<(), ()> {
    let num_planes = av1_num_planes(&cpi.common) as i32;
    cpi.td.mb.rdmult = cpi.rd.rdmult;

    let cm = &cpi.common;
    debug_assert!(
        !is_lossless_requested(&cpi.oxcf.rc_cfg)
            || (cm.features.coded_lossless && cm.features.all_lossless)
    );

    let use_loopfilter = is_loopfilter_used(cm) && !cpi.mt_info.pipeline_lpf_mt_with_enc;
    let use_cdef = is_cdef_used(cm);
    let use_superres = av1_superres_scaled(cm);
    let use_restoration = is_restoration_used(cm);

    let skip_apply_postproc_filters = derive_skip_apply_postproc_filters(
        cpi,
        use_loopfilter,
        use_cdef,
        use_superres,
        use_restoration,
    );

    if use_loopfilter {
        let lpf_pick = cpi.sf.lpf_sf.lpf_pick;
        av1_pick_filter_level(cpi, lpf_pick);
        let lf = &cpi.common.lf;
        if (lf.filter_level[0] != 0 || lf.filter_level[1] != 0)
            && (skip_apply_postproc_filters & SKIP_APPLY_LOOPFILTER) == 0
        {
            debug_assert!(!cpi.ppi.rtc_ref.non_reference_frame);
            // lpf_opt_level = 1 : Enables dual/quad loop-filtering.
            // lpf_opt_level is set to 1 if transform size search depth in inter
            // blocks is limited to one as quad loop filtering assumes that all the
            // transform blocks within a 16x8/8x16/16x16 prediction block are of the
            // same size. lpf_opt_level = 2 : Filters both chroma planes together, in
            // addition to enabling dual/quad loop-filtering. This is enabled when lpf
            // pick method is LPF_PICK_FROM_Q as u and v plane filter levels are
            // equal.
            let lpf_opt_level = get_lpf_opt_level(&cpi.sf);
            let lossless = cpi.td.mb.e_mbd.lossless;
            av1_loop_filter_frame(
                &mut cpi.common,
                &lossless,
                0,
                num_planes as usize,
                false,
                lpf_opt_level,
            )?;
        }
    }

    cdef_restoration_frame(cpi, use_cdef, skip_apply_postproc_filters)?;
    debug_assert!(!use_restoration);
    Ok(())
}

/// Translation of `get_lpf_opt_level()` (encoder.h).
#[inline]
fn get_lpf_opt_level(sf: &SpeedFeatures) -> i32 {
    let mut lpf_opt_level = 0;
    if is_inter_tx_size_search_level_one(&sf.tx_sf) {
        lpf_opt_level = if sf.lpf_sf.lpf_pick == LPF_PICK_FROM_Q {
            2
        } else {
            1
        };
    }
    lpf_opt_level
}

/// Translation of `is_inter_tx_size_search_level_one()` (encoder.h).
#[inline]
fn is_inter_tx_size_search_level_one(tx_sf: &TxSpeedFeatures) -> bool {
    tx_sf.inter_tx_size_search_init_depth_rect >= 1
        && tx_sf.inter_tx_size_search_init_depth_sqr >= 1
}

/// Translation of `update_motion_stat()`.
fn update_motion_stat(cpi: &mut Av1Comp) {
    let mi_params = &cpi.common.mi_params;
    let rc = &mut cpi.rc;
    let avg_cnt_zeromv = 100 * rc.cnt_zeromv / (mi_params.mi_rows * mi_params.mi_cols);
    // (no SVC)
    rc.avg_frame_low_motion = if rc.avg_frame_low_motion == 0 {
        avg_cnt_zeromv
    } else {
        (3 * rc.avg_frame_low_motion + avg_cnt_zeromv) / 4
    };
}

/// Translation of `encode_without_recode()`.
fn encode_without_recode(cpi: &mut Av1Comp) -> Result<(), ()> {
    let mut top_index = 0;
    let mut bottom_index = 0;
    let mut q = 0;
    let resize_pending = is_frame_resize_pending(cpi);
    // (no SVC: the scaler filter only matters with a scaled source)

    set_size_independent_vars(cpi);
    av1_setup_frame_size(cpi)?;
    cpi.common.prev_frame = get_primary_ref_frame_buf(&cpi.common);
    av1_set_size_dependent_vars(cpi, &mut q, &mut bottom_index, &mut top_index);
    av1_set_mv_search_params(cpi);

    // (the unscaled source is coded as it is: no resizing, no spatial
    // layers)
    debug_assert!(
        cpi.source().y_crop_width() == cpi.common.width
            && cpi.source().y_crop_height() == cpi.common.height
    );

    // (allocate_gradient_info_for_hog() and
    // allocate_src_var_of_4x4_sub_block_buf(): the macroblock's caches are
    // sized when used)

    if cpi.sf.part_sf.partition_search_type == VAR_BASED_PARTITION {
        variance_partition_alloc(cpi);
    }

    if cpi.common.current_frame.frame_type == KEY_FRAME
        || (cpi.sf.inter_sf.extra_prune_warped != 0 && cpi.refresh_frame.golden_frame)
    {
        copy_frame_prob_info(cpi);
    }

    // (av1_realloc_and_scale_if_required(): no scaling, the source is the
    // unscaled source)
    if frame_is_intra_only(&cpi.common) || resize_pending {
        let n = ((cpi.common.mi_params.mi_rows * cpi.common.mi_params.mi_cols) >> 2) as usize;
        cpi.consec_zero_mv[..n].fill(0);
    }

    // (cpi->last_source: the unscaled last source)

    // (av1_update_noise_estimate(): use_temporal_noise_estimate is only set
    // above 640x480)
    if cpi.sf.rt_sf.use_temporal_noise_estimate != 0 {
        return Err(());
    }

    // For 1 spatial layer encoding: if the (non-LAST) reference has different
    // resolution from the source then disable that reference. This is to avoid
    // significant increase in encode time from scaling the references in
    // av1_scale_references. Note GOLDEN is forced to update on the (first/tigger)
    // resized frame and ALTREF will be refreshed ~4 frames later, so both
    // references become available again after few frames.
    if cpi.svc.number_spatial_layers == 1 {
        for (ref_frame, flag) in [(GOLDEN_FRAME, AOM_GOLD_FLAG), (ALTREF_FRAME, AOM_ALT_FLAG)] {
            if cpi.ref_frame_flags & AV1_REF_FRAME_FLAG_LIST[ref_frame as usize] != 0 {
                if let Some(buf) = get_ref_frame_buf(&cpi.common, ref_frame) {
                    let r = &cpi.common.buffer_pool.frame_bufs[buf].buf;
                    if r.y_crop_width() != cpi.common.width
                        || r.y_crop_height() != cpi.common.height
                    {
                        cpi.ref_frame_flags ^= flag;
                    }
                }
            }
        }
    }

    if cpi.ppi.gf_group.frame_parallel_level[cpi.gf_frame_index as usize] == 0 {
        // av1_scale_if_required will scale the references (avoiding an
        // extra frame-copy). Only scale if the reference has different
        // resolution from the source, or the TL0 reference for spatial layers
        // (no SVC)
        if !frame_is_intra_only(&cpi.common) {
            av1_scale_references(cpi);
        }
    }

    let q_cfg = cpi.oxcf.q_cfg;
    av1_set_quantizer(
        &mut cpi.common,
        q_cfg.qm_minlevel,
        q_cfg.qm_maxlevel,
        q,
        q_cfg.enable_chroma_deltaq,
        q_cfg.enable_hdr_deltaq,
    );
    let speed = cpi.oxcf.speed;
    av1_set_speed_features_qindex_dependent(cpi, speed);
    if q_cfg.deltaq_mode != NO_DELTA_Q || q_cfg.enable_chroma_deltaq != 0 {
        av1_init_quantizer(
            &mut cpi.enc_quant_dequant_params,
            &cpi.common.quant_params,
            cpi.common.seq_params.bit_depth,
        );
    }
    av1_set_variance_partition_thresholds(cpi, q);
    av1_setup_frame(cpi);

    // Check if this high_source_sad (scene/slide change) frame should be
    // encoded at high/max QP, and if so, set the q and adjust some rate
    // control parameters.
    if cpi.sf.rt_sf.overshoot_detection_cbr == FAST_DETECTION_MAXQ && cpi.rc.high_source_sad != 0 {
        // (av1_encodedframe_overshoot_cbr(): the overshoot re-encode of scene
        // changes, not translated)
        return Err(());
    }

    debug_assert!(q_cfg.aq_mode != CYCLIC_REFRESH_AQ);
    av1_apply_active_map(cpi);
    debug_assert!(cpi.common.seg.enabled == 0);
    cpi.common.seg = Segmentation::default();
    {
        let cm = &mut cpi.common;
        let seg = cm.seg;
        segfeatures_copy(&mut cm.cur_frame_mut().seg, &seg);
        cm.cur_frame_mut().seg.enabled = seg.enabled;
    }

    // (the RTC temporal filter and the PSNR source copy: off)

    // Set the motion vector precision based on mv stats from the last coded
    // frame.
    if !frame_is_intra_only(&cpi.common) {
        av1_pick_and_set_high_precision_mv(cpi, q);
    }

    // transform / motion compensation build reconstruction frame
    av1_encode_frame(cpi)?;

    // Update some stats from cyclic refresh.
    // (no cyclic refresh)

    if !cpi.rc.rtc_external_ratectrl && !frame_is_intra_only(&cpi.common) {
        update_motion_stat(cpi);
    }

    // Adjust the refresh of the golden (longer-term) reference based on QP
    // selected for this frame. This is for CBR with 1 layer/non-svc RTC mode.
    if !frame_is_intra_only(&cpi.common)
        && cpi.oxcf.rc_cfg.mode == AOM_CBR
        && cpi.oxcf.mode == REALTIME
        && cpi.svc.number_spatial_layers == 1
        && cpi.svc.number_temporal_layers == 1
        && !cpi.rc.rtc_external_ratectrl
        && cpi.sf.rt_sf.gf_refresh_based_on_qp != 0
    {
        av1_adjust_gf_refresh_qp_one_pass_rt(cpi);
    }

    Ok(())
}

/// Translation of `encode_with_recode_loop_and_filter()` (no recode loop
/// in one pass, no PSNR).
fn encode_with_recode_loop_and_filter(
    cpi: &mut Av1Comp,
    dest: &mut [u8],
    largest_tile_id: &mut i32,
) -> Result<usize, ()> {
    // (the frame probability update flags of the recodes: the probabilities
    // are updated in encode_frame_internal())
    debug_assert!(cpi.sf.hl_sf.recode_loop == DISALLOW_RECODE);
    encode_without_recode(cpi)?;

    // Special case code to reduce pulsing when key frames are forced at a
    // fixed interval. Note the reconstruction error if it is the frame before
    // the force key frame
    // (p_rc.next_key_frame_forced is a two pass flag)
    debug_assert!(cpi.ppi.p_rc.next_key_frame_forced == 0);

    let cm = &mut cpi.common;
    let seq_params = cm.seq_params;
    let (render_width, render_height) = (cm.render_width, cm.render_height);
    let buf = &mut cm.cur_frame_mut().buf;
    buf.color_primaries = seq_params.color_primaries;
    buf.transfer_characteristics = seq_params.transfer_characteristics;
    buf.matrix_coefficients = seq_params.matrix_coefficients;
    buf.monochrome = seq_params.monochrome;
    buf.chroma_sample_position = seq_params.chroma_sample_position;
    buf.color_range = seq_params.color_range;
    buf.render_width = render_width;
    buf.render_height = render_height;

    if !cpi.mt_info.pipeline_lpf_mt_with_enc {
        set_postproc_filter_default_params(&mut cpi.common);
    }

    if !cpi.common.features.allow_intrabc {
        loopfilter_frame(cpi)?;
    }

    if cpi.oxcf.mode != ALLINTRA && !cpi.ppi.rtc_ref.non_reference_frame {
        extend_frame_borders(cpi);
    }

    // (film grain: the content is never AOM_CONTENT_FILM)

    av1_finalize_encoded_frame(cpi);
    // Build the bitstream
    cpi.rc.coefficient_size = 0;
    let size = av1_pack_bitstream(cpi, dest, largest_tile_id)?;

    // (sse and rate: not asked for)
    Ok(size)
}

/// Translation of `encode_frame_to_data_rate()`.
fn encode_frame_to_data_rate(cpi: &mut Av1Comp, dest: &mut [u8]) -> Result<usize, ()> {
    // (cpi->td.mb.e_mbd.cur_buf = cpi->source)
    cpi.td.mb.e_mbd.cur_buf_flags = cpi.source().flags;
    if frame_is_intra_only(&cpi.common) {
        let mut features = cpi.common.features;
        av1_set_screen_content_options(cpi, &mut features);
        cpi.common.features = features;
    }

    // (calculate_frame_avg_haar_energy(): returns at once in one pass
    // real-time and with no perceptual delta q)

    let cm = &mut cpi.common;
    // frame type has been decided outside of this function call
    let frame_type = cm.current_frame.frame_type;
    cm.cur_frame_mut().frame_type = frame_type;

    let tile_cfg = cpi.oxcf.tile_cfg;
    cm.tiles.large_scale = tile_cfg.enable_large_scale_tile as u32;
    cm.tiles.single_tile_decoding = tile_cfg.enable_single_tile_decoding as u32;

    cm.features.allow_ref_frame_mvs &= frame_might_allow_ref_frame_mvs(cm);
    // features->allow_ref_frame_mvs needs to be written into the frame header
    // while cm->tiles.large_scale is 1, therefore, "cm->tiles.large_scale=1" case
    // is separated from frame_might_allow_ref_frame_mvs().
    cm.features.allow_ref_frame_mvs &= cm.tiles.large_scale == 0;

    cm.features.allow_warped_motion =
        cpi.oxcf.motion_mode_cfg.allow_warped_motion && frame_might_allow_warped_motion(cm);

    cpi.last_frame_type = cm.current_frame.frame_type;

    if frame_is_intra_only(cm) {
        cpi.frames_since_last_update = 0;
    }

    debug_assert!(!frame_is_sframe(cm) && !encode_show_existing_frame(cm));

    // (integer motion vectors: the screen content tools are off)
    cm.features.cur_frame_force_integer_mv = false;

    // This is used by av1_pack_bitstream. So this needs to be set in case of
    // row-mt where the encoding code will use a temporary structure.
    cpi.td.mb.e_mbd.cur_frame_force_integer_mv = cm.features.cur_frame_force_integer_mv as i32;

    // Set default state for segment based loop filter update flags.
    cm.lf.mode_ref_delta_update = 0;

    // Set various flags etc to special state if it is a key frame.
    if frame_is_intra_only(cm) || frame_is_sframe(cm) {
        // Reset the loop filter deltas and segmentation map.
        av1_reset_segment_features(cm);

        // If segmentation is enabled force a map update for key frames.
        debug_assert!(cm.seg.enabled == 0);
    }
    if tile_cfg.mtu == 0 {
        cpi.num_tg = tile_cfg.num_tile_groups;
    } else {
        // Use a default value for the purposes of weighting costs in probability
        // updates
        cpi.num_tg = DEFAULT_MAX_NUM_TG;
    }

    // For 1 pass CBR, check if we are dropping this frame.
    // Never drop on key frame.
    if has_no_stats_stage(cpi)
        && cpi.oxcf.rc_cfg.mode == AOM_CBR
        && cpi.common.current_frame.frame_type != KEY_FRAME
        && av1_rc_drop_frame(cpi)
    {
        // (frame dropping needs rc_dropframe_thresh, which libavif leaves
        // at 0)
        return Err(());
    }

    if cpi.oxcf.tune_cfg.tuning == AOM_TUNE_SSIM {
        av1_set_mb_ssim_rdmult_scaling(cpi);
    }

    // (the perceptual and user rating based delta q modes are not used)
    debug_assert!(cpi.common.seq_params.frame_id_numbers_present_flag == 0);

    match cpi.oxcf.algo_cfg.cdf_update_mode {
        0 => {
            // No CDF update for any frames(4~6% compression loss).
            cpi.common.features.disable_cdf_update = true;
        }
        1 => {
            // Enable CDF update for all frames.
            if cpi.sf.rt_sf.disable_cdf_update_non_reference_frame
                && cpi.ppi.rtc_ref.non_reference_frame
                && cpi.rc.frames_since_key > 2
            {
                cpi.common.features.disable_cdf_update = true;
            } else {
                cpi.common.features.disable_cdf_update = false;
            }
        }
        _ => {
            // Strategically determine at which frames to do CDF update.
            // Currently only enable CDF update for all-intra and no-show frames(4%
            // compression loss).
            // TODO(huisu@google.com): design schemes for various trade-offs between
            // compression quality and decoding speed.
            let cm = &mut cpi.common;
            cm.features.disable_cdf_update = !(frame_is_intra_only(cm) || cm.show_frame == 0);
        }
    }

    let mut largest_tile_id = 0;
    debug_assert!(!av1_superres_in_recode_allowed(cpi));
    let orig_superres_mode = cpi.superres_mode; // save
    cpi.superres_mode = cpi.oxcf.superres_cfg.superres_mode;
    let size = encode_with_recode_loop_and_filter(cpi, dest, &mut largest_tile_id)?;
    cpi.superres_mode = orig_superres_mode; // restore

    // Update reference frame ids for reference frames this frame will overwrite
    // (no frame ids)

    if cpi.svc.spatial_layer_id == cpi.svc.number_spatial_layers - 1 {
        cpi.svc.num_encoded_top_layer += 1;
    }

    debug_assert!(cpi.common.seg.enabled == 0);

    if cpi.ppi.gf_group.frame_parallel_level[cpi.gf_frame_index as usize] == 0
        && !frame_is_intra_only(&cpi.common)
    {
        release_scaled_references(cpi);
    }

    let cm = &mut cpi.common;
    if cm.show_frame != 0 {
        cpi.last_show_frame_buf = cm.cur_frame;
    }

    if cm.features.refresh_frame_context == REFRESH_FRAME_CONTEXT_BACKWARD {
        *cm.fc = (*cpi.tile_data[largest_tile_id as usize].tctx).clone();
        av1_reset_cdf_symbol_counters(&mut cm.fc);
    }
    if cm.tiles.large_scale == 0 {
        let fc = cm.fc.clone();
        cm.cur_frame_mut().frame_context = fc;
    }

    cpi.last_frame_type = cm.current_frame.frame_type;

    if cm.features.disable_cdf_update {
        cpi.frames_since_last_update += 1;
    } else {
        cpi.frames_since_last_update = 1;
    }

    // Clear the one shot update flags for segmentation map and mode/ref loop
    // filter deltas.
    cm.seg.update_map = 0;
    cm.seg.update_data = 0;
    cm.lf.mode_ref_delta_update = 0;

    // A droppable frame might not be shown but it always
    // takes a space in the gf group. Therefore, even when
    // it is not shown, we still need update the count down.
    if cm.show_frame != 0 {
        // (update_frame_index_set(): parallel frames only)
        cm.current_frame.frame_number += 1;
    }

    Ok(size)
}

/// `EncodeFrameParams`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EncodeFrameParams {
    pub(crate) error_resilient_mode: bool,
    pub(crate) frame_type: FrameType,
    pub(crate) primary_ref_frame: i32,
    pub(crate) order_offset: i32,
    pub(crate) show_frame: i32,
    pub(crate) refresh_frame_flags: i32,
    pub(crate) show_existing_frame: i32,
    pub(crate) existing_fb_idx_to_show: i32,
    pub(crate) ref_frame_flags: i32,
    pub(crate) remapped_ref_idx: [i32; REF_FRAMES as usize],
    pub(crate) refresh_frame: RefreshFrameInfo,
    pub(crate) speed: i32,
}

/// Translation of `av1_encode()`: `frame_input`'s sources are
/// `cpi.source_buf` and `cpi.last_source_buf` (moved there by the caller).
pub(crate) fn av1_encode(
    cpi: &mut Av1Comp,
    dest: &mut [u8],
    frame_input: &EncodeFrameInput,
    frame_params: &EncodeFrameParams,
) -> Result<usize, ()> {
    // (cpi->unscaled_source = cpi->source = frame_input->source;
    // cpi->unscaled_last_source = frame_input->last_source)
    debug_assert!(frame_input.has_last_source == cpi.has_last_source);
    let cm = &mut cpi.common;

    cm.current_frame.refresh_frame_flags = frame_params.refresh_frame_flags;
    cm.features.error_resilient_mode = frame_params.error_resilient_mode;
    cm.features.primary_ref_frame = frame_params.primary_ref_frame;
    cm.current_frame.frame_type = frame_params.frame_type;
    cm.show_frame = frame_params.show_frame;
    cpi.ref_frame_flags = frame_params.ref_frame_flags;
    cpi.speed = frame_params.speed;
    cm.show_existing_frame = frame_params.show_existing_frame;
    cpi.existing_fb_idx_to_show = frame_params.existing_fb_idx_to_show;

    cm.remapped_ref_idx = frame_params.remapped_ref_idx;

    cpi.refresh_frame = frame_params.refresh_frame;

    let current_frame = &mut cm.current_frame;
    if current_frame.frame_type == KEY_FRAME
        && cpi.ppi.gf_group.refbuf_state[cpi.gf_frame_index as usize] == REFBUF_RESET
    {
        current_frame.frame_number = 0;
    }

    current_frame.order_hint =
        (current_frame.frame_number as i32 + frame_params.order_offset) as u32;

    current_frame.display_order_hint = current_frame.order_hint;
    current_frame.order_hint %= 1u32 << (cm.seq_params.order_hint_info.order_hint_bits_minus_1 + 1);

    current_frame.pyramid_level = get_true_pyr_level(
        cpi.ppi.gf_group.layer_depth[cpi.gf_frame_index as usize] as i32,
        current_frame.display_order_hint as i32,
        cpi.ppi.gf_group.max_layer_depth,
    ) as u32;

    debug_assert!(!is_stat_generation_stage(cpi));
    debug_assert!(cpi.oxcf.pass == AOM_RC_ONE_PASS || cpi.oxcf.pass >= AOM_RC_SECOND_PASS);
    encode_frame_to_data_rate(cpi, dest)
}

/// Translation of `av1_receive_raw_frame()`.
pub(crate) fn av1_receive_raw_frame(
    cpi: &mut Av1Comp,
    frame_flags: u32,
    sd: &Yv12BufferConfig,
    sd_planes: [&[u16]; 3],
    time_stamp: i64,
    end_time: i64,
) -> Result<(), ()> {
    let seq_params = &cpi.common.seq_params;
    let subsampling_x = sd.subsampling_x;
    let subsampling_y = sd.subsampling_y;
    let use_highbitdepth = (sd.flags & YV12_FLAG_HIGHBITDEPTH) != 0;

    // (no denoising: the noise level is 0)
    let lookahead = cpi.ppi.lookahead.as_mut().ok_or(())?;
    av1_lookahead_push(
        lookahead,
        sd,
        sd_planes,
        time_stamp,
        end_time,
        use_highbitdepth,
        frame_flags,
    )?;

    if seq_params.profile == PROFILE_0
        && seq_params.monochrome == 0
        && (subsampling_x != 1 || subsampling_y != 1)
    {
        // aom_internal_error(): "Non-4:2:0 color format requires profile 1 or 2"
        return Err(());
    }
    if seq_params.profile == PROFILE_1 && !(subsampling_x == 0 && subsampling_y == 0) {
        // aom_internal_error(): "Profile 1 requires 4:4:4 color format"
        return Err(());
    }
    if seq_params.profile == PROFILE_2
        && seq_params.bit_depth <= AOM_BITS_10
        && !(subsampling_x == 1 && subsampling_y == 0)
    {
        // aom_internal_error(): "Profile 2 bit-depth <= 10 requires 4:2:2
        // color format"
        return Err(());
    }

    Ok(())
}

/// `AV1_COMP_DATA`
#[derive(Debug, Default)]
pub(crate) struct Av1CompData {
    pub(crate) frame_size: usize,
    pub(crate) lib_flags: u32,
    pub(crate) ts_frame_start: i64,
    pub(crate) ts_frame_end: i64,
    pub(crate) flush: bool,
    pub(crate) pop_lookahead: bool,
    /// `*cpi_data->timestamp_ratio`: `{ num, den }`
    pub(crate) timestamp_ratio: (i64, i64),
}

/// Translation of `update_keyframe_counters()`.
#[inline]
fn update_keyframe_counters(cpi: &mut Av1Comp) {
    if cpi.common.show_frame != 0 && cpi.rc.frames_to_key != 0 {
        // (!CONFIG_REALTIME_ONLY: the first pass statistics queue is empty
        // in one pass)
        if cpi.svc.spatial_layer_id == cpi.svc.number_spatial_layers - 1 {
            cpi.rc.frames_since_key += 1;
            cpi.rc.frames_to_key -= 1;
            cpi.rc.frames_to_fwd_kf -= 1;
        }
    }
}

/// Translation of `update_frames_till_gf_update()`.
#[inline]
fn update_frames_till_gf_update(cpi: &mut Av1Comp) {
    // TODO(weitinglin): Updating this counter for is_frame_droppable
    // is a work-around to handle the condition when a frame is drop.
    // We should fix the cpi->common.show_frame flag
    // instead of checking the other condition to update the counter properly.
    if cpi.common.show_frame != 0
        || is_frame_droppable(&cpi.ppi.rtc_ref, &cpi.ext_flags.refresh_frame)
    {
        // Decrement count down till next gf
        if cpi.rc.frames_till_gf_update_due > 0 {
            cpi.rc.frames_till_gf_update_due -= 1;
        }
    }
}

/// Translation of `update_gf_group_index()`.
#[inline]
fn update_gf_group_index(cpi: &mut Av1Comp) {
    // Increment the gf group index ready for the next frame.
    cpi.gf_frame_index += 1;
}

/// Translation of `update_fb_of_context_type()`.
fn update_fb_of_context_type(cpi: &Av1Comp, fb_of_context_type: &mut [i32; REF_FRAMES as usize]) {
    let cm = &cpi.common;
    let current_frame_ref_type = get_current_frame_ref_type(cpi) as usize;

    if frame_is_intra_only(cm)
        || cm.features.error_resilient_mode
        || cpi.ext_flags.use_primary_ref_none
    {
        for i in 0..REF_FRAMES as usize {
            fb_of_context_type[i] = -1;
        }
        fb_of_context_type[current_frame_ref_type] = if cm.show_frame != 0 {
            get_ref_frame_map_idx(cm, GOLDEN_FRAME)
        } else {
            get_ref_frame_map_idx(cm, ALTREF_FRAME)
        };
    }

    if !encode_show_existing_frame(cm) {
        // Refresh fb_of_context_type[]: see encoder.h for explanation
        if cm.current_frame.frame_type == KEY_FRAME {
            // All ref frames are refreshed, pick one that will live long enough
            fb_of_context_type[current_frame_ref_type] = 0;
        } else {
            // If more than one frame is refreshed, it doesn't matter which one we
            // pick so pick the first.  LST sometimes doesn't refresh any: this is ok
            for i in 0..REF_FRAMES {
                if cm.current_frame.refresh_frame_flags & (1 << i) != 0 {
                    fb_of_context_type[current_frame_ref_type] = i;
                    break;
                }
            }
        }
    }
}

/// Translation of `update_rc_counts()`.
fn update_rc_counts(cpi: &mut Av1Comp) {
    update_keyframe_counters(cpi);
    update_frames_till_gf_update(cpi);
    update_gf_group_index(cpi);
}

/// Translation of `update_end_of_frame_stats()`.
fn update_end_of_frame_stats(cpi: &mut Av1Comp) {
    if cpi.do_frame_data_update {
        // Store current frame loopfilter levels in ppi, if update flag is set.
        if cpi.common.show_existing_frame == 0 {
            let lf = &cpi.common.lf;
            cpi.ppi.filter_level[0] = lf.filter_level[0];
            cpi.ppi.filter_level[1] = lf.filter_level[1];
            cpi.ppi.filter_level_u = lf.filter_level_u;
            cpi.ppi.filter_level_v = lf.filter_level_v;
        }
    }
    // Store frame level mv_stats from cpi to ppi.
    cpi.ppi.mv_stats = cpi.mv_stats;
}

// Updates frame level stats related to global motion
/// Translation of `update_gm_stats()`.
fn update_gm_stats(cpi: &mut Av1Comp) {
    let update_type = cpi.ppi.gf_group.update_type[cpi.gf_frame_index as usize] as usize;
    let mut is_gm_present = 0;

    // Check if the current frame has any valid global motion model across its
    // reference frames
    for i in 0..REF_FRAMES as usize {
        if cpi.common.global_motion[i].wmtype != IDENTITY {
            is_gm_present = 1;
            break;
        }
    }
    if cpi.ppi.valid_gm_model_found[update_type] == i32::MAX {
        cpi.ppi.valid_gm_model_found[update_type] = is_gm_present;
    } else {
        cpi.ppi.valid_gm_model_found[update_type] |= is_gm_present;
    }
}

/// Translation of `av1_post_encode_updates()`.
pub(crate) fn av1_post_encode_updates(cpi: &mut Av1Comp, cpi_data: &Av1CompData) {
    update_gm_stats(cpi);

    // (!CONFIG_REALTIME_ONLY: no two pass statistics)

    if !is_stat_generation_stage(cpi) && !cpi.is_dropped_frame {
        // Before calling refresh_reference_frames(), copy ppi->ref_frame_map_copy
        // to cm->ref_frame_map for frame_parallel_level 2 frame in a parallel
        // encode set of lower layer frames.
        // TODO(Remya): Move ref_frame_map from AV1_COMMON to AV1_PRIMARY to avoid
        // copy.
        // (no parallel frames)
        refresh_reference_frames(cpi);
        av1_rc_postencode_update(cpi, cpi_data.frame_size as u64);
    }

    if cpi_data.pop_lookahead {
        if let Some(lookahead) = cpi.ppi.lookahead.as_mut() {
            av1_lookahead_pop(lookahead, cpi_data.flush, cpi.compressor_stage);
        }
    }
    if cpi.common.show_frame != 0 {
        cpi.ppi.ts_start_last_show_frame = cpi_data.ts_frame_start;
        cpi.ppi.ts_end_last_show_frame = cpi_data.ts_frame_end;
    }
    // (no level statistics)

    if !is_stat_generation_stage(cpi) {
        let mut fb_of_context_type = cpi.ppi.fb_of_context_type;
        update_fb_of_context_type(cpi, &mut fb_of_context_type);
        cpi.ppi.fb_of_context_type = fb_of_context_type;
        update_rc_counts(cpi);
        update_end_of_frame_stats(cpi);
    }

    // (no third pass, no SVC, no PSNR, no second pass log)
}

/// Translation of `av1_get_compressed_data()`: returns `None` (the C code's
/// -1) when no frame was coded.
pub(crate) fn av1_get_compressed_data(
    cpi: &mut Av1Comp,
    cpi_data: &mut Av1CompData,
    dest: &mut [u8],
) -> Result<Option<()>, ()> {
    debug_assert!(!(cpi.ppi.use_svc && cpi.ppi.number_spatial_layers > 1));

    cpi.is_dropped_frame = false;
    cpi.common.showable_frame = 0;
    cpi_data.frame_size = 0;
    av1_set_high_precision_mv(cpi, true, false);

    // Normal defaults
    cpi.common.features.refresh_frame_context = if cpi.oxcf.tool_cfg.frame_parallel_decoding_mode {
        REFRESH_FRAME_CONTEXT_DISABLED
    } else {
        REFRESH_FRAME_CONTEXT_BACKWARD
    };
    if cpi.oxcf.tile_cfg.enable_large_scale_tile {
        cpi.common.features.refresh_frame_context = REFRESH_FRAME_CONTEXT_DISABLED;
    }
    if assign_cur_frame_new_fb(&mut cpi.common).is_none() {
        // aom_internal_error(): "Failed to allocate new cur_frame"
        return Err(());
    }

    let result = av1_encode_strategy(cpi, cpi_data, dest);
    match result {
        Ok(None) => {
            // Returning -1 indicates no frame encoded; more input is required
            // (the unused frame buffer is released, as the next call's
            // assign_cur_frame_new_fb() does)
            if let Some(cur) = cpi.common.cur_frame.take() {
                cpi.common.buffer_pool.frame_bufs[cur].ref_count -= 1;
            }
            Ok(None)
        }
        Ok(Some(())) => Ok(Some(())),
        Err(()) => Err(()),
    }
}

// ---------------------------------------------------------------------------
// Small helpers of other files the pipeline reaches
// ---------------------------------------------------------------------------

/// Translation of `set_sb_size()` (av1_common_int.h).
#[inline]
pub(crate) fn set_sb_size(seq_params: &mut SequenceHeader, sb_size: BlockSize) {
    seq_params.sb_size = sb_size;
    seq_params.mib_size = MI_SIZE_WIDE[sb_size as usize] as i32;
    seq_params.mib_size_log2 = MI_SIZE_WIDE_LOG2[sb_size as usize] as i32;
}

/// Translation of `is_coded_lossless()` (av1_common_int.h).
#[inline]
pub(crate) fn is_coded_lossless(cm: &Av1Common, xd: &MacroBlockD) -> bool {
    let mut coded_lossless = true;
    if cm.seg.enabled != 0 {
        for i in 0..MAX_SEGMENTS {
            if xd.lossless[i] == 0 {
                coded_lossless = false;
                break;
            }
        }
    } else {
        coded_lossless = xd.lossless[0] != 0;
    }
    coded_lossless
}

/// Translation of `av1_superres_scaled()` (resize.h).
#[inline]
pub(crate) fn av1_superres_scaled(cm: &Av1Common) -> bool {
    // Note: for some corner cases (e.g. cm->width of 1), there may be no scaling
    // required even though cm->superres_scale_denominator != SCALE_NUMERATOR.
    // So, the following check is more accurate.
    cm.width != cm.superres_upscaled_width
}

/// Translation of `av1_resize_scaled()` (resize.h).
#[inline]
pub(crate) fn av1_resize_scaled(cm: &Av1Common) -> bool {
    cm.superres_upscaled_width != cm.render_width || cm.superres_upscaled_height != cm.render_height
}

/// Translation of `av1_frame_scaled()` (resize.h).
#[inline]
pub(crate) fn av1_frame_scaled(cm: &Av1Common) -> bool {
    av1_superres_scaled(cm) || av1_resize_scaled(cm)
}

/// Translation of `coded_to_superres_mi()` (resize.h).
#[inline]
pub(crate) fn coded_to_superres_mi(mi_col: i32, denom: i32) -> i32 {
    (mi_col * denom + SCALE_NUMERATOR / 2) / SCALE_NUMERATOR
}

/// Translation of `is_switchable_motion_mode_allowed()` (encoder.h).
#[inline]
pub(crate) fn is_switchable_motion_mode_allowed(
    allow_warped_motion: bool,
    enable_obmc: bool,
) -> bool {
    allow_warped_motion || enable_obmc
}

/// Translation of `av1_reset_segment_features()` (segmentation.c).
pub(crate) fn av1_reset_segment_features(cm: &mut Av1Common) {
    let seg = &mut cm.seg;

    // Set up default state for MB feature flags
    seg.enabled = 0;
    seg.update_map = 0;
    seg.update_data = 0;
    av1_clearall_segfeatures(seg);
}

/// Translation of `av1_disable_segfeature()` (segmentation.c).
#[inline]
pub(crate) fn av1_disable_segfeature(
    seg: &mut Segmentation,
    segment_id: usize,
    feature_id: SegLvlFeatures,
) {
    seg.feature_mask[segment_id] &= !(1u32 << feature_id);
}

/// Translation of `get_true_pyr_level()` (encoder.h).
#[inline]
pub(crate) fn get_true_pyr_level(frame_level: i32, frame_order: i32, max_layer_depth: i32) -> i32 {
    if frame_order == 0 {
        // Keyframe case
        return 1;
    } else if frame_level == MAX_ARF_LAYERS {
        // Leaves
        return max_layer_depth;
    } else if frame_level == (MAX_ARF_LAYERS + 1) {
        // Altrefs
        return 1;
    }
    frame_level.max(1)
}

/// The quantizer index range, for the rate control clamps.
pub(crate) const QINDEX_MAX: i32 = QINDEX_RANGE - 1;
