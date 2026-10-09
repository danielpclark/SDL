// Rust translation of src/internal.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The decoder, frame, tile and task contexts.
//!
//! This translation decodes one frame at a time on the calling thread
//! (dav1d with `n_threads = 1` and `max_frame_delay = 1`): there is one
//! frame context and one task context, the frame and task threading state
//! (task queues, progress counters, two-pass `frame_thread` buffers and
//! the `lowest_pixel` tracking) is gone, and the DSP function tables are
//! direct calls of the plain C functions' translations.
//!
//! Pointers into frame-wide arrays are indices; the pixel buffers whose
//! type depends on the bit depth are kept per pixel type ([`FramePx`],
//! [`TaskPx`]), only the one in use being allocated.

use std::sync::Arc;

use super::cdf::{cdf_context_new, CdfContext, CdfThreadContext};
use super::data::{Dav1dData, Dav1dDataProps};
use super::env::BlockContext;
use super::headers::{
    Dav1dContentLightLevel, Dav1dFrameHeader, Dav1dITUTT35, Dav1dMasteringDisplay,
    Dav1dSequenceHeader, Dav1dWarpedMotionParams,
};
use super::levels::N_RECT_TX_SIZES;
use super::lf_mask::{Av1Filter, Av1FilterLUT, Av1Restoration};
use super::msac::MsacContext;
use super::picture::{Dav1dPicture, Dav1dThreadPicture, PictureData};
use super::refmvs::{RefmvsFrame, RefmvsTemporalBlock, RefmvsTile};

/// Translation of `struct Dav1dTileGroup`.
#[derive(Clone, Default)]
pub(crate) struct Dav1dTileGroup {
    pub(crate) data: Dav1dData,
    pub(crate) start: i32,
    pub(crate) end: i32,
}

/// `enum Dav1dInloopFilterType`
pub(crate) const DAV1D_INLOOPFILTER_NONE: u32 = 0;
pub(crate) const DAV1D_INLOOPFILTER_DEBLOCK: u32 = 1;
pub(crate) const DAV1D_INLOOPFILTER_CDEF: u32 = 2;
pub(crate) const DAV1D_INLOOPFILTER_RESTORATION: u32 = 4;
pub(crate) const DAV1D_INLOOPFILTER_ALL: u32 =
    DAV1D_INLOOPFILTER_DEBLOCK | DAV1D_INLOOPFILTER_CDEF | DAV1D_INLOOPFILTER_RESTORATION;

/// `enum Dav1dDecodeFrameType`
pub(crate) const DAV1D_DECODEFRAMETYPE_ALL: u32 = 0;
pub(crate) const DAV1D_DECODEFRAMETYPE_REFERENCE: u32 = 1;
pub(crate) const DAV1D_DECODEFRAMETYPE_INTRA: u32 = 2;
pub(crate) const DAV1D_DECODEFRAMETYPE_KEY: u32 = 3;

/// The logger callback (`Dav1dLogger`); `None` is "no logging".
pub(crate) type Dav1dLogger = Option<Arc<dyn Fn(&str) + Send + Sync>>;

/// One reference slot (`c->refs[]`).
#[derive(Clone, Default)]
pub(crate) struct Dav1dRefSlot {
    pub(crate) p: Dav1dThreadPicture,
    pub(crate) segmap: Option<Arc<Vec<u8>>>,
    pub(crate) refmvs: Option<Arc<Vec<RefmvsTemporalBlock>>>,
    pub(crate) refpoc: [u32; 7],
}

/// Translation of `struct Dav1dContext`.
pub(crate) struct Dav1dContext {
    pub(crate) fc: Box<Dav1dFrameContext>,
    pub(crate) tc: Box<Dav1dTaskContext>,

