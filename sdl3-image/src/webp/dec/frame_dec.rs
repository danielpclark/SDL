// Rust translation of src/dec/frame_dec.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2010 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Frame-reconstruction function. Memory allocation.
//!
//! Single-threaded (`mt_method_` 0, as upstream without the `use_threads`
//! option), without dithering (a decoding option): the worker and
//! `VP8InitDithering()`/`DitherRow()` are not translated.

use crate::webp::dec::alpha_dec::vp8_decompress_alpha_rows;
use crate::webp::dec::io_dec::{custom_put, custom_setup, custom_teardown, WebPDecParams};
use crate::webp::dec::vp8_dec::{
    vp8_init_scanline, vp8_set_error, VP8Decoder, VP8FInfo, VP8Io, VP8MBData, VP8TopSamples, U_OFF,
    VP8_TOP_SAMPLES_SIZE, V_OFF, YUV_SIZE, Y_OFF,
};
use crate::webp::dec::{
    B_DC_PRED, B_DC_PRED_NOLEFT, B_DC_PRED_NOTOP, B_DC_PRED_NOTOPLEFT, NUM_MB_SEGMENTS,
};
use crate::webp::decode::VP8StatusCode;
use crate::webp::dsp::dec::{
    h_filter16, h_filter16i, h_filter8, h_filter8i, pred_chroma8, pred_luma16, pred_luma4,
    simple_h_filter16, simple_h_filter16i, simple_v_filter16, simple_v_filter16i, transform,
    transform_ac3, transform_dc, transform_dcuv, transform_uv, v_filter16, v_filter16i, v_filter8,
    v_filter8i,
};
use crate::webp::dsp::BPS;
use crate::webp::utils::{check_size_overflow, safe_alloc};

//------------------------------------------------------------------------------
// Main reconstruction function.

static K_SCAN: [u16; 16] = [
    0,
    4,
    8,
    12,
    (4 * BPS) as u16,
    (4 + 4 * BPS) as u16,
    (8 + 4 * BPS) as u16,
    (12 + 4 * BPS) as u16,
    (8 * BPS) as u16,
    (4 + 8 * BPS) as u16,
    (8 + 8 * BPS) as u16,
    (12 + 8 * BPS) as u16,
    (12 * BPS) as u16,
    (4 + 12 * BPS) as u16,
    (8 + 12 * BPS) as u16,
    (12 + 12 * BPS) as u16,
];

/// Translation of `CheckMode()`.
fn check_mode(mb_x: i32, mb_y: i32, mode: i32) -> i32 {
    if mode == B_DC_PRED {
        if mb_x == 0 {
            return if mb_y == 0 {
                B_DC_PRED_NOTOPLEFT
            } else {
                B_DC_PRED_NOLEFT
            };
        } else {
            return if mb_y == 0 {
                B_DC_PRED_NOTOP
            } else {
                B_DC_PRED
            };
        }
    }
    mode
}

/// Translation of `DoTransform()`.
fn do_transform(bits: u32, src: &[i16], dst: &mut [u8], off: usize) {
    match bits >> 30 {
        3 => transform(src, dst, off, false),
        2 => transform_ac3(src, dst, off),
        1 => transform_dc(src, dst, off),
        _ => {}
    }
}

/// Translation of `DoUVTransform()`.
fn do_uv_transform(bits: u32, src: &[i16], dst: &mut [u8], off: usize) {
    if bits & 0xff != 0 {
        // any non-zero coeff at all?
        if bits & 0xaa != 0 {
            // any non-zero AC coefficient?
            transform_uv(src, dst, off); // note we don't use the AC3 variant for U/V
        } else {
            transform_dcuv(src, dst, off);
        }
    }
}

