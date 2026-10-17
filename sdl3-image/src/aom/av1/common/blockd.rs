// Rust translation of av1/common/blockd.c and av1/common/blockd.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The coding block's mode info (`MB_MODE_INFO`) and the block-level state
//! shared by encoder and decoder (`MACROBLOCKD`).
//!
//! Upstream's pointers become indices: `xd->mi` is an offset into the mode
//! info grid, a grid entry is an index into the mode info allocation
//! (`MI_NONE` for NULL), and plane buffer pointers are sample offsets into
//! the frame buffer they belong to. While a frame's tiles are encoded (and
//! while the bitstream is packed) the frame-wide state upstream reaches
//! through `cm` -- the mode info arrays, the above contexts and the
//! reconstructed frame -- is moved into `MacroBlockD`, so the rest of the
//! encoder state can be shared immutably.

use super::av1_common_int::CommonContexts;
use super::av1_common_int::MvRef;
use super::cfl::CflCtx;
use super::entropy::EntropyContext;
use super::entropymode::FrameContext;
use super::enums::*;
use super::mv::{IntMv, TransformationType, TRANSLATION};
use super::seg_common::MAX_SEGMENTS;
use super::tile_common::TileInfo;
use crate::aom::aom_scale::yv12config::Yv12BufferConfig;

pub(crate) const MAX_MB_PLANE: usize = 3;

pub(crate) const MAX_DIFFWTD_MASK_BITS: i32 = 1;

pub(crate) const INTERINTRA_WEDGE_SIGN: i32 = 0;

pub(crate) const DEFAULT_INTER_TX_TYPE: TxType = DCT_DCT;

pub(crate) const MAX_PALETTE_BLOCK_WIDTH: i32 = 64;

pub(crate) const MAX_PALETTE_BLOCK_HEIGHT: i32 = 64;

// (FRAME_TYPE)
pub(crate) const USE_B_QUANT_NO_TRELLIS: i32 = 1;

pub(crate) type FrameType = i32;
pub(crate) const KEY_FRAME: FrameType = 0;
pub(crate) const INTER_FRAME: FrameType = 1;
pub(crate) const INTRA_ONLY_FRAME: FrameType = 2; // replaces intra-only
pub(crate) const S_FRAME: FrameType = 3;
pub(crate) const FRAME_TYPES: FrameType = 4;

#[inline]
pub(crate) fn is_comp_ref_allowed(bsize: BlockSize) -> bool {
    BLOCK_SIZE_WIDE[bsize as usize].min(BLOCK_SIZE_HIGH[bsize as usize]) >= 8
}

/// An empty mode info grid entry (a NULL `MB_MODE_INFO *`).
pub(crate) const MI_NONE: u32 = u32::MAX;

/* For keyframes, intra block modes are predicted by the (already decoded)
modes for the Y blocks to the left and above us; for interframes, there
is a single probability table. */

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PaletteModeInfo {
    // Value of base colors for Y, U, and V
    pub(crate) palette_colors: [u16; 3 * PALETTE_MAX_SIZE as usize],
    // Number of base colors for Y (0) and UV (1)
    pub(crate) palette_size: [u8; 2],
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FilterIntraModeInfo {
    pub(crate) filter_intra_mode: FilterIntraMode,
    pub(crate) use_filter_intra: u8,
}

pub(crate) const INTER_TX_SIZE_BUF_LEN: usize = 16;

/// Translation of `MB_MODE_INFO`, without the inter-only motion data the
/// encoder never fills for intra frames (warp parameters, inter-inter
/// compound data); the bit fields are plain bytes.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MbModeInfo {
    // General Info of the Coding Block
    /// The block size of the current coding block
    pub(crate) bsize: BlockSize,
    /// The partition type of the current coding block.
    pub(crate) partition: PartitionType,
    /// The prediction mode used
    pub(crate) mode: PredictionMode,
    /// The UV mode when intra is used
    pub(crate) uv_mode: UvPredictionMode,
    /// The q index for the current coding block.
    pub(crate) current_qindex: i32,

    // Inter Mode Info
    /// The motion vectors used by the current inter mode
    pub(crate) mv: [IntMv; 2],
    /// The reference frames for the MV
    pub(crate) ref_frame: [MvReferenceFrame; 2],
    /// Filter used in subpel interpolation.
    pub(crate) interp_filters: u32,
    /// The motion mode used by the inter prediction.
    pub(crate) motion_mode: MotionMode,
    /// Number of samples used by warp causal
    pub(crate) num_proj_ref: u8,
    /// The number of overlapped neighbors above/left for obmc/warp motion
    /// mode.
    pub(crate) overlappable_neighbors: u8,
    /// The type of intra mode used by inter-intra
    pub(crate) interintra_mode: i32,
    /// The type of wedge used in interintra mode.
    pub(crate) interintra_wedge_index: i8,

    // Intra Mode Info
    /// Directional mode delta: the angle is base angle + (angle_delta *
    /// step).
    pub(crate) angle_delta: [i8; PLANE_TYPES as usize],
    /// The type of filter intra mode used (if applicable).
    pub(crate) filter_intra_mode_info: FilterIntraModeInfo,
    /// Chroma from Luma: Joint sign of alpha Cb and alpha Cr
    pub(crate) cfl_alpha_signs: i8,
    /// Chroma from Luma: Index of the alpha Cb and alpha Cr combination
    pub(crate) cfl_alpha_idx: u8,
    /// Stores the size and colors of palette mode
    pub(crate) palette_mode_info: PaletteModeInfo,

    // Transform Info
    /// Whether to skip transforming and sending.
    pub(crate) skip_txfm: u8,
    /// Transform size when fixed size txfm is used (e.g. intra modes).
    pub(crate) tx_size: TxSize,
    /// Transform size when recursive txfm tree is on.
    pub(crate) inter_tx_size: [TxSize; INTER_TX_SIZE_BUF_LEN],

    // Loop Filter Info
    pub(crate) delta_lf_from_base: i8,
    pub(crate) delta_lf: [i8; FRAME_LF_COUNT as usize],

    // Bitfield for Memory Reduction
    /// The segment id
    pub(crate) segment_id: u8,
    /// Only valid when temporal update if off.
    pub(crate) seg_id_predicted: u8,
    /// Which ref_mv to use
    pub(crate) ref_mv_idx: u8,
    /// Inter skip mode
    pub(crate) skip_mode: u8,
    /// Whether intrabc is used.
    pub(crate) use_intrabc: u8,
    /// Indicates if masked compound is used(1) or not (0).
    pub(crate) comp_group_idx: u8,
    /// Indicates whether dist_wtd_comp(0) is used or not (0).
    pub(crate) compound_idx: u8,
    /// Whether to use interintra wedge
    pub(crate) use_wedge_interintra: u8,
    /// CDEF strength per BLOCK_64X64
    pub(crate) cdef_strength: i8,
}

