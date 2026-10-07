// Rust translation of src/dec/io_dec.c and the WebPDecParams parts of
// src/dec/webpi_dec.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! functions for sample output: the default `VP8Io` hooks, converting the
//! lossy decoder's YUV rows to RGB(A) with the fancy upsampler and adding
//! the alpha plane.
//!
//! What SDL_image's loader reaches is translated: RGB(A) output with fancy
//! upsampling and without scaling. The YUV(A) emitters, the point sampler
//! (for `no_fancy_upsampling`), the 4444 alpha emitter, premultiplication
//! and the rescalers are not.

use crate::webp::dec::vp8_dec::VP8Io;
use crate::webp::dec::webp_dec::webp_io_init_from_options;
use crate::webp::decode::{
    webp_is_alpha_mode, webp_is_premultiplied_mode, webp_is_rgb_mode, WebPDecBuffer, WebpCspMode,
};
use crate::webp::dsp::alpha_processing::webp_dispatch_alpha;
use crate::webp::dsp::upsampling::{webp_upsample, LinePair};
use crate::webp::utils::safe_alloc;

/// The `OutputFunc` set up: `emit`. Translation of the emitters' choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OutputFunc {
    None,
    /// `EmitFancyRGB()`
    FancyRgb,
}

/// The `OutputAlphaFunc` set up: `emit_alpha`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OutputAlphaFunc {
    None,
    /// `EmitAlphaRGB()`
    AlphaRgb,
}

/// WebPDecParams: Decoding output parameters. Transient internal object.
/// Translation of `WebPDecParams` (owning its output buffer; the decoding
/// options are always NULL here).
#[derive(Debug)]
pub(crate) struct WebPDecParams<'o> {
    /// output buffer.
    pub(crate) output: WebPDecBuffer<'o>,
    /// cache for the fancy upsampler (`tmp_y`, `tmp_u` and `tmp_v` are
    /// offsets in `memory`)
    pub(crate) tmp_y: usize,
    pub(crate) tmp_u: usize,
    pub(crate) tmp_v: usize,
    /// coordinate of the line that was last output
    pub(crate) last_y: i32,
    /// overall scratch memory for the output work.
    pub(crate) memory: Vec<u8>,
    /// output RGB or YUV samples
    pub(crate) emit: OutputFunc,
    /// output alpha channel
    pub(crate) emit_alpha: OutputAlphaFunc,
}