    // cache of OBUs that make up a single frame before we submit them
    // to a frame worker to be decoded
    pub(crate) tile: Vec<Dav1dTileGroup>,
    pub(crate) n_tile_data: i32,
    pub(crate) n_tiles: i32,
    pub(crate) seq_hdr: Option<Arc<Dav1dSequenceHeader>>,
    pub(crate) frame_hdr: Option<Arc<Dav1dFrameHeader>>,

    pub(crate) content_light: Option<Arc<Dav1dContentLightLevel>>,
    pub(crate) mastering_display: Option<Arc<Dav1dMasteringDisplay>>,
    pub(crate) itut_t35: Option<Arc<Vec<Dav1dITUTT35>>>,

    // decoded output picture queue
    pub(crate) in_: Dav1dData,
    pub(crate) out: Dav1dThreadPicture,
    pub(crate) cache: Dav1dThreadPicture,

    // reference/entropy state
    pub(crate) refs: [Dav1dRefSlot; 8],
    pub(crate) cdf: [CdfThreadContext; 8],

    pub(crate) apply_grain: bool,
    pub(crate) operating_point: i32,
    pub(crate) operating_point_idc: u32,
    pub(crate) all_layers: bool,
    pub(crate) max_spatial_id: i32,
    pub(crate) frame_size_limit: u32,
    pub(crate) strict_std_compliance: bool,
    pub(crate) output_invisible_frames: bool,
    pub(crate) inloop_filters: u32,
    pub(crate) decode_frame_type: u32,
    pub(crate) drain: bool,
    pub(crate) frame_flags: u32,
    pub(crate) event_flags: u32,
    pub(crate) cached_error_props: Dav1dDataProps,
    pub(crate) cached_error: i32,

    pub(crate) logger: Dav1dLogger,
}

/// `struct ScalableMotionParams`
#[derive(Clone, Copy, Default)]
pub(crate) struct ScalableMotionParams {
    pub(crate) scale: i32, // if no scaling, this is 0
    pub(crate) step: i32,
}

/// The frame's pixel-typed line buffers (`f->ipred_edge`,
/// `f->lf.cdef_line`, `f->lf.lr_lpf_line`).
#[derive(Default)]
pub(crate) struct FramePixBufs<P> {
    pub(crate) ipred_edge: [Vec<P>; 3],
    /// `[pre, post][plane]`, two lines of the plane's stride each.
    pub(crate) cdef_line: [[Vec<P>; 3]; 2],
    /// 12 lines of the (upscaled) plane's stride each.
    pub(crate) lr_lpf_line: [Vec<P>; 3],
}

/// [`FramePixBufs`] for both pixel types.
#[derive(Default)]
pub(crate) struct FramePx {
    pub(crate) b8: FramePixBufs<u8>,
    pub(crate) b16: FramePixBufs<u16>,
}

/// The `lf` member of `struct Dav1dFrameContext`.
#[derive(Default)]
pub(crate) struct FrameLf {
    pub(crate) level: Vec<[u8; 4]>,
    pub(crate) mask: Vec<Av1Filter>,
    pub(crate) lr_mask: Vec<Av1Restoration>,
    pub(crate) mask_sz: i32, /* w*h */
    pub(crate) lr_mask_sz: i32,
    pub(crate) cdef_buf_plane_sz: [isize; 2], /* stride*sbh*4 */
    pub(crate) cdef_buf_sbh: i32,
    pub(crate) lr_buf_plane_sz: [isize; 2], /* stride*4 */
    pub(crate) re_sz: i32,                  /* h */
    pub(crate) lim_lut: Av1FilterLUT,
    pub(crate) last_sharpness: i32,
    pub(crate) lvl: [[[[u8; 2]; 8]; 4]; 8], /* [seg_id][dir][ref][is_gmv] */
    pub(crate) tx_lpf_right_edge: [Vec<u8>; 2],

    // in-loop filter per-frame state keeping
    pub(crate) start_of_tile_row: Vec<u8>,
    pub(crate) restore_planes: i32, // enum LrRestorePlanes
}