/// Translation of `ReconstructRow()`.
fn reconstruct_row(dec: &mut VP8Decoder<'_>) {
    let ctx = dec.thread_ctx;
    let mb_y = ctx.mb_y;
    let cache_id = ctx.id as usize;
    let yuv_b = &mut dec.yuv_b;
    let y_dst = Y_OFF;
    let u_dst = U_OFF;
    let v_dst = V_OFF;

    // Initialize left-most block.
    for j in 0..16 {
        yuv_b[y_dst + j * BPS - 1] = 129;
    }
    for j in 0..8 {
        yuv_b[u_dst + j * BPS - 1] = 129;
        yuv_b[v_dst + j * BPS - 1] = 129;
    }

    // Init top-left sample on left column too.
    if mb_y > 0 {
        yuv_b[y_dst - 1 - BPS] = 129;
        yuv_b[u_dst - 1 - BPS] = 129;
        yuv_b[v_dst - 1 - BPS] = 129;
    } else {
        // we only need to do this init once at block (0,0).
        // Afterward, it remains valid for the whole topmost row.
        yuv_b[y_dst - BPS - 1..y_dst - BPS - 1 + 16 + 4 + 1].fill(127);
        yuv_b[u_dst - BPS - 1..u_dst - BPS - 1 + 8 + 1].fill(127);
        yuv_b[v_dst - BPS - 1..v_dst - BPS - 1 + 8 + 1].fill(127);
    }

    // Reconstruct one row.
    for mb_x in 0..dec.mb_w {
        let block: &VP8MBData = &dec.mb_data[mb_x as usize];

        // Rotate in the left samples from previously decoded block. We move four
        // pixels at a time for alignment reason, and because of in-loop filter.
        if mb_x > 0 {
            for j in -1isize..16 {
                let row = (y_dst as isize + j * BPS as isize) as usize;
                yuv_b.copy_within(row + 12..row + 16, row - 4);
            }
            for j in -1isize..8 {
                let row = (u_dst as isize + j * BPS as isize) as usize;
                yuv_b.copy_within(row + 4..row + 8, row - 4);
                let row = (v_dst as isize + j * BPS as isize) as usize;
                yuv_b.copy_within(row + 4..row + 8, row - 4);
            }
        }
        {
            // bring top samples into the cache
            let top_yuv = mb_x as usize;
            let coeffs = &block.coeffs;
            let mut bits = block.non_zero_y;

            if mb_y > 0 {
                let top = &dec.yuv_t[top_yuv];
                yuv_b[y_dst - BPS..y_dst - BPS + 16].copy_from_slice(&top.y);
                yuv_b[u_dst - BPS..u_dst - BPS + 8].copy_from_slice(&top.u);
                yuv_b[v_dst - BPS..v_dst - BPS + 8].copy_from_slice(&top.v);
            }

            // predict and add residuals
            if block.is_i4x4 {
                // 4x4
                let top_right = y_dst - BPS + 16;

                if mb_y > 0 {
                    if mb_x >= dec.mb_w - 1 {
                        // on rightmost border
                        let v = dec.yuv_t[top_yuv].y[15];
                        yuv_b[top_right..top_right + 4].fill(v);
                    } else {
                        yuv_b[top_right..top_right + 4]
                            .copy_from_slice(&dec.yuv_t[top_yuv + 1].y[..4]);
                    }
                }
                // replicate the top-right pixels below
                // (top_right[BPS], [2 * BPS] and [3 * BPS] of a uint32_t*)
                for k in 1..4 {
                    yuv_b.copy_within(top_right..top_right + 4, top_right + 4 * BPS * k);
                }

                // predict and add residuals for all 4x4 blocks in turn.
                for n in 0..16 {
                    let dst = y_dst + K_SCAN[n] as usize;
                    pred_luma4(block.imodes[n], yuv_b, dst);
                    do_transform(bits, &coeffs[n * 16..], yuv_b, dst);
                    bits <<= 2;
                }
            } else {
                // 16x16
                let pred_func = check_mode(mb_x, mb_y, block.imodes[0] as i32);
                pred_luma16(pred_func, yuv_b, y_dst);
                if bits != 0 {
                    for n in 0..16 {
                        do_transform(bits, &coeffs[n * 16..], yuv_b, y_dst + K_SCAN[n] as usize);
                        bits <<= 2;
                    }
                }
            }
            {
                // Chroma
                let bits_uv = block.non_zero_uv;
                let pred_func = check_mode(mb_x, mb_y, block.uvmode as i32);
                pred_chroma8(pred_func, yuv_b, u_dst);
                pred_chroma8(pred_func, yuv_b, v_dst);
                do_uv_transform(bits_uv, &coeffs[16 * 16..], yuv_b, u_dst);
                do_uv_transform(bits_uv >> 8, &coeffs[20 * 16..], yuv_b, v_dst);
            }

            // stash away top samples for next block
            if mb_y < dec.mb_h - 1 {
                let top = &mut dec.yuv_t[top_yuv];
                top.y
                    .copy_from_slice(&yuv_b[y_dst + 15 * BPS..y_dst + 15 * BPS + 16]);
                top.u
                    .copy_from_slice(&yuv_b[u_dst + 7 * BPS..u_dst + 7 * BPS + 8]);
                top.v
                    .copy_from_slice(&yuv_b[v_dst + 7 * BPS..v_dst + 7 * BPS + 8]);
            }
        }
        // Transfer reconstructed samples from yuv_b_ cache to final destination.
        {
            let y_stride = dec.cache_y_stride as usize;
            let uv_stride = dec.cache_uv_stride as usize;
            let y_offset = cache_id * 16 * y_stride;
            let uv_offset = cache_id * 8 * uv_stride;
            let y_out = dec.cache_y + mb_x as usize * 16 + y_offset;
            let u_out = dec.cache_u + mb_x as usize * 8 + uv_offset;
            let v_out = dec.cache_v + mb_x as usize * 8 + uv_offset;
            for j in 0..16 {
                dec.cache[y_out + j * y_stride..y_out + j * y_stride + 16]
                    .copy_from_slice(&yuv_b[y_dst + j * BPS..y_dst + j * BPS + 16]);
            }
            for j in 0..8 {
                dec.cache[u_out + j * uv_stride..u_out + j * uv_stride + 8]
                    .copy_from_slice(&yuv_b[u_dst + j * BPS..u_dst + j * BPS + 8]);
                dec.cache[v_out + j * uv_stride..v_out + j * uv_stride + 8]
                    .copy_from_slice(&yuv_b[v_dst + j * BPS..v_dst + j * BPS + 8]);
            }
        }
    }
}

