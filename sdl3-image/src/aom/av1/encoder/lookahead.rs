// Rust translation of av1/encoder/lookahead.c and lookahead.h from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches), without frame
// metadata.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The lookahead queue of source frames. A pushed frame is copied into the
//! queue's own buffer with its borders extended.

use super::encoder::{ENCODE_STAGE, LAP_STAGE};
use super::extend::av1_copy_and_extend_frame;
use crate::aom::aom_scale::yv12config::*;

pub(crate) const MAX_LAG_BUFFERS: i32 = 48;
pub(crate) const MAX_LAP_BUFFERS: i32 = 48;
const MAX_TOTAL_BUFFERS: i32 = MAX_LAG_BUFFERS + MAX_LAP_BUFFERS;
pub(crate) const LAP_LAG_IN_FRAMES: i32 = 17;

/// `struct lookahead_entry`
#[derive(Debug, Default)]
pub(crate) struct LookaheadEntry {
    pub(crate) img: Yv12BufferConfig,
    pub(crate) ts_start: i64,
    pub(crate) ts_end: i64,
    pub(crate) display_idx: i32,
    pub(crate) flags: u32,
}

// The max of past frames we want to keep in the queue.
const MAX_PRE_FRAMES: i32 = 1;

/// `struct read_ctx`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ReadCtx {
    pub(crate) sz: i32,       /* Number of buffers currently in the queue */
    pub(crate) read_idx: i32, /* Read index */
    pub(crate) pop_sz: i32,   /* Size to check for pop condition */
    pub(crate) valid: i32,    /* Is this ctx valid? */
}

/// `struct lookahead_ctx`
#[derive(Debug, Default)]
pub(crate) struct LookaheadCtx {
    pub(crate) max_sz: i32,              /* Absolute size of the queue */
    pub(crate) write_idx: i32,           /* Write index */
    pub(crate) read_ctxs: [ReadCtx; 2],  /* Read context */
    pub(crate) buf: Vec<LookaheadEntry>, /* Buffer list */
    pub(crate) push_frame_count: i32,    /* Number of frames that have been pushed in the queue*/
    pub(crate) max_pre_frames: u8,       /* Maximum number of past frames allowed in the queue */
}

/// Translation of `pop()`: returns the entry's index.
fn pop(max_sz: i32, idx: &mut i32) -> usize {
    let mut index = *idx;
    let buf = index as usize;

    debug_assert!(index < max_sz);
    index += 1;
    if index >= max_sz {
        index -= max_sz;
    }
    *idx = index;
    buf
}

/// Translation of `av1_lookahead_init()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn av1_lookahead_init(
    width: u32,
    height: u32,
    subsampling_x: u32,
    subsampling_y: u32,
    use_highbitdepth: bool,
    depth: u32,
    border_in_pixels: i32,
    byte_alignment: i32,
    num_lap_buffers: i32,
    is_all_intra: bool,
    enable_global_motion: bool,
) -> Result<LookaheadCtx, ()> {
    let _ = enable_global_motion;
    let lag_in_frames = 1.max(depth as i32);

    // For all-intra frame encoding, previous source frames are not required.
    // Hence max_pre_frames is set to 0 in this case. As previous source frames
    // are accessed using a negative index to av1_lookahead_peek(), setting
    // max_pre_frames to 0 will cause av1_lookahead_peek() to return NULL for a
    // negative index.
    let max_pre_frames: u8 = if is_all_intra {
        0
    } else {
        MAX_PRE_FRAMES as u8
    };

    // Add the lags to depth and clamp
    let mut depth = depth as i32 + num_lap_buffers;
    depth = depth.clamp(1, MAX_TOTAL_BUFFERS);

    // Allocate memory to keep previous source frames available.
    depth += max_pre_frames as i32;

    // Allocate the lookahead structures
    let mut ctx = LookaheadCtx {
        max_sz: depth,
        push_frame_count: 0,
        max_pre_frames,
        ..LookaheadCtx::default()
    };
    ctx.read_ctxs[ENCODE_STAGE as usize].pop_sz = ctx.max_sz - ctx.max_pre_frames as i32;
    ctx.read_ctxs[ENCODE_STAGE as usize].valid = 1;
    if num_lap_buffers != 0 {
        ctx.read_ctxs[LAP_STAGE as usize].pop_sz = lag_in_frames;
        ctx.read_ctxs[LAP_STAGE as usize].valid = 1;
    }
    ctx.buf.try_reserve_exact(depth as usize).map_err(|_| ())?;
    for _ in 0..depth {
        let mut entry = LookaheadEntry::default();
        aom_realloc_frame_buffer(
            &mut entry.img,
            width as i32,
            height as i32,
            subsampling_x as i32,
            subsampling_y as i32,
            use_highbitdepth as i32,
            border_in_pixels,
            byte_alignment,
            0,
        )?;
        ctx.buf.push(entry);
    }
    Ok(ctx)
}