/// Translation of `struct Dav1dFrameContext`.
pub(crate) struct Dav1dFrameContext {
    pub(crate) seq_hdr: Option<Arc<Dav1dSequenceHeader>>,
    pub(crate) frame_hdr: Option<Arc<Dav1dFrameHeader>>,
    pub(crate) refp: [Dav1dThreadPicture; 7],
    /// during block coding / reconstruction (its pixels are `cur_px`)
    pub(crate) cur: Dav1dPicture,
    /// after super-resolution upscaling (its pixels are `sr_px` with
    /// super-resolution, `cur_px` without)
    pub(crate) sr_cur: Dav1dThreadPicture,
    pub(crate) cur_px: Option<PictureData>,
    pub(crate) sr_px: Option<PictureData>,
    pub(crate) cur_segmap: Option<Arc<Vec<u8>>>,
    pub(crate) prev_segmap: Option<Arc<Vec<u8>>>,
    pub(crate) refpoc: [u32; 7],
    pub(crate) refrefpoc: [[u32; 7]; 7],
    pub(crate) gmv_warp_allowed: [u8; 7],
    pub(crate) in_cdf: CdfThreadContext,
    pub(crate) out_cdf: Option<Box<CdfContext>>,
    pub(crate) tile: Vec<Dav1dTileGroup>,
    pub(crate) n_tile_data: i32,

    // for scalable references
    pub(crate) svc: [[ScalableMotionParams; 2]; 7],
    pub(crate) resize_step: [i32; 2],
    pub(crate) resize_start: [i32; 2],

    pub(crate) ts: Vec<Dav1dTileState>,
    pub(crate) n_ts: i32,

    pub(crate) ipred_edge_sz: i32,
    pub(crate) px: FramePx,
    pub(crate) b4_stride: usize,
    pub(crate) w4: i32,
    pub(crate) h4: i32,
    pub(crate) bw: i32,
    pub(crate) bh: i32,
    pub(crate) sb128w: i32,
    pub(crate) sb128h: i32,
    pub(crate) sbh: i32,
    pub(crate) sb_shift: i32,
    pub(crate) sb_step: i32,
    pub(crate) sr_sb128w: i32,
    pub(crate) dq: [[[u16; 2]; 3]; 8],
    pub(crate) qm: [[Option<&'static [u8]>; 3]; N_RECT_TX_SIZES],
    pub(crate) a: Vec<BlockContext>,
    pub(crate) a_sz: i32, /* w*tile_rows */
    pub(crate) rf: RefmvsFrame,
    pub(crate) jnt_weights: [[u8; 7]; 7],
    pub(crate) bitdepth_max: i32,

    /// `f->task_thread.update_set`: whether we need to update CDF reference
    pub(crate) update_set: bool,

    // loopfilter
    pub(crate) lf: FrameLf,
}

impl Default for Dav1dFrameContext {
    fn default() -> Self {
        Dav1dFrameContext {
            seq_hdr: None,
            frame_hdr: None,
            refp: Default::default(),
            cur: Default::default(),
            sr_cur: Default::default(),
            cur_px: None,
            sr_px: None,
            cur_segmap: None,
            prev_segmap: None,
            refpoc: [0; 7],
            refrefpoc: [[0; 7]; 7],
            gmv_warp_allowed: [0; 7],
            in_cdf: Default::default(),
            out_cdf: None,
            tile: Vec::new(),
            n_tile_data: 0,
            svc: [[ScalableMotionParams::default(); 2]; 7],
            resize_step: [0; 2],
            resize_start: [0; 2],
            ts: Vec::new(),
            n_ts: 0,
            ipred_edge_sz: 0,
            px: Default::default(),
            b4_stride: 0,
            w4: 0,
            h4: 0,
            bw: 0,
            bh: 0,
            sb128w: 0,
            sb128h: 0,
            sbh: 0,
            sb_shift: 0,
            sb_step: 0,
            sr_sb128w: 0,
            dq: [[[0; 2]; 3]; 8],
            qm: [[None; 3]; N_RECT_TX_SIZES],
            a: Vec::new(),
            a_sz: 0,
            rf: Default::default(),
            jnt_weights: [[0; 7]; 7],
            bitdepth_max: 0,
            update_set: false,
            lf: FrameLf {
                last_sharpness: -1,
                ..Default::default()
            },
        }
    }
}

impl Dav1dFrameContext {
    /// `f->frame_hdr` (set for the whole decode of a frame).
    #[inline]
    pub(crate) fn frame_hdr(&self) -> &Dav1dFrameHeader {
        self.frame_hdr.as_deref().expect("frame header")
    }

    /// `f->seq_hdr` (set for the whole decode of a frame).
    #[inline]
    pub(crate) fn seq_hdr(&self) -> &Dav1dSequenceHeader {
        self.seq_hdr.as_deref().expect("sequence header")
    }
}

/// `ts->dq`: the frame's or the tile's (`dqmem`) dequantizers.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TileDq {
    #[default]
    Frame,
    Tile,
}

/// `ts->lflvl`: the frame's or the tile's (`lflvlmem`) filter levels.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TileLflvl {
    #[default]
    Frame,
    Tile,
}

