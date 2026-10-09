// Rust translation of include/dav1d/headers.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2020, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The parsed sequence and frame headers.
//!
//! The C enums of these headers are stored as plain integers (`i32`, as
//! the C `int`-sized enums), with their values as constants, since dav1d
//! compares and indexes with them and keeps unknown values (the color
//! description, say) as read from the bitstream.

#![allow(dead_code)]

// Constants from Section 3. "Symbols and abbreviated terms"
pub(crate) const DAV1D_MAX_CDEF_STRENGTHS: usize = 8;
pub(crate) const DAV1D_MAX_OPERATING_POINTS: usize = 32;
pub(crate) const DAV1D_MAX_TILE_COLS: usize = 64;
pub(crate) const DAV1D_MAX_TILE_ROWS: usize = 64;
pub(crate) const DAV1D_MAX_SEGMENTS: usize = 8;
pub(crate) const DAV1D_NUM_REF_FRAMES: usize = 8;
pub(crate) const DAV1D_PRIMARY_REF_NONE: i32 = 7;
pub(crate) const DAV1D_REFS_PER_FRAME: usize = 7;
pub(crate) const DAV1D_TOTAL_REFS_PER_FRAME: usize = DAV1D_REFS_PER_FRAME + 1;

// enum Dav1dObuType
pub(crate) const DAV1D_OBU_SEQ_HDR: u32 = 1;
pub(crate) const DAV1D_OBU_TD: u32 = 2;
pub(crate) const DAV1D_OBU_FRAME_HDR: u32 = 3;
pub(crate) const DAV1D_OBU_TILE_GRP: u32 = 4;
pub(crate) const DAV1D_OBU_METADATA: u32 = 5;
pub(crate) const DAV1D_OBU_FRAME: u32 = 6;
pub(crate) const DAV1D_OBU_REDUNDANT_FRAME_HDR: u32 = 7;
pub(crate) const DAV1D_OBU_PADDING: u32 = 15;

// enum Dav1dTxfmMode
pub(crate) const DAV1D_TX_4X4_ONLY: i32 = 0;
pub(crate) const DAV1D_TX_LARGEST: i32 = 1;
pub(crate) const DAV1D_TX_SWITCHABLE: i32 = 2;
pub(crate) const DAV1D_N_TX_MODES: i32 = 3;

// enum Dav1dFilterMode
pub(crate) const DAV1D_FILTER_8TAP_REGULAR: i32 = 0;
pub(crate) const DAV1D_FILTER_8TAP_SMOOTH: i32 = 1;
pub(crate) const DAV1D_FILTER_8TAP_SHARP: i32 = 2;
pub(crate) const DAV1D_N_SWITCHABLE_FILTERS: i32 = 3;
pub(crate) const DAV1D_FILTER_BILINEAR: i32 = DAV1D_N_SWITCHABLE_FILTERS;
pub(crate) const DAV1D_N_FILTERS: i32 = 4;
pub(crate) const DAV1D_FILTER_SWITCHABLE: i32 = DAV1D_N_FILTERS;

// enum Dav1dAdaptiveBoolean
pub(crate) const DAV1D_OFF: i32 = 0;
pub(crate) const DAV1D_ON: i32 = 1;
pub(crate) const DAV1D_ADAPTIVE: i32 = 2;

// enum Dav1dRestorationType
pub(crate) const DAV1D_RESTORATION_NONE: u8 = 0;
pub(crate) const DAV1D_RESTORATION_SWITCHABLE: u8 = 1;
pub(crate) const DAV1D_RESTORATION_WIENER: u8 = 2;
pub(crate) const DAV1D_RESTORATION_SGRPROJ: u8 = 3;

// enum Dav1dWarpedMotionType
pub(crate) const DAV1D_WM_TYPE_IDENTITY: i32 = 0;
pub(crate) const DAV1D_WM_TYPE_TRANSLATION: i32 = 1;
pub(crate) const DAV1D_WM_TYPE_ROT_ZOOM: i32 = 2;
pub(crate) const DAV1D_WM_TYPE_AFFINE: i32 = 3;

