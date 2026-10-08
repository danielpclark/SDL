// Rust translation of src/enc/vp8i_enc.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebP encoder: internal header.
//!
//! The encoder's planes (`mb_info_`, `preds_`, `nz_`, the top samples and
//! the diffusion errors) are vectors; the iterator addresses them by index
//! and is passed alongside the encoder rather than pointing back to it.
//! The statistics (`WebPAuxStats`), the progress report, the extra-info map
//! and the threaded alpha worker are not translated.

use crate::webp::dec::{
    MAX_NUM_PARTITIONS, NUM_BANDS, NUM_CTX, NUM_MB_SEGMENTS, NUM_PROBAS, NUM_TYPES,
};
use crate::webp::dsp::BPS;
use crate::webp::encode::{WebPConfig, WebPPicture};
use crate::webp::utils::bit_writer_utils::VP8BitWriter;

//------------------------------------------------------------------------------
// Various defines and enums

// (MAX_LF_LEVELS: the autofilter's, not translated.)
/// last (inclusive) level with variable cost
pub(crate) const MAX_VARIABLE_LEVEL: usize = 67;
/// max level (note: max codable is 2047 + 67)
pub(crate) const MAX_LEVEL: i32 = 2047;

/// Rate-distortion optimization levels. Translation of `VP8RDLevel`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum VP8RDLevel {
    /// no rd-opt
    None = 0,
    /// basic scoring (no trellis)
    Basic = 1,
    /// perform trellis-quant on the final decision only
    Trellis = 2,
    /// trellis-quant for every scoring (much slower)
    TrellisAll = 3,
}

// YUV-cache parameters. Cache is 32-bytes wide (= one cacheline).
// The original or reconstructed samples can be accessed using VP8Scan[].
// The predicted blocks can be accessed using offsets to yuv_p_ and
// the arrays VP8*ModeOffsets[].
// * YUV Samples area (yuv_in_/yuv_out_/yuv_out2_)
//   (see VP8Scan[] for accessing the blocks, along with
//   Y_OFF_ENC/U_OFF_ENC/V_OFF_ENC):
//             +----+----+
//  Y_OFF_ENC  |YYYY|UUVV|
//  U_OFF_ENC  |YYYY|UUVV|
//  V_OFF_ENC  |YYYY|....| <- 25% wasted U/V area
//             |YYYY|....|
//             +----+----+
// * Prediction area ('yuv_p_', size = PRED_SIZE_ENC)
//   Intra16 predictions (16x16 block each, two per row):
//         |I16DC16|I16TM16|
//         |I16VE16|I16HE16|
//   Chroma U/V predictions (16x8 block each, two per row):
//         |C8DC8|C8TM8|
//         |C8VE8|C8HE8|
//   Intra 4x4 predictions (4x4 block each)
//         |I4DC4 I4TM4 I4VE4 I4HE4|I4RD4 I4VR4 I4LD4 I4VL4|
//         |I4HD4 I4HU4 I4TMP .....|.......................| <- ~31% wasted
pub(crate) const YUV_SIZE_ENC: usize = BPS * 16;
/// I16+Chroma+I4 preds
pub(crate) const PRED_SIZE_ENC: usize = 32 * BPS + 16 * BPS + 8 * BPS;
pub(crate) const Y_OFF_ENC: usize = 0;
pub(crate) const U_OFF_ENC: usize = 16;
pub(crate) const V_OFF_ENC: usize = 16 + 8;

// Layout of prediction blocks
// intra 16x16
pub(crate) const I16DC16: usize = 0;
pub(crate) const I16TM16: usize = I16DC16 + 16;
pub(crate) const I16VE16: usize = 16 * BPS;
pub(crate) const I16HE16: usize = I16VE16 + 16;
// chroma 8x8, two U/V blocks side by side (hence: 16x8 each)
pub(crate) const C8DC8: usize = 2 * 16 * BPS;
pub(crate) const C8TM8: usize = C8DC8 + 16;
pub(crate) const C8VE8: usize = 2 * 16 * BPS + 8 * BPS;
pub(crate) const C8HE8: usize = C8VE8 + 16;
// intra 4x4
pub(crate) const I4DC4: usize = 3 * 16 * BPS;
pub(crate) const I4TM4: usize = I4DC4 + 4;
pub(crate) const I4VE4: usize = I4DC4 + 8;
pub(crate) const I4HE4: usize = I4DC4 + 12;
pub(crate) const I4RD4: usize = I4DC4 + 16;
pub(crate) const I4VR4: usize = I4DC4 + 20;
pub(crate) const I4LD4: usize = I4DC4 + 24;
pub(crate) const I4VL4: usize = I4DC4 + 28;
pub(crate) const I4HD4: usize = 3 * 16 * BPS + 4 * BPS;
pub(crate) const I4HU4: usize = I4HD4 + 4;
pub(crate) const I4TMP: usize = I4HD4 + 8;