//------------------------------------------------------------------------------
// Filtering

// kFilterExtraRows[] = How many extra lines are needed on the MB boundary
// for caching, given a filtering level.
// Simple filter:  up to 2 luma samples are read and 1 is written.
// Complex filter: up to 4 luma samples are read and 3 are written. Same for
//                 U/V, so it's 8 samples total (because of the 2x upsampling).
static K_FILTER_EXTRA_ROWS: [u8; 3] = [0, 2, 8];

/// Translation of `DoFilter()`.
fn do_filter(dec: &mut VP8Decoder<'_>, mb_x: i32, mb_y: i32) {
    let ctx = &dec.thread_ctx;
    let cache_id = ctx.id as usize;
    let y_bps = dec.cache_y_stride as usize;
    let f_info: VP8FInfo = dec.f_info[mb_x as usize];
    let y_dst = dec.cache_y + cache_id * 16 * y_bps + mb_x as usize * 16;
    let ilevel = f_info.f_ilevel as i32;
    let limit = f_info.f_limit as i32;
    let cache = &mut dec.cache;
    if limit == 0 {
        return;
    }
    debug_assert!(limit >= 3);
    if dec.filter_type == 1 {
        // simple
        if mb_x > 0 {
            simple_h_filter16(cache, y_dst, y_bps, limit + 4);
        }
        if f_info.f_inner != 0 {
            simple_h_filter16i(cache, y_dst, y_bps, limit);
        }
        if mb_y > 0 {
            simple_v_filter16(cache, y_dst, y_bps, limit + 4);
        }
        if f_info.f_inner != 0 {
            simple_v_filter16i(cache, y_dst, y_bps, limit);
        }
    } else {
        // complex
        let uv_bps = dec.cache_uv_stride as usize;
        let u_dst = dec.cache_u + cache_id * 8 * uv_bps + mb_x as usize * 8;
        let v_dst = dec.cache_v + cache_id * 8 * uv_bps + mb_x as usize * 8;
        let hev_thresh = f_info.hev_thresh as i32;
        if mb_x > 0 {
            h_filter16(cache, y_dst, y_bps, limit + 4, ilevel, hev_thresh);
            h_filter8(cache, u_dst, v_dst, uv_bps, limit + 4, ilevel, hev_thresh);
        }
        if f_info.f_inner != 0 {
            h_filter16i(cache, y_dst, y_bps, limit, ilevel, hev_thresh);
            h_filter8i(cache, u_dst, v_dst, uv_bps, limit, ilevel, hev_thresh);
        }
        if mb_y > 0 {
            v_filter16(cache, y_dst, y_bps, limit + 4, ilevel, hev_thresh);
            v_filter8(cache, u_dst, v_dst, uv_bps, limit + 4, ilevel, hev_thresh);
        }
        if f_info.f_inner != 0 {
            v_filter16i(cache, y_dst, y_bps, limit, ilevel, hev_thresh);
            v_filter8i(cache, u_dst, v_dst, uv_bps, limit, ilevel, hev_thresh);
        }
    }
}

