// Rust translation of av1/common/av1_common_int.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The state shared by encoder and decoder (`AV1_COMMON`) and its helpers.
//! Frame buffer pointers are indices into the buffer pool; the sequence
//! header lives here (upstream points `cm->seq_params` at the primary
//! compressor's copy, and the saver has exactly one compressor). The
//! film grain, global motion, CDEF and loop restoration state the saver
//! never uses beyond signaling "off" is left out.

use super::av1_loopfilter::{LoopFilterInfoN, Loopfilter};
use super::blockd::*;
use super::entropy::EntropyContext;
use super::entropymode::FrameContext;
use super::enums::*;
use super::mv::IntMv;
use super::scale::ScaleFactors;
use super::seg_common::{Segmentation, MAX_SEGMENTS};
use super::tile_common::TileInfo;
use crate::aom::aom_dsp::aom_dsp_common::align_power_of_two;
use crate::aom::aom_dsp::prob::{aom_icdf, CDF_PROB_TOP};
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;

pub(crate) const CDEF_MAX_STRENGTHS: usize = 16;

/* Constant values while waiting for the sequence header */
pub(crate) const FRAME_ID_LENGTH: i32 = 15;
pub(crate) const DELTA_FRAME_ID_LENGTH: i32 = 14;

pub(crate) const FRAME_CONTEXTS: i32 = FRAME_BUFFERS + 1;
pub(crate) const FRAME_CONTEXT_DEFAULTS: i32 = FRAME_CONTEXTS - 1;
pub(crate) const PRIMARY_REF_BITS: i32 = 3;
pub(crate) const PRIMARY_REF_NONE: i32 = 7;

pub(crate) const NUM_PING_PONG_BUFFERS: i32 = 2;

pub(crate) const MAX_NUM_TEMPORAL_LAYERS: usize = 8;
pub(crate) const MAX_NUM_SPATIAL_LAYERS: usize = 4;
pub(crate) const MAX_NUM_OPERATING_POINTS: usize = MAX_NUM_TEMPORAL_LAYERS * MAX_NUM_SPATIAL_LAYERS;

// (REFERENCE_MODE)
pub(crate) type ReferenceMode = i32;
pub(crate) const SINGLE_REFERENCE: ReferenceMode = 0;
pub(crate) const COMPOUND_REFERENCE: ReferenceMode = 1;
pub(crate) const REFERENCE_MODE_SELECT: ReferenceMode = 2;
pub(crate) const REFERENCE_MODES: ReferenceMode = 3;

// (REFRESH_FRAME_CONTEXT_MODE)
pub(crate) const REFRESH_FRAME_CONTEXT_DISABLED: i32 = 0;
pub(crate) const REFRESH_FRAME_CONTEXT_BACKWARD: i32 = 1;

pub(crate) const MFMV_STACK_SIZE: i32 = 3;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TplMvRef {
    pub(crate) mfmv0: IntMv,
    pub(crate) ref_frame_offset: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MvRef {
    pub(crate) mv: IntMv,
    pub(crate) ref_frame: MvReferenceFrame,
}

pub(crate) const SWITCHABLE_FILTERS: usize = 3;
pub(crate) const SWITCHABLE: i32 = 4;
pub(crate) const EIGHTTAP_REGULAR: i32 = 0;

/// Translation of `RefCntBuffer`, without the film grain, global motion and
/// external frame buffer data.
#[derive(Debug, Clone, Default)]
pub(crate) struct RefCntBuffer {
    pub(crate) ref_count: i32,

    pub(crate) order_hint: u32,
    pub(crate) ref_order_hints: [u32; INTER_REFS_PER_FRAME as usize],

    // These variables are used only in encoder and compare the absolute
    // display order hint to compute the relative distance and overcome
    // the limitation of get_relative_dist() which returns incorrect
    // distance when a very old frame is used as a reference.
    pub(crate) display_order_hint: u32,
    pub(crate) ref_display_order_hint: [u32; INTER_REFS_PER_FRAME as usize],
    // Frame's level within the hierarchical structure.
    pub(crate) pyramid_level: u32,
    pub(crate) mvs: Vec<MvRef>,
    pub(crate) seg_map: Vec<u8>,
    pub(crate) seg: Segmentation,
    pub(crate) mi_rows: i32,
    pub(crate) mi_cols: i32,
    // Width and height give the size of the buffer (before any upscaling, unlike
    // the sizes that can be derived from the buf structure)
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) showable_frame: i32, // frame can be used as show existing frame in future
    pub(crate) film_grain_params_present: u8,
    pub(crate) buf: Yv12BufferConfig,
    pub(crate) temporal_id: i32, // Temporal layer ID of the frame
    pub(crate) spatial_id: i32,  // Spatial layer ID of the frame
    pub(crate) frame_type: FrameType,

    // This is only used in the encoder but needs to be indexed per ref frame
    // so it's extremely convenient to keep it here.
    pub(crate) interp_filter_selected: [i32; SWITCHABLE as usize],

    // Inter frame reference frame delta for loop filter
    pub(crate) ref_deltas: [i8; REF_FRAMES as usize],

    // 0 = ZERO_MV, MV
    pub(crate) mode_deltas: [i8; MAX_MODE_LF_DELTAS as usize],

    pub(crate) frame_context: Box<FrameContext>,
}

/// Translation of `BufferPool` (no frame buffer callbacks; single-threaded,
/// so no mutex).
#[derive(Debug)]
pub(crate) struct BufferPool {
    pub(crate) frame_bufs: Vec<RefCntBuffer>,
}

impl Default for BufferPool {
    fn default() -> Self {
        BufferPool {
            frame_bufs: (0..FRAME_BUFFERS)
                .map(|_| RefCntBuffer::default())
                .collect(),
        }
    }
}

/// Parameters related to CDEF (the line and column buffers are never used:
/// CDEF is off).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CdefInfo {
    pub(crate) cdef_damping: i32,
    pub(crate) nb_cdef_strengths: i32,
    pub(crate) cdef_strengths: [i32; CDEF_MAX_STRENGTHS],
    pub(crate) cdef_uv_strengths: [i32; CDEF_MAX_STRENGTHS],
    pub(crate) cdef_bits: i32,
    pub(crate) allocated_mi_rows: i32,
    pub(crate) allocated_num_workers: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DeltaQInfo {
    pub(crate) delta_q_present_flag: i32,
    pub(crate) delta_q_res: i32,
    pub(crate) delta_lf_present_flag: i32,
    pub(crate) delta_lf_res: i32,
    pub(crate) delta_lf_multi: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OrderHintInfo {
    pub(crate) enable_order_hint: i32, // 0 - disable order hint, and related tools
    pub(crate) order_hint_bits_minus_1: i32, // dist_wtd_comp, ref_frame_mvs,
    // frame_sign_bias
    // if 0, enable_dist_wtd_comp and
    // enable_ref_frame_mvs must be set as 0.
    pub(crate) enable_dist_wtd_comp: i32, // 0 - disable dist-wtd compound modes
    // 1 - enable it
    pub(crate) enable_ref_frame_mvs: i32, // 0 - disable ref frame mvs
                                          // 1 - enable it
}

/// Translation of `aom_timing_info_t`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AomTimingInfo {
    pub(crate) num_units_in_display_tick: u32,
    pub(crate) time_scale: u32,
    pub(crate) equal_picture_interval: i32,
    pub(crate) num_ticks_per_picture: u32,
}

/// Translation of `aom_dec_model_info_t`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AomDecModelInfo {
    pub(crate) num_units_in_decoding_tick: u32,
    pub(crate) encoder_decoder_buffer_delay_length: i32,
    pub(crate) buffer_removal_time_length: i32,
    pub(crate) frame_presentation_time_length: i32,
}

/// Translation of `aom_dec_model_op_parameters_t`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AomDecModelOpParameters {
    pub(crate) decoder_model_param_present_flag: i32,
    pub(crate) bitrate: i64,
    pub(crate) buffer_size: i64,
    pub(crate) decoder_buffer_delay: u32,
    pub(crate) encoder_buffer_delay: u32,
    pub(crate) low_delay_mode_flag: i32,
    pub(crate) display_model_param_present_flag: i32,
    pub(crate) initial_display_delay: i32,
}