/// Translation of `Dav1dWarpedMotionParams`. The union of the four shear
/// parameters and their `abcd` array view is the `abcd` array, with
/// accessors for the named members.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dWarpedMotionParams {
    pub(crate) type_: i32,
    pub(crate) matrix: [i32; 6],
    pub(crate) abcd: [i16; 4],
}

impl Dav1dWarpedMotionParams {
    pub(crate) fn alpha(&self) -> i32 {
        self.abcd[0] as i32
    }
    pub(crate) fn beta(&self) -> i32 {
        self.abcd[1] as i32
    }
    pub(crate) fn gamma(&self) -> i32 {
        self.abcd[2] as i32
    }
    pub(crate) fn delta(&self) -> i32 {
        self.abcd[3] as i32
    }
}

// enum Dav1dPixelLayout
pub(crate) const DAV1D_PIXEL_LAYOUT_I400: i32 = 0; // monochrome
pub(crate) const DAV1D_PIXEL_LAYOUT_I420: i32 = 1; // 4:2:0 planar
pub(crate) const DAV1D_PIXEL_LAYOUT_I422: i32 = 2; // 4:2:2 planar
pub(crate) const DAV1D_PIXEL_LAYOUT_I444: i32 = 3; // 4:4:4 planar

// enum Dav1dFrameType
pub(crate) const DAV1D_FRAME_TYPE_KEY: i32 = 0; // Key Intra frame
pub(crate) const DAV1D_FRAME_TYPE_INTER: i32 = 1; // Inter frame
pub(crate) const DAV1D_FRAME_TYPE_INTRA: i32 = 2; // Non key Intra frame
pub(crate) const DAV1D_FRAME_TYPE_SWITCH: i32 = 3; // Switch Inter frame

// enum Dav1dColorPrimaries
pub(crate) const DAV1D_COLOR_PRI_BT709: i32 = 1;
pub(crate) const DAV1D_COLOR_PRI_UNKNOWN: i32 = 2;
pub(crate) const DAV1D_COLOR_PRI_BT470M: i32 = 4;
pub(crate) const DAV1D_COLOR_PRI_BT470BG: i32 = 5;
pub(crate) const DAV1D_COLOR_PRI_BT601: i32 = 6;
pub(crate) const DAV1D_COLOR_PRI_SMPTE240: i32 = 7;
pub(crate) const DAV1D_COLOR_PRI_FILM: i32 = 8;
pub(crate) const DAV1D_COLOR_PRI_BT2020: i32 = 9;
pub(crate) const DAV1D_COLOR_PRI_XYZ: i32 = 10;
pub(crate) const DAV1D_COLOR_PRI_SMPTE431: i32 = 11;
pub(crate) const DAV1D_COLOR_PRI_SMPTE432: i32 = 12;
pub(crate) const DAV1D_COLOR_PRI_EBU3213: i32 = 22;
pub(crate) const DAV1D_COLOR_PRI_RESERVED: i32 = 255;

// enum Dav1dTransferCharacteristics
pub(crate) const DAV1D_TRC_BT709: i32 = 1;
pub(crate) const DAV1D_TRC_UNKNOWN: i32 = 2;
pub(crate) const DAV1D_TRC_BT470M: i32 = 4;
pub(crate) const DAV1D_TRC_BT470BG: i32 = 5;
pub(crate) const DAV1D_TRC_BT601: i32 = 6;
pub(crate) const DAV1D_TRC_SMPTE240: i32 = 7;
pub(crate) const DAV1D_TRC_LINEAR: i32 = 8;
pub(crate) const DAV1D_TRC_LOG100: i32 = 9; // logarithmic (100:1 range)
pub(crate) const DAV1D_TRC_LOG100_SQRT10: i32 = 10; // lograithmic (100*sqrt(10):1 range)
pub(crate) const DAV1D_TRC_IEC61966: i32 = 11;
pub(crate) const DAV1D_TRC_BT1361: i32 = 12;
pub(crate) const DAV1D_TRC_SRGB: i32 = 13;
pub(crate) const DAV1D_TRC_BT2020_10BIT: i32 = 14;
pub(crate) const DAV1D_TRC_BT2020_12BIT: i32 = 15;
pub(crate) const DAV1D_TRC_SMPTE2084: i32 = 16; // PQ
pub(crate) const DAV1D_TRC_SMPTE428: i32 = 17;
pub(crate) const DAV1D_TRC_HLG: i32 = 18; // hybrid log/gamma (BT.2100 / ARIB STD-B67)
pub(crate) const DAV1D_TRC_RESERVED: i32 = 255;