/// Filter the decoded macroblock row (if needed). Translation of
/// `FilterRow()`.
fn filter_row(dec: &mut VP8Decoder<'_>) {
    let mb_y = dec.thread_ctx.mb_y;
    debug_assert!(dec.thread_ctx.filter_row);
    for mb_x in dec.tl_mb_x..dec.br_mb_x {
        do_filter(dec, mb_x, mb_y);
    }
}

//------------------------------------------------------------------------------
// Precompute the filtering strength for each segment and each i4x4/i16x16 mode.

/// Translation of `PrecomputeFilterStrengths()`.
fn precompute_filter_strengths(dec: &mut VP8Decoder<'_>) {
    if dec.filter_type > 0 {
        let hdr = &dec.filter_hdr;
        for s in 0..NUM_MB_SEGMENTS {
            // First, compute the initial level
            let base_level;
            if dec.segment_hdr.use_segment {
                let mut l = dec.segment_hdr.filter_strength[s] as i32;
                if !dec.segment_hdr.absolute_delta {
                    l += hdr.level;
                }
                base_level = l;
            } else {
                base_level = hdr.level;
            }
            for i4x4 in 0..=1usize {
                let info = &mut dec.fstrengths[s][i4x4];
                let mut level = base_level;
                if hdr.use_lf_delta {
                    level += hdr.ref_lf_delta[0];
                    if i4x4 != 0 {
                        level += hdr.mode_lf_delta[0];
                    }
                }
                level = level.clamp(0, 63);
                if level > 0 {
                    let mut ilevel = level;
                    if hdr.sharpness > 0 {
                        if hdr.sharpness > 4 {
                            ilevel >>= 2;
                        } else {
                            ilevel >>= 1;
                        }
                        if ilevel > 9 - hdr.sharpness {
                            ilevel = 9 - hdr.sharpness;
                        }
                    }
                    if ilevel < 1 {
                        ilevel = 1;
                    }
                    info.f_ilevel = ilevel as u8;
                    info.f_limit = (2 * level + ilevel) as u8;
                    info.hev_thresh = if level >= 40 {
                        2
                    } else if level >= 15 {
                        1
                    } else {
                        0
                    };
                } else {
                    info.f_limit = 0; // no filtering
                }
                info.f_inner = i4x4 as u8;
            }
        }
    }
}

//------------------------------------------------------------------------------
// This function is called after a row of macroblocks is finished decoding.
// It also takes into account the following restrictions:
//  * In case of in-loop filtering, we must hold off sending some of the bottom
//    pixels as they are yet unfiltered. They will be when the next macroblock
//    row is decoded. Meanwhile, we must preserve them by rotating them in the
//    cache area. This doesn't hold for the very bottom row of the uncropped
//    picture of course.
//  * we must clip the remaining pixels against the cropping area. The VP8Io
//    struct must have the following fields set correctly before calling put():

/// vertical position of a MB. Translation of `MACROBLOCK_VPOS()`.
fn macroblock_vpos(mb_y: i32) -> i32 {
    mb_y * 16
}