/// `CANDIDATE_MV`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CandidateMv {
    pub(crate) this_mv: IntMv,
    pub(crate) comp_mv: IntMv,
}

/// Translation of `have_newmv_in_inter_mode()`.
#[inline]
pub(crate) fn have_newmv_in_inter_mode(mode: PredictionMode) -> bool {
    mode == NEWMV
        || mode == NEW_NEWMV
        || mode == NEAREST_NEWMV
        || mode == NEW_NEARESTMV
        || mode == NEAR_NEWMV
        || mode == NEW_NEARMV
}

/// Translation of `is_global_mv_block()`.
#[inline]
pub(crate) fn is_global_mv_block(mbmi: &MbModeInfo, type_: TransformationType) -> bool {
    let mode = mbmi.mode;
    let bsize = mbmi.bsize;
    let block_size_allowed =
        BLOCK_SIZE_WIDE[bsize as usize].min(BLOCK_SIZE_HIGH[bsize as usize]) >= 8;
    (mode == GLOBALMV || mode == GLOBAL_GLOBALMV) && type_ > TRANSLATION && block_size_allowed
}

#[inline]
pub(crate) fn is_intrabc_block(mbmi: &MbModeInfo) -> bool {
    mbmi.use_intrabc != 0
}

#[inline]
pub(crate) fn get_uv_mode(mode: UvPredictionMode) -> PredictionMode {
    debug_assert!(mode < UV_INTRA_MODES);
    static UV2Y: [PredictionMode; 16] = [
        DC_PRED,       // UV_DC_PRED
        V_PRED,        // UV_V_PRED
        H_PRED,        // UV_H_PRED
        D45_PRED,      // UV_D45_PRED
        D135_PRED,     // UV_D135_PRED
        D113_PRED,     // UV_D113_PRED
        D157_PRED,     // UV_D157_PRED
        D203_PRED,     // UV_D203_PRED
        D67_PRED,      // UV_D67_PRED
        SMOOTH_PRED,   // UV_SMOOTH_PRED
        SMOOTH_V_PRED, // UV_SMOOTH_V_PRED
        SMOOTH_H_PRED, // UV_SMOOTH_H_PRED
        PAETH_PRED,    // UV_PAETH_PRED
        DC_PRED,       // UV_CFL_PRED
        INTRA_INVALID, // UV_INTRA_MODES
        INTRA_INVALID, // UV_MODE_INVALID
    ];
    UV2Y[mode as usize]
}

#[inline]
pub(crate) fn is_inter_block(mbmi: &MbModeInfo) -> bool {
    is_intrabc_block(mbmi) || mbmi.ref_frame[0] > INTRA_FRAME
}

#[inline]
pub(crate) fn has_second_ref(mbmi: &MbModeInfo) -> bool {
    mbmi.ref_frame[1] > INTRA_FRAME
}

/// Translation of `struct buf_2d`: `buf` and `buf0` are sample offsets
/// into the frame buffer the plane belongs to.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Buf2d {
    pub(crate) buf: usize,
    pub(crate) buf0: usize,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) stride: i32,
}

/// Translation of `MACROBLOCKD_PLANE`. The entropy context pointers are
/// offsets into `xd.above_contexts.entropy[plane][tile_row]` and
/// `xd.left_entropy_context[plane]`; the quantizer matrices are always the
/// flat ones (NULL) and are left out, as are the palette color map and the
/// inter prediction buffers.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MacroblockdPlane {
    pub(crate) plane_type: i32,
    pub(crate) subsampling_x: i32,
    pub(crate) subsampling_y: i32,
    pub(crate) dst: Buf2d,
    pub(crate) above_entropy_context: usize,
    pub(crate) left_entropy_context: usize,
    // The dequantizers below are true dequantizers used only in the
    // dequantization process.  They have the same coefficient
    // shift/scale as TX.
    pub(crate) seg_dequant_qtx: [[i16; 2]; MAX_SEGMENTS],
    // block size in pixels
    pub(crate) width: u8,
    pub(crate) height: u8,
}