// Sequence header structure.
// Note: All syntax elements of sequence_header_obu that need to be
// bit-identical across multiple sequence headers must be part of this struct,
// so that consistency is checked by are_seq_headers_consistent() function.
// One exception is the last member 'op_params' that is ignored by
// are_seq_headers_consistent() function.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SequenceHeader {
    pub(crate) num_bits_width: i32,
    pub(crate) num_bits_height: i32,
    pub(crate) max_frame_width: i32,
    pub(crate) max_frame_height: i32,
    // Whether current and reference frame IDs are signaled in the bitstream.
    // Frame id numbers are additional information that do not affect the
    // decoding process, but provide decoders with a way of detecting missing
    // reference frames so that appropriate action can be taken.
    pub(crate) frame_id_numbers_present_flag: u8,
    pub(crate) frame_id_length: i32,
    pub(crate) delta_frame_id_length: i32,
    pub(crate) sb_size: BlockSize, // Size of the superblock used for this frame
    pub(crate) mib_size: i32,      // Size of the superblock in units of MI blocks
    pub(crate) mib_size_log2: i32, // Log 2 of above.

    pub(crate) order_hint_info: OrderHintInfo,

    pub(crate) force_screen_content_tools: u8, // 0 - force off
    // 1 - force on
    // 2 - adaptive
    pub(crate) still_picture: u8, // Video is a single frame still picture
    pub(crate) reduced_still_picture_hdr: u8, // Use reduced header for still picture
    pub(crate) force_integer_mv: u8, // 0 - Don't force. MV can use subpel
    // 1 - force to integer
    // 2 - adaptive
    pub(crate) enable_filter_intra: u8, // enables/disables filterintra
    pub(crate) enable_intra_edge_filter: u8, // enables/disables edge upsampling
    pub(crate) enable_interintra_compound: u8, // enables/disables interintra_compound
    pub(crate) enable_masked_compound: u8, // enables/disables masked compound
    pub(crate) enable_dual_filter: u8,  // 0 - disable dual interpolation filter
    // 1 - enable vert/horz filter selection
    pub(crate) enable_warped_motion: u8, // 0 - disable warp for the sequence
    // 1 - enable warp for the sequence
    pub(crate) enable_superres: u8, // 0 - Disable superres for the sequence
    // and no frame level superres flag
    // 1 - Enable superres for the sequence
    //     enable per-frame superres flag
    pub(crate) enable_cdef: u8,        // To turn on/off CDEF
    pub(crate) enable_restoration: u8, // To turn on/off loop restoration
    pub(crate) profile: i32,

    // Color config.
    pub(crate) bit_depth: i32, // AOM_BITS_8 in profile 0 or 1,
    // AOM_BITS_10 or AOM_BITS_12 in profile 2 or 3.
    pub(crate) use_highbitdepth: u8, // If true, we need to use 16bit frame buffers.
    pub(crate) monochrome: u8,       // Monochrome video
    pub(crate) color_primaries: i32,
    pub(crate) transfer_characteristics: i32,
    pub(crate) matrix_coefficients: i32,
    pub(crate) color_range: i32,
    pub(crate) subsampling_x: i32, // Chroma subsampling for x
    pub(crate) subsampling_y: i32, // Chroma subsampling for y
    pub(crate) chroma_sample_position: i32,
    pub(crate) separate_uv_delta_q: u8,
    pub(crate) film_grain_params_present: u8,

    // Operating point info.
    pub(crate) operating_points_cnt_minus_1: i32,
    pub(crate) operating_point_idc: [i32; MAX_NUM_OPERATING_POINTS],
    pub(crate) timing_info_present: i32,
    pub(crate) timing_info: AomTimingInfo,
    pub(crate) decoder_model_info_present_flag: u8,
    pub(crate) decoder_model_info: AomDecModelInfo,
    pub(crate) display_model_info_present_flag: u8,
    pub(crate) seq_level_idx: [Av1Level; MAX_NUM_OPERATING_POINTS],
    pub(crate) tier: [u8; MAX_NUM_OPERATING_POINTS], // seq_tier in spec. One bit: 0 or 1.

    // IMPORTANT: the op_params member must be at the end of the struct so that
    // are_seq_headers_consistent() can be implemented with a memcmp() call.
    // TODO(urvang): We probably don't need the +1 here.
    pub(crate) op_params: [AomDecModelOpParameters; MAX_NUM_OPERATING_POINTS + 1],
}