// enum Dav1dMatrixCoefficients
pub(crate) const DAV1D_MC_IDENTITY: i32 = 0;
pub(crate) const DAV1D_MC_BT709: i32 = 1;
pub(crate) const DAV1D_MC_UNKNOWN: i32 = 2;
pub(crate) const DAV1D_MC_FCC: i32 = 4;
pub(crate) const DAV1D_MC_BT470BG: i32 = 5;
pub(crate) const DAV1D_MC_BT601: i32 = 6;
pub(crate) const DAV1D_MC_SMPTE240: i32 = 7;
pub(crate) const DAV1D_MC_SMPTE_YCGCO: i32 = 8;
pub(crate) const DAV1D_MC_BT2020_NCL: i32 = 9;
pub(crate) const DAV1D_MC_BT2020_CL: i32 = 10;
pub(crate) const DAV1D_MC_SMPTE2085: i32 = 11;
pub(crate) const DAV1D_MC_CHROMAT_NCL: i32 = 12; // Chromaticity-derived
pub(crate) const DAV1D_MC_CHROMAT_CL: i32 = 13;
pub(crate) const DAV1D_MC_ICTCP: i32 = 14;
pub(crate) const DAV1D_MC_RESERVED: i32 = 255;

// enum Dav1dChromaSamplePosition
pub(crate) const DAV1D_CHR_UNKNOWN: i32 = 0;
pub(crate) const DAV1D_CHR_VERTICAL: i32 = 1; // Horizontally co-located with luma(0, 0)
                                              // sample, between two vertical samples
pub(crate) const DAV1D_CHR_COLOCATED: i32 = 2; // Co-located with luma(0, 0) sample

/// Translation of `Dav1dContentLightLevel`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dContentLightLevel {
    pub(crate) max_content_light_level: i32,
    pub(crate) max_frame_average_light_level: i32,
}

/// Translation of `Dav1dMasteringDisplay`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dMasteringDisplay {
    /// 0.16 fixed point
    pub(crate) primaries: [[u16; 2]; 3],
    /// 0.16 fixed point
    pub(crate) white_point: [u16; 2],
    /// 24.8 fixed point
    pub(crate) max_luminance: u32,
    /// 18.14 fixed point
    pub(crate) min_luminance: u32,
}

/// Translation of `Dav1dITUTT35`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dITUTT35 {
    pub(crate) country_code: u8,
    pub(crate) country_code_extension_byte: u8,
    pub(crate) payload: Vec<u8>,
}

/// Translation of `struct Dav1dSequenceHeaderOperatingPoint`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dSequenceHeaderOperatingPoint {
    pub(crate) major_level: i32,
    pub(crate) minor_level: i32,
    pub(crate) initial_display_delay: i32,
    pub(crate) idc: i32,
    pub(crate) tier: i32,
    pub(crate) decoder_model_param_present: i32,
    pub(crate) display_model_param_present: i32,
}

/// Translation of `struct Dav1dSequenceHeaderOperatingParameterInfo`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dSequenceHeaderOperatingParameterInfo {
    pub(crate) decoder_buffer_delay: i32,
    pub(crate) encoder_buffer_delay: i32,
    pub(crate) low_delay_mode: i32,
}