/// The `tiling` member of `struct Dav1dTileState`.
#[derive(Clone, Copy, Default)]
pub(crate) struct TileStateTiling {
    // in 4px units
    pub(crate) col_start: i32,
    pub(crate) col_end: i32,
    pub(crate) row_start: i32,
    pub(crate) row_end: i32,
    // in tile units
    pub(crate) col: i32,
    pub(crate) row: i32,
}

/// Translation of `struct Dav1dTileState`.
pub(crate) struct Dav1dTileState {
    pub(crate) cdf: Box<CdfContext>,
    pub(crate) msac: MsacContext,

    pub(crate) tiling: TileStateTiling,

    pub(crate) dqmem: [[[u16; 2]; 3]; 8],
    pub(crate) dq: TileDq,
    pub(crate) last_qidx: i32,

    pub(crate) last_delta_lf: [i8; 4],
    pub(crate) lflvlmem: [[[[u8; 2]; 8]; 4]; 8],
    pub(crate) lflvl: TileLflvl,

    /// `lr_ref[plane]`: the restoration unit `lf.lr_mask[.0].lr[plane][.1]`.
    pub(crate) lr_ref: [Option<(usize, usize)>; 3],
}

impl Default for Dav1dTileState {
    fn default() -> Self {
        Dav1dTileState {
            cdf: cdf_context_new(),
            msac: MsacContext::default(),
            tiling: TileStateTiling::default(),
            dqmem: [[[0; 2]; 3]; 8],
            dq: TileDq::Frame,
            last_qidx: 0,
            last_delta_lf: [0; 4],
            lflvlmem: [[[[0; 2]; 8]; 4]; 8],
            lflvl: TileLflvl::Frame,
            lr_ref: [None; 3],
        }
    }
}

/// The task context's pixel-typed scratch buffers (`t->scratch.lap`,
/// `.emu_edge`, `.interintra`, `.edge`).
pub(crate) struct TaskPixScratch<P> {
    pub(crate) lap: Vec<P>,
    // stride=192 for non-SVC, or 320 for SVC
    pub(crate) emu_edge: Vec<P>,
    pub(crate) interintra: Vec<P>,
    pub(crate) edge: Vec<P>,
}

impl<P> Default for TaskPixScratch<P> {
    fn default() -> Self {
        TaskPixScratch {
            lap: Vec::new(),
            emu_edge: Vec::new(),
            interintra: Vec::new(),
            edge: Vec::new(),
        }
    }
}