impl Default for SequenceHeader {
    fn default() -> Self {
        SequenceHeader {
            num_bits_width: 0,
            num_bits_height: 0,
            max_frame_width: 0,
            max_frame_height: 0,
            frame_id_numbers_present_flag: 0,
            frame_id_length: 0,
            delta_frame_id_length: 0,
            sb_size: BlockSize::default(),
            mib_size: 0,
            mib_size_log2: 0,
            order_hint_info: OrderHintInfo::default(),
            force_screen_content_tools: 0,
            still_picture: 0,
            reduced_still_picture_hdr: 0,
            force_integer_mv: 0,
            enable_filter_intra: 0,
            enable_intra_edge_filter: 0,
            enable_interintra_compound: 0,
            enable_masked_compound: 0,
            enable_dual_filter: 0,
            enable_warped_motion: 0,
            enable_superres: 0,
            enable_cdef: 0,
            enable_restoration: 0,
            profile: 0,
            bit_depth: 0,
            use_highbitdepth: 0,
            monochrome: 0,
            color_primaries: 0,
            transfer_characteristics: 0,
            matrix_coefficients: 0,
            color_range: 0,
            subsampling_x: 0,
            subsampling_y: 0,
            chroma_sample_position: 0,
            separate_uv_delta_q: 0,
            film_grain_params_present: 0,
            operating_points_cnt_minus_1: 0,
            operating_point_idc: [0; MAX_NUM_OPERATING_POINTS],
            timing_info_present: 0,
            timing_info: AomTimingInfo::default(),
            decoder_model_info_present_flag: 0,
            decoder_model_info: AomDecModelInfo::default(),
            display_model_info_present_flag: 0,
            seq_level_idx: [Av1Level::default(); MAX_NUM_OPERATING_POINTS],
            tier: [0; MAX_NUM_OPERATING_POINTS],
            op_params: [AomDecModelOpParameters::default(); MAX_NUM_OPERATING_POINTS + 1],
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SkipModeInfo {
    pub(crate) skip_mode_allowed: i32,
    pub(crate) skip_mode_flag: i32,
    pub(crate) ref_frame_idx_0: i32,
    pub(crate) ref_frame_idx_1: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CurrentFrame {
    pub(crate) frame_type: FrameType,
    pub(crate) reference_mode: ReferenceMode,
    pub(crate) order_hint: u32,
    pub(crate) display_order_hint: u32,
    // Frame's level within the hierarchical structure.
    pub(crate) pyramid_level: u32,
    pub(crate) frame_number: u32,
    pub(crate) skip_mode_info: SkipModeInfo,
    pub(crate) refresh_frame_flags: i32, // Which ref frames are overwritten by this frame
    pub(crate) frame_refs_short_signaling: i32,
}

/// Frame level features.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FeatureFlags {
    /// If true, CDF update in the symbol encoding/decoding process is disabled.
    pub(crate) disable_cdf_update: bool,
    /// If true, motion vectors are specified to eighth pel precision; and
    /// if false, motion vectors are specified to quarter pel precision.
    pub(crate) allow_high_precision_mv: bool,
    /// If true, force integer motion vectors; if false, use the default.
    pub(crate) cur_frame_force_integer_mv: bool,
    /// If true, palette tool and/or intra block copy tools may be used.
    pub(crate) allow_screen_content_tools: bool,
    pub(crate) allow_intrabc: bool, // If true, intra block copy tool may be used.
    pub(crate) allow_warped_motion: bool, // If true, frame may use warped motion mode.
    /// If true, using previous frames' motion vectors for prediction is allowed.
    pub(crate) allow_ref_frame_mvs: bool,
    /// If true, frame is fully lossless at coded resolution.
    pub(crate) coded_lossless: bool,
    /// If true, frame is fully lossless at coded and upscaled resolutions.
    pub(crate) all_lossless: bool,
    /// If true, the frame is restricted to a reduced subset of the full set of
    /// transform types.
    pub(crate) reduced_tx_set_used: bool,
    /// If true, error resilient mode is enabled.
    /// Note: Error resilient mode allows the syntax of a frame to be parsed
    /// independently of previously decoded frames.
    pub(crate) error_resilient_mode: bool,
    /// If false, only MOTION_MODE that may be used is SIMPLE_TRANSLATION;
    /// if true, all MOTION_MODES may be used.
    pub(crate) switchable_motion_mode: bool,
    pub(crate) tx_mode: TxMode,    // Transform mode at frame level.
    pub(crate) interp_filter: i32, // Interpolation filter at frame level.
    /// The frame buffer containing the reference frame
    pub(crate) primary_ref_frame: i32,
    /// Byte alignment of the planes in the reference buffers.
    pub(crate) byte_alignment: i32,
    /// Flag signaling how frame contexts should be updated at the end of
    /// a frame decode.
    pub(crate) refresh_frame_context: i32,
}

/// Params related to tiles.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommonTileParams {
    pub(crate) cols: i32, // number of tile columns that frame is divided into
    pub(crate) rows: i32, // number of tile rows that frame is divided into
    pub(crate) max_width_sb: i32, // maximum tile width in superblock units.
    pub(crate) max_height_sb: i32, // maximum tile height in superblock units.

    /// Min width of non-rightmost tile in MI units. Only valid if cols > 1.
    pub(crate) min_inner_width: i32,

    /// If true, tiles are uniformly spaced with power-of-2 number of rows and
    /// columns.
    /// If false, tiles have explicitly configured widths and heights.
    pub(crate) uniform_spacing: i32,

    // Following members are only valid when uniform_spacing == 1
    pub(crate) log2_cols: i32, // log2 of 'cols'.
    pub(crate) log2_rows: i32, // log2 of 'rows'.
    pub(crate) width: i32,     // tile width in MI units
    pub(crate) height: i32,    // tile height in MI units
    // End of members that are only valid when uniform_spacing == 1
    /// Min log2 of number of tile columns.
    pub(crate) min_log2_cols: i32,
    /// Min log2 of number of tile rows.
    pub(crate) min_log2_rows: i32,
    /// Max log2 of number of tile columns.
    pub(crate) max_log2_cols: i32,
    /// Max log2 of number of tile rows.
    pub(crate) max_log2_rows: i32,
    /// log2 of min number of tiles (same as min_log2_cols + min_log2_rows).
    pub(crate) min_log2: i32,
    /// col_start_sb[i] is the start position of tile column i in superblock
    /// units.
    /// valid for 0 <= i <= cols
    pub(crate) col_start_sb: [i32; MAX_TILE_COLS as usize + 1],
    /// row_start_sb[i] is the start position of tile row i in superblock
    /// units.
    /// valid for 0 <= i <= rows
    pub(crate) row_start_sb: [i32; MAX_TILE_ROWS as usize + 1],
    /// If true, we are using large scale tile mode.
    pub(crate) large_scale: u32,
    /// Only relevant when large_scale == 1.
    /// If true, the independent decoding of a single tile or a section of a
    /// frame is allowed.
    pub(crate) single_tile_decoding: u32,
}

impl Default for CommonTileParams {
    fn default() -> Self {
        CommonTileParams {
            cols: 0,
            rows: 0,
            max_width_sb: 0,
            max_height_sb: 0,
            min_inner_width: 0,
            uniform_spacing: 0,
            log2_cols: 0,
            log2_rows: 0,
            width: 0,
            height: 0,
            min_log2_cols: 0,
            min_log2_rows: 0,
            max_log2_cols: 0,
            max_log2_rows: 0,
            min_log2: 0,
            col_start_sb: [0; MAX_TILE_COLS as usize + 1],
            row_start_sb: [0; MAX_TILE_ROWS as usize + 1],
            large_scale: 0,
            single_tile_decoding: 0,
        }
    }
}

/// Encoder and decoder common parameters related to mode info.
/// (The free/setup/set_mb_mi function pointers are always the encoder's,
/// which are called directly.)
#[derive(Debug, Clone, Default)]
pub(crate) struct CommonModeInfoParams {
    /// Number of rows in the frame in 16 pixel units.
    /// This is computed from frame height aligned to a multiple of 8.
    pub(crate) mb_rows: i32,
    /// Number of cols in the frame in 16 pixel units.
    /// This is computed from frame width aligned to a multiple of 8.
    pub(crate) mb_cols: i32,

    /// Total MBs = mb_rows * mb_cols.
    pub(crate) mbs: i32,

    /// Number of rows in the frame in 4 pixel (MB_MODE_INFO) units.
    /// This is computed from frame height aligned to a multiple of 8.
    pub(crate) mi_rows: i32,
    /// Number of cols in the frame in 4 pixel (MB_MODE_INFO) units.
    /// This is computed from frame width aligned to a multiple of 8.
    pub(crate) mi_cols: i32,

    /// An array of MB_MODE_INFO structs for every 'mi_alloc_bsize' sized block
    /// in the frame.
    /// Note: This array should be treated like a scratch memory, and should NOT
    /// be accessed directly, in most cases. Please use 'mi_grid_base' array
    /// instead.
    pub(crate) mi_alloc: Vec<MbModeInfo>,
    /// Number of allocated elements in 'mi_alloc'.
    pub(crate) mi_alloc_size: i32,
    /// Stride for 'mi_alloc' array.
    pub(crate) mi_alloc_stride: i32,
    /// The minimum block size that each element in 'mi_alloc' can correspond
    /// to.
    /// For decoder, this is always BLOCK_4X4.
    /// For encoder, this is BLOCK_8X8 for resolution >= 4k case or REALTIME
    /// mode case. Otherwise, this is BLOCK_4X4.
    /// TODO(urvang): Switch to BLOCK_4X4 for all cases.
    pub(crate) mi_alloc_bsize: BlockSize,

    /// Grid of pointers to 4x4 MB_MODE_INFO structs allocated in 'mi_alloc'.
    /// It's possible that:
    /// - Multiple pointers in the grid point to the same element in
    ///   'mi_alloc' (e.g. for 8x8 block sizes for encoder).
    /// - Some pointers can be NULL (e.g. outside visible area).
    pub(crate) mi_grid_base: Vec<u32>,
    /// Number of allocated elements in 'mi_grid_base' (and 'tx_type_map' also).
    pub(crate) mi_grid_size: i32,
    /// Stride for 'mi_grid_base' (and 'tx_type_map' also).
    pub(crate) mi_stride: i32,

    /// An array of tx types for each 4x4 block in the frame.
    /// Number of allocated elements is same as 'mi_grid_size', and stride is
    /// same as 'mi_grid_size'. So, indexing into 'tx_type_map' is same as that
    /// of 'mi_grid_base'.
    pub(crate) tx_type_map: Vec<u8>,
}

/// Parameters related to quantization at the frame level. (The quantizer
/// matrix pointers are always the flat matrix, NULL: SDL_image's libavif
/// never enables quantizer matrices.)
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CommonQuantParams {
    /// Base qindex of the frame in the range 0 to 255.
    pub(crate) base_qindex: i32,

    /// Delta of qindex (from base_qindex) for Y plane DC coefficient.
    /// Note: y_ac_delta_q is implicitly 0.
    pub(crate) y_dc_delta_q: i32,

    /// Delta of qindex (from base_qindex) for U plane DC coefficients.
    pub(crate) u_dc_delta_q: i32,
    /// Delta of qindex (from base_qindex) for U plane AC coefficients.
    pub(crate) v_dc_delta_q: i32,

    /// Delta of qindex (from base_qindex) for V plane DC coefficients.
    /// Same as those for U plane if cm->seq_params->separate_uv_delta_q == 0.
    pub(crate) u_ac_delta_q: i32,
    /// Delta of qindex (from base_qindex) for V plane AC coefficients.
    /// Same as those for U plane if cm->seq_params->separate_uv_delta_q == 0.
    pub(crate) v_ac_delta_q: i32,

    // The dequantizers below are true dequantizers used only in the
    // dequantization process.  They have the same coefficient
    // shift/scale as TX.
    pub(crate) y_dequant_qtx: [[i16; 2]; MAX_SEGMENTS], // Dequant for Y plane
    pub(crate) u_dequant_qtx: [[i16; 2]; MAX_SEGMENTS], // Dequant for U plane
    pub(crate) v_dequant_qtx: [[i16; 2]; MAX_SEGMENTS], // Dequant for V plane

    /// Whether the quantizer matrices are used.
    pub(crate) using_qmatrix: bool,
    pub(crate) qmatrix_level_y: i32, // Level index for Y plane
    pub(crate) qmatrix_level_u: i32, // Level index for U plane
    pub(crate) qmatrix_level_v: i32, // Level index for V plane
}

/// Context used for transmitting various symbols in the bistream.
#[derive(Debug, Clone, Default)]
pub(crate) struct CommonContexts {
    /// Context used by 'partition' symbol in a given tile row:
    /// partition[i][j] is the context for ith tile row, jth mi_col.
    pub(crate) partition: Vec<Vec<PartitionContext>>,

    /// Context used to derive context for multiple symbols in a given tile
    /// row: entropy[i][j][k] is the context for ith plane, jth tile row, kth
    /// mi_col.
    pub(crate) entropy: [Vec<Vec<EntropyContext>>; MAX_MB_PLANE],

    /// Context used to derive context for 'txfm' symbol in a given tile row:
    /// txfm[i][j] is the context for ith tile row, jth mi_col.
    pub(crate) txfm: Vec<Vec<TxfmContext>>,

    /// Dimensions that were used to allocate the arrays above.
    /// If these dimensions change, the arrays may have to be re-allocated.
    pub(crate) num_planes: i32, // Corresponds to av1_num_planes(cm)
    pub(crate) num_tile_rows: i32, // Corresponds to cm->tiles.row
    pub(crate) num_mi_cols: i32,   // Corresponds to cm->mi_params.mi_cols
}

/// The loop restoration state the saver signals (every plane's type is
/// RESTORE_NONE; the unit data is never used).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RestorationInfo {
    pub(crate) frame_restoration_type: i32,
    pub(crate) restoration_unit_size: i32,
    pub(crate) units_per_tile: i32,
    pub(crate) vert_units_per_tile: i32,
    pub(crate) horz_units_per_tile: i32,
    pub(crate) optimized_lr: i32,
}

/// Top level common structure used by both encoder and decoder.
#[derive(Debug)]
pub(crate) struct Av1Common {
    /// Information about the current frame that is being coded.
    pub(crate) current_frame: CurrentFrame,