/// Where `xd->tx_type_map` points: the frame's map (at the block's grid
/// offset) or the macroblock's own `txfm_search_info.tx_type_map_` (kept
/// here as `MacroBlockD::tx_type_map_blk`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TxTypeMapRef {
    Frame(usize),
    Block,
}

/// Translation of `MACROBLOCKD`, with the frame-wide state moved in while
/// tiles are encoded or packed (see the module documentation).
#[derive(Debug)]
pub(crate) struct MacroBlockD {
    /// Row position in mi units.
    pub(crate) mi_row: i32,
    /// Column position in mi units.
    pub(crate) mi_col: i32,
    pub(crate) mi_stride: i32,
    pub(crate) is_chroma_ref: bool,
    pub(crate) plane: [MacroblockdPlane; MAX_MB_PLANE],
    pub(crate) tile: TileInfo,
    /// `xd->mi`: the offset of the block's entry in the mode info grid.
    pub(crate) mi: usize,
    pub(crate) up_available: bool,
    pub(crate) left_available: bool,
    pub(crate) chroma_up_available: bool,
    pub(crate) chroma_left_available: bool,
    pub(crate) left_mbmi: Option<u32>,
    pub(crate) above_mbmi: Option<u32>,
    pub(crate) chroma_left_mbmi: Option<u32>,
    pub(crate) chroma_above_mbmi: Option<u32>,
    pub(crate) tx_type_map: TxTypeMapRef,
    pub(crate) tx_type_map_stride: i32,
    /// Distance from left edge
    pub(crate) mb_to_left_edge: i32,
    /// Distance from right edge
    pub(crate) mb_to_right_edge: i32,
    /// Distance from top edge
    pub(crate) mb_to_top_edge: i32,
    /// Distance from bottom edge
    pub(crate) mb_to_bottom_edge: i32,
    /// `xd->cur_buf->flags` (the source frame's; always high bit depth)
    pub(crate) cur_buf_flags: i32,
    /// The tile row of `xd->above_entropy_context[]`,
    /// `xd->above_partition_context` and `xd->above_txfm_context` (the
    /// pointers' offsets are 0, `above_txfm_context` below).
    pub(crate) above_contexts_tile_row: usize,
    pub(crate) left_entropy_context: [[EntropyContext; MAX_MIB_SIZE as usize]; MAX_MB_PLANE],
    pub(crate) left_partition_context: [PartitionContext; MAX_MIB_SIZE as usize],
    /// `xd->above_txfm_context`: offset into the tile row's txfm contexts.
    pub(crate) above_txfm_context: usize,
    /// `xd->left_txfm_context`: offset into `left_txfm_context_buffer`.
    pub(crate) left_txfm_context: usize,
    pub(crate) left_txfm_context_buffer: [TxfmContext; MAX_MIB_SIZE as usize],
    /// Block width in MB_MODE_INFO units
    pub(crate) width: u8,
    /// Block height in MB_MODE_INFO units
    pub(crate) height: u8,
    pub(crate) is_last_vertical_rect: bool,
    pub(crate) is_first_horizontal_rect: bool,
    pub(crate) neighbors_ref_counts: [u8; REF_FRAMES as usize],
    /// ref_mv_stack and weight are populated by av1_find_mv_refs(); they keep
    /// their values from one block to the next, as upstream's.
    pub(crate) ref_mv_stack:
        Box<[[CandidateMv; MAX_REF_MV_STACK_SIZE as usize]; MODE_CTX_REF_FRAMES as usize]>,
    pub(crate) weight: Box<[[u16; MAX_REF_MV_STACK_SIZE as usize]; MODE_CTX_REF_FRAMES as usize]>,
    /// `xd->tile_ctx`: the tile's frame context, moved in while the tile is
    /// encoded or packed.
    pub(crate) tile_ctx: Box<FrameContext>,
    pub(crate) bd: i32,
    pub(crate) qindex: [i32; MAX_SEGMENTS],
    pub(crate) lossless: [i32; MAX_SEGMENTS],
    pub(crate) current_base_qindex: i32,
    pub(crate) cur_frame_force_integer_mv: i32,
    pub(crate) delta_lf_from_base: i8,
    pub(crate) delta_lf: [i8; FRAME_LF_COUNT as usize],
    pub(crate) cdef_transmitted: [bool; 4],
    pub(crate) cfl: CflCtx,

    // (the frame-wide state moved in from `cm`)
    /// `cm->mi_params.mi_alloc`
    pub(crate) mi_alloc: Vec<MbModeInfo>,
    /// `cm->mi_params.mi_grid_base`
    pub(crate) mi_grid_base: Vec<u32>,
    /// `cm->cur_frame->mvs`
    pub(crate) cur_frame_mvs: Vec<MvRef>,
    /// `cm->mi_params.tx_type_map`
    pub(crate) tx_type_map_frame: Vec<u8>,
    /// The macroblock's `txfm_search_info.tx_type_map_`
    pub(crate) tx_type_map_blk: Vec<u8>,
    /// `cm->above_contexts`
    pub(crate) above_contexts: CommonContexts,
    /// `cm->cur_frame->buf`, the reconstructed frame
    pub(crate) recon: Yv12BufferConfig,
}