impl<P: Copy + Default> TaskPixScratch<P> {
    /// Allocates the buffers on first use.
    pub(crate) fn ensure(&mut self) {
        if self.edge.is_empty() {
            self.lap = vec![P::default(); 128 * 32];
            self.emu_edge = vec![P::default(); 320 * (256 + 7)];
            self.interintra = vec![P::default(); 64 * 64];
            self.edge = vec![P::default(); 257];
        }
    }
}

/// [`TaskPixScratch`] for both pixel types.
#[derive(Default)]
pub(crate) struct TaskPx {
    pub(crate) b8: TaskPixScratch<u8>,
    pub(crate) b16: TaskPixScratch<u16>,
}

/// The non-pixel members of the task context's `scratch` union (kept
/// apart here; no member is read through another).
pub(crate) struct TaskScratch {
    pub(crate) compinter: [Vec<i16>; 2],
    pub(crate) seg_mask: Vec<u8>,
    pub(crate) levels: [u8; 32 * 34],
    pub(crate) pal_order: [[u8; 8]; 64],
    pub(crate) pal_ctx: [u8; 64],
    pub(crate) ac: Vec<i16>,
    pub(crate) pal_idx: Vec<u8>,
    pub(crate) pal: [[u16; 8]; 3],
}

impl Default for TaskScratch {
    fn default() -> Self {
        TaskScratch {
            compinter: [vec![0; 128 * 128], vec![0; 128 * 128]],
            seg_mask: vec![0; 128 * 128],
            levels: [0; 32 * 34],
            pal_order: [[0; 8]; 64],
            pal_ctx: [0; 64],
            ac: vec![0; 32 * 32],
            pal_idx: vec![0; 2 * 64 * 64],
            pal: [[0; 8]; 3],
        }
    }
}

/// Translation of `struct Dav1dTaskContext`.
pub(crate) struct Dav1dTaskContext {
    pub(crate) bx: i32,
    pub(crate) by: i32,
    pub(crate) l: BlockContext,
    /// `t->a`: the index in `f->a`
    pub(crate) a: usize,
    pub(crate) rt: RefmvsTile,
    pub(crate) cf: Vec<i32>,
    // FIXME types can be changed to pixel (and dynamically allocated)
    // which would make copy/assign operations slightly faster?
    pub(crate) al_pal: [[[[u16; 8]; 3]; 32]; 2],
    pub(crate) pal_sz_uv: [[u8; 32]; 2],
    pub(crate) txtp_map: [u8; 32 * 32], // inter-only
    pub(crate) scratch: TaskScratch,
    pub(crate) px: TaskPx,

    pub(crate) warpmv: Dav1dWarpedMotionParams,
    /// `t->lf_mask`: the index in `f->lf.mask`
    pub(crate) lf_mask: usize,
    pub(crate) top_pre_cdef_toggle: i32,
    /// `t->cur_sb_cdef_idx_ptr`: the index in `f->lf.mask` and the offset
    /// in its `cdef_idx`
    pub(crate) cur_sb_cdef_idx: (usize, usize),
    // for chroma sub8x8, we need to know the filter for all 4 subblocks in
    // a 4x4 area, but the top/left one can go out of cache already, so this
    // keeps it accessible
    pub(crate) tl_4x4_filter: u8,
}

impl Default for Dav1dTaskContext {
    fn default() -> Self {
        Dav1dTaskContext {
            bx: 0,
            by: 0,
            l: BlockContext::default(),
            a: 0,
            rt: RefmvsTile::default(),
            cf: vec![0; 32 * 32],
            al_pal: [[[[0; 8]; 3]; 32]; 2],
            pal_sz_uv: [[0; 32]; 2],
            txtp_map: [0; 32 * 32],
            scratch: TaskScratch::default(),
            px: TaskPx::default(),
            warpmv: Dav1dWarpedMotionParams::default(),
            lf_mask: 0,
            top_pre_cdef_toggle: 0,
            cur_sb_cdef_idx: (0, 0),
            tl_4x4_filter: 0,
        }
    }
}