    // Dimensions of the frame
    pub(crate) width: i32,                    // Coded frame width
    pub(crate) height: i32,                   // Coded frame height
    pub(crate) render_width: i32,             // Rendered frame width
    pub(crate) render_height: i32,            // Rendered frame height
    pub(crate) superres_upscaled_width: i32,  // Super-resolved frame width
    pub(crate) superres_upscaled_height: i32, // Super-resolved frame height

    /// The denominator of the superres scale used by this frame.
    /// Note: The numerator is fixed to be SCALE_NUMERATOR.
    pub(crate) superres_scale_denominator: u8,

    /// buffer_removal_times[op_num] specifies the frame removal time in units
    /// of DecCT clock ticks counted from the removal time of the last random
    /// access point for operating point op_num.
    pub(crate) buffer_removal_times: [u32; MAX_NUM_OPERATING_POINTS + 1],
    /// Presentation time of the frame in clock ticks DispCT counted from the
    /// removal time of the last random access point for the operating point
    /// that is being decoded.
    pub(crate) frame_presentation_time: u32,

    /// Buffer where previous frame is stored (an index into the buffer pool).
    pub(crate) prev_frame: Option<usize>,

    /// Buffer into which the current frame will be stored and other related
    /// info (an index into the buffer pool).
    pub(crate) cur_frame: Option<usize>,

    /// For encoder, we have a two-level mapping from reference frame type to
    /// the corresponding buffer in the buffer pool.
    pub(crate) remapped_ref_idx: [i32; REF_FRAMES as usize],

    /// Scale of the current frame with respect to itself.
    pub(crate) sf_identity: ScaleFactors,

    /// Scale factors of the reference frame with respect to the current frame.
    pub(crate) ref_scale_factors: [ScaleFactors; REF_FRAMES as usize],

    /// For decoder, ref_frame_map[i] maps reference type 'i' to a pointer to
    /// the buffer in the buffer pool 'cm->buffer_pool.frame_bufs'.
    pub(crate) ref_frame_map: [Option<usize>; REF_FRAMES as usize],

    /// If true, this frame is actually shown after decoding.
    pub(crate) show_frame: i32,
    /// If true, this frame can be used as a show-existing frame for other
    /// frames coded later.
    pub(crate) showable_frame: i32,
    /// If true, show an existing frame coded before, instead of actually coding
    /// a frame. The existing frame comes from one of the existing reference
    /// buffers, as signaled in the bitstream.
    pub(crate) show_existing_frame: i32,

    /// Whether some features are allowed or not.
    pub(crate) features: FeatureFlags,

    /// Params related to MB_MODE_INFO arrays and related info.
    pub(crate) mi_params: CommonModeInfoParams,

    /// Quantization params.
    pub(crate) quant_params: CommonQuantParams,

    /// Segmentation info for current frame.
    pub(crate) seg: Segmentation,

    /// Loop filter info (thresholds; never applied by the saver).
    pub(crate) lf_info: Box<LoopFilterInfoN>,
    /// Loop filter params.
    pub(crate) lf: Loopfilter,

    /// Loop Restoration filter info.
    pub(crate) rst_info: [RestorationInfo; MAX_MB_PLANE],

    /// CDEF (Constrained Directional Enhancement Filter) parameters.
    pub(crate) cdef_info: CdefInfo,

    /// Parameters related to delta quantization and delta loop filter.
    pub(crate) delta_q_info: DeltaQInfo,

    /// Elements part of the sequence header, that are applicable for all the
    /// frames in the video.
    pub(crate) seq_params: SequenceHeader,

    /// Current CDFs of all the symbols for the current frame.
    pub(crate) fc: Box<FrameContext>,
    /// Default CDFs used when features.primary_ref_frame = PRIMARY_REF_NONE
    /// (e.g. for a keyframe). These default CDFs are defined by the bitstream
    /// and copied from default CDF tables for each symbol.
    pub(crate) default_frame_context: Box<FrameContext>,

    /// Parameters related to tiling.
    pub(crate) tiles: CommonTileParams,

    /// External BufferPool passed from outside.
    pub(crate) buffer_pool: BufferPool,

    /// Above context buffers and their sizes.
    /// Note: above contexts are allocated in this struct, as their size is
    /// dependent on frame width, while left contexts are declared and allocated
    /// in MACROBLOCKD struct, as they have a fixed size.
    pub(crate) above_contexts: CommonContexts,

    // Frame IDs
    pub(crate) current_frame_id: i32, // frame ID for the current frame.
    pub(crate) ref_frame_id: [i32; REF_FRAMES as usize], // frame IDs for the reference frames.

    /// Motion vectors provided by motion field estimation.
    pub(crate) tpl_mvs: Vec<TplMvRef>,
    /// Allocated size of 'tpl_mvs' array. Refer to 'ensure_mv_buffer()' function.
    pub(crate) tpl_mvs_mem_size: i32,
    /// ref_frame_sign_bias[k] is 1 if the reference frame type k is a future
    /// frame and 0 if it is a past frame.
    pub(crate) ref_frame_sign_bias: [i32; REF_FRAMES as usize],

    /// Indicates how far away is the reference frame from the current frame.
    pub(crate) ref_frame_side: [i8; REF_FRAMES as usize],