impl Default for MacroBlockD {
    fn default() -> Self {
        MacroBlockD {
            mi_row: 0,
            mi_col: 0,
            mi_stride: 0,
            is_chroma_ref: false,
            plane: [MacroblockdPlane::default(); MAX_MB_PLANE],
            tile: TileInfo::default(),
            mi: 0,
            up_available: false,
            left_available: false,
            chroma_up_available: false,
            chroma_left_available: false,
            left_mbmi: None,
            above_mbmi: None,
            chroma_left_mbmi: None,
            chroma_above_mbmi: None,
            tx_type_map: TxTypeMapRef::Frame(0),
            tx_type_map_stride: 0,
            mb_to_left_edge: 0,
            mb_to_right_edge: 0,
            mb_to_top_edge: 0,
            mb_to_bottom_edge: 0,
            cur_buf_flags: 0,
            above_contexts_tile_row: 0,
            left_entropy_context: [[0; MAX_MIB_SIZE as usize]; MAX_MB_PLANE],
            left_partition_context: [0; MAX_MIB_SIZE as usize],
            above_txfm_context: 0,
            left_txfm_context: 0,
            left_txfm_context_buffer: [0; MAX_MIB_SIZE as usize],
            width: 0,
            height: 0,
            is_last_vertical_rect: false,
            is_first_horizontal_rect: false,
            neighbors_ref_counts: [0; REF_FRAMES as usize],
            ref_mv_stack: Box::new(
                [[CandidateMv::default(); MAX_REF_MV_STACK_SIZE as usize];
                    MODE_CTX_REF_FRAMES as usize],
            ),
            weight: Box::new([[0; MAX_REF_MV_STACK_SIZE as usize]; MODE_CTX_REF_FRAMES as usize]),
            tile_ctx: Box::default(),
            bd: 0,
            qindex: [0; MAX_SEGMENTS],
            lossless: [0; MAX_SEGMENTS],
            current_base_qindex: 0,
            cur_frame_force_integer_mv: 0,
            delta_lf_from_base: 0,
            delta_lf: [0; FRAME_LF_COUNT as usize],
            cdef_transmitted: [false; 4],
            cfl: CflCtx::default(),
            mi_alloc: Vec::new(),
            mi_grid_base: Vec::new(),
            cur_frame_mvs: Vec::new(),
            tx_type_map_frame: Vec::new(),
            tx_type_map_blk: Vec::new(),
            above_contexts: CommonContexts::default(),
            recon: Yv12BufferConfig::default(),
        }
    }
}

impl MacroBlockD {
    /// The `MB_MODE_INFO` index of grid entry `xd->mi[off]`.
    #[inline]
    pub(crate) fn mi_idx(&self, off: isize) -> u32 {
        self.mi_grid_base[(self.mi as isize + off) as usize]
    }
    /// `xd->mi[0]`
    #[inline]
    pub(crate) fn mbmi(&self) -> &MbModeInfo {
        &self.mi_alloc[self.mi_grid_base[self.mi] as usize]
    }
    /// `xd->mi[0]`, mutably
    #[inline]
    pub(crate) fn mbmi_mut(&mut self) -> &mut MbModeInfo {
        let i = self.mi_grid_base[self.mi] as usize;
        &mut self.mi_alloc[i]
    }
    /// The mode info a neighbor index (`xd->above_mbmi` and the like) names.
    #[inline]
    pub(crate) fn mi_at(&self, idx: Option<u32>) -> Option<&MbModeInfo> {
        idx.map(|i| &self.mi_alloc[i as usize])
    }
    /// `xd->tx_type_map[i]`
    #[inline]
    pub(crate) fn tx_type_map_get(&self, i: usize) -> u8 {
        match self.tx_type_map {
            TxTypeMapRef::Frame(off) => self.tx_type_map_frame[off + i],
            TxTypeMapRef::Block => self.tx_type_map_blk[i],
        }
    }
    /// `xd->tx_type_map[i] = v`
    #[inline]
    pub(crate) fn tx_type_map_set(&mut self, i: usize, v: u8) {
        match self.tx_type_map {
            TxTypeMapRef::Frame(off) => self.tx_type_map_frame[off + i] = v,
            TxTypeMapRef::Block => self.tx_type_map_blk[i] = v,
        }
    }
    /// `xd->above_entropy_context[plane]` (the tile row's array)
    #[inline]
    pub(crate) fn above_entropy(&self, plane: usize) -> &[EntropyContext] {
        &self.above_contexts.entropy[plane][self.above_contexts_tile_row]
    }
    /// `xd->above_partition_context` (the tile row's array)
    #[inline]
    pub(crate) fn above_partition(&self) -> &[PartitionContext] {
        &self.above_contexts.partition[self.above_contexts_tile_row]
    }
    /// `xd->above_partition_context`, mutably
    #[inline]
    pub(crate) fn above_partition_mut(&mut self) -> &mut [PartitionContext] {
        &mut self.above_contexts.partition[self.above_contexts_tile_row]
    }
    /// The tile row's txfm contexts `xd->above_txfm_context` points into.
    #[inline]
    pub(crate) fn above_txfm(&self) -> &[TxfmContext] {
        &self.above_contexts.txfm[self.above_contexts_tile_row]
    }
}