/// Translation of `Dav1dSequenceHeader`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Dav1dSequenceHeader {
    /// Stream profile, 0 for 8-10 bits/component 4:2:0 or monochrome;
    /// 1 for 8-10 bits/component 4:4:4; 2 for 4:2:2 at any bits/component,
    /// or 12 bits/component at any chroma subsampling.
    pub(crate) profile: i32,
    /// Maximum dimensions for this stream. In non-scalable streams, these
    /// are often the actual dimensions of the stream, although that is not
    /// a normative requirement.
    pub(crate) max_width: i32,
    pub(crate) max_height: i32,
    /// format of the picture
    pub(crate) layout: i32,
    /// color primaries (av1)
    pub(crate) pri: i32,
    /// transfer characteristics (av1)
    pub(crate) trc: i32,
    /// matrix coefficients (av1)
    pub(crate) mtrx: i32,
    /// chroma sample position (av1)
    pub(crate) chr: i32,
    /// 0, 1 and 2 mean 8, 10 or 12 bits/component, respectively. This is not
    /// exactly the same as 'hbd' from the spec; the spec's hbd distinguishes
    /// between 8 (0) and 10-12 (1) bits/component, and another element
    /// (twelve_bit) to distinguish between 10 and 12 bits/component. To get
    /// the spec's hbd, use !!our_hbd, and to get twelve_bit, use hbd == 2.
    pub(crate) hbd: i32,
    /// Pixel data uses JPEG pixel range ([0,255] for 8bits) instead of
    /// MPEG pixel range ([16,235] for 8bits luma, [16,240] for 8bits chroma).
    pub(crate) color_range: i32,

    pub(crate) num_operating_points: i32,
    pub(crate) operating_points: [Dav1dSequenceHeaderOperatingPoint; DAV1D_MAX_OPERATING_POINTS],

    pub(crate) still_picture: i32,
    pub(crate) reduced_still_picture_header: i32,
    pub(crate) timing_info_present: i32,
    pub(crate) num_units_in_tick: i32,
    pub(crate) time_scale: i32,
    pub(crate) equal_picture_interval: i32,
    pub(crate) num_ticks_per_picture: u32,
    pub(crate) decoder_model_info_present: i32,
    pub(crate) encoder_decoder_buffer_delay_length: i32,
    pub(crate) num_units_in_decoding_tick: i32,
    pub(crate) buffer_removal_delay_length: i32,
    pub(crate) frame_presentation_delay_length: i32,
    pub(crate) display_model_info_present: i32,
    pub(crate) width_n_bits: i32,
    pub(crate) height_n_bits: i32,
    pub(crate) frame_id_numbers_present: i32,
    pub(crate) delta_frame_id_n_bits: i32,
    pub(crate) frame_id_n_bits: i32,
    pub(crate) sb128: i32,
    pub(crate) filter_intra: i32,
    pub(crate) intra_edge_filter: i32,
    pub(crate) inter_intra: i32,
    pub(crate) masked_compound: i32,
    pub(crate) warped_motion: i32,
    pub(crate) dual_filter: i32,
    pub(crate) order_hint: i32,
    pub(crate) jnt_comp: i32,
    pub(crate) ref_frame_mvs: i32,
    pub(crate) screen_content_tools: i32,
    pub(crate) force_integer_mv: i32,
    pub(crate) order_hint_n_bits: i32,
    pub(crate) super_res: i32,
    pub(crate) cdef: i32,
    pub(crate) restoration: i32,
    pub(crate) ss_hor: i32,
    pub(crate) ss_ver: i32,
    pub(crate) monochrome: i32,
    pub(crate) color_description_present: i32,
    pub(crate) separate_uv_delta_q: i32,
    pub(crate) film_grain_present: i32,

    // Dav1dSequenceHeaders of the same sequence are required to be
    // bit-identical until this offset. See 7.5 "Ordering of OBUs":
    //   Within a particular coded video sequence, the contents of
    //   sequence_header_obu must be bit-identical each time the
    //   sequence header appears except for the contents of
    //   operating_parameters_info.
    pub(crate) operating_parameter_info:
        [Dav1dSequenceHeaderOperatingParameterInfo; DAV1D_MAX_OPERATING_POINTS],
}