    /// Temporal layer ID of this frame
    /// (in the range 0 ... (number_temporal_layers - 1)).
    pub(crate) temporal_layer_id: i32,

    /// Spatial layer ID of this frame
    /// (in the range 0 ... (number_spatial_layers - 1)).
    pub(crate) spatial_layer_id: i32,
}

impl Default for Av1Common {
    fn default() -> Self {
        Av1Common {
            current_frame: CurrentFrame::default(),
            width: 0,
            height: 0,
            render_width: 0,
            render_height: 0,
            superres_upscaled_width: 0,
            superres_upscaled_height: 0,
            superres_scale_denominator: 0,
            buffer_removal_times: [0; MAX_NUM_OPERATING_POINTS + 1],
            frame_presentation_time: 0,
            prev_frame: None,
            cur_frame: None,
            remapped_ref_idx: [0; REF_FRAMES as usize],
            sf_identity: ScaleFactors::default(),
            ref_scale_factors: [ScaleFactors::default(); REF_FRAMES as usize],
            ref_frame_map: [None; REF_FRAMES as usize],
            show_frame: 0,
            showable_frame: 0,
            show_existing_frame: 0,
            features: FeatureFlags::default(),
            mi_params: CommonModeInfoParams::default(),
            quant_params: CommonQuantParams::default(),
            seg: Segmentation::default(),
            lf_info: Box::default(),
            lf: Loopfilter::default(),
            rst_info: [RestorationInfo::default(); MAX_MB_PLANE],
            cdef_info: CdefInfo::default(),
            delta_q_info: DeltaQInfo::default(),
            seq_params: SequenceHeader::default(),
            fc: Box::default(),
            default_frame_context: Box::default(),
            tiles: CommonTileParams::default(),
            buffer_pool: BufferPool::default(),
            above_contexts: CommonContexts::default(),
            current_frame_id: 0,
            ref_frame_id: [0; REF_FRAMES as usize],
            tpl_mvs: Vec::new(),
            tpl_mvs_mem_size: 0,
            ref_frame_sign_bias: [0; REF_FRAMES as usize],
            ref_frame_side: [0; REF_FRAMES as usize],
            temporal_layer_id: 0,
            spatial_layer_id: 0,
        }
    }
}

impl Av1Common {
    /// `cm->cur_frame` (which must be set).
    #[inline]
    pub(crate) fn cur_frame(&self) -> &RefCntBuffer {
        &self.buffer_pool.frame_bufs[self.cur_frame.expect("cur_frame")]
    }
    /// `cm->cur_frame`, mutably.
    #[inline]
    pub(crate) fn cur_frame_mut(&mut self) -> &mut RefCntBuffer {
        let i = self.cur_frame.expect("cur_frame");
        &mut self.buffer_pool.frame_bufs[i]
    }
}

// TODO(hkuang): Don't need to lock the whole pool after implementing atomic
// frame reference count.
// (lock_buffer_pool() and unlock_buffer_pool() do nothing without threads.)

// Return the frame buffer index of a free buffer (FRAME_BUFFERS, i.e.
// INVALID_IDX, if there is none).
#[inline]
pub(crate) fn get_free_fb(cm: &mut Av1Common) -> i32 {
    let frame_bufs = &mut cm.buffer_pool.frame_bufs;
    let mut i = 0;
    while i < FRAME_BUFFERS as usize {
        if frame_bufs[i].ref_count == 0 {
            break;
        }
        i += 1;
    }

    if i != FRAME_BUFFERS as usize {
        frame_bufs[i].ref_count = 1;
    } else {
        // We should never run out of free buffers. If this assertion fails, there
        // is a reference leak.
        debug_assert!(
            false,
            "Ran out of free frame buffers. Likely a reference leak."
        );
        // Reset i to be INVALID_IDX to indicate no free buffer found.
        return -1;
    }

    i as i32
}

#[inline]
pub(crate) fn assign_cur_frame_new_fb(cm: &mut Av1Common) -> Option<usize> {
    // Release the previously-used frame-buffer
    if let Some(cur) = cm.cur_frame {
        cm.buffer_pool.frame_bufs[cur].ref_count -= 1;
        cm.cur_frame = None;
    }

    // Assign a new framebuffer
    let new_fb_idx = get_free_fb(cm);
    if new_fb_idx == -1 {
        return None;
    }

    cm.cur_frame = Some(new_fb_idx as usize);
    cm.cur_frame_mut().interp_filter_selected = [0; SWITCHABLE as usize];
    cm.cur_frame
}

// Modify 'lhs_ptr' to reference the buffer at 'rhs_ptr', and update the ref
// counts accordingly.
#[inline]
pub(crate) fn assign_frame_buffer_p(
    pool: &mut BufferPool,
    lhs_ptr: &mut Option<usize>,
    rhs_ptr: usize,
) {
    let old_ptr = *lhs_ptr;
    if let Some(old) = old_ptr {
        debug_assert!(pool.frame_bufs[old].ref_count > 0);
        // One less reference to the buffer at 'old_ptr', so decrease ref count.
        pool.frame_bufs[old].ref_count -= 1;
    }

    *lhs_ptr = Some(rhs_ptr);
    // One more reference to the buffer at 'rhs_ptr', so increase ref count.
    pool.frame_bufs[rhs_ptr].ref_count += 1;
}

#[inline]
pub(crate) fn frame_is_intra_only(cm: &Av1Common) -> bool {
    cm.current_frame.frame_type == KEY_FRAME || cm.current_frame.frame_type == INTRA_ONLY_FRAME
}

#[inline]
pub(crate) fn frame_is_sframe(cm: &Av1Common) -> bool {
    cm.current_frame.frame_type == S_FRAME
}

// Returns 1 if this frame might allow mvs from some reference frame.
#[inline]
pub(crate) fn frame_might_allow_ref_frame_mvs(cm: &Av1Common) -> bool {
    !cm.features.error_resilient_mode
        && cm.seq_params.order_hint_info.enable_ref_frame_mvs != 0
        && cm.seq_params.order_hint_info.enable_order_hint != 0
        && !frame_is_intra_only(cm)
}

// Returns 1 if this frame might use warped_motion
#[inline]
pub(crate) fn frame_might_allow_warped_motion(cm: &Av1Common) -> bool {
    !cm.features.error_resilient_mode
        && !frame_is_intra_only(cm)
        && cm.seq_params.enable_warped_motion != 0
}

/// Translation of `ensure_mv_buffer()` (`Err(())` is the allocation error
/// CHECK_MEM_ERROR raises).
#[inline]
pub(crate) fn ensure_mv_buffer(buf_idx: usize, cm: &mut Av1Common) -> Result<(), ()> {
    let mi_rows = cm.mi_params.mi_rows;
    let mi_cols = cm.mi_params.mi_cols;
    let mi_stride = cm.mi_params.mi_stride;
    let buf = &mut cm.buffer_pool.frame_bufs[buf_idx];
    let buf_rows = buf.mi_rows;
    let buf_cols = buf.mi_cols;

    if buf.mvs.is_empty() || buf_rows != mi_rows || buf_cols != mi_cols {
        buf.mvs = Vec::new();
        buf.mi_rows = mi_rows;
        buf.mi_cols = mi_cols;
        let n = (((mi_rows + 1) >> 1) * ((mi_cols + 1) >> 1)) as usize;
        let mut mvs = Vec::new();
        mvs.try_reserve_exact(n).map_err(|_| ())?;
        mvs.resize(n, MvRef::default());
        buf.mvs = mvs;
        buf.seg_map = Vec::new();
        let n = (mi_rows * mi_cols) as usize;
        let mut seg_map = Vec::new();
        seg_map.try_reserve_exact(n).map_err(|_| ())?;
        seg_map.resize(n, 0);
        buf.seg_map = seg_map;
    }

    let mem_size = ((mi_rows + MAX_MIB_SIZE) >> 1) * (mi_stride >> 1);
    let mut realloc = cm.tpl_mvs.is_empty();
    if !cm.tpl_mvs.is_empty() {
        realloc |= cm.tpl_mvs_mem_size < mem_size;
    }

    if realloc {
        cm.tpl_mvs = Vec::new();
        let mut tpl_mvs = Vec::new();
        tpl_mvs
            .try_reserve_exact(mem_size as usize)
            .map_err(|_| ())?;
        tpl_mvs.resize(mem_size as usize, TplMvRef::default());
        cm.tpl_mvs = tpl_mvs;
        cm.tpl_mvs_mem_size = mem_size;
    }
    Ok(())
}

#[inline]
pub(crate) fn av1_num_planes(cm: &Av1Common) -> usize {
    if cm.seq_params.monochrome != 0 {
        1
    } else {
        MAX_MB_PLANE
    }
}

#[inline]
pub(crate) fn av1_init_above_context(tile_row: i32, xd: &mut MacroBlockD) {
    // (the entropy, partition and txfm pointers all point at the tile row's
    // arrays)
    xd.above_contexts_tile_row = tile_row as usize;
    xd.above_txfm_context = 0;
}

/// Translation of `av1_init_macroblockd()` (`xd->error_info` is not kept:
/// errors are returned).
#[inline]
pub(crate) fn av1_init_macroblockd(cm: &Av1Common, xd: &mut MacroBlockD) {
    let num_planes = av1_num_planes(cm);
    let quant_params = &cm.quant_params;

    for i in 0..num_planes {
        if xd.plane[i].plane_type == PLANE_TYPE_Y {
            xd.plane[i].seg_dequant_qtx = quant_params.y_dequant_qtx;
        } else {
            if i == 1 {
                xd.plane[i].seg_dequant_qtx = quant_params.u_dequant_qtx;
            } else {
                xd.plane[i].seg_dequant_qtx = quant_params.v_dequant_qtx;
            }
        }
    }
    xd.mi_stride = cm.mi_params.mi_stride;
    super::cfl::cfl_init(&mut xd.cfl, &cm.seq_params);
}

#[inline]
pub(crate) fn set_entropy_context(
    xd: &mut MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    num_planes: usize,
) {
    let mut row_offset = mi_row;
    let mut col_offset = mi_col;
    for i in 0..num_planes {
        let bsize = xd.mbmi().bsize;
        let pd = &mut xd.plane[i];
        // Offset the buffer pointer
        if pd.subsampling_y != 0 && (mi_row & 0x01) != 0 && (MI_SIZE_HIGH[bsize as usize] == 1) {
            row_offset = mi_row - 1;
        }
        if pd.subsampling_x != 0 && (mi_col & 0x01) != 0 && (MI_SIZE_WIDE[bsize as usize] == 1) {
            col_offset = mi_col - 1;
        }
        let above_idx = col_offset;
        let left_idx = row_offset & MAX_MIB_MASK;
        pd.above_entropy_context = (above_idx >> pd.subsampling_x) as usize;
        pd.left_entropy_context = (left_idx >> pd.subsampling_y) as usize;
    }
}

#[inline]
pub(crate) fn set_plane_n4(xd: &mut MacroBlockD, bw: i32, bh: i32, num_planes: usize) {
    for i in 0..num_planes {
        let w = (bw * MI_SIZE) >> xd.plane[i].subsampling_x;
        let h = (bh * MI_SIZE) >> xd.plane[i].subsampling_y;

        xd.plane[i].width = w.max(4) as u8;
        xd.plane[i].height = h.max(4) as u8;
    }
}

#[inline]
pub(crate) fn set_mi_row_col(
    xd: &mut MacroBlockD,
    tile: &TileInfo,
    mi_row: i32,
    bh: i32,
    mi_col: i32,
    bw: i32,
    mi_rows: i32,
    mi_cols: i32,
) {
    use super::mv::get_mv_subpel;
    xd.mb_to_top_edge = -get_mv_subpel(mi_row * MI_SIZE);
    xd.mb_to_bottom_edge = get_mv_subpel((mi_rows - bh - mi_row) * MI_SIZE);
    xd.mb_to_left_edge = -get_mv_subpel(mi_col * MI_SIZE);
    xd.mb_to_right_edge = get_mv_subpel((mi_cols - bw - mi_col) * MI_SIZE);

    xd.mi_row = mi_row;
    xd.mi_col = mi_col;

    // Are edges available for intra prediction?
    xd.up_available = mi_row > tile.mi_row_start;

    let ss_x = xd.plane[1].subsampling_x;
    let ss_y = xd.plane[1].subsampling_y;

    xd.left_available = mi_col > tile.mi_col_start;
    xd.chroma_up_available = xd.up_available;
    xd.chroma_left_available = xd.left_available;
    if ss_x != 0 && bw < MI_SIZE_WIDE[BLOCK_8X8 as usize] as i32 {
        xd.chroma_left_available = (mi_col - 1) > tile.mi_col_start;
    }
    if ss_y != 0 && bh < MI_SIZE_HIGH[BLOCK_8X8 as usize] as i32 {
        xd.chroma_up_available = (mi_row - 1) > tile.mi_row_start;
    }
    let mi_stride = xd.mi_stride as isize;
    if xd.up_available {
        xd.above_mbmi = Some(xd.mi_idx(-mi_stride));
    } else {
        xd.above_mbmi = None;
    }

    if xd.left_available {
        xd.left_mbmi = Some(xd.mi_idx(-1));
    } else {
        xd.left_mbmi = None;
    }

    let chroma_ref = ((mi_row & 0x01) != 0 || (bh & 0x01) == 0 || ss_y == 0)
        && ((mi_col & 0x01) != 0 || (bw & 0x01) == 0 || ss_x == 0);
    xd.is_chroma_ref = chroma_ref;
    if chroma_ref {
        // To help calculate the "above" and "left" chroma blocks, note that the
        // current block may cover multiple luma blocks (e.g., if partitioned into
        // 4x4 luma blocks).
        // First, find the top-left-most luma block covered by this chroma block
        let base_mi = -((mi_row & ss_y) as isize) * mi_stride - (mi_col & ss_x) as isize;

        // Then, we consider the luma region covered by the left or above 4x4 chroma
        // prediction. We want to point to the chroma reference block in that
        // region, which is the bottom-right-most mi unit.
        // This leads to the following offsets:
        let chroma_above_mi = if xd.chroma_up_available {
            Some(xd.mi_idx(base_mi - mi_stride + ss_x as isize))
        } else {
            None
        };
        xd.chroma_above_mbmi = chroma_above_mi;

        let chroma_left_mi = if xd.chroma_left_available {
            Some(xd.mi_idx(base_mi + ss_y as isize * mi_stride - 1))
        } else {
            None
        };
        xd.chroma_left_mbmi = chroma_left_mi;
    }

    xd.height = bh as u8;
    xd.width = bw as u8;

    xd.is_last_vertical_rect = false;
    if xd.width < xd.height {
        if ((mi_col + xd.width as i32) & (xd.height as i32 - 1)) == 0 {
            xd.is_last_vertical_rect = true;
        }
    }

    xd.is_first_horizontal_rect = false;
    if xd.width > xd.height {
        if (mi_row & (xd.width as i32 - 1)) == 0 {
            xd.is_first_horizontal_rect = true;
        }
    }
}

#[inline]
pub(crate) fn get_y_mode_cdf<'a>(
    tile_ctx: &'a mut FrameContext,
    above_mi: Option<&MbModeInfo>,
    left_mi: Option<&MbModeInfo>,
) -> &'a mut [u16] {
    let above = av1_above_block_mode(above_mi);
    let left = av1_left_block_mode(left_mi);
    let above_ctx = INTRA_MODE_CONTEXT[above as usize] as usize;
    let left_ctx = INTRA_MODE_CONTEXT[left as usize] as usize;
    &mut tile_ctx.kf_y_cdf[above_ctx][left_ctx]
}