impl<'o> WebPDecParams<'o> {
    /// Should be called first, before any use of the WebPDecParams object.
    /// Translation of `WebPResetDecParams()`.
    pub(crate) fn new(output: WebPDecBuffer<'o>) -> WebPDecParams<'o> {
        WebPDecParams {
            output,
            tmp_y: 0,
            tmp_u: 0,
            tmp_v: 0,
            last_y: 0,
            memory: Vec::new(),
            emit: OutputFunc::None,
            emit_alpha: OutputAlphaFunc::None,
        }
    }
}

//------------------------------------------------------------------------------
// Fancy upsampling

/// Translation of `EmitFancyRGB()`: `samples` holds the rows `io->y`,
/// `io->u` and `io->v` point to.
fn emit_fancy_rgb(io: &VP8Io<'_>, p: &mut WebPDecParams<'_>, samples: &[u8]) -> i32 {
    let mut num_lines_out = io.mb_h; // a priori guess
    let stride = p.output.rgba.stride as usize;
    let colorspace = p.output.colorspace;
    let mut dst = io.mb_y as usize * stride;
    let mut cur_y = io.y;
    let mut cur_u = io.u;
    let mut cur_v = io.v;
    let mut y = io.mb_y;
    let y_end = io.mb_y + io.mb_h;
    let mb_w = io.mb_w;
    let uv_w = ((mb_w + 1) / 2) as usize;
    let y_stride = io.y_stride as usize;
    let uv_stride = io.uv_stride as usize;
    let mem = &p.memory;
    let out = &mut *p.output.rgba.rgba;

    if y == 0 {
        // First line is special cased. We mirror the u/v samples at boundary.
        let l = LinePair {
            top_y: &samples[cur_y..],
            bottom_y: None,
            top_u: &samples[cur_u..],
            top_v: &samples[cur_v..],
            cur_u: &samples[cur_u..],
            cur_v: &samples[cur_v..],
            top_dst: dst,
            bottom_dst: None,
        };
        webp_upsample(colorspace, &l, out, mb_w);
    } else {
        // We can finish the left-over line from previous call.
        let l = LinePair {
            top_y: &mem[p.tmp_y..],
            bottom_y: Some(&samples[cur_y..]),
            top_u: &mem[p.tmp_u..],
            top_v: &mem[p.tmp_v..],
            cur_u: &samples[cur_u..],
            cur_v: &samples[cur_v..],
            top_dst: dst - stride,
            bottom_dst: Some(dst),
        };
        webp_upsample(colorspace, &l, out, mb_w);
        num_lines_out += 1;
    }
    // Loop over each output pairs of row.
    while y + 2 < y_end {
        let top_u = cur_u;
        let top_v = cur_v;
        cur_u += uv_stride;
        cur_v += uv_stride;
        dst += 2 * stride;
        cur_y += 2 * y_stride;
        let l = LinePair {
            top_y: &samples[cur_y - y_stride..],
            bottom_y: Some(&samples[cur_y..]),
            top_u: &samples[top_u..],
            top_v: &samples[top_v..],
            cur_u: &samples[cur_u..],
            cur_v: &samples[cur_v..],
            top_dst: dst - stride,
            bottom_dst: Some(dst),
        };
        webp_upsample(colorspace, &l, out, mb_w);
        y += 2;
    }
    // move to last row
    cur_y += y_stride;
    if io.crop_top + y_end < io.crop_bottom {
        // Save the unfinished samples for next call (as we're not done yet).
        let (tmp_y, tmp_u, tmp_v) = (p.tmp_y, p.tmp_u, p.tmp_v);
        let mem = &mut p.memory;
        mem[tmp_y..tmp_y + mb_w as usize].copy_from_slice(&samples[cur_y..cur_y + mb_w as usize]);
        mem[tmp_u..tmp_u + uv_w].copy_from_slice(&samples[cur_u..cur_u + uv_w]);
        mem[tmp_v..tmp_v + uv_w].copy_from_slice(&samples[cur_v..cur_v + uv_w]);
        // The fancy upsampler leaves a row unfinished behind
        // (except for the very last row)
        num_lines_out -= 1;
    } else {
        // Process the very last row of even-sized picture
        if y_end & 1 == 0 {
            let l = LinePair {
                top_y: &samples[cur_y..],
                bottom_y: None,
                top_u: &samples[cur_u..],
                top_v: &samples[cur_v..],
                cur_u: &samples[cur_u..],
                cur_v: &samples[cur_v..],
                top_dst: dst + stride,
                bottom_dst: None,
            };
            webp_upsample(colorspace, &l, out, mb_w);
        }
    }
    num_lines_out
}

//------------------------------------------------------------------------------

/// Translation of `GetAlphaSourceRow()`: the first row and the number of
/// rows of alpha to output, and where their alpha starts.
fn get_alpha_source_row(io: &VP8Io<'_>, alpha: &mut usize) -> (i32, i32) {
    let mut start_y = io.mb_y;
    let mut num_rows = io.mb_h;

    // Compensate for the 1-line delay of the fancy upscaler.
    // This is similar to EmitFancyRGB().
    if io.fancy_upsampling {
        if start_y == 0 {
            // We don't process the last row yet. It'll be done during the next call.
            num_rows -= 1;
        } else {
            start_y -= 1;
            // Fortunately, *alpha data is persistent, so we can go back
            // one row and finish alpha blending, now that the fancy upscaler
            // completed the YUV->RGB interpolation.
            *alpha -= io.width as usize;
        }
        if io.crop_top + io.mb_y + io.mb_h == io.crop_bottom {
            // If it's the very last call, we process all the remaining rows!
            num_rows = io.crop_bottom - io.crop_top - start_y;
        }
    }
    (start_y, num_rows)
}

/// Translation of `EmitAlphaRGB()` (`alpha_plane` holds the row `io->a`
/// points to).
fn emit_alpha_rgb(
    io: &VP8Io<'_>,
    p: &mut WebPDecParams<'_>,
    expected_num_lines_out: i32,
    alpha_plane: &[u8],
) -> i32 {
    if let Some(mut alpha) = io.a {
        let mb_w = io.mb_w as usize;
        let colorspace = p.output.colorspace;
        let alpha_first =
            colorspace == WebpCspMode::Argb || colorspace == WebpCspMode::ArgbPremultiplied;
        let buf = &mut p.output.rgba;
        let (start_y, num_rows) = get_alpha_source_row(io, &mut alpha);
        let stride = buf.stride as usize;
        let base_rgba = start_y as usize * stride;
        let dst = base_rgba + if alpha_first { 0 } else { 3 };
        let has_alpha = webp_dispatch_alpha(
            alpha_plane,
            alpha,
            io.width as usize,
            mb_w,
            num_rows as usize,
            buf.rgba,
            dst,
            stride,
        );
        let _ = expected_num_lines_out;
        debug_assert!(expected_num_lines_out == num_rows);
        // has_alpha is true if there's non-trivial alpha to premultiply with.
        // (WebPApplyAlphaMultiply() for the premultiplied colorspaces is not
        // translated)
        let _ = has_alpha && webp_is_premultiplied_mode(colorspace);
    }
    0
}

//------------------------------------------------------------------------------
// Default custom functions

/// Translation of `CustomSetup()`.
pub(crate) fn custom_setup(io: &mut VP8Io<'_>, p: &mut WebPDecParams<'_>) -> bool {
    let colorspace = p.output.colorspace;
    let is_rgb = webp_is_rgb_mode(colorspace);
    let is_alpha = webp_is_alpha_mode(colorspace);

    p.memory = Vec::new();
    p.emit = OutputFunc::None;
    p.emit_alpha = OutputAlphaFunc::None;
    if !webp_io_init_from_options(
        io,
        if is_alpha {
            WebpCspMode::Yuv
        } else {
            WebpCspMode::Yuva
        },
    ) {
        return false;
    }
    // (io->use_scaling needs a decoding option: no rescaler)
    if is_rgb {
        // (EmitSampledRGB(), the default, is not translated: without decoding
        // options, the fancy upsampler is always used)
        if !io.fancy_upsampling {
            return false;
        }
        let uv_width = ((io.mb_w + 1) >> 1) as usize;
        // (WebPSafeMalloc(1ULL, (size_t)(io->mb_w + 2 * uv_width)))
        let Some(memory) = safe_alloc((io.mb_w as usize + 2 * uv_width) as u64, 0u8) else {
            return false; // memory error.
        };
        p.memory = memory;
        p.tmp_y = 0;
        p.tmp_u = p.tmp_y + io.mb_w as usize;
        p.tmp_v = p.tmp_u + uv_width;
        p.emit = OutputFunc::FancyRgb;
    } else {
        // (EmitYUV() is not translated)
        return false;
    }
    if is_alpha {
        // need transparency output
        // (EmitAlphaRGBA4444() and EmitAlphaYUV() are not translated)
        if colorspace == WebpCspMode::Rgba4444 || colorspace == WebpCspMode::RgbA4444Premultiplied {
            return false;
        }
        p.emit_alpha = OutputAlphaFunc::AlphaRgb;
    }

    true
}

//------------------------------------------------------------------------------

/// Translation of `CustomPut()`: `samples` holds the YUV rows and
/// `alpha_plane` the alpha plane `io` points into.
pub(crate) fn custom_put(
    io: &VP8Io<'_>,
    p: &mut WebPDecParams<'_>,
    samples: &[u8],
    alpha_plane: &[u8],
) -> bool {
    let mb_w = io.mb_w;
    let mb_h = io.mb_h;
    debug_assert!(io.mb_y & 1 == 0);

    if mb_w <= 0 || mb_h <= 0 {
        return false;
    }
    let num_lines_out = match p.emit {
        OutputFunc::FancyRgb => emit_fancy_rgb(io, p, samples),
        OutputFunc::None => return false,
    };
    if p.emit_alpha == OutputAlphaFunc::AlphaRgb {
        emit_alpha_rgb(io, p, num_lines_out, alpha_plane);
    }
    p.last_y += num_lines_out;
    true
}

//------------------------------------------------------------------------------

/// Translation of `CustomTeardown()`.
pub(crate) fn custom_teardown(p: &mut WebPDecParams<'_>) {
    p.memory = Vec::new();
}