/// type used for scores, rate, distortion. Translation of `score_t`.
pub(crate) type ScoreT = i64;
/// Note that MAX_COST is not the maximum allowed by sizeof(score_t),
/// in order to allow overflowing computations.
pub(crate) const MAX_COST: ScoreT = 0x7fffffffffffff;

pub(crate) const QFIX: u32 = 17;

/// Translation of the `BIAS()` macro.
pub(crate) const fn bias(b: u32) -> u32 {
    b << (QFIX - 8)
}

/// Fun fact: this is the _only_ line where we're actually being lossy and
/// discarding bits. Translation of `QUANTDIV()`.
pub(crate) fn quantdiv(n: u32, iq: u32, b: u32) -> i32 {
    (n.wrapping_mul(iq).wrapping_add(b) >> QFIX) as i32
}

/// quality below which error-diffusion is enabled
pub(crate) const ERROR_DIFFUSION_QUALITY: f32 = 98.0;

//------------------------------------------------------------------------------
// Headers

/// 16b + 16b. Translation of `proba_t`.
pub(crate) type ProbaT = u32;
pub(crate) type ProbaArray = [[u8; NUM_PROBAS]; NUM_CTX];
pub(crate) type StatsArray = [[ProbaT; NUM_PROBAS]; NUM_CTX];
pub(crate) type CostArray = [[u16; MAX_VARIABLE_LEVEL + 1]; NUM_CTX];

/// segment features. Translation of `VP8EncSegmentHeader`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8EncSegmentHeader {
    /// Actual number of segments. 1 segment only = unused.
    pub(crate) num_segments: i32,
    /// whether to update the segment map or not.
    /// must be 0 if there's only 1 segment.
    pub(crate) update_map: bool,
    /// bit-cost for transmitting the segment map
    pub(crate) size: i32,
}

/// Struct collecting all frame-persistent probabilities.
/// Translation of `VP8EncProba` (`remapped_costs_` is the band lookup
/// `level_cost[t][VP8EncBands[n]]`).
#[derive(Clone)]
pub(crate) struct VP8EncProba {
    /// probabilities for segment tree
    pub(crate) segments: [u8; 3],
    /// final probability of being skipped.
    pub(crate) skip_proba: u8,
    /// 1056 bytes
    pub(crate) coeffs: [[ProbaArray; NUM_BANDS]; NUM_TYPES],
    /// 4224 bytes
    pub(crate) stats: [[StatsArray; NUM_BANDS]; NUM_TYPES],
    /// 13056 bytes
    pub(crate) level_cost: [[CostArray; NUM_BANDS]; NUM_TYPES],
    /// if true, need to call VP8CalculateLevelCosts()
    pub(crate) dirty: bool,
    /// Note: we always use skip_proba for now.
    pub(crate) use_skip_proba: bool,
    /// number of skipped blocks
    pub(crate) nb_skip: i32,
}

impl Default for VP8EncProba {
    fn default() -> Self {
        VP8EncProba {
            segments: [0; 3],
            skip_proba: 0,
            coeffs: [[[[0; NUM_PROBAS]; NUM_CTX]; NUM_BANDS]; NUM_TYPES],
            stats: [[[[0; NUM_PROBAS]; NUM_CTX]; NUM_BANDS]; NUM_TYPES],
            level_cost: [[[[0; MAX_VARIABLE_LEVEL + 1]; NUM_CTX]; NUM_BANDS]; NUM_TYPES],
            dirty: false,
            use_skip_proba: false,
            nb_skip: 0,
        }
    }
}