#[inline]
pub(crate) fn update_partition_context(
    xd: &mut MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    subsize: BlockSize,
    bsize: BlockSize,
) {
    let left_ctx = (mi_row & MAX_MIB_MASK) as usize;

    let bw = MI_SIZE_WIDE[bsize as usize] as usize;
    let bh = MI_SIZE_HIGH[bsize as usize] as usize;
    let above = &mut xd.above_partition_mut()[mi_col as usize..mi_col as usize + bw];
    above.fill(PARTITION_CONTEXT_LOOKUP[subsize as usize].above);
    xd.left_partition_context[left_ctx..left_ctx + bh]
        .fill(PARTITION_CONTEXT_LOOKUP[subsize as usize].left);
}

#[inline]
pub(crate) fn is_chroma_reference(
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
    subsampling_x: i32,
    subsampling_y: i32,
) -> bool {
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let bw = MI_SIZE_WIDE[bsize as usize] as i32;
    let bh = MI_SIZE_HIGH[bsize as usize] as i32;
    let ref_pos = ((mi_row & 0x01) != 0 || (bh & 0x01) == 0 || subsampling_y == 0)
        && ((mi_col & 0x01) != 0 || (bw & 0x01) == 0 || subsampling_x == 0);
    ref_pos
}

#[inline]
pub(crate) fn cdf_element_prob(cdf: &[u16], element: usize) -> u16 {
    (if element > 0 {
        cdf[element - 1] as u32
    } else {
        CDF_PROB_TOP
    })
    .wrapping_sub(cdf[element] as u32) as u16
}