impl Dav1dSequenceHeader {
    /// Compare everything before `operating_parameter_info`, as upstream's
    /// `memcmp(seq_hdr, c->seq_hdr, offsetof(Dav1dSequenceHeader,
    /// operating_parameter_info))`.
    pub(crate) fn differs_before_op_params_info(&self, o: &Dav1dSequenceHeader) -> bool {
        let a = self;
        let b = o;
        !(a.profile == b.profile
            && a.max_width == b.max_width
            && a.max_height == b.max_height
            && a.layout == b.layout
            && a.pri == b.pri
            && a.trc == b.trc
            && a.mtrx == b.mtrx
            && a.chr == b.chr
            && a.hbd == b.hbd
            && a.color_range == b.color_range
            && a.num_operating_points == b.num_operating_points
            && a.operating_points == b.operating_points
            && a.still_picture == b.still_picture
            && a.reduced_still_picture_header == b.reduced_still_picture_header
            && a.timing_info_present == b.timing_info_present
            && a.num_units_in_tick == b.num_units_in_tick
            && a.time_scale == b.time_scale
            && a.equal_picture_interval == b.equal_picture_interval
            && a.num_ticks_per_picture == b.num_ticks_per_picture
            && a.decoder_model_info_present == b.decoder_model_info_present
            && a.encoder_decoder_buffer_delay_length == b.encoder_decoder_buffer_delay_length
            && a.num_units_in_decoding_tick == b.num_units_in_decoding_tick
            && a.buffer_removal_delay_length == b.buffer_removal_delay_length
            && a.frame_presentation_delay_length == b.frame_presentation_delay_length
            && a.display_model_info_present == b.display_model_info_present
            && a.width_n_bits == b.width_n_bits
            && a.height_n_bits == b.height_n_bits
            && a.frame_id_numbers_present == b.frame_id_numbers_present
            && a.delta_frame_id_n_bits == b.delta_frame_id_n_bits
            && a.frame_id_n_bits == b.frame_id_n_bits
            && a.sb128 == b.sb128
            && a.filter_intra == b.filter_intra
            && a.intra_edge_filter == b.intra_edge_filter
            && a.inter_intra == b.inter_intra
            && a.masked_compound == b.masked_compound
            && a.warped_motion == b.warped_motion
            && a.dual_filter == b.dual_filter
            && a.order_hint == b.order_hint
            && a.jnt_comp == b.jnt_comp
            && a.ref_frame_mvs == b.ref_frame_mvs
            && a.screen_content_tools == b.screen_content_tools
            && a.force_integer_mv == b.force_integer_mv
            && a.order_hint_n_bits == b.order_hint_n_bits
            && a.super_res == b.super_res
            && a.cdef == b.cdef
            && a.restoration == b.restoration
            && a.ss_hor == b.ss_hor
            && a.ss_ver == b.ss_ver
            && a.monochrome == b.monochrome
            && a.color_description_present == b.color_description_present
            && a.separate_uv_delta_q == b.separate_uv_delta_q
            && a.film_grain_present == b.film_grain_present)
    }
}

/// Translation of `Dav1dSegmentationData`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dSegmentationData {
    pub(crate) delta_q: i32,
    pub(crate) delta_lf_y_v: i32,
    pub(crate) delta_lf_y_h: i32,
    pub(crate) delta_lf_u: i32,
    pub(crate) delta_lf_v: i32,
    pub(crate) ref_: i32,
    pub(crate) skip: i32,
    pub(crate) globalmv: i32,
}

/// Translation of `Dav1dSegmentationDataSet`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dSegmentationDataSet {
    pub(crate) d: [Dav1dSegmentationData; DAV1D_MAX_SEGMENTS],
    pub(crate) preskip: i32,
    pub(crate) last_active_segid: i32,
}

/// Translation of `Dav1dLoopfilterModeRefDeltas`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dLoopfilterModeRefDeltas {
    pub(crate) mode_delta: [i32; 2],
    pub(crate) ref_delta: [i32; DAV1D_TOTAL_REFS_PER_FRAME],
}

/// Translation of `Dav1dFilmGrainData`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFilmGrainData {
    pub(crate) seed: u32,
    pub(crate) num_y_points: i32,
    pub(crate) y_points: [[u8; 2]; 14],
    pub(crate) chroma_scaling_from_luma: i32,
    pub(crate) num_uv_points: [i32; 2],
    pub(crate) uv_points: [[[u8; 2]; 10]; 2],
    pub(crate) scaling_shift: i32,
    pub(crate) ar_coeff_lag: i32,
    pub(crate) ar_coeffs_y: [i8; 24],
    pub(crate) ar_coeffs_uv: [[i8; 25 + 3]; 2],
    pub(crate) ar_coeff_shift: u64,
    pub(crate) grain_scale_shift: i32,
    pub(crate) uv_mult: [i32; 2],
    pub(crate) uv_luma_mult: [i32; 2],
    pub(crate) uv_offset: [i32; 2],
    pub(crate) overlap_flag: i32,
    pub(crate) clip_to_restricted_range: i32,
}