#[inline]
pub(crate) fn is_cur_buf_hbd(xd: &MacroBlockD) -> bool {
    xd.cur_buf_flags & crate::aom::aom_scale::yv12config::YV12_FLAG_HIGHBITDEPTH != 0
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BitDepthInfo {
    pub(crate) bit_depth: i32,
    pub(crate) use_highbitdepth_buf: bool,
}

#[inline]
pub(crate) fn get_bit_depth_info(xd: &MacroBlockD) -> BitDepthInfo {
    BitDepthInfo {
        bit_depth: xd.bd,
        use_highbitdepth_buf: is_cur_buf_hbd(xd),
    }
}

#[inline]
pub(crate) fn get_sqr_bsize_idx(bsize: BlockSize) -> i32 {
    match bsize {
        BLOCK_4X4 => 0,
        BLOCK_8X8 => 1,
        BLOCK_16X16 => 2,
        BLOCK_32X32 => 3,
        BLOCK_64X64 => 4,
        BLOCK_128X128 => 5,
        _ => SQR_BLOCK_SIZES,
    }
}

// For a square block size 'bsize', returns the size of the sub-blocks used by
// the given partition type. If the partition produces sub-blocks of different
// sizes, then the function returns the largest sub-block size.
// Implements the Partition_Subsize lookup table in the spec (Section 9.3.
// Conversion tables).
// Note: the input block size should be square.
// Otherwise it's considered invalid.
#[inline]
pub(crate) fn get_partition_subsize(bsize: BlockSize, partition: PartitionType) -> BlockSize {
    if partition == PARTITION_INVALID {
        BLOCK_INVALID
    } else {
        let sqr_bsize_idx = get_sqr_bsize_idx(bsize);
        if sqr_bsize_idx >= SQR_BLOCK_SIZES {
            BLOCK_INVALID
        } else {
            SUBSIZE_LOOKUP[partition as usize][sqr_bsize_idx as usize]
        }
    }
}

pub(crate) fn intra_mode_to_tx_type(mbmi: &MbModeInfo, plane_type: i32) -> TxType {
    static INTRA_MODE_TO_TX_TYPE: [TxType; INTRA_MODES as usize] = [
        DCT_DCT,   // DC_PRED
        ADST_DCT,  // V_PRED
        DCT_ADST,  // H_PRED
        DCT_DCT,   // D45_PRED
        ADST_ADST, // D135_PRED
        ADST_DCT,  // D113_PRED
        DCT_ADST,  // D157_PRED
        DCT_ADST,  // D203_PRED
        ADST_DCT,  // D67_PRED
        ADST_ADST, // SMOOTH_PRED
        ADST_DCT,  // SMOOTH_V_PRED
        DCT_ADST,  // SMOOTH_H_PRED
        ADST_ADST, // PAETH_PRED
    ];
    let mode = if plane_type == PLANE_TYPE_Y {
        mbmi.mode
    } else {
        get_uv_mode(mbmi.uv_mode)
    };
    debug_assert!(mode < INTRA_MODES);
    INTRA_MODE_TO_TX_TYPE[mode as usize]
}

#[inline]
pub(crate) fn is_rect_tx(tx_size: TxSize) -> bool {
    tx_size >= TX_SIZES
}

#[inline]
pub(crate) fn block_signals_txsize(bsize: BlockSize) -> bool {
    bsize > BLOCK_4X4
}

// Number of transform types in each set type
pub(crate) static AV1_NUM_EXT_TX_SET: [i32; EXT_TX_SET_TYPES as usize] = [1, 2, 5, 7, 12, 16];

pub(crate) static AV1_EXT_TX_USED: [[i32; TX_TYPES as usize]; EXT_TX_SET_TYPES as usize] = [
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
    [1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
    [1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
];

pub(crate) static AV1_EXT_TX_SET_LOOKUP: [[TxSetType; 2]; 2] = [
    [EXT_TX_SET_DTT4_IDTX_1DDCT, EXT_TX_SET_DTT4_IDTX],
    [EXT_TX_SET_ALL16, EXT_TX_SET_DTT9_IDTX_1DDCT],
];

#[inline]
pub(crate) fn av1_get_ext_tx_set_type(
    tx_size: TxSize,
    is_inter: i32,
    use_reduced_set: i32,
) -> TxSetType {
    let tx_size_sqr_up = TXSIZE_SQR_UP_MAP[tx_size as usize];
    if tx_size_sqr_up > TX_32X32 {
        return EXT_TX_SET_DCTONLY;
    }
    if tx_size_sqr_up == TX_32X32 {
        return if is_inter != 0 {
            EXT_TX_SET_DCT_IDTX
        } else {
            EXT_TX_SET_DCTONLY
        };
    }
    if use_reduced_set != 0 {
        return if is_inter != 0 {
            EXT_TX_SET_DCT_IDTX
        } else {
            EXT_TX_SET_DTT4_IDTX
        };
    }
    let tx_size_sqr = TXSIZE_SQR_MAP[tx_size as usize];
    AV1_EXT_TX_SET_LOOKUP[is_inter as usize][(tx_size_sqr == TX_16X16) as usize]
}

// Maps tx set types to the indices.
static EXT_TX_SET_INDEX: [[i32; EXT_TX_SET_TYPES as usize]; 2] = [
    // Intra
    [0, -1, 2, 1, -1, -1],
    // Inter
    [0, 3, -1, -1, 2, 1],
];

#[inline]
pub(crate) fn get_ext_tx_set(tx_size: TxSize, is_inter: i32, use_reduced_set: i32) -> i32 {
    let set_type = av1_get_ext_tx_set_type(tx_size, is_inter, use_reduced_set);
    EXT_TX_SET_INDEX[is_inter as usize][set_type as usize]
}

#[inline]
pub(crate) fn get_ext_tx_types(tx_size: TxSize, is_inter: i32, use_reduced_set: i32) -> i32 {
    let set_type = av1_get_ext_tx_set_type(tx_size, is_inter, use_reduced_set);
    AV1_NUM_EXT_TX_SET[set_type as usize]
}

#[inline]
pub(crate) fn tx_size_from_tx_mode(bsize: BlockSize, tx_mode: TxMode) -> TxSize {
    let largest_tx_size = TX_MODE_TO_BIGGEST_TX_SIZE[tx_mode as usize];
    let max_rect_tx_size = MAX_TXSIZE_RECT_LOOKUP[bsize as usize];
    if bsize == BLOCK_4X4 {
        return MAX_TXSIZE_LOOKUP[bsize as usize].min(largest_tx_size);
    }
    if TXSIZE_SQR_MAP[max_rect_tx_size as usize] <= largest_tx_size {
        max_rect_tx_size
    } else {
        largest_tx_size
    }
}

pub(crate) static MODE_TO_ANGLE_MAP: [u8; 13] =
    [0, 90, 180, 45, 135, 113, 157, 203, 67, 0, 0, 0, 0];

// Implements the get_plane_residual_size() function in the spec (Section
// 5.11.38. Get plane residual size function).
#[inline]
pub(crate) fn get_plane_block_size(
    bsize: BlockSize,
    subsampling_x: i32,
    subsampling_y: i32,
) -> BlockSize {
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    debug_assert!((0..2).contains(&subsampling_x));
    debug_assert!((0..2).contains(&subsampling_y));
    AV1_SS_SIZE_LOOKUP[bsize as usize][subsampling_x as usize][subsampling_y as usize]
}

/// Translation of `update_txk_array()`.
#[inline]
pub(crate) fn update_txk_array(
    xd: &mut MacroBlockD,
    blk_row: i32,
    blk_col: i32,
    tx_size: TxSize,
    tx_type: TxType,
) {
    let stride = xd.tx_type_map_stride;
    xd.tx_type_map_set((blk_row * stride + blk_col) as usize, tx_type as u8);

    let txw = TX_SIZE_WIDE_UNIT[tx_size as usize];
    let txh = TX_SIZE_HIGH_UNIT[tx_size as usize];
    // The 16x16 unit is due to the constraint from tx_64x64 which sets the
    // maximum tx size for chroma as 32x32. Coupled with 4x1 transform block
    // size, the constraint takes effect in 32x16 / 16x32 size too. To solve
    // the intricacy, cover all the 16x16 units inside a 64 level transform.
    if txw == TX_SIZE_WIDE_UNIT[TX_64X64 as usize] || txh == TX_SIZE_HIGH_UNIT[TX_64X64 as usize] {
        let tx_unit = TX_SIZE_WIDE_UNIT[TX_16X16 as usize];
        let mut idy = 0;
        while idy < txh {
            let mut idx = 0;
            while idx < txw {
                xd.tx_type_map_set(
                    ((blk_row + idy) * stride + blk_col + idx) as usize,
                    tx_type as u8,
                );
                idx += tx_unit;
            }
            idy += tx_unit;
        }
    }
}

pub(crate) fn av1_get_tx_type(
    xd: &MacroBlockD,
    plane_type: i32,
    blk_row: i32,
    blk_col: i32,
    tx_size: TxSize,
    reduced_tx_set: i32,
) -> TxType {
    let mbmi = xd.mbmi();
    if xd.lossless[mbmi.segment_id as usize] != 0 || TXSIZE_SQR_UP_MAP[tx_size as usize] > TX_32X32
    {
        return DCT_DCT;
    }

    let mut tx_type: TxType;
    if plane_type == PLANE_TYPE_Y {
        tx_type =
            xd.tx_type_map_get((blk_row * xd.tx_type_map_stride + blk_col) as usize) as TxType;
    } else {
        // (the saver's blocks are all intra)
        debug_assert!(!is_inter_block(mbmi));
        tx_type = intra_mode_to_tx_type(mbmi, PLANE_TYPE_UV);
        let tx_set_type =
            av1_get_ext_tx_set_type(tx_size, is_inter_block(mbmi) as i32, reduced_tx_set);
        if AV1_EXT_TX_USED[tx_set_type as usize][tx_type as usize] == 0 {
            tx_type = DCT_DCT;
        }
    }
    debug_assert!(tx_type < TX_TYPES);
    tx_type
}

/*
 * Logic to generate the lookup tables:
 *
 * TX_SIZE tx_size = max_txsize_rect_lookup[bsize];
 * for (int depth = 0; depth < MAX_TX_DEPTH; ++depth) {
 *   tx_size = sub_tx_size_map[tx_size];
 * }
 */
#[inline]
pub(crate) fn bsize_to_max_depth(bsize: BlockSize) -> i32 {
    static BSIZE_TO_MAX_DEPTH_TABLE: [u8; BLOCK_SIZES_ALL as usize] = [
        0, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    ];
    BSIZE_TO_MAX_DEPTH_TABLE[bsize as usize] as i32
}

/*
 * Logic to generate the lookup table:
 *
 * TX_SIZE tx_size = max_txsize_rect_lookup[bsize];
 * assert(tx_size != TX_4X4);
 * int depth = 0;
 * while (tx_size != TX_4X4) {
 *   depth++;
 *   tx_size = sub_tx_size_map[tx_size];
 * }
 * assert(depth < 10);
 */
#[inline]
pub(crate) fn bsize_to_tx_size_cat(bsize: BlockSize) -> i32 {
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    static BSIZE_TO_TX_SIZE_DEPTH_TABLE: [u8; BLOCK_SIZES_ALL as usize] = [
        0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4, 4, 4, 4, 2, 2, 3, 3, 4, 4,
    ];
    let depth = BSIZE_TO_TX_SIZE_DEPTH_TABLE[bsize as usize] as i32;
    debug_assert!(depth <= MAX_TX_CATS);
    depth - 1
}

#[inline]
pub(crate) fn depth_to_tx_size(depth: i32, bsize: BlockSize) -> TxSize {
    let max_rect_tx_size = MAX_TXSIZE_RECT_LOOKUP[bsize as usize];
    let mut tx_size = max_rect_tx_size;
    for _d in 0..depth {
        tx_size = SUB_TX_SIZE_MAP[tx_size as usize];
    }
    tx_size
}

#[inline]
pub(crate) fn av1_get_adjusted_tx_size(tx_size: TxSize) -> TxSize {
    match tx_size {
        TX_64X64 | TX_64X32 | TX_32X64 => TX_32X32,
        TX_64X16 => TX_32X16,
        TX_16X64 => TX_16X32,
        _ => tx_size,
    }
}

#[inline]
pub(crate) fn av1_get_max_uv_txsize(
    bsize: BlockSize,
    subsampling_x: i32,
    subsampling_y: i32,
) -> TxSize {
    let plane_bsize = get_plane_block_size(bsize, subsampling_x, subsampling_y);
    debug_assert!(plane_bsize < BLOCK_SIZES_ALL);
    let uv_tx = MAX_TXSIZE_RECT_LOOKUP[plane_bsize as usize];
    av1_get_adjusted_tx_size(uv_tx)
}

#[inline]
pub(crate) fn av1_get_tx_size(plane: usize, xd: &MacroBlockD) -> TxSize {
    let mbmi = xd.mbmi();
    if xd.lossless[mbmi.segment_id as usize] != 0 {
        return TX_4X4;
    }
    if plane == 0 {
        return mbmi.tx_size;
    }
    let pd = &xd.plane[plane];
    av1_get_max_uv_txsize(mbmi.bsize, pd.subsampling_x, pd.subsampling_y)
}

#[inline]
pub(crate) fn av1_allow_palette(allow_screen_content_tools: bool, sb_type: BlockSize) -> bool {
    debug_assert!(sb_type < BLOCK_SIZES_ALL);
    allow_screen_content_tools
        && BLOCK_SIZE_WIDE[sb_type as usize] as i32 <= MAX_PALETTE_BLOCK_WIDTH
        && BLOCK_SIZE_HIGH[sb_type as usize] as i32 <= MAX_PALETTE_BLOCK_HEIGHT
        && sb_type >= BLOCK_8X8
}

#[inline]
pub(crate) fn av1_use_angle_delta(bsize: BlockSize) -> bool {
    bsize >= BLOCK_8X8
}

#[inline]
pub(crate) fn get_plane_type(plane: usize) -> i32 {
    if plane == 0 {
        PLANE_TYPE_Y
    } else {
        PLANE_TYPE_UV
    }
}

#[inline]
pub(crate) fn av1_get_max_eob(tx_size: TxSize) -> i32 {
    if tx_size == TX_64X64 || tx_size == TX_64X32 || tx_size == TX_32X64 {
        return 1024;
    }
    if tx_size == TX_16X64 || tx_size == TX_64X16 {
        return 512;
    }
    TX_SIZE_2D[tx_size as usize]
}

#[inline]
pub(crate) fn is_2d_transform(tx_type: TxType) -> bool {
    tx_type < IDTX
}

// (blockd.c)

pub(crate) fn av1_left_block_mode(left_mi: Option<&MbModeInfo>) -> PredictionMode {
    match left_mi {
        None => DC_PRED,
        Some(left_mi) => {
            debug_assert!(!is_inter_block(left_mi) || is_intrabc_block(left_mi));
            left_mi.mode
        }
    }
}

pub(crate) fn av1_above_block_mode(above_mi: Option<&MbModeInfo>) -> PredictionMode {
    match above_mi {
        None => DC_PRED,
        Some(above_mi) => {
            debug_assert!(!is_inter_block(above_mi) || is_intrabc_block(above_mi));
            above_mi.mode
        }
    }
}

pub(crate) fn av1_reset_entropy_context(xd: &mut MacroBlockD, bsize: BlockSize, num_planes: usize) {
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let nplanes = 1 + (num_planes - 1) * xd.is_chroma_ref as usize;
    let tile_row = xd.above_contexts_tile_row;
    for i in 0..nplanes {
        let pd = xd.plane[i];
        let plane_bsize = get_plane_block_size(bsize, pd.subsampling_x, pd.subsampling_y);
        let txs_wide = MI_SIZE_WIDE[plane_bsize as usize] as usize;
        let txs_high = MI_SIZE_HIGH[plane_bsize as usize] as usize;
        let a = pd.above_entropy_context;
        xd.above_contexts.entropy[i][tile_row][a..a + txs_wide].fill(0);
        let l = pd.left_entropy_context;
        xd.left_entropy_context[i][l..l + txs_high].fill(0);
    }
}

pub(crate) fn av1_reset_loop_filter_delta(xd: &mut MacroBlockD, num_planes: usize) {
    xd.delta_lf_from_base = 0;
    let frame_lf_count = if num_planes > 1 {
        FRAME_LF_COUNT as usize
    } else {
        FRAME_LF_COUNT as usize - 2
    };
    for lf_id in 0..frame_lf_count {
        xd.delta_lf[lf_id] = 0;
    }
}

/// Translation of `max_block_wide()` (av1_common_int.h).
#[inline]
pub(crate) fn max_block_wide(xd: &MacroBlockD, bsize: BlockSize, plane: usize) -> i32 {
    debug_assert!(bsize < BLOCK_SIZES_ALL);
    let mut max_blocks_wide = BLOCK_SIZE_WIDE[bsize as usize] as i32;

    if xd.mb_to_right_edge < 0 {
        let pd = &xd.plane[plane];
        max_blocks_wide += xd.mb_to_right_edge >> (3 + pd.subsampling_x);
    }

    // Scale the width in the transform block unit.
    max_blocks_wide >> MI_SIZE_LOG2
}

/// Translation of `max_block_high()` (av1_common_int.h).
#[inline]
pub(crate) fn max_block_high(xd: &MacroBlockD, bsize: BlockSize, plane: usize) -> i32 {
    let mut max_blocks_high = BLOCK_SIZE_HIGH[bsize as usize] as i32;

    if xd.mb_to_bottom_edge < 0 {
        let pd = &xd.plane[plane];
        max_blocks_high += xd.mb_to_bottom_edge >> (3 + pd.subsampling_y);
    }

    // Scale the height in the transform block unit.
    max_blocks_high >> MI_SIZE_LOG2
}

pub(crate) fn av1_set_entropy_contexts(
    xd: &mut MacroBlockD,
    plane: usize,
    plane_bsize: BlockSize,
    tx_size: TxSize,
    has_eob: i32,
    aoff: i32,
    loff: i32,
) {
    let a = xd.plane[plane].above_entropy_context + aoff as usize;
    let l = xd.plane[plane].left_entropy_context + loff as usize;
    let txs_wide = TX_SIZE_WIDE_UNIT[tx_size as usize];
    let txs_high = TX_SIZE_HIGH_UNIT[tx_size as usize];
    let tile_row = xd.above_contexts_tile_row;

    // above
    if has_eob != 0 && xd.mb_to_right_edge < 0 {
        let blocks_wide = max_block_wide(xd, plane_bsize, plane);
        let above_contexts = (blocks_wide - aoff).clamp(0, txs_wide);
        let above = &mut xd.above_contexts.entropy[plane][tile_row];
        above[a..a + above_contexts as usize].fill(has_eob as EntropyContext);
        above[a + above_contexts as usize..a + txs_wide as usize].fill(0);
    } else {
        xd.above_contexts.entropy[plane][tile_row][a..a + txs_wide as usize]
            .fill(has_eob as EntropyContext);
    }

    // left
    if has_eob != 0 && xd.mb_to_bottom_edge < 0 {
        let blocks_high = max_block_high(xd, plane_bsize, plane);
        let left_contexts = (blocks_high - loff).clamp(0, txs_high);
        let left = &mut xd.left_entropy_context[plane];
        left[l..l + left_contexts as usize].fill(has_eob as EntropyContext);
        left[l + left_contexts as usize..l + txs_high as usize].fill(0);
    } else {
        xd.left_entropy_context[plane][l..l + txs_high as usize].fill(has_eob as EntropyContext);
    }
}

pub(crate) fn av1_setup_block_planes(
    xd: &mut MacroBlockD,
    ss_x: i32,
    ss_y: i32,
    num_planes: usize,
) {
    for i in 0..num_planes {
        xd.plane[i].plane_type = get_plane_type(i);
        xd.plane[i].subsampling_x = if i != 0 { ss_x } else { 0 };
        xd.plane[i].subsampling_y = if i != 0 { ss_y } else { 0 };
    }
    for i in num_planes..MAX_MB_PLANE {
        xd.plane[i].subsampling_x = 1;
        xd.plane[i].subsampling_y = 1;
    }
}