#[inline]
pub(crate) fn partition_gather_horz_alike(out: &mut [u16; 3], input: &[u16], bsize: BlockSize) {
    let mut o = CDF_PROB_TOP as u16;
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_HORZ as usize));
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_SPLIT as usize));
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_HORZ_A as usize));
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_HORZ_B as usize));
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_VERT_A as usize));
    if bsize != BLOCK_128X128 {
        o = o.wrapping_sub(cdf_element_prob(input, PARTITION_HORZ_4 as usize));
    }
    out[0] = aom_icdf(o as u32);
    out[1] = aom_icdf(CDF_PROB_TOP);
}

#[inline]
pub(crate) fn partition_gather_vert_alike(out: &mut [u16; 3], input: &[u16], bsize: BlockSize) {
    let mut o = CDF_PROB_TOP as u16;
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_VERT as usize));
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_SPLIT as usize));
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_HORZ_A as usize));
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_VERT_A as usize));
    o = o.wrapping_sub(cdf_element_prob(input, PARTITION_VERT_B as usize));
    if bsize != BLOCK_128X128 {
        o = o.wrapping_sub(cdf_element_prob(input, PARTITION_VERT_4 as usize));
    }
    out[0] = aom_icdf(o as u32);
    out[1] = aom_icdf(CDF_PROB_TOP);
}

/// Translation of `update_ext_partition_context()` for the partition types
/// the saver's variance-based partitioning produces (none, horizontal,
/// vertical and split).
#[inline]
pub(crate) fn update_ext_partition_context(
    xd: &mut MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    subsize: BlockSize,
    bsize: BlockSize,
    partition: PartitionType,
) {
    if bsize >= BLOCK_8X8 {
        match partition {
            PARTITION_SPLIT => {
                if bsize != BLOCK_8X8 {
                    return;
                }
                update_partition_context(xd, mi_row, mi_col, subsize, bsize);
            }
            PARTITION_NONE | PARTITION_HORZ | PARTITION_VERT | PARTITION_HORZ_4
            | PARTITION_VERT_4 => {
                update_partition_context(xd, mi_row, mi_col, subsize, bsize);
            }
            _ => debug_assert!(false, "Invalid partition type"),
        }
    }
}

#[inline]
pub(crate) fn partition_plane_context(
    xd: &MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) -> i32 {
    let above_ctx = xd.above_partition()[mi_col as usize];
    let left_ctx = xd.left_partition_context[(mi_row & MAX_MIB_MASK) as usize];
    // Minimum partition point is 8x8. Offset the bsl accordingly.
    let bsl =
        MI_SIZE_WIDE_LOG2[bsize as usize] as i32 - MI_SIZE_WIDE_LOG2[BLOCK_8X8 as usize] as i32;
    let above = (above_ctx as i32 >> bsl) & 1;
    let left = (left_ctx as i32 >> bsl) & 1;

    debug_assert!(MI_SIZE_WIDE_LOG2[bsize as usize] == MI_SIZE_HIGH_LOG2[bsize as usize]);
    debug_assert!(bsl >= 0);

    (left * 2 + above) + bsl * PARTITION_PLOFFSET
}

// Return the number of elements in the partition CDF when
// partitioning the (square) block with luma block size of bsize.
#[inline]
pub(crate) fn partition_cdf_length(bsize: BlockSize) -> i32 {
    if bsize <= BLOCK_8X8 {
        PARTITION_TYPES
    } else if bsize == BLOCK_128X128 {
        EXT_PARTITION_TYPES - 2
    } else {
        EXT_PARTITION_TYPES
    }
}

#[inline]
pub(crate) fn av1_zero_above_context(
    cm: &Av1Common,
    above_contexts: &mut CommonContexts,
    mi_col_start: i32,
    mi_col_end: i32,
    tile_row: i32,
) {
    let seq_params = &cm.seq_params;
    let num_planes = av1_num_planes(cm);
    let width = mi_col_end - mi_col_start;
    let aligned_width = align_power_of_two(width, seq_params.mib_size_log2);
    let offset_y = mi_col_start as usize;
    let width_y = aligned_width as usize;
    let offset_uv = (mi_col_start >> seq_params.subsampling_x) as usize;
    let width_uv = width_y >> seq_params.subsampling_x;
    let tile_row = tile_row as usize;

    above_contexts.entropy[0][tile_row][offset_y..offset_y + width_y].fill(0);
    if num_planes > 1 {
        above_contexts.entropy[1][tile_row][offset_uv..offset_uv + width_uv].fill(0);
        above_contexts.entropy[2][tile_row][offset_uv..offset_uv + width_uv].fill(0);
    }

    let mcs = mi_col_start as usize;
    above_contexts.partition[tile_row][mcs..mcs + aligned_width as usize].fill(0);

    above_contexts.txfm[tile_row][mcs..mcs + aligned_width as usize]
        .fill(TX_SIZE_WIDE[TX_SIZES_LARGEST as usize] as TxfmContext);
}

#[inline]
pub(crate) fn av1_zero_left_context(xd: &mut MacroBlockD) {
    xd.left_entropy_context = [[0; MAX_MIB_SIZE as usize]; MAX_MB_PLANE];
    xd.left_partition_context = [0; MAX_MIB_SIZE as usize];

    xd.left_txfm_context_buffer =
        [TX_SIZE_HIGH[TX_SIZES_LARGEST as usize] as TxfmContext; MAX_MIB_SIZE as usize];
}

#[inline]
pub(crate) fn set_txfm_ctx(txfm_ctx: &mut [TxfmContext], txs: u8, len: i32) {
    for i in 0..len as usize {
        txfm_ctx[i] = txs;
    }
}

#[inline]
pub(crate) fn set_txfm_ctxs(
    tx_size: TxSize,
    n4_w: i32,
    n4_h: i32,
    skip: bool,
    xd: &mut MacroBlockD,
) {
    let mut bw = TX_SIZE_WIDE[tx_size as usize] as u8;
    let mut bh = TX_SIZE_HIGH[tx_size as usize] as u8;

    if skip {
        bw = (n4_w * MI_SIZE) as u8;
        bh = (n4_h * MI_SIZE) as u8;
    }

    let a = xd.above_txfm_context;
    let tile_row = xd.above_contexts_tile_row;
    set_txfm_ctx(&mut xd.above_contexts.txfm[tile_row][a..], bw, n4_w);
    let l = xd.left_txfm_context;
    set_txfm_ctx(&mut xd.left_txfm_context_buffer[l..], bh, n4_h);
}

#[inline]
pub(crate) fn get_mi_grid_idx(mi_params: &CommonModeInfoParams, mi_row: i32, mi_col: i32) -> i32 {
    mi_row * mi_params.mi_stride + mi_col
}