/// Finalize and transmit a complete row. Return false in case of
/// user-abort. Translation of `FinishRow()`.
fn finish_row(
    dec: &mut VP8Decoder<'_>,
    io: &mut VP8Io<'_>,
    params: &mut WebPDecParams<'_>,
) -> bool {
    let mut ok = true;
    let ctx = dec.thread_ctx;
    let cache_id = ctx.id;
    let extra_y_rows = K_FILTER_EXTRA_ROWS[dec.filter_type as usize] as i32;
    let ysize = (extra_y_rows * dec.cache_y_stride) as usize;
    let uvsize = ((extra_y_rows / 2) * dec.cache_uv_stride) as usize;
    let y_offset = (cache_id * 16 * dec.cache_y_stride) as usize;
    let uv_offset = (cache_id * 8 * dec.cache_uv_stride) as usize;
    let ydst = dec.cache_y - ysize + y_offset;
    let udst = dec.cache_u - uvsize + uv_offset;
    let vdst = dec.cache_v - uvsize + uv_offset;
    let mb_y = ctx.mb_y;
    let is_first_row = mb_y == 0;
    let is_last_row = mb_y >= dec.br_mb_y - 1;

    if ctx.filter_row {
        filter_row(dec);
    }

    {
        // (io->put is CustomPut())
        let mut y_start = macroblock_vpos(mb_y);
        let mut y_end = macroblock_vpos(mb_y + 1);
        if !is_first_row {
            y_start -= extra_y_rows;
            io.y = ydst;
            io.u = udst;
            io.v = vdst;
        } else {
            io.y = dec.cache_y + y_offset;
            io.u = dec.cache_u + uv_offset;
            io.v = dec.cache_v + uv_offset;
        }

        if !is_last_row {
            y_end -= extra_y_rows;
        }
        if y_end > io.crop_bottom {
            y_end = io.crop_bottom; // make sure we don't overflow on last row.
        }
        // If dec->alpha_data_ is not NULL, we have some alpha plane present.
        io.a = None;
        if dec.alpha_data.is_some() && y_start < y_end {
            io.a = vp8_decompress_alpha_rows(dec, io, y_start, y_end - y_start);
            if io.a.is_none() {
                return vp8_set_error(
                    dec,
                    VP8StatusCode::BitstreamError,
                    "Could not decode alpha data.",
                );
            }
        }
        if y_start < io.crop_top {
            let delta_y = io.crop_top - y_start;
            y_start = io.crop_top;
            debug_assert!(delta_y & 1 == 0);
            io.y += (dec.cache_y_stride * delta_y) as usize;
            io.u += (dec.cache_uv_stride * (delta_y >> 1)) as usize;
            io.v += (dec.cache_uv_stride * (delta_y >> 1)) as usize;
            if let Some(a) = io.a.as_mut() {
                *a += (io.width * delta_y) as usize;
            }
        }
        if y_start < y_end {
            io.y += io.crop_left as usize;
            io.u += (io.crop_left >> 1) as usize;
            io.v += (io.crop_left >> 1) as usize;
            if let Some(a) = io.a.as_mut() {
                *a += io.crop_left as usize;
            }
            io.mb_y = y_start - io.crop_top;
            io.mb_w = io.crop_right - io.crop_left;
            io.mb_h = y_end - y_start;
            ok = custom_put(io, params, &dec.cache, &dec.alpha_plane);
        }
    }
    // rotate top samples if needed
    if cache_id + 1 == dec.num_caches && !is_last_row {
        let y_stride = dec.cache_y_stride as usize;
        let uv_stride = dec.cache_uv_stride as usize;
        dec.cache.copy_within(
            ydst + 16 * y_stride..ydst + 16 * y_stride + ysize,
            dec.cache_y - ysize,
        );
        dec.cache.copy_within(
            udst + 8 * uv_stride..udst + 8 * uv_stride + uvsize,
            dec.cache_u - uvsize,
        );
        dec.cache.copy_within(
            vdst + 8 * uv_stride..vdst + 8 * uv_stride + uvsize,
            dec.cache_v - uvsize,
        );
    }

    ok
}

//------------------------------------------------------------------------------

/// Process the last decoded row (filtering + output). Translation of
/// `VP8ProcessRow()`.
pub(crate) fn vp8_process_row(
    dec: &mut VP8Decoder<'_>,
    io: &mut VP8Io<'_>,
    params: &mut WebPDecParams<'_>,
) -> bool {
    let filter_row =
        (dec.filter_type > 0) && (dec.mb_y >= dec.tl_mb_y) && (dec.mb_y <= dec.br_mb_y);
    // (mt_method_ == 0)
    // ctx->id_ and ctx->f_info_ are already set
    dec.thread_ctx.mb_y = dec.mb_y;
    dec.thread_ctx.filter_row = filter_row;
    reconstruct_row(dec);
    finish_row(dec, io, params)
}