/// Translation of `av1_lookahead_push()`: `src_planes` are the source
/// frame's sample planes (`src->y_buffer`, `u_buffer`, `v_buffer`).
pub(crate) fn av1_lookahead_push(
    ctx: &mut LookaheadCtx,
    src: &Yv12BufferConfig,
    src_planes: [&[u16]; 3],
    ts_start: i64,
    ts_end: i64,
    use_highbitdepth: bool,
    flags: u32,
) -> Result<(), ()> {
    let width = src.y_crop_width();
    let height = src.y_crop_height();
    let uv_width = src.uv_crop_width();
    let uv_height = src.uv_crop_height();
    let subsampling_x = src.subsampling_x;
    let subsampling_y = src.subsampling_y;

    debug_assert!(ctx.read_ctxs[ENCODE_STAGE as usize].valid == 1);
    if ctx.read_ctxs[ENCODE_STAGE as usize].sz + ctx.max_pre_frames as i32 > ctx.max_sz {
        return Err(());
    }

    ctx.read_ctxs[ENCODE_STAGE as usize].sz += 1;
    if ctx.read_ctxs[LAP_STAGE as usize].valid != 0 {
        ctx.read_ctxs[LAP_STAGE as usize].sz += 1;
    }

    let max_sz = ctx.max_sz;
    let buf = &mut ctx.buf[pop(max_sz, &mut ctx.write_idx)];

    let new_dimensions = width != buf.img.y_crop_width()
        || height != buf.img.y_crop_height()
        || uv_width != buf.img.uv_crop_width()
        || uv_height != buf.img.uv_crop_height();
    let larger_dimensions = width > buf.img.y_width()
        || height > buf.img.y_height()
        || uv_width > buf.img.widths[1]
        || uv_height > buf.img.heights[1];
    debug_assert!(!larger_dimensions || new_dimensions);

    if larger_dimensions {
        let mut new_img = Yv12BufferConfig::default();
        aom_alloc_frame_buffer(
            &mut new_img,
            width,
            height,
            subsampling_x,
            subsampling_y,
            use_highbitdepth as i32,
            AOM_BORDER_IN_PIXELS,
            0,
            0,
        )?;
        buf.img = new_img;
    } else if new_dimensions {
        buf.img.crop_widths[0] = src.y_crop_width();
        buf.img.crop_heights[0] = src.y_crop_height();
        buf.img.crop_widths[1] = src.uv_crop_width();
        buf.img.crop_heights[1] = src.uv_crop_height();
        buf.img.subsampling_x = src.subsampling_x;
        buf.img.subsampling_y = src.subsampling_y;
    }
    // Partial copy not implemented yet
    av1_copy_and_extend_frame(src, src_planes, &mut buf.img);

    buf.ts_start = ts_start;
    buf.ts_end = ts_end;
    buf.display_idx = ctx.push_frame_count;
    buf.flags = flags;
    ctx.push_frame_count += 1;
    // (no metadata)
    Ok(())
}

/// Translation of `av1_lookahead_pop()`: returns the popped entry's
/// index.
pub(crate) fn av1_lookahead_pop(ctx: &mut LookaheadCtx, drain: bool, stage: i32) -> Option<usize> {
    let max_sz = ctx.max_sz;
    let read_ctx = &mut ctx.read_ctxs[stage as usize];
    debug_assert!(read_ctx.valid == 1);
    if read_ctx.sz != 0 && (drain || read_ctx.sz == read_ctx.pop_sz) {
        let buf = pop(max_sz, &mut read_ctx.read_idx);
        read_ctx.sz -= 1;
        return Some(buf);
    }
    None
}

/// Translation of `av1_lookahead_peek()`: returns the entry's index.
pub(crate) fn av1_lookahead_peek(ctx: &LookaheadCtx, index: i32, stage: i32) -> Option<usize> {
    let read_ctx = &ctx.read_ctxs[stage as usize];
    debug_assert!(read_ctx.valid == 1);
    let mut index = index;
    if index >= 0 {
        // Forward peek
        if index < read_ctx.sz {
            index += read_ctx.read_idx;
            if index >= ctx.max_sz {
                index -= ctx.max_sz;
            }
            return Some(index as usize);
        }
    } else if index < 0 {
        // Backward peek
        if -index <= ctx.max_pre_frames as i32 {
            index += read_ctx.read_idx;
            if index < 0 {
                index += ctx.max_sz;
            }
            return Some(index as usize);
        }
    }

    None
}

/// Translation of `av1_lookahead_depth()`.
pub(crate) fn av1_lookahead_depth(ctx: &LookaheadCtx, stage: i32) -> u32 {
    let read_ctx = &ctx.read_ctxs[stage as usize];
    debug_assert!(read_ctx.valid == 1);
    read_ctx.sz as u32
}

/// Translation of `av1_lookahead_pop_sz()`.
pub(crate) fn av1_lookahead_pop_sz(ctx: &LookaheadCtx, stage: i32) -> i32 {
    let read_ctx = &ctx.read_ctxs[stage as usize];
    debug_assert!(read_ctx.valid == 1);
    read_ctx.pop_sz
}