#[inline]
pub(crate) fn get_alloc_mi_idx(mi_params: &CommonModeInfoParams, mi_row: i32, mi_col: i32) -> i32 {
    let mi_alloc_size_1d = MI_SIZE_WIDE[mi_params.mi_alloc_bsize as usize] as i32;
    let mi_alloc_row = mi_row / mi_alloc_size_1d;
    let mi_alloc_col = mi_col / mi_alloc_size_1d;

    mi_alloc_row * mi_params.mi_alloc_stride + mi_alloc_col
}

// For this partition block, set pointers in mi_params->mi_grid_base and xd->mi.
// (The grid is the one moved into xd.)
#[inline]
pub(crate) fn set_mi_offsets(
    mi_params: &CommonModeInfoParams,
    xd: &mut MacroBlockD,
    mi_row: i32,
    mi_col: i32,
) {
    // 'mi_grid_base' should point to appropriate memory in 'mi'.
    let mi_grid_idx = get_mi_grid_idx(mi_params, mi_row, mi_col) as usize;
    let mi_alloc_idx = get_alloc_mi_idx(mi_params, mi_row, mi_col);
    xd.mi_grid_base[mi_grid_idx] = mi_alloc_idx as u32;
    // 'xd->mi' should point to an offset in 'mi_grid_base';
    xd.mi = mi_grid_idx;
    // 'xd->tx_type_map' should point to an offset in 'mi_params->tx_type_map'.
    xd.tx_type_map = TxTypeMapRef::Frame(mi_grid_idx);
    xd.tx_type_map_stride = mi_params.mi_stride;
}

// Compute the next partition in the direction of the sb_type stored in the mi
// array, starting with bsize.
// (The mode info arrays are the ones moved into xd.)
#[inline]
pub(crate) fn get_partition(
    mi_params: &CommonModeInfoParams,
    xd: &MacroBlockD,
    mi_row: i32,
    mi_col: i32,
    bsize: BlockSize,
) -> PartitionType {
    if mi_row >= mi_params.mi_rows || mi_col >= mi_params.mi_cols {
        return PARTITION_INVALID;
    }

    let offset = (mi_row * mi_params.mi_stride + mi_col) as usize;
    let mi = |i: usize| &xd.mi_alloc[xd.mi_grid_base[offset + i] as usize];
    let subsize = mi(0).bsize;

    debug_assert!(bsize < BLOCK_SIZES_ALL);

    if subsize == bsize {
        return PARTITION_NONE;
    }

    let bhigh = MI_SIZE_HIGH[bsize as usize] as i32;
    let bwide = MI_SIZE_WIDE[bsize as usize] as i32;
    let sshigh = MI_SIZE_HIGH[subsize as usize] as i32;
    let sswide = MI_SIZE_WIDE[subsize as usize] as i32;

    if bsize > BLOCK_8X8
        && mi_row + bwide / 2 < mi_params.mi_rows
        && mi_col + bhigh / 2 < mi_params.mi_cols
    {
        // In this case, the block might be using an extended partition
        // type.
        let mbmi_right = mi((bwide / 2) as usize);
        let mbmi_below = mi((bhigh / 2 * mi_params.mi_stride) as usize);

        if sswide == bwide {
            // Smaller height but same width. Is PARTITION_HORZ_4, PARTITION_HORZ or
            // PARTITION_HORZ_B. To distinguish the latter two, check if the lower
            // half was split.
            if sshigh * 4 == bhigh {
                return PARTITION_HORZ_4;
            }
            debug_assert!(sshigh * 2 == bhigh);

            if mbmi_below.bsize == subsize {
                return PARTITION_HORZ;
            } else {
                return PARTITION_HORZ_B;
            }
        } else if sshigh == bhigh {
            // Smaller width but same height. Is PARTITION_VERT_4, PARTITION_VERT or
            // PARTITION_VERT_B. To distinguish the latter two, check if the right
            // half was split.
            if sswide * 4 == bwide {
                return PARTITION_VERT_4;
            }
            // FIXME (upstream): this compares the width with the height; it
            // holds for the square blocks partitioned here.
            debug_assert!(sswide * 2 == bhigh);

            if mbmi_right.bsize == subsize {
                return PARTITION_VERT;
            } else {
                return PARTITION_VERT_B;
            }
        } else {
            // Smaller width and smaller height. Might be PARTITION_SPLIT or could be
            // PARTITION_HORZ_A or PARTITION_VERT_A. If subsize isn't halved in both
            // dimensions, we immediately know this is a split (which will recurse to
            // get to subsize). Otherwise look down and to the right. With
            // PARTITION_VERT_A, the right block will have height bhigh; with
            // PARTITION_HORZ_A, the lower block with have width bwide. Otherwise
            // it's PARTITION_SPLIT.
            if sswide * 2 != bwide || sshigh * 2 != bhigh {
                return PARTITION_SPLIT;
            }

            if MI_SIZE_WIDE[mbmi_below.bsize as usize] as i32 == bwide {
                return PARTITION_HORZ_A;
            }
            if MI_SIZE_HIGH[mbmi_right.bsize as usize] as i32 == bhigh {
                return PARTITION_VERT_A;
            }

            return PARTITION_SPLIT;
        }
    }
    let vert_split = (sswide < bwide) as usize;
    let horz_split = (sshigh < bhigh) as usize;
    let split_idx = (vert_split << 1) | horz_split;
    debug_assert!(split_idx != 0);

    static BASE_PARTITIONS: [PartitionType; 4] = [
        PARTITION_INVALID,
        PARTITION_HORZ,
        PARTITION_VERT,
        PARTITION_SPLIT,
    ];

    BASE_PARTITIONS[split_idx]
}

#[inline]
pub(crate) fn is_valid_seq_level_idx(seq_level_idx: Av1Level) -> bool {
    seq_level_idx == SEQ_LEVEL_MAX
        || (seq_level_idx < SEQ_LEVELS
            // The following levels are currently undefined.
            && seq_level_idx != SEQ_LEVEL_2_2
            && seq_level_idx != SEQ_LEVEL_2_3
            && seq_level_idx != SEQ_LEVEL_3_2
            && seq_level_idx != SEQ_LEVEL_3_3
            && seq_level_idx != SEQ_LEVEL_4_2
            && seq_level_idx != SEQ_LEVEL_4_3
            && seq_level_idx != SEQ_LEVEL_7_0
            && seq_level_idx != SEQ_LEVEL_7_1
            && seq_level_idx != SEQ_LEVEL_7_2
            && seq_level_idx != SEQ_LEVEL_7_3
            && seq_level_idx != SEQ_LEVEL_8_0
            && seq_level_idx != SEQ_LEVEL_8_1
            && seq_level_idx != SEQ_LEVEL_8_2
            && seq_level_idx != SEQ_LEVEL_8_3)
}

pub(crate) const INVALID_IDX: i32 = -1;

#[inline]
pub(crate) fn get_ref_frame_map_idx(cm: &Av1Common, ref_frame: MvReferenceFrame) -> i32 {
    if (LAST_FRAME..=EXTREF_FRAME).contains(&ref_frame) {
        cm.remapped_ref_idx[(ref_frame - LAST_FRAME) as usize]
    } else {
        INVALID_IDX
    }
}

/// `get_ref_frame_buf()`: the buffer pool index of the reference.
#[inline]
pub(crate) fn get_ref_frame_buf(cm: &Av1Common, ref_frame: MvReferenceFrame) -> Option<usize> {
    let map_idx = get_ref_frame_map_idx(cm, ref_frame);
    if map_idx != INVALID_IDX {
        cm.ref_frame_map[map_idx as usize]
    } else {
        None
    }
}

#[inline]
pub(crate) fn get_primary_ref_frame_buf(cm: &Av1Common) -> Option<usize> {
    let primary_ref_frame = cm.features.primary_ref_frame;
    if primary_ref_frame == PRIMARY_REF_NONE {
        return None;
    }
    let map_idx = get_ref_frame_map_idx(cm, (primary_ref_frame + 1) as MvReferenceFrame);
    if map_idx != INVALID_IDX {
        cm.ref_frame_map[map_idx as usize]
    } else {
        None
    }
}