/// Filter parameters. Not actually used in the code (we don't perform
/// the in-loop filtering), but filled from user's config
/// Translation of `VP8EncFilterHeader`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8EncFilterHeader {
    /// filtering type: 0=complex, 1=simple
    pub(crate) simple: bool,
    /// base filter level [0..63]
    pub(crate) level: i32,
    /// [0..7]
    pub(crate) sharpness: i32,
    /// delta filter level for i4x4 relative to i16x16
    pub(crate) i4x4_lf_delta: i32,
}

//------------------------------------------------------------------------------
// Informations about the macroblocks.

/// Translation of `VP8MBInfo`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8MBInfo {
    // block type
    /// 0=i4x4, 1=i16x16
    pub(crate) type_: u8,
    pub(crate) uv_mode: u8,
    pub(crate) skip: bool,
    pub(crate) segment: u8,
    /// quantization-susceptibility
    pub(crate) alpha: u8,
}

/// Translation of `VP8Matrix`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8Matrix {
    /// quantizer steps
    pub(crate) q: [u16; 16],
    /// reciprocals, fixed point.
    pub(crate) iq: [u16; 16],
    /// rounding bias
    pub(crate) bias: [u32; 16],
    /// value below which a coefficient is zeroed
    pub(crate) zthresh: [u32; 16],
    /// frequency boosters for slight sharpening
    pub(crate) sharpen: [u16; 16],
}

/// Translation of `VP8SegmentInfo`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8SegmentInfo {
    /// quantization matrices
    pub(crate) y1: VP8Matrix,
    pub(crate) y2: VP8Matrix,
    pub(crate) uv: VP8Matrix,
    /// quant-susceptibility, range [-127,127]. Zero is neutral.
    /// Lower values indicate a lower risk of blurriness.
    pub(crate) alpha: i32,
    /// filter-susceptibility, range [0,255].
    pub(crate) beta: i32,
    /// final segment quantizer.
    pub(crate) quant: i32,
    /// final in-loop filtering strength
    pub(crate) fstrength: i32,
    /// max edge delta (for filtering strength)
    pub(crate) max_edge: i32,
    /// minimum distortion required to trigger filtering record
    pub(crate) min_disto: i32,
    // reactivities
    pub(crate) lambda_i16: i32,
    pub(crate) lambda_i4: i32,
    pub(crate) lambda_uv: i32,
    pub(crate) lambda_mode: i32,
    #[allow(dead_code)] // (unused upstream too)
    pub(crate) lambda_trellis: i32,
    pub(crate) tlambda: i32,
    pub(crate) lambda_trellis_i16: i32,
    pub(crate) lambda_trellis_i4: i32,
    pub(crate) lambda_trellis_uv: i32,

    // lambda values for distortion-based evaluation
    /// penalty for using Intra4
    pub(crate) i4_penalty: ScoreT,
}

/// Translation of `DError`: [u/v][top or left].
pub(crate) type DError = [[i8; 2]; 2];

/// Handy transient struct to accumulate score and info during RD-optimization
/// and mode evaluation. Translation of `VP8ModeScore`.
#[derive(Clone, Default)]
pub(crate) struct VP8ModeScore {
    /// Distortion, spectral distortion
    pub(crate) d: ScoreT,
    pub(crate) sd: ScoreT,
    /// header bits, rate, score.
    pub(crate) h: ScoreT,
    pub(crate) r: ScoreT,
    pub(crate) score: ScoreT,
    /// Quantized levels for luma-DC, luma-AC, chroma.
    pub(crate) y_dc_levels: [i16; 16],
    pub(crate) y_ac_levels: [[i16; 16]; 16],
    pub(crate) uv_levels: [[i16; 16]; 4 + 4],
    /// mode number for intra16 prediction
    pub(crate) mode_i16: i32,
    /// mode numbers for intra4 predictions
    pub(crate) modes_i4: [u8; 16],
    /// mode number of chroma prediction
    pub(crate) mode_uv: i32,
    /// non-zero blocks
    pub(crate) nz: u32,
    /// DC diffusion errors for U/V for blocks #1/2/3
    pub(crate) derr: [[i8; 3]; 2],
}