/// The `film_grain` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderFilmGrain {
    pub(crate) data: Dav1dFilmGrainData,
    pub(crate) present: i32,
    pub(crate) update: i32,
}

/// Translation of `struct Dav1dFrameHeaderOperatingPoint`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderOperatingPoint {
    pub(crate) buffer_removal_time: i32,
}

/// The `super_res` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderSuperRes {
    pub(crate) width_scale_denominator: i32,
    pub(crate) enabled: i32,
}

/// The `tiling` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderTiling {
    pub(crate) uniform: i32,
    pub(crate) n_bytes: u32,
    pub(crate) min_log2_cols: i32,
    pub(crate) max_log2_cols: i32,
    pub(crate) log2_cols: i32,
    pub(crate) cols: i32,
    pub(crate) min_log2_rows: i32,
    pub(crate) max_log2_rows: i32,
    pub(crate) log2_rows: i32,
    pub(crate) rows: i32,
    pub(crate) col_start_sb: [u16; DAV1D_MAX_TILE_COLS + 1],
    pub(crate) row_start_sb: [u16; DAV1D_MAX_TILE_ROWS + 1],
    pub(crate) update: i32,
}

impl Default for Dav1dFrameHeaderTiling {
    fn default() -> Self {
        Dav1dFrameHeaderTiling {
            uniform: 0,
            n_bytes: 0,
            min_log2_cols: 0,
            max_log2_cols: 0,
            log2_cols: 0,
            cols: 0,
            min_log2_rows: 0,
            max_log2_rows: 0,
            log2_rows: 0,
            rows: 0,
            col_start_sb: [0; DAV1D_MAX_TILE_COLS + 1],
            row_start_sb: [0; DAV1D_MAX_TILE_ROWS + 1],
            update: 0,
        }
    }
}

/// The `quant` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderQuant {
    pub(crate) yac: i32,
    pub(crate) ydc_delta: i32,
    pub(crate) udc_delta: i32,
    pub(crate) uac_delta: i32,
    pub(crate) vdc_delta: i32,
    pub(crate) vac_delta: i32,
    pub(crate) qm: i32,
    pub(crate) qm_y: i32,
    pub(crate) qm_u: i32,
    pub(crate) qm_v: i32,
}

/// The `segmentation` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderSegmentation {
    pub(crate) enabled: i32,
    pub(crate) update_map: i32,
    pub(crate) temporal: i32,
    pub(crate) update_data: i32,
    pub(crate) seg_data: Dav1dSegmentationDataSet,
    pub(crate) lossless: [i32; DAV1D_MAX_SEGMENTS],
    pub(crate) qidx: [i32; DAV1D_MAX_SEGMENTS],
}

/// The `delta.q` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderDeltaQ {
    pub(crate) present: i32,
    pub(crate) res_log2: i32,
}

/// The `delta.lf` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderDeltaLf {
    pub(crate) present: i32,
    pub(crate) res_log2: i32,
    pub(crate) multi: i32,
}

/// The `delta` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderDelta {
    pub(crate) q: Dav1dFrameHeaderDeltaQ,
    pub(crate) lf: Dav1dFrameHeaderDeltaLf,
}

/// The `loopfilter` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderLoopfilter {
    pub(crate) level_y: [i32; 2],
    pub(crate) level_u: i32,
    pub(crate) level_v: i32,
    pub(crate) mode_ref_delta_enabled: i32,
    pub(crate) mode_ref_delta_update: i32,
    pub(crate) mode_ref_deltas: Dav1dLoopfilterModeRefDeltas,
    pub(crate) sharpness: i32,
}

/// The `cdef` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderCdef {
    pub(crate) damping: i32,
    pub(crate) n_bits: i32,
    pub(crate) y_strength: [i32; DAV1D_MAX_CDEF_STRENGTHS],
    pub(crate) uv_strength: [i32; DAV1D_MAX_CDEF_STRENGTHS],
}