//------------------------------------------------------------------------------
// Finish setting up the decoding parameter once user's setup() is called.

/// Call io->setup() and finish setting up scan parameters. After this call
/// returns, one must always call VP8ExitCritical() with the same
/// parameters. Both functions should be used in pair. Returns
/// VP8_STATUS_OK if ok, otherwise sets and returns the error status on
/// *dec. Translation of `VP8EnterCritical()`.
pub(crate) fn vp8_enter_critical(
    dec: &mut VP8Decoder<'_>,
    io: &mut VP8Io<'_>,
    params: &mut WebPDecParams<'_>,
) -> VP8StatusCode {
    // Call setup() first. This may trigger additional decoding features on 'io'.
    // Note: Afterward, we must call teardown() no matter what.
    if !custom_setup(io, params) {
        vp8_set_error(dec, VP8StatusCode::UserAbort, "Frame setup failed");
        return dec.status;
    }

    // Disable filtering per user request
    if io.bypass_filtering {
        dec.filter_type = 0;
    }

    // Define the area where we can skip in-loop filtering, in case of cropping.
    //
    // 'Simple' filter reads two luma samples outside of the macroblock
    // and filters one. It doesn't filter the chroma samples. Hence, we can
    // avoid doing the in-loop filtering before crop_top/crop_left position.
    // For the 'Complex' filter, 3 samples are read and up to 3 are filtered.
    // Means: there's a dependency chain that goes all the way up to the
    // top-left corner of the picture (MB #0). We must filter all the previous
    // macroblocks.
    {
        let extra_pixels = K_FILTER_EXTRA_ROWS[dec.filter_type as usize] as i32;
        if dec.filter_type == 2 {
            // For complex filter, we need to preserve the dependency chain.
            dec.tl_mb_x = 0;
            dec.tl_mb_y = 0;
        } else {
            // For simple filter, we can filter only the cropped region.
            // We include 'extra_pixels' on the other side of the boundary, since
            // vertical or horizontal filtering of the previous macroblock can
            // modify some abutting pixels.
            dec.tl_mb_x = (io.crop_left - extra_pixels) >> 4;
            dec.tl_mb_y = (io.crop_top - extra_pixels) >> 4;
            if dec.tl_mb_x < 0 {
                dec.tl_mb_x = 0;
            }
            if dec.tl_mb_y < 0 {
                dec.tl_mb_y = 0;
            }
        }
        // We need some 'extra' pixels on the right/bottom.
        dec.br_mb_y = (io.crop_bottom + 15 + extra_pixels) >> 4;
        dec.br_mb_x = (io.crop_right + 15 + extra_pixels) >> 4;
        if dec.br_mb_x > dec.mb_w {
            dec.br_mb_x = dec.mb_w;
        }
        if dec.br_mb_y > dec.mb_h {
            dec.br_mb_y = dec.mb_h;
        }
    }
    precompute_filter_strengths(dec);
    VP8StatusCode::Ok
}

/// Must always be called in pair with VP8EnterCritical(). Returns false in
/// case of error. Translation of `VP8ExitCritical()`.
pub(crate) fn vp8_exit_critical(
    _dec: &mut VP8Decoder<'_>,
    _io: &mut VP8Io<'_>,
    params: &mut WebPDecParams<'_>,
) -> bool {
    custom_teardown(params);
    true
}

//------------------------------------------------------------------------------

/// 1 cache row only for single-threaded case. Translation of
/// `ST_CACHE_LINES`.
const ST_CACHE_LINES: i32 = 1;

/// Initialize multi/single-thread worker. Translation of
/// `InitThreadContext()` (single-threaded).
fn init_thread_context(dec: &mut VP8Decoder<'_>) -> bool {
    dec.cache_id = 0;
    dec.num_caches = ST_CACHE_LINES;
    true
}

//------------------------------------------------------------------------------
// Memory setup