/// The index of `y_left_[0]` in `yuv_left_mem`.
pub(crate) const Y_LEFT: usize = 1;
/// The index of `u_left_[0]` in `yuv_left_mem`.
pub(crate) const U_LEFT: usize = Y_LEFT + 16 + 16;
/// The index of `v_left_[0]` in `yuv_left_mem`.
pub(crate) const V_LEFT: usize = U_LEFT + 16;

/// Iterator structure to iterate through macroblocks, pointing to the
/// right neighbouring data (samples, predictions, contexts, ...)
/// Translation of `VP8EncIterator`: the pointers into the encoder are
/// indexes, and `yuv_mem_` is the four buffers it is split into.
#[derive(Clone)]
pub(crate) struct VP8EncIterator {
    /// current macroblock
    pub(crate) x: i32,
    pub(crate) y: i32,
    /// input samples
    pub(crate) yuv_in: Vec<u8>,
    /// output samples
    pub(crate) yuv_out: Vec<u8>,
    /// secondary buffer swapped with yuv_out_.
    pub(crate) yuv_out2: Vec<u8>,
    /// scratch buffer for prediction
    pub(crate) yuv_p: Vec<u8>,
    /// current macroblock (index into `mb_info`)
    pub(crate) mb: usize,
    /// current bit-writer (index into `parts`)
    pub(crate) bw: usize,
    /// intra mode predictors (4x4 blocks) (index into `preds`)
    pub(crate) preds: usize,
    /// non-zero pattern (index into `nz`)
    pub(crate) nz: usize,
    /// 32+5 boundary samples needed by intra4x4
    pub(crate) i4_boundary: [u8; 37],
    /// the current top boundary sample (index into `i4_boundary`)
    pub(crate) i4_top: usize,
    /// current intra4x4 mode being tested
    pub(crate) i4: i32,
    /// top-non-zero context.
    pub(crate) top_nz: [i32; 9],
    /// left-non-zero. left_nz[8] is independent.
    pub(crate) left_nz: [i32; 9],
    /// if true, perform extra level optimisation
    pub(crate) do_trellis: bool,
    /// number of mb still to be processed
    pub(crate) count_down: i32,
    /// starting counter value (for progress)
    pub(crate) count_down0: i32,

    /// left error diffusion (u/v)
    pub(crate) left_derr: DError,
    /// top diffusion error - false if disabled (`top_derr_ != NULL`)
    pub(crate) has_top_derr: bool,

    /// memory for storing y/u/v_left_: `y_left_` (addressable from index
    /// -1 to 15) is at `Y_LEFT`, `u_left_` and `v_left_` (from -1 to 7) at
    /// `U_LEFT` and `V_LEFT`.
    pub(crate) yuv_left_mem: [u8; 64],

    /// top luma samples at position 'x_' (index into `y_top`)
    pub(crate) y_top: usize,
    /// top u/v samples at position 'x_', packed as 16 bytes (index into
    /// `uv_top`)
    pub(crate) uv_top: usize,
    /// whether `y_top_`/`uv_top_` point to the `tmp_32` scratch rather
    /// than to the encoder's arrays
    pub(crate) use_tmp_32: bool,
    /// the analysis' boundary scratch (`tmp_32` upstream)
    pub(crate) tmp_32: [u8; 32],
}

impl Default for VP8EncIterator {
    fn default() -> Self {
        VP8EncIterator {
            x: 0,
            y: 0,
            yuv_in: vec![0; YUV_SIZE_ENC],
            yuv_out: vec![0; YUV_SIZE_ENC],
            yuv_out2: vec![0; YUV_SIZE_ENC],
            yuv_p: vec![0; PRED_SIZE_ENC],
            mb: 0,
            bw: 0,
            preds: 0,
            nz: 0,
            i4_boundary: [0; 37],
            i4_top: 0,
            i4: 0,
            top_nz: [0; 9],
            left_nz: [0; 9],
            do_trellis: false,
            count_down: 0,
            count_down0: 0,
            left_derr: [[0; 2]; 2],
            has_top_derr: false,
            yuv_left_mem: [0; 64],
            y_top: 0,
            uv_top: 0,
            use_tmp_32: false,
            tmp_32: [0; 32],
        }
    }
}