/// The `restoration` member of `Dav1dFrameHeader`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Dav1dFrameHeaderRestoration {
    pub(crate) type_: [u8; 3],
    pub(crate) unit_size: [i32; 2],
}

/// Translation of `Dav1dFrameHeader`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Dav1dFrameHeader {
    /// film grain parameters
    pub(crate) film_grain: Dav1dFrameHeaderFilmGrain,
    /// type of the picture
    pub(crate) frame_type: i32,
    /// { coded_width, superresolution_upscaled_width }
    pub(crate) width: [i32; 2],
    pub(crate) height: i32,
    /// frame number
    pub(crate) frame_offset: i32,
    /// temporal id of the frame for SVC
    pub(crate) temporal_id: i32,
    /// spatial id of the frame for SVC
    pub(crate) spatial_id: i32,

    pub(crate) show_existing_frame: i32,
    pub(crate) existing_frame_idx: i32,
    pub(crate) frame_id: i32,
    pub(crate) frame_presentation_delay: i32,
    pub(crate) show_frame: i32,
    pub(crate) showable_frame: i32,
    pub(crate) error_resilient_mode: i32,
    pub(crate) disable_cdf_update: i32,
    pub(crate) allow_screen_content_tools: i32,
    pub(crate) force_integer_mv: i32,
    pub(crate) frame_size_override: i32,
    pub(crate) primary_ref_frame: i32,
    pub(crate) buffer_removal_time_present: i32,
    pub(crate) operating_points: [Dav1dFrameHeaderOperatingPoint; DAV1D_MAX_OPERATING_POINTS],
    pub(crate) refresh_frame_flags: i32,
    pub(crate) render_width: i32,
    pub(crate) render_height: i32,
    pub(crate) super_res: Dav1dFrameHeaderSuperRes,
    pub(crate) have_render_size: i32,
    pub(crate) allow_intrabc: i32,
    pub(crate) frame_ref_short_signaling: i32,
    pub(crate) refidx: [i32; DAV1D_REFS_PER_FRAME],
    pub(crate) hp: i32,
    pub(crate) subpel_filter_mode: i32,
    pub(crate) switchable_motion_mode: i32,
    pub(crate) use_ref_frame_mvs: i32,
    pub(crate) refresh_context: i32,
    pub(crate) tiling: Dav1dFrameHeaderTiling,
    pub(crate) quant: Dav1dFrameHeaderQuant,
    pub(crate) segmentation: Dav1dFrameHeaderSegmentation,
    pub(crate) delta: Dav1dFrameHeaderDelta,
    pub(crate) all_lossless: i32,
    pub(crate) loopfilter: Dav1dFrameHeaderLoopfilter,
    pub(crate) cdef: Dav1dFrameHeaderCdef,
    pub(crate) restoration: Dav1dFrameHeaderRestoration,
    pub(crate) txfm_mode: i32,
    pub(crate) switchable_comp_refs: i32,
    pub(crate) skip_mode_allowed: i32,
    pub(crate) skip_mode_enabled: i32,
    pub(crate) skip_mode_refs: [i32; 2],
    pub(crate) warp_motion: i32,
    pub(crate) reduced_txtp_set: i32,
    pub(crate) gmv: [Dav1dWarpedMotionParams; DAV1D_REFS_PER_FRAME],
}

/// Translation of the `IS_INTER_OR_SWITCH()` macro of common/frame.h:
/// checks whether Dav1dFrameType == INTER || == SWITCH. Both are defined
/// as odd numbers {1, 3} and therefore have the LSB set. See also: AV1
/// spec 6.8.2.
pub(crate) fn is_inter_or_switch(frame_header: &Dav1dFrameHeader) -> bool {
    frame_header.frame_type & 1 != 0
}

/// Translation of the `IS_KEY_OR_INTRA()` macro of common/frame.h: checks
/// whether Dav1dFrameType == KEY || == INTRA. See also: AV1 spec 6.8.2.
pub(crate) fn is_key_or_intra(frame_header: &Dav1dFrameHeader) -> bool {
    !is_inter_or_switch(frame_header)
}