/// Translation of `AllocateMemory()`: the parts of upstream's memory
/// chunk are vectors, the size check is upstream's on their total.
fn allocate_memory(dec: &mut VP8Decoder<'_>) -> bool {
    let num_caches = dec.num_caches as usize;
    let mb_w = dec.mb_w as usize;
    // Note: we use 'size_t' when there's no overflow risk, uint64_t otherwise.
    let intra_pred_mode_size = 4 * mb_w;
    let top_size = VP8_TOP_SAMPLES_SIZE * mb_w;
    let mb_info_size = (mb_w + 1) * 2;
    let f_info_size = if dec.filter_type > 0 { mb_w * 4 } else { 0 };
    let yuv_size = YUV_SIZE;
    let mb_data_size = mb_w * 800; // sizeof(VP8MBData)
    let cache_height =
        (16 * num_caches + K_FILTER_EXTRA_ROWS[dec.filter_type as usize] as usize) * 3 / 2;
    let cache_size = top_size * cache_height;
    // alpha_size is the only one that scales as width x height.
    let alpha_size: u64 = if dec.alpha_data.is_some() {
        dec.pic_hdr.width as u64 * dec.pic_hdr.height as u64
    } else {
        0
    };
    let needed: u64 = intra_pred_mode_size as u64
        + top_size as u64
        + mb_info_size as u64
        + f_info_size as u64
        + yuv_size as u64
        + mb_data_size as u64
        + cache_size as u64
        + alpha_size
        + 31;

    if !check_size_overflow(needed) {
        return false; // check for overflow
    }
    // (WebPSafeMalloc(needed, 1)'s size check, then the vectors)
    let allocated = (|| {
        if needed > crate::webp::utils::WEBP_MAX_ALLOCABLE_MEMORY {
            return None;
        }
        dec.intra_t = safe_alloc(intra_pred_mode_size as u64, 0u8)?;
        dec.yuv_t = safe_alloc(mb_w as u64, VP8TopSamples::default())?;
        dec.mb_info = safe_alloc(mb_w as u64 + 1, Default::default())?;
        dec.f_info = safe_alloc(
            if dec.filter_type > 0 { mb_w as u64 } else { 0 },
            VP8FInfo::default(),
        )?;
        dec.yuv_b = safe_alloc(yuv_size as u64, 0u8)?;
        dec.mb_data = safe_alloc(mb_w as u64, VP8MBData::default())?;
        dec.cache = safe_alloc(cache_size as u64, 0u8)?;
        Some(())
    })();
    if allocated.is_none() {
        return vp8_set_error(
            dec,
            VP8StatusCode::OutOfMemory,
            "no memory during frame initialization.",
        );
    }

    dec.thread_ctx.id = 0;

    dec.cache_y_stride = 16 * mb_w as i32;
    dec.cache_uv_stride = 8 * mb_w as i32;
    {
        let extra_rows = K_FILTER_EXTRA_ROWS[dec.filter_type as usize] as usize;
        let extra_y = extra_rows * dec.cache_y_stride as usize;
        let extra_uv = (extra_rows / 2) * dec.cache_uv_stride as usize;
        dec.cache_y = extra_y;
        dec.cache_u = dec.cache_y + 16 * num_caches * dec.cache_y_stride as usize + extra_uv;
        dec.cache_v = dec.cache_u + 8 * num_caches * dec.cache_uv_stride as usize + extra_uv;
        dec.cache_id = 0;
    }

    // alpha plane
    // (allocated by the alpha decoder, AllocateAlphaPlane())

    // note: left/top-info is initialized once for all.
    vp8_init_scanline(dec); // initialize left too.

    // initialize top
    dec.intra_t.fill(B_DC_PRED as u8);

    true
}

/// Translation of `InitIo()`.
fn init_io(dec: &VP8Decoder<'_>, io: &mut VP8Io<'_>) {
    // prepare 'io'
    io.mb_y = 0;
    io.y = dec.cache_y;
    io.u = dec.cache_u;
    io.v = dec.cache_v;
    io.y_stride = dec.cache_y_stride;
    io.uv_stride = dec.cache_uv_stride;
    io.a = None;
}

/// Translation of `VP8InitFrame()`.
pub(crate) fn vp8_init_frame(dec: &mut VP8Decoder<'_>, io: &mut VP8Io<'_>) -> bool {
    if !init_thread_context(dec) {
        return false; // call first. Sets dec->num_caches_.
    }
    if !allocate_memory(dec) {
        return false;
    }
    init_io(dec, io);
    true
}