//------------------------------------------------------------------------------
// Paginated token buffer

/// Translation of `VP8TBuffer`: the pages are one vector, in the order
/// the tokens are emitted.
#[derive(Clone, Default)]
pub(crate) struct VP8TBuffer {
    /// the tokens
    pub(crate) tokens: Vec<u16>,
    /// number of tokens per page
    pub(crate) page_size: i32,
    /// true in case of malloc error
    pub(crate) error: bool,
}

//------------------------------------------------------------------------------
// VP8Encoder

/// Translation of `struct VP8Encoder`.
pub(crate) struct VP8Encoder<'a> {
    /// user configuration and parameters
    pub(crate) config: &'a WebPConfig,
    /// input / output picture
    pub(crate) pic: &'a WebPPicture,

    // headers
    /// filtering information
    pub(crate) filter_hdr: VP8EncFilterHeader,
    /// segment information
    pub(crate) segment_hdr: VP8EncSegmentHeader,

    /// VP8's profile, deduced from Config.
    pub(crate) profile: i32,

    // dimension, in macroblock units.
    pub(crate) mb_w: i32,
    pub(crate) mb_h: i32,
    /// stride of the *preds_ prediction plane (=4*mb_w + 1)
    pub(crate) preds_w: i32,

    /// number of partitions (1, 2, 4 or 8 = MAX_NUM_PARTITIONS)
    pub(crate) num_parts: i32,

    // per-partition boolean decoders.
    /// part0
    pub(crate) bw: VP8BitWriter,
    /// token partitions
    pub(crate) parts: [VP8BitWriter; MAX_NUM_PARTITIONS],
    /// token buffer
    pub(crate) tokens: VP8TBuffer,

    // transparency blob
    pub(crate) has_alpha: bool,
    /// non-empty if transparency is present
    pub(crate) alpha_data: Vec<u8>,

    /// quantization info (one set of DC/AC dequant factor per segment)
    pub(crate) dqm: [VP8SegmentInfo; NUM_MB_SEGMENTS],
    /// nominal quantizer value. Only used
    /// for relative coding of segments' quant.
    pub(crate) base_quant: i32,
    /// global susceptibility (<=> complexity)
    pub(crate) alpha: i32,
    /// U/V quantization susceptibility
    pub(crate) uv_alpha: i32,
    // global offset of quantizers, shared by all segments
    pub(crate) dq_y1_dc: i32,
    pub(crate) dq_y2_dc: i32,
    pub(crate) dq_y2_ac: i32,
    pub(crate) dq_uv_dc: i32,
    pub(crate) dq_uv_ac: i32,

    /// probabilities and statistics
    pub(crate) proba: Box<VP8EncProba>,

    // quality/speed settings
    /// 0=fastest, 6=best/slowest.
    pub(crate) method: i32,
    /// Deduced from method_.
    pub(crate) rd_opt_level: VP8RDLevel,
    /// partition #0 safeness factor
    pub(crate) max_i4_header_bits: i32,
    /// rough limit for header bits per MB
    pub(crate) mb_header_limit: ScoreT,
    /// derived from config->thread_level
    pub(crate) thread_level: i32,
    /// derived from config->target_XXX
    pub(crate) do_search: bool,
    /// if true, use token buffer
    pub(crate) use_tokens: bool,

    // Memory
    /// contextual macroblock infos (mb_w_ + 1)
    pub(crate) mb_info: Vec<VP8MBInfo>,
    /// predictions modes: (4*mb_w+1) * (4*mb_h+1); `preds_` is the index
    /// `1 + preds_w`.
    pub(crate) preds: Vec<u8>,
    /// non-zero bit context: mb_w+1; `nz_` is the index 1.
    pub(crate) nz: Vec<u32>,
    /// top luma samples.
    pub(crate) y_top: Vec<u8>,
    /// top u/v samples.
    /// U and V are packed into 16 bytes (8 U + 8 V)
    pub(crate) uv_top: Vec<u8>,
    /// diffusion error (empty if disabled)
    pub(crate) top_derr: Vec<DError>,
}
