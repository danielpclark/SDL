// Rust translation of src/mux/anim_encode.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2014 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! AnimEncoder implementation.
//!
//! The frame being added (`curr_canvas_`) is passed along rather than
//! stored. The sub-frame pictures (`WebPPictureView()`s of the canvas
//! copy) are rectangles: the encoder of a candidate copies the rectangle's
//! pixels out of the canvas copy, encodes them, and copies them back, as
//! the encoding of a view writes through to the canvas (the lossless
//! encoder replaces the transparent pixels' colors).

use crate::webp::dec::webp_dec::{webp_decode_bgra_into, webp_get_features};
use crate::webp::decode::{
    VP8StatusCode, WebPBitstreamFeatures, WebPMuxAnimBlend, WebPMuxAnimDispose, MAX_DURATION,
    MAX_IMAGE_AREA,
};
use crate::webp::enc::config_enc::{webp_config_init, webp_validate_config};
use crate::webp::enc::picture_csp_enc::webp_picture_yuva_to_argb;
use crate::webp::enc::picture_enc::{
    webp_picture_alloc, webp_picture_init, webp_picture_set_memory_writer,
    webp_picture_take_written,
};
use crate::webp::enc::webp_enc::webp_encode;
use crate::webp::encode::{WebPConfig, WebPEncodingError, WebPPicture};
use crate::webp::mux::muxedit::{
    webp_mux_assemble, webp_mux_new, webp_mux_push_frame, webp_mux_set_animation_params,
    webp_mux_set_canvas_size, webp_mux_set_image,
};
use crate::webp::mux::muxread::{webp_mux_create, webp_mux_get_canvas_size, webp_mux_get_frame};
use crate::webp::mux::{
    WebPAnimEncoderOptions, WebPChunkId, WebPMux, WebPMuxAnimParams, WebPMuxError, WebPMuxFrameInfo,
};
use crate::webp::utils::{webp_copy_pixels, webp_get_color_palette};

//------------------------------------------------------------------------------
// Internal structs.

/// Stores frame rectangle dimensions. Translation of `FrameRectangle`.
#[derive(Clone, Copy, Default, Debug)]
struct FrameRectangle {
    x_offset: i32,
    y_offset: i32,
    width: i32,
    height: i32,
}

/// Used to store two candidates of encoded data for an animation frame. One of
/// the two will be chosen later. Translation of `EncodedFrame`.
#[derive(Clone, Default, Debug)]
struct EncodedFrame {
    /// Encoded frame rectangle.
    sub_frame: WebPMuxFrameInfo,
    /// Encoded frame if it is a key-frame.
    key_frame: WebPMuxFrameInfo,
    /// True if 'key_frame' has been chosen.
    is_key_frame: bool,
}

/// Translation of `struct WebPAnimEncoder`.
pub(crate) struct WebPAnimEncoder {
    /// Canvas width.
    canvas_width: i32,
    /// Canvas height.
    canvas_height: i32,
    /// Global encoding options.
    options: WebPAnimEncoderOptions,

    /// Previous WebP frame rectangle.
    prev_rect: FrameRectangle,
    /// Cached in case a re-encode is needed.
    last_config: WebPConfig,
    /// If 'last_config_' uses lossless, then
    /// this config uses lossy and vice versa;
    /// only valid if 'options_.allow_mixed'
    /// is true.
    last_config_reversed: WebPConfig,

    // Canvas buffers.
    /// Possibly modified current canvas.
    curr_canvas_copy: WebPPicture,
    /// True if pixels in 'curr_canvas_copy_'
    /// differ from those in 'curr_canvas_'.
    curr_canvas_copy_modified: bool,

    /// Previous canvas.
    prev_canvas: WebPPicture,
    /// Previous canvas disposed to background.
    prev_canvas_disposed: WebPPicture,

    // Encoded data.
    /// Array of encoded frames.
    encoded_frames: Vec<EncodedFrame>,
    /// Number of allocated frames.
    size: usize,
    /// Frame start index.
    start: usize,
    /// Number of valid frames.
    count: usize,
    /// If >0, 'flush_count' frames starting from
    /// 'start' are ready to be added to mux.
    flush_count: usize,

    // key-frame related.
    /// min(canvas size - frame size) over the frames.
    /// Can be negative in certain cases due to
    /// transparent pixels in a frame.
    best_delta: i64,
    /// Index of selected key-frame relative to 'start_'.
    keyframe: i32,
    /// Frames seen since the last key-frame.
    count_since_key_frame: i32,

    /// Timestamp of the first frame.
    first_timestamp: i32,
    /// Timestamp of the last added frame.
    prev_timestamp: i32,
    /// True if it's not yet decided if previous
    /// frame would be a sub-frame or a key-frame.
    prev_candidate_undecided: bool,

    // Misc.
    /// True if first frame is yet to be added/being added.
    is_first_frame: bool,
    /// True if WebPAnimEncoderAdd() has already been called
    /// with a NULL frame.
    got_null_frame: bool,

    /// Number of input frames processed so far.
    in_frame_count: usize,
    /// Number of frames added to mux so far. This may be
    /// different from 'in_frame_count_' due to merging.
    out_frame_count: usize,

    /// Muxer to assemble the WebP bitstream.
    mux: WebPMux,
    /// Error string. Empty if no error.
    error_str: String,
}

// -----------------------------------------------------------------------------
// Life of WebPAnimEncoder object.

const DELTA_INFINITY: i64 = 1 << 32;
const KEYFRAME_NONE: i32 = -1;

/// Reset the counters in the WebPAnimEncoder. Translation of
/// `ResetCounters()`.
fn reset_counters(enc: &mut WebPAnimEncoder) {
    enc.start = 0;
    enc.count = 0;
    enc.flush_count = 0;
    enc.best_delta = DELTA_INFINITY;
    enc.keyframe = KEYFRAME_NONE;
}

/// Translation of `DisableKeyframes()`.
fn disable_keyframes(enc_options: &mut WebPAnimEncoderOptions) {
    enc_options.kmax = i32::MAX;
    enc_options.kmin = enc_options.kmax - 1;
}

/// Translation of `DefaultEncoderOptions()`.
fn default_encoder_options(enc_options: &mut WebPAnimEncoderOptions) {
    enc_options.anim_params.loop_count = 0;
    enc_options.anim_params.bgcolor = 0xffffffff; // White.
    enc_options.minimize_size = false;
    disable_keyframes(enc_options);
    enc_options.allow_mixed = false;
    enc_options.verbose = false;
}

/// This starting value is more fit to WebPCleanupTransparentAreaLossless().
const TRANSPARENT_COLOR: u32 = 0x00000000;

/// Translation of `ClearRectangle()`.
fn clear_rectangle(picture: &mut WebPPicture, left: i32, top: i32, width: i32, height: i32) {
    let stride = picture.argb_stride as usize;
    for j in top as usize..(top + height) as usize {
        picture.argb[j * stride + left as usize..j * stride + (left + width) as usize]
            .fill(TRANSPARENT_COLOR);
    }
}

/// Translation of `WebPUtilClearPic()`.
fn webp_util_clear_pic(picture: &mut WebPPicture, rect: Option<&FrameRectangle>) {
    if let Some(rect) = rect {
        clear_rectangle(
            picture,
            rect.x_offset,
            rect.y_offset,
            rect.width,
            rect.height,
        );
    } else {
        let (w, h) = (picture.width, picture.height);
        clear_rectangle(picture, 0, 0, w, h);
    }
}

/// Translation of `MarkNoError()`.
fn mark_no_error(enc: &mut WebPAnimEncoder) {
    enc.error_str.clear(); // Empty string.
}

/// Translation of `MarkError()`.
fn mark_error(enc: &mut WebPAnimEncoder, s: &str) {
    enc.error_str = format!("{s}.");
}

/// Translation of `MarkError2()`.
fn mark_error2(enc: &mut WebPAnimEncoder, s: &str, error_code: i32) {
    enc.error_str = format!("{s}: {error_code}.");
}

/// Translation of `WebPPictureCopy()` for an ARGB picture: a new
/// picture with `src`'s specs and pixels.
fn webp_picture_copy(src: &WebPPicture) -> Option<WebPPicture> {
    let mut dst = WebPPicture {
        use_argb: src.use_argb,
        colorspace: src.colorspace,
        width: src.width,
        height: src.height,
        ..WebPPicture::default()
    };
    debug_assert!(src.use_argb);
    if !webp_picture_alloc(&mut dst) {
        return None;
    }
    webp_copy_pixels(src, &mut dst);
    Some(dst)
}

/// Translation of `WebPAnimEncoderNew()` (`WebPAnimEncoderNewInternal()`)
/// with the default options (SDL_image passes none).
pub(crate) fn webp_anim_encoder_new(width: i32, height: i32) -> Option<Box<WebPAnimEncoder>> {
    if width <= 0 || height <= 0 || (width as u64 * height as u64) >= MAX_IMAGE_AREA {
        return None;
    }

    // Dimensions and options.
    let mut options = WebPAnimEncoderOptions::default();
    default_encoder_options(&mut options);

    // Canvas buffers.
    let mut curr_canvas_copy = WebPPicture::default();
    if !webp_picture_init(&mut curr_canvas_copy) {
        return None;
    }
    curr_canvas_copy.width = width;
    curr_canvas_copy.height = height;
    curr_canvas_copy.use_argb = true;
    if !webp_picture_alloc(&mut curr_canvas_copy) {
        return None;
    }
    let mut prev_canvas = webp_picture_copy(&curr_canvas_copy)?;
    let prev_canvas_disposed = webp_picture_copy(&curr_canvas_copy)?;
    webp_util_clear_pic(&mut prev_canvas, None);

    let mut enc = Box::new(WebPAnimEncoder {
        canvas_width: width,
        canvas_height: height,
        options,
        prev_rect: FrameRectangle::default(),
        last_config: WebPConfig::default(),
        last_config_reversed: WebPConfig::default(),
        curr_canvas_copy,
        curr_canvas_copy_modified: true,
        prev_canvas,
        prev_canvas_disposed,
        encoded_frames: Vec::new(),
        size: 0,
        start: 0,
        count: 0,
        flush_count: 0,
        best_delta: 0,
        keyframe: 0,
        count_since_key_frame: 0,
        first_timestamp: 0,
        prev_timestamp: 0,
        prev_candidate_undecided: false,
        is_first_frame: true,
        got_null_frame: false,
        in_frame_count: 0,
        out_frame_count: 0,
        mux: webp_mux_new(),
        error_str: String::new(),
    });
    mark_no_error(&mut enc);

    // Encoded frames.
    reset_counters(&mut enc);
    // Note: one extra storage is for the previous frame.
    enc.size = (enc.options.kmax - enc.options.kmin + 1) as usize;
    // We need space for at least 2 frames. But when kmin, kmax are both zero,
    // enc->size_ will be 1. So we handle that special case below.
    if enc.size < 2 {
        enc.size = 2;
    }
    enc.encoded_frames = vec![EncodedFrame::default(); enc.size];

    Some(enc) // All OK.
}

/// Release the data contained by 'encoded_frame'. Translation of
/// `FrameRelease()`.
fn frame_release(encoded_frame: &mut EncodedFrame) {
    *encoded_frame = EncodedFrame::default();
}

// -----------------------------------------------------------------------------
// Frame addition.

/// Returns cached frame at the given 'position' (its index). Translation
/// of `GetFrame()`.
fn get_frame(enc: &WebPAnimEncoder, position: usize) -> usize {
    debug_assert!(enc.start + position < enc.size);
    enc.start + position
}

/// Returns true if 'length' number of pixels in 'src' and 'dst' are equal,
/// assuming the given step sizes between pixels.
/// 'max_allowed_diff' is unused and only there to allow function pointer use.
/// Translation of `ComparePixelsLossless()`: the pixels from `src[s]` and
/// `dst[d]`.
#[allow(clippy::too_many_arguments)]
fn compare_pixels_lossless(
    src: &[u32],
    s: usize,
    src_step: usize,
    dst: &[u32],
    d: usize,
    dst_step: usize,
    length: i32,
    _max_allowed_diff: i32,
) -> bool {
    debug_assert!(length > 0);
    (0..length as usize).all(|k| src[s + k * src_step] == dst[d + k * dst_step])
}

/// Helper to check if each channel in 'src' and 'dst' is at most off by
/// 'max_allowed_diff'. Translation of `PixelsAreSimilar()`.
fn pixels_are_similar(src: u32, dst: u32, max_allowed_diff: i32) -> bool {
    let src_a = ((src >> 24) & 0xff) as i32;
    let src_r = ((src >> 16) & 0xff) as i32;
    let src_g = ((src >> 8) & 0xff) as i32;
    let src_b = (src & 0xff) as i32;
    let dst_a = ((dst >> 24) & 0xff) as i32;
    let dst_r = ((dst >> 16) & 0xff) as i32;
    let dst_g = ((dst >> 8) & 0xff) as i32;
    let dst_b = (dst & 0xff) as i32;

    (src_a == dst_a)
        && ((src_r - dst_r).abs() * dst_a <= (max_allowed_diff * 255))
        && ((src_g - dst_g).abs() * dst_a <= (max_allowed_diff * 255))
        && ((src_b - dst_b).abs() * dst_a <= (max_allowed_diff * 255))
}

/// Returns true if 'length' number of pixels in 'src' and 'dst' are within an
/// error bound, assuming the given step sizes between pixels.
/// Translation of `ComparePixelsLossy()`.
#[allow(clippy::too_many_arguments)]
fn compare_pixels_lossy(
    src: &[u32],
    s: usize,
    src_step: usize,
    dst: &[u32],
    d: usize,
    dst_step: usize,
    length: i32,
    max_allowed_diff: i32,
) -> bool {
    debug_assert!(length > 0);
    (0..length as usize).all(|k| {
        pixels_are_similar(
            src[s + k * src_step],
            dst[d + k * dst_step],
            max_allowed_diff,
        )
    })
}

/// Translation of `IsEmptyRect()`.
fn is_empty_rect(rect: &FrameRectangle) -> bool {
    (rect.width == 0) || (rect.height == 0)
}

/// Translation of `QualityToMaxDiff()`.
fn quality_to_max_diff(quality: f32) -> i32 {
    let val = (quality as f64 / 100.).powf(0.5);
    let max_diff = 31. * (1. - val) + 1. * val;
    (max_diff + 0.5) as i32
}

/// Assumes that an initial valid guess of change rectangle 'rect' is passed.
/// Translation of `MinimizeChangeRectangle()`.
fn minimize_change_rectangle(
    src: &WebPPicture,
    dst: &WebPPicture,
    rect: &mut FrameRectangle,
    is_lossless: bool,
    quality: f32,
) {
    let compare_pixels = if is_lossless {
        compare_pixels_lossless
    } else {
        compare_pixels_lossy
    };
    let max_allowed_diff_lossy = quality_to_max_diff(quality);
    let max_allowed_diff = if is_lossless {
        0
    } else {
        max_allowed_diff_lossy
    };
    let (ss, ds) = (src.argb_stride as usize, dst.argb_stride as usize);

    // Assumption/correctness checks.
    debug_assert!(src.width == dst.width && src.height == dst.height);
    debug_assert!(rect.x_offset + rect.width <= dst.width);
    debug_assert!(rect.y_offset + rect.height <= dst.height);

    'no_change: {
        // Left boundary.
        let mut i = rect.x_offset;
        while i < rect.x_offset + rect.width {
            let s = rect.y_offset as usize * ss + i as usize;
            let d = rect.y_offset as usize * ds + i as usize;
            if compare_pixels(
                &src.argb,
                s,
                ss,
                &dst.argb,
                d,
                ds,
                rect.height,
                max_allowed_diff,
            ) {
                rect.width -= 1; // Redundant column.
                rect.x_offset += 1;
            } else {
                break;
            }
            i += 1;
        }
        if rect.width == 0 {
            break 'no_change;
        }

        // Right boundary.
        let mut i = rect.x_offset + rect.width - 1;
        while i >= rect.x_offset {
            let s = rect.y_offset as usize * ss + i as usize;
            let d = rect.y_offset as usize * ds + i as usize;
            if compare_pixels(
                &src.argb,
                s,
                ss,
                &dst.argb,
                d,
                ds,
                rect.height,
                max_allowed_diff,
            ) {
                rect.width -= 1; // Redundant column.
            } else {
                break;
            }
            i -= 1;
        }
        if rect.width == 0 {
            break 'no_change;
        }

        // Top boundary.
        let mut j = rect.y_offset;
        while j < rect.y_offset + rect.height {
            let s = j as usize * ss + rect.x_offset as usize;
            let d = j as usize * ds + rect.x_offset as usize;
            if compare_pixels(
                &src.argb,
                s,
                1,
                &dst.argb,
                d,
                1,
                rect.width,
                max_allowed_diff,
            ) {
                rect.height -= 1; // Redundant row.
                rect.y_offset += 1;
            } else {
                break;
            }
            j += 1;
        }
        if rect.height == 0 {
            break 'no_change;
        }

        // Bottom boundary.
        let mut j = rect.y_offset + rect.height - 1;
        while j >= rect.y_offset {
            let s = j as usize * ss + rect.x_offset as usize;
            let d = j as usize * ds + rect.x_offset as usize;
            if compare_pixels(
                &src.argb,
                s,
                1,
                &dst.argb,
                d,
                1,
                rect.width,
                max_allowed_diff,
            ) {
                rect.height -= 1; // Redundant row.
            } else {
                break;
            }
            j -= 1;
        }
        if rect.height == 0 {
            break 'no_change;
        }

        if !is_empty_rect(rect) {
            return;
        }
    }
    // NoChange:
    rect.x_offset = 0;
    rect.y_offset = 0;
    rect.width = 0;
    rect.height = 0;
}

/// Snap rectangle to even offsets (and adjust dimensions if needed).
/// Translation of `SnapToEvenOffsets()`.
fn snap_to_even_offsets(rect: &mut FrameRectangle) {
    rect.width += rect.x_offset & 1;
    rect.height += rect.y_offset & 1;
    rect.x_offset &= !1;
    rect.y_offset &= !1;
}

/// Translation of `SubFrameParams` (the sub-frame pictures are the views
/// of their rectangles).
#[derive(Clone, Copy, Default)]
struct SubFrameParams {
    /// Should try this set of parameters.
    should_try: bool,
    /// Frame with empty rectangle can be skipped.
    empty_rect_allowed: bool,
    /// Frame rectangle for lossless compression.
    rect_ll: FrameRectangle,
    /// Frame rectangle for lossy compression.
    /// Could be smaller than rect_ll_ as pixels
    /// with small diffs can be ignored.
    rect_lossy: FrameRectangle,
}

/// Translation of `SubFrameParamsInit()`.
fn sub_frame_params_init(should_try: bool, empty_rect_allowed: bool) -> SubFrameParams {
    SubFrameParams {
        should_try,
        empty_rect_allowed,
        ..Default::default()
    }
}

/// Translation of `AdjustAndCheckRectangle()` for an ARGB picture: whether
/// `WebPPictureView()` can view the rectangle.
fn view_is_valid(pic: &WebPPicture, left: i32, top: i32, width: i32, height: i32) -> bool {
    debug_assert!(pic.use_argb);
    if left < 0 || top < 0 {
        return false;
    }
    if width <= 0 || height <= 0 {
        return false;
    }
    if left + width > pic.width {
        return false;
    }
    if top + height > pic.height {
        return false;
    }
    true
}

/// Given previous and current canvas, picks the optimal rectangle for the
/// current frame based on 'is_lossless' and other parameters. Assumes that the
/// initial guess 'rect' is valid. Translation of `GetSubRect()`.
#[allow(clippy::too_many_arguments)]
fn get_sub_rect(
    prev_canvas: &WebPPicture,
    curr_canvas: &WebPPicture,
    is_key_frame: bool,
    is_first_frame: bool,
    empty_rect_allowed: bool,
    is_lossless: bool,
    quality: f32,
    rect: &mut FrameRectangle,
) -> bool {
    if !is_key_frame || is_first_frame {
        // Optimize frame rectangle.
        // Note: This behaves as expected for first frame, as 'prev_canvas' is
        // initialized to a fully transparent canvas in the beginning.
        minimize_change_rectangle(prev_canvas, curr_canvas, rect, is_lossless, quality);
    }

    if is_empty_rect(rect) {
        if empty_rect_allowed {
            // No need to get 'sub_frame'.
            return true;
        } else {
            // Force a 1x1 rectangle.
            rect.width = 1;
            rect.height = 1;
            debug_assert!(rect.x_offset == 0);
            debug_assert!(rect.y_offset == 0);
        }
    }

    snap_to_even_offsets(rect);
    view_is_valid(
        curr_canvas,
        rect.x_offset,
        rect.y_offset,
        rect.width,
        rect.height,
    )
}

/// Picks optimal frame rectangle for both lossless and lossy compression. The
/// initial guess for frame rectangles will be the full canvas.
/// Translation of `GetSubRects()`.
fn get_sub_rects(
    prev_canvas: &WebPPicture,
    curr_canvas: &WebPPicture,
    is_key_frame: bool,
    is_first_frame: bool,
    quality: f32,
    params: &mut SubFrameParams,
) -> bool {
    // Lossless frame rectangle.
    params.rect_ll.x_offset = 0;
    params.rect_ll.y_offset = 0;
    params.rect_ll.width = curr_canvas.width;
    params.rect_ll.height = curr_canvas.height;
    if !get_sub_rect(
        prev_canvas,
        curr_canvas,
        is_key_frame,
        is_first_frame,
        params.empty_rect_allowed,
        true,
        quality,
        &mut params.rect_ll,
    ) {
        return false;
    }
    // Lossy frame rectangle.
    params.rect_lossy = params.rect_ll; // seed with lossless rect.
    get_sub_rect(
        prev_canvas,
        curr_canvas,
        is_key_frame,
        is_first_frame,
        params.empty_rect_allowed,
        false,
        quality,
        &mut params.rect_lossy,
    )
}

/// Translation of `DisposeFrameRectangle()`.
fn dispose_frame_rectangle(
    dispose_method: WebPMuxAnimDispose,
    rect: &FrameRectangle,
    curr_canvas: &mut WebPPicture,
) {
    if dispose_method == WebPMuxAnimDispose::Background {
        webp_util_clear_pic(curr_canvas, Some(rect));
    }
}

/// Translation of `RectArea()`.
fn rect_area(rect: &FrameRectangle) -> u32 {
    (rect.width as u32).wrapping_mul(rect.height as u32)
}

/// Translation of `IsLosslessBlendingPossible()`.
fn is_lossless_blending_possible(
    src: &WebPPicture,
    dst: &WebPPicture,
    rect: &FrameRectangle,
) -> bool {
    debug_assert!(src.width == dst.width && src.height == dst.height);
    debug_assert!(rect.x_offset + rect.width <= dst.width);
    debug_assert!(rect.y_offset + rect.height <= dst.height);
    let (ss, ds) = (src.argb_stride as usize, dst.argb_stride as usize);
    for j in rect.y_offset as usize..(rect.y_offset + rect.height) as usize {
        for i in rect.x_offset as usize..(rect.x_offset + rect.width) as usize {
            let src_pixel = src.argb[j * ss + i];
            let dst_pixel = dst.argb[j * ds + i];
            let dst_alpha = dst_pixel >> 24;
            if dst_alpha != 0xff && src_pixel != dst_pixel {
                // In this case, if we use blending, we can't attain the desired
                // 'dst_pixel' value for this pixel. So, blending is not possible.
                return false;
            }
        }
    }
    true
}

/// Translation of `IsLossyBlendingPossible()`.
fn is_lossy_blending_possible(
    src: &WebPPicture,
    dst: &WebPPicture,
    rect: &FrameRectangle,
    quality: f32,
) -> bool {
    let max_allowed_diff_lossy = quality_to_max_diff(quality);
    debug_assert!(src.width == dst.width && src.height == dst.height);
    debug_assert!(rect.x_offset + rect.width <= dst.width);
    debug_assert!(rect.y_offset + rect.height <= dst.height);
    let (ss, ds) = (src.argb_stride as usize, dst.argb_stride as usize);
    for j in rect.y_offset as usize..(rect.y_offset + rect.height) as usize {
        for i in rect.x_offset as usize..(rect.x_offset + rect.width) as usize {
            let src_pixel = src.argb[j * ss + i];
            let dst_pixel = dst.argb[j * ds + i];
            let dst_alpha = dst_pixel >> 24;
            if dst_alpha != 0xff
                && !pixels_are_similar(src_pixel, dst_pixel, max_allowed_diff_lossy)
            {
                // In this case, if we use blending, we can't attain the desired
                // 'dst_pixel' value for this pixel. So, blending is not possible.
                return false;
            }
        }
    }
    true
}

/// For pixels in 'rect', replace those pixels in 'dst' that are same as 'src' by
/// transparent pixels.
/// Returns true if at least one pixel gets modified.
/// Translation of `IncreaseTransparency()`.
fn increase_transparency(src: &WebPPicture, rect: &FrameRectangle, dst: &mut WebPPicture) -> bool {
    let mut modified = false;
    debug_assert!(src.width == dst.width && src.height == dst.height);
    let (ss, ds) = (src.argb_stride as usize, dst.argb_stride as usize);
    for j in rect.y_offset as usize..(rect.y_offset + rect.height) as usize {
        for i in rect.x_offset as usize..(rect.x_offset + rect.width) as usize {
            let psrc = src.argb[j * ss + i];
            let pdst = &mut dst.argb[j * ds + i];
            if psrc == *pdst && *pdst != TRANSPARENT_COLOR {
                *pdst = TRANSPARENT_COLOR;
                modified = true;
            }
        }
    }
    modified
}

/// Replace similar blocks of pixels by a 'see-through' transparent block
/// with uniform average color.
/// Assumes lossy compression is being used.
/// Returns true if at least one pixel gets modified.
/// Translation of `FlattenSimilarBlocks()`.
fn flatten_similar_blocks(
    src: &WebPPicture,
    rect: &FrameRectangle,
    dst: &mut WebPPicture,
    quality: f32,
) -> bool {
    let max_allowed_diff_lossy = quality_to_max_diff(quality);
    let mut modified = false;
    const BLOCK_SIZE: i32 = 8;
    let y_start = (rect.y_offset + BLOCK_SIZE) & !(BLOCK_SIZE - 1);
    let y_end = (rect.y_offset + rect.height) & !(BLOCK_SIZE - 1);
    let x_start = (rect.x_offset + BLOCK_SIZE) & !(BLOCK_SIZE - 1);
    let x_end = (rect.x_offset + rect.width) & !(BLOCK_SIZE - 1);
    debug_assert!(src.width == dst.width && src.height == dst.height);
    let (ss, ds) = (src.argb_stride as usize, dst.argb_stride as usize);
    let bs = BLOCK_SIZE as usize;
    // Iterate over each block and count similar pixels.
    let mut j = y_start;
    while j < y_end {
        let mut i = x_start;
        while i < x_end {
            let mut cnt = 0;
            let mut avg_r = 0;
            let mut avg_g = 0;
            let mut avg_b = 0;
            let psrc = j as usize * ss + i as usize;
            let pdst = j as usize * ds + i as usize;
            for y in 0..bs {
                for x in 0..bs {
                    let src_pixel = src.argb[psrc + x + y * ss];
                    let alpha = (src_pixel >> 24) as i32;
                    if alpha == 0xff
                        && pixels_are_similar(
                            src_pixel,
                            dst.argb[pdst + x + y * ds],
                            max_allowed_diff_lossy,
                        )
                    {
                        cnt += 1;
                        avg_r += ((src_pixel >> 16) & 0xff) as i32;
                        avg_g += ((src_pixel >> 8) & 0xff) as i32;
                        avg_b += (src_pixel & 0xff) as i32;
                    }
                }
            }
            // If we have a fully similar block, we replace it with an
            // average transparent block. This compresses better in lossy mode.
            if cnt == BLOCK_SIZE * BLOCK_SIZE {
                let color: u32 = (((avg_r / cnt) as u32) << 16)
                    | (((avg_g / cnt) as u32) << 8)
                    | ((avg_b / cnt) as u32); // (alpha 0x00)
                for y in 0..bs {
                    for x in 0..bs {
                        dst.argb[pdst + x + y * ds] = color;
                    }
                }
                modified = true;
            }
            i += BLOCK_SIZE;
        }
        j += BLOCK_SIZE;
    }
    modified
}

/// Translation of `EncodeFrame()`: the encoded bytes are in `memory`.
fn encode_frame(config: &WebPConfig, pic: &mut WebPPicture, memory: &mut Vec<u8>) -> bool {
    pic.use_argb = true;
    webp_picture_set_memory_writer(pic);
    let ok = webp_encode(config, pic);
    *memory = webp_picture_take_written(pic);
    ok
}

/// Struct representing a candidate encoded frame including its metadata.
/// Translation of `Candidate`.
#[derive(Clone, Default)]
struct Candidate {
    mem: Vec<u8>,
    info: WebPMuxFrameInfo,
    rect: FrameRectangle,
    /// True if this candidate should be evaluated.
    evaluate: bool,
}

/// Generates a candidate encoded frame given a picture and metadata.
/// Translation of `EncodeCandidate()`: the sub-frame is the view of
/// `rect` in the canvas copy (`WebPPictureView()`), whose pixels the
/// encoding may change.
fn encode_candidate(
    curr_canvas_copy: &mut WebPPicture,
    rect: &FrameRectangle,
    encoder_config: &WebPConfig,
    use_blending: bool,
    candidate: &mut Candidate,
) -> WebPEncodingError {
    let mut config = *encoder_config;
    let mut error_code = WebPEncodingError::Ok;
    *candidate = Candidate::default();

    // Set frame rect and info.
    candidate.rect = *rect;
    candidate.info.id = WebPChunkId::Anmf;
    candidate.info.x_offset = rect.x_offset;
    candidate.info.y_offset = rect.y_offset;
    candidate.info.dispose_method = WebPMuxAnimDispose::None; // Set later.
    candidate.info.blend_method = if use_blending {
        WebPMuxAnimBlend::Blend
    } else {
        WebPMuxAnimBlend::NoBlend
    };
    candidate.info.duration = 0; // Set in next call to WebPAnimEncoderAdd().

    // Encode picture.
    // (the view: PictureGrabSpecs() of the canvas copy, its pixels at rect)
    let (x, y) = (rect.x_offset as usize, rect.y_offset as usize);
    let (w, h) = (rect.width as usize, rect.height as usize);
    let stride = curr_canvas_copy.argb_stride as usize;
    let mut sub_frame = WebPPicture {
        use_argb: curr_canvas_copy.use_argb,
        colorspace: curr_canvas_copy.colorspace,
        width: rect.width,
        height: rect.height,
        argb_stride: rect.width,
        ..WebPPicture::default()
    };
    sub_frame.argb = Vec::with_capacity(w * h);
    for j in 0..h {
        let row = (y + j) * stride + x;
        sub_frame
            .argb
            .extend_from_slice(&curr_canvas_copy.argb[row..row + w]);
    }

    if config.lossless == 0 && use_blending {
        // Disable filtering to avoid blockiness in reconstructed frames at the
        // time of decoding.
        config.autofilter = 0;
        config.filter_strength = 0;
    }
    let ok = encode_frame(&config, &mut sub_frame, &mut candidate.mem);
    // (the view's pixels, back in the canvas copy)
    for j in 0..h {
        let row = (y + j) * stride + x;
        curr_canvas_copy.argb[row..row + w].copy_from_slice(&sub_frame.argb[j * w..j * w + w]);
    }
    if !ok {
        error_code = sub_frame.error_code.get();
        // Err:
        candidate.mem = Vec::new();
        return error_code;
    }

    candidate.evaluate = true;
    error_code
}

/// Translation of `CopyCurrentCanvas()`.
fn copy_current_canvas(enc: &mut WebPAnimEncoder, curr_canvas: &WebPPicture) {
    if enc.curr_canvas_copy_modified {
        webp_copy_pixels(curr_canvas, &mut enc.curr_canvas_copy);
        enc.curr_canvas_copy_modified = false;
    }
}

const LL_DISP_NONE: usize = 0;
const LL_DISP_BG: usize = 1;
const LOSSY_DISP_NONE: usize = 2;
const LOSSY_DISP_BG: usize = 3;
const CANDIDATE_COUNT: usize = 4;

/// Don't try lossy below this threshold.
const MIN_COLORS_LOSSY: i32 = 31;
/// Don't try lossless above this threshold.
const MAX_COLORS_LOSSLESS: i32 = 194;

/// Generates candidates for a given dispose method given pre-filled sub-frame
/// 'params'. Translation of `GenerateCandidates()`.
#[allow(clippy::too_many_arguments)]
fn generate_candidates(
    enc: &mut WebPAnimEncoder,
    curr_canvas_frame: &WebPPicture,
    candidates: &mut [Candidate; CANDIDATE_COUNT],
    dispose_method: WebPMuxAnimDispose,
    is_lossless: bool,
    is_key_frame: bool,
    params: &SubFrameParams,
    config_ll: &WebPConfig,
    config_lossy: &WebPConfig,
) -> WebPEncodingError {
    let mut error_code = WebPEncodingError::Ok;
    let is_dispose_none = dispose_method == WebPMuxAnimDispose::None;
    let candidate_ll = if is_dispose_none {
        LL_DISP_NONE
    } else {
        LL_DISP_BG
    };
    let candidate_lossy = if is_dispose_none {
        LOSSY_DISP_NONE
    } else {
        LOSSY_DISP_BG
    };
    // curr_canvas = &enc->curr_canvas_copy_; prev_canvas = &enc->prev_canvas_
    // or &enc->prev_canvas_disposed_
    copy_current_canvas(enc, curr_canvas_frame);
    let prev_canvas = if is_dispose_none {
        &enc.prev_canvas
    } else {
        &enc.prev_canvas_disposed
    };
    let curr_canvas = &enc.curr_canvas_copy;
    let use_blending_ll =
        !is_key_frame && is_lossless_blending_possible(prev_canvas, curr_canvas, &params.rect_ll);
    let use_blending_lossy = !is_key_frame
        && is_lossy_blending_possible(
            prev_canvas,
            curr_canvas,
            &params.rect_lossy,
            config_lossy.quality,
        );

    // Pick candidates to be tried.
    let (evaluate_ll, evaluate_lossy) = if !enc.options.allow_mixed {
        (is_lossless, !is_lossless)
    } else if enc.options.minimize_size {
        (true, true)
    } else {
        // Use a heuristic for trying lossless and/or lossy compression.
        let rect = &params.rect_ll;
        let (x, y, w, h) = (
            rect.x_offset as usize,
            rect.y_offset as usize,
            rect.width as usize,
            rect.height as usize,
        );
        let stride = curr_canvas.argb_stride as usize;
        let mut sub_frame_ll = WebPPicture {
            use_argb: true,
            width: rect.width,
            height: rect.height,
            argb_stride: rect.width,
            ..WebPPicture::default()
        };
        for j in 0..h {
            let row = (y + j) * stride + x;
            sub_frame_ll
                .argb
                .extend_from_slice(&curr_canvas.argb[row..row + w]);
        }
        let num_colors = webp_get_color_palette(&sub_frame_ll, None);
        (
            num_colors < MAX_COLORS_LOSSLESS,
            num_colors >= MIN_COLORS_LOSSY,
        )
    };

    // Generate candidates.
    if evaluate_ll {
        copy_current_canvas(enc, curr_canvas_frame);
        if use_blending_ll {
            let prev_canvas = if is_dispose_none {
                &enc.prev_canvas
            } else {
                &enc.prev_canvas_disposed
            };
            enc.curr_canvas_copy_modified =
                increase_transparency(prev_canvas, &params.rect_ll, &mut enc.curr_canvas_copy);
        }
        error_code = encode_candidate(
            &mut enc.curr_canvas_copy,
            &params.rect_ll,
            config_ll,
            use_blending_ll,
            &mut candidates[candidate_ll],
        );
        if error_code != WebPEncodingError::Ok {
            return error_code;
        }
    }
    if evaluate_lossy {
        copy_current_canvas(enc, curr_canvas_frame);
        if use_blending_lossy {
            let prev_canvas = if is_dispose_none {
                &enc.prev_canvas
            } else {
                &enc.prev_canvas_disposed
            };
            enc.curr_canvas_copy_modified = flatten_similar_blocks(
                prev_canvas,
                &params.rect_lossy,
                &mut enc.curr_canvas_copy,
                config_lossy.quality,
            );
        }
        error_code = encode_candidate(
            &mut enc.curr_canvas_copy,
            &params.rect_lossy,
            config_lossy,
            use_blending_lossy,
            &mut candidates[candidate_lossy],
        );
        if error_code != WebPEncodingError::Ok {
            return error_code;
        }
        enc.curr_canvas_copy_modified = true;
    }
    error_code
}

/// Sets dispose method of the previous frame to be 'dispose_method'.
/// Translation of `SetPreviousDisposeMethod()`.
fn set_previous_dispose_method(enc: &mut WebPAnimEncoder, dispose_method: WebPMuxAnimDispose) {
    debug_assert!(enc.count >= 2); // As current and previous frames are in enc.
    let position = enc.count - 2;
    let prev = get_frame(enc, position);
    let prev_enc_frame = &mut enc.encoded_frames[prev];

    if enc.prev_candidate_undecided {
        debug_assert!(dispose_method == WebPMuxAnimDispose::None);
        prev_enc_frame.sub_frame.dispose_method = dispose_method;
        prev_enc_frame.key_frame.dispose_method = dispose_method;
    } else {
        let prev_info = if prev_enc_frame.is_key_frame {
            &mut prev_enc_frame.key_frame
        } else {
            &mut prev_enc_frame.sub_frame
        };
        prev_info.dispose_method = dispose_method;
    }
}

/// Translation of `IncreasePreviousDuration()`.
fn increase_previous_duration(enc: &mut WebPAnimEncoder, duration: i32) -> bool {
    debug_assert!(enc.count >= 1);
    let position = enc.count - 1;
    let prev = get_frame(enc, position);

    {
        let prev_enc_frame = &enc.encoded_frames[prev];
        debug_assert!(
            !prev_enc_frame.is_key_frame
                || prev_enc_frame.sub_frame.duration == prev_enc_frame.key_frame.duration
        );
        debug_assert!(
            prev_enc_frame.sub_frame.duration
                == (prev_enc_frame.sub_frame.duration & (MAX_DURATION as i32 - 1))
        );
    }
    debug_assert!(duration == (duration & (MAX_DURATION as i32 - 1)));

    let new_duration = enc.encoded_frames[prev].sub_frame.duration + duration;
    if new_duration >= MAX_DURATION as i32 {
        // Special case.
        // Separate out previous frame from earlier merged frames to avoid overflow.
        // We add a 1x1 transparent frame for the previous frame, with blending on.
        let rect = FrameRectangle {
            x_offset: 0,
            y_offset: 0,
            width: 1,
            height: 1,
        };
        const LOSSLESS_1X1_BYTES: [u8; 28] = [
            0x52, 0x49, 0x46, 0x46, 0x14, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50,
            0x38, 0x4c, 0x08, 0x00, 0x00, 0x00, 0x2f, 0x00, 0x00, 0x00, 0x10, 0x88, 0x88, 0x08,
        ];
        const LOSSY_1X1_BYTES: [u8; 72] = [
            0x52, 0x49, 0x46, 0x46, 0x40, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50,
            0x38, 0x58, 0x0a, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x41, 0x4c, 0x50, 0x48, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x56, 0x50,
            0x38, 0x20, 0x18, 0x00, 0x00, 0x00, 0x30, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x01, 0x00,
            0x01, 0x00, 0x02, 0x00, 0x34, 0x25, 0xa4, 0x00, 0x03, 0x70, 0x00, 0xfe, 0xfb, 0xfd,
            0x50, 0x00,
        ];
        let can_use_lossless = enc.last_config.lossless != 0 || enc.options.allow_mixed;
        let curr = get_frame(enc, enc.count);
        let curr_enc_frame = &mut enc.encoded_frames[curr];
        curr_enc_frame.is_key_frame = false;
        curr_enc_frame.sub_frame.id = WebPChunkId::Anmf;
        curr_enc_frame.sub_frame.x_offset = 0;
        curr_enc_frame.sub_frame.y_offset = 0;
        curr_enc_frame.sub_frame.dispose_method = WebPMuxAnimDispose::None;
        curr_enc_frame.sub_frame.blend_method = WebPMuxAnimBlend::Blend;
        curr_enc_frame.sub_frame.duration = duration;
        curr_enc_frame.sub_frame.bitstream = if can_use_lossless {
            LOSSLESS_1X1_BYTES.to_vec()
        } else {
            LOSSY_1X1_BYTES.to_vec()
        };
        enc.count += 1;
        enc.count_since_key_frame += 1;
        enc.flush_count = enc.count - 1;
        enc.prev_candidate_undecided = false;
        enc.prev_rect = rect;
    } else {
        // Regular case.
        // Increase duration of the previous frame by 'duration'.
        let prev_enc_frame = &mut enc.encoded_frames[prev];
        prev_enc_frame.sub_frame.duration = new_duration;
        prev_enc_frame.key_frame.duration = new_duration;
    }
    true
}

/// Pick the candidate encoded frame with smallest size and release other
/// candidates.
/// TODO(later): Perhaps a rough SSIM/PSNR produced by the encoder should
/// also be a criteria, in addition to sizes.
/// Translation of `PickBestCandidate()`.
fn pick_best_candidate(
    enc: &mut WebPAnimEncoder,
    candidates: &mut [Candidate; CANDIDATE_COUNT],
    is_key_frame: bool,
    encoded_frame: usize,
) {
    let mut best_idx: i32 = -1;
    let mut best_size = usize::MAX;
    for (i, candidate) in candidates.iter().enumerate() {
        if candidate.evaluate {
            let candidate_size = candidate.mem.len();
            if candidate_size < best_size {
                best_idx = i as i32;
                best_size = candidate_size;
            }
        }
    }
    debug_assert!(best_idx != -1);
    for (i, candidate) in candidates.iter_mut().enumerate() {
        if candidate.evaluate {
            if i as i32 == best_idx {
                let mut info = candidate.info.clone();
                info.bitstream = std::mem::take(&mut candidate.mem);
                let frame = &mut enc.encoded_frames[encoded_frame];
                if is_key_frame {
                    frame.key_frame = info;
                } else {
                    frame.sub_frame = info;
                }
                if !is_key_frame {
                    // Note: Previous dispose method only matters for non-keyframes.
                    // Also, we don't want to modify previous dispose method that was
                    // selected when a non key-frame was assumed.
                    let prev_dispose_method = if i == LL_DISP_NONE || i == LOSSY_DISP_NONE {
                        WebPMuxAnimDispose::None
                    } else {
                        WebPMuxAnimDispose::Background
                    };
                    set_previous_dispose_method(enc, prev_dispose_method);
                }
                enc.prev_rect = candidate.rect; // save for next frame.
            } else {
                candidate.mem = Vec::new();
                candidate.evaluate = false;
            }
        }
    }
}

/// Depending on the configuration, tries different compressions
/// (lossy/lossless), dispose methods, blending methods etc to encode the current
/// frame and outputs the best one in 'encoded_frame'.
/// 'frame_skipped' will be set to true if this frame should actually be skipped.
/// Translation of `SetFrame()`: `encoded_frame` is the frame's index.
fn set_frame(
    enc: &mut WebPAnimEncoder,
    curr_canvas_frame: &WebPPicture,
    config: &WebPConfig,
    is_key_frame: bool,
    encoded_frame: usize,
    frame_skipped: &mut bool,
) -> WebPEncodingError {
    let mut error_code = WebPEncodingError::Ok;
    // curr_canvas = &enc->curr_canvas_copy_; prev_canvas = &enc->prev_canvas_
    let mut candidates: [Candidate; CANDIDATE_COUNT] = Default::default();
    let is_lossless = config.lossless != 0;
    let consider_lossless = is_lossless || enc.options.allow_mixed;
    let consider_lossy = !is_lossless || enc.options.allow_mixed;
    let is_first_frame = enc.is_first_frame;

    // First frame cannot be skipped as there is no 'previous frame' to merge it
    // to. So, empty rectangle is not allowed for the first frame.
    let empty_rect_allowed_none = !is_first_frame;

    // Even if there is exact pixel match between 'disposed previous canvas' and
    // 'current canvas', we can't skip current frame, as there may not be exact
    // pixel match between 'previous canvas' and 'current canvas'. So, we don't
    // allow empty rectangle in this case.
    let empty_rect_allowed_bg = false;

    // If current frame is a key-frame, dispose method of previous frame doesn't
    // matter, so we don't try dispose to background.
    // Also, if key-frame insertion is on, and previous frame could be picked as
    // either a sub-frame or a key-frame, then we can't be sure about what frame
    // rectangle would be disposed. In that case too, we don't try dispose to
    // background.
    let dispose_bg_possible = !is_key_frame && !enc.prev_candidate_undecided;

    let mut config_ll = *config;
    let mut config_lossy = *config;
    config_ll.lossless = 1;
    config_lossy.lossless = 0;
    enc.last_config = *config;
    enc.last_config_reversed = if config.lossless != 0 {
        config_lossy
    } else {
        config_ll
    };
    *frame_skipped = false;

    let mut dispose_none_params = sub_frame_params_init(true, empty_rect_allowed_none);
    let mut dispose_bg_params = sub_frame_params_init(false, empty_rect_allowed_bg);

    'end: {
        // Change-rectangle assuming previous frame was DISPOSE_NONE.
        if !get_sub_rects(
            &enc.prev_canvas,
            &enc.curr_canvas_copy,
            is_key_frame,
            is_first_frame,
            config_lossy.quality,
            &mut dispose_none_params,
        ) {
            error_code = WebPEncodingError::InvalidConfiguration;
            break 'end;
        }

        if (consider_lossless && is_empty_rect(&dispose_none_params.rect_ll))
            || (consider_lossy && is_empty_rect(&dispose_none_params.rect_lossy))
        {
            // Don't encode the frame at all. Instead, the duration of the previous
            // frame will be increased later.
            debug_assert!(empty_rect_allowed_none);
            *frame_skipped = true;
            break 'end;
        }

        if dispose_bg_possible {
            // Change-rectangle assuming previous frame was DISPOSE_BACKGROUND.
            webp_copy_pixels(&enc.prev_canvas, &mut enc.prev_canvas_disposed);
            let prev_rect = enc.prev_rect;
            dispose_frame_rectangle(
                WebPMuxAnimDispose::Background,
                &prev_rect,
                &mut enc.prev_canvas_disposed,
            );

            if !get_sub_rects(
                &enc.prev_canvas_disposed,
                &enc.curr_canvas_copy,
                is_key_frame,
                is_first_frame,
                config_lossy.quality,
                &mut dispose_bg_params,
            ) {
                error_code = WebPEncodingError::InvalidConfiguration;
                break 'end;
            }
            debug_assert!(!is_empty_rect(&dispose_bg_params.rect_ll));
            debug_assert!(!is_empty_rect(&dispose_bg_params.rect_lossy));

            if enc.options.minimize_size {
                // Try both dispose methods.
                dispose_bg_params.should_try = true;
                dispose_none_params.should_try = true;
            } else if (is_lossless
                && rect_area(&dispose_bg_params.rect_ll) < rect_area(&dispose_none_params.rect_ll))
                || (!is_lossless
                    && rect_area(&dispose_bg_params.rect_lossy)
                        < rect_area(&dispose_none_params.rect_lossy))
            {
                dispose_bg_params.should_try = true; // Pick DISPOSE_BACKGROUND.
                dispose_none_params.should_try = false;
            }
        }

        if dispose_none_params.should_try {
            error_code = generate_candidates(
                enc,
                curr_canvas_frame,
                &mut candidates,
                WebPMuxAnimDispose::None,
                is_lossless,
                is_key_frame,
                &dispose_none_params,
                &config_ll,
                &config_lossy,
            );
            if error_code != WebPEncodingError::Ok {
                break 'end; // (Err: the candidates are released)
            }
        }

        if dispose_bg_params.should_try {
            debug_assert!(!enc.is_first_frame);
            debug_assert!(dispose_bg_possible);
            error_code = generate_candidates(
                enc,
                curr_canvas_frame,
                &mut candidates,
                WebPMuxAnimDispose::Background,
                is_lossless,
                is_key_frame,
                &dispose_bg_params,
                &config_ll,
                &config_lossy,
            );
            if error_code != WebPEncodingError::Ok {
                break 'end; // (Err: the candidates are released)
            }
        }

        pick_best_candidate(enc, &mut candidates, is_key_frame, encoded_frame);
    }

    // End:
    error_code
}

/// Calculate the penalty incurred if we encode given frame as a key frame
/// instead of a sub-frame. Translation of `KeyFramePenalty()`.
fn key_frame_penalty(encoded_frame: &EncodedFrame) -> i64 {
    encoded_frame.key_frame.bitstream.len() as i64 - encoded_frame.sub_frame.bitstream.len() as i64
}

/// Translation of `CacheFrame()`.
fn cache_frame(enc: &mut WebPAnimEncoder, curr_canvas: &WebPPicture, config: &WebPConfig) -> bool {
    let mut ok = false;
    let mut frame_skipped = false;
    let mut error_code;
    let position = enc.count;
    let encoded_frame = get_frame(enc, position);

    enc.count += 1;

    'end: {
        'skip: {
            if enc.is_first_frame {
                // Add this as a key-frame.
                error_code = set_frame(
                    enc,
                    curr_canvas,
                    config,
                    true,
                    encoded_frame,
                    &mut frame_skipped,
                );
                if error_code != WebPEncodingError::Ok {
                    break 'end;
                }
                debug_assert!(!frame_skipped); // First frame can't be skipped, even if empty.
                debug_assert!(position == 0 && enc.count == 1);
                enc.encoded_frames[encoded_frame].is_key_frame = true;
                enc.flush_count = 0;
                enc.count_since_key_frame = 0;
                enc.prev_candidate_undecided = false;
            } else {
                enc.count_since_key_frame += 1;
                if enc.count_since_key_frame <= enc.options.kmin {
                    // Add this as a frame rectangle.
                    error_code = set_frame(
                        enc,
                        curr_canvas,
                        config,
                        false,
                        encoded_frame,
                        &mut frame_skipped,
                    );
                    if error_code != WebPEncodingError::Ok {
                        break 'end;
                    }
                    if frame_skipped {
                        break 'skip;
                    }
                    enc.encoded_frames[encoded_frame].is_key_frame = false;
                    enc.flush_count = enc.count - 1;
                    enc.prev_candidate_undecided = false;
                } else {
                    // Add this as a frame rectangle to enc.
                    error_code = set_frame(
                        enc,
                        curr_canvas,
                        config,
                        false,
                        encoded_frame,
                        &mut frame_skipped,
                    );
                    if error_code != WebPEncodingError::Ok {
                        break 'end;
                    }
                    if frame_skipped {
                        break 'skip;
                    }
                    let prev_rect_sub = enc.prev_rect;

                    // Add this as a key-frame to enc, too.
                    error_code = set_frame(
                        enc,
                        curr_canvas,
                        config,
                        true,
                        encoded_frame,
                        &mut frame_skipped,
                    );
                    if error_code != WebPEncodingError::Ok {
                        break 'end;
                    }
                    debug_assert!(!frame_skipped); // Key-frame cannot be an empty rectangle.
                    let prev_rect_key = enc.prev_rect;

                    // Analyze size difference of the two variants.
                    let curr_delta = key_frame_penalty(&enc.encoded_frames[encoded_frame]);
                    if curr_delta <= enc.best_delta {
                        // Pick this as the key-frame.
                        if enc.keyframe != KEYFRAME_NONE {
                            let old_keyframe = get_frame(enc, enc.keyframe as usize);
                            debug_assert!(enc.encoded_frames[old_keyframe].is_key_frame);
                            enc.encoded_frames[old_keyframe].is_key_frame = false;
                        }
                        enc.encoded_frames[encoded_frame].is_key_frame = true;
                        enc.prev_candidate_undecided = true;
                        enc.keyframe = position as i32;
                        enc.best_delta = curr_delta;
                        enc.flush_count = enc.count - 1; // We can flush previous frames.
                    } else {
                        enc.encoded_frames[encoded_frame].is_key_frame = false;
                        enc.prev_candidate_undecided = false;
                    }
                    // Note: We need '>=' below because when kmin and kmax are both zero,
                    // count_since_key_frame will always be > kmax.
                    if enc.count_since_key_frame >= enc.options.kmax {
                        enc.flush_count = enc.count - 1;
                        enc.count_since_key_frame = 0;
                        enc.keyframe = KEYFRAME_NONE;
                        enc.best_delta = DELTA_INFINITY;
                    }
                    if !enc.prev_candidate_undecided {
                        enc.prev_rect = if enc.encoded_frames[encoded_frame].is_key_frame {
                            prev_rect_key
                        } else {
                            prev_rect_sub
                        };
                    }
                }
            }

            // Update previous to previous and previous canvases for next call.
            webp_copy_pixels(curr_canvas, &mut enc.prev_canvas);
            enc.is_first_frame = false;
        }

        // Skip:
        ok = true;
        enc.in_frame_count += 1;
    }

    // End:
    if !ok || frame_skipped {
        frame_release(&mut enc.encoded_frames[encoded_frame]);
        // We reset some counters, as the frame addition failed/was skipped.
        enc.count -= 1;
        if !enc.is_first_frame {
            enc.count_since_key_frame -= 1;
        }
        if !ok {
            mark_error2(
                enc,
                "ERROR adding frame. WebPEncodingError",
                error_code as i32,
            );
        }
    }
    curr_canvas.error_code.set(error_code); // report error_code
    debug_assert!(ok || error_code != WebPEncodingError::Ok);
    ok
}

/// Translation of `FlushFrames()`.
fn flush_frames(enc: &mut WebPAnimEncoder) -> bool {
    while enc.flush_count > 0 {
        let curr = get_frame(enc, 0);
        let frame = &enc.encoded_frames[curr];
        let info = if frame.is_key_frame {
            &frame.key_frame
        } else {
            &frame.sub_frame
        };
        let err = webp_mux_push_frame(&mut enc.mux, info);
        if err != WebPMuxError::Ok {
            mark_error2(enc, "ERROR adding frame. WebPMuxError", err as i32);
            return false;
        }
        if enc.options.verbose {
            eprintln!(
                "INFO: Added frame. offset:{},{} dispose:{} blend:{}",
                info.x_offset, info.y_offset, info.dispose_method as i32, info.blend_method as i32
            );
        }
        enc.out_frame_count += 1;
        frame_release(&mut enc.encoded_frames[curr]);
        enc.start += 1;
        enc.flush_count -= 1;
        enc.count -= 1;
        if enc.keyframe != KEYFRAME_NONE {
            enc.keyframe -= 1;
        }
    }

    if enc.count == 1 && enc.start != 0 {
        // Move enc->start to index 0.
        let enc_start_tmp = enc.start;
        enc.encoded_frames.swap(0, enc_start_tmp);
        frame_release(&mut enc.encoded_frames[enc_start_tmp]);
        enc.start = 0;
    }
    true
}

/// Translation of `WebPAnimEncoderAdd()`: `frame` is `None` for the last
/// call, and the configuration is the default lossless one if `None`.
pub(crate) fn webp_anim_encoder_add(
    enc: &mut WebPAnimEncoder,
    frame: Option<&mut WebPPicture>,
    timestamp: i32,
    encoder_config: Option<&WebPConfig>,
) -> bool {
    let mut config = WebPConfig::default();

    mark_no_error(enc);

    if !enc.is_first_frame {
        // Make sure timestamps are non-decreasing (integer wrap-around is OK).
        let prev_frame_duration = (timestamp as u32).wrapping_sub(enc.prev_timestamp as u32);
        if prev_frame_duration >= MAX_DURATION {
            if let Some(frame) = &frame {
                frame
                    .error_code
                    .set(WebPEncodingError::InvalidConfiguration);
            }
            mark_error(enc, "ERROR adding frame: timestamps must be non-decreasing");
            return false;
        }
        if !increase_previous_duration(enc, prev_frame_duration as i32) {
            return false;
        }
        // IncreasePreviousDuration() may add a frame to avoid exceeding
        // MAX_DURATION which could cause CacheFrame() to over read encoded_frames_
        // before the next flush.
        if enc.count == enc.size && !flush_frames(enc) {
            return false;
        }
    } else {
        enc.first_timestamp = timestamp;
    }

    let Some(frame) = frame else {
        // Special: last call.
        enc.got_null_frame = true;
        enc.prev_timestamp = timestamp;
        return true;
    };

    if frame.width != enc.canvas_width || frame.height != enc.canvas_height {
        frame
            .error_code
            .set(WebPEncodingError::InvalidConfiguration);
        mark_error(enc, "ERROR adding frame: Invalid frame dimensions");
        return false;
    }

    if !frame.use_argb {
        // Convert frame from YUV(A) to ARGB.
        if enc.options.verbose {
            eprintln!(
                "WARNING: Converting frame from YUV(A) to ARGB format; this incurs a small loss."
            );
        }
        if !webp_picture_yuva_to_argb(frame) {
            mark_error(enc, "ERROR converting frame from YUV(A) to ARGB");
            return false;
        }
    }

    if let Some(encoder_config) = encoder_config {
        if !webp_validate_config(encoder_config) {
            mark_error(enc, "ERROR adding frame: Invalid WebPConfig");
            return false;
        }
        config = *encoder_config;
    } else {
        webp_config_init(&mut config);
        config.lossless = 1;
    }
    // (enc->curr_canvas_ = frame: the frame is passed along)
    debug_assert!(enc.curr_canvas_copy_modified);
    copy_current_canvas(enc, frame);

    let ok = cache_frame(enc, frame, &config) && flush_frames(enc);

    enc.curr_canvas_copy_modified = true;
    if ok {
        enc.prev_timestamp = timestamp;
    }
    ok
}

// -----------------------------------------------------------------------------
// Bitstream assembly.

/// Translation of `DecodeFrameOntoCanvas()`.
fn decode_frame_onto_canvas(frame: &WebPMuxFrameInfo, canvas: &mut WebPPicture) -> bool {
    let image = &frame.bitstream;
    let mut features = WebPBitstreamFeatures::default();
    webp_util_clear_pic(canvas, None);
    if webp_get_features(image, &mut features) != VP8StatusCode::Ok {
        return false;
    }
    let (x, y) = (frame.x_offset, frame.y_offset);
    let (w, h) = (features.width, features.height);
    if !view_is_valid(canvas, x, y, w, h) {
        return false;
    }
    // (the view of the canvas, in MODE_BGRA: the pixels' little-endian bytes)
    let (w, h) = (w as usize, h as usize);
    let mut rgba = vec![0u8; w * 4 * h];
    if webp_decode_bgra_into(image, &mut rgba, (w * 4) as i32).is_none() {
        return false;
    }
    let stride = canvas.argb_stride as usize;
    for j in 0..h {
        let row = (y as usize + j) * stride + x as usize;
        for i in 0..w {
            let p = &rgba[(j * w + i) * 4..(j * w + i) * 4 + 4];
            canvas.argb[row + i] = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
        }
    }
    true
}

/// Translation of `FrameToFullCanvas()`: the full image, if encoded.
fn frame_to_full_canvas(enc: &mut WebPAnimEncoder, frame: &WebPMuxFrameInfo) -> Option<Vec<u8>> {
    let canvas_buf = &mut enc.curr_canvas_copy;
    let mut mem1 = Vec::new();
    let mut mem2 = Vec::new();

    if !decode_frame_onto_canvas(frame, canvas_buf) {
        return None;
    }
    if !encode_frame(&enc.last_config, canvas_buf, &mut mem1) {
        return None;
    }
    let mut full_image = mem1;

    if enc.options.allow_mixed {
        if !encode_frame(&enc.last_config_reversed, canvas_buf, &mut mem2) {
            return None;
        }
        if mem2.len() < full_image.len() {
            full_image = mem2;
        }
    }
    Some(full_image)
}

/// Convert a single-frame animation to a non-animated image if appropriate.
/// TODO(urvang): Can we pick one of the two heuristically (based on frame
/// rectangle and/or presence of alpha)?
/// Translation of `OptimizeSingleFrame()`.
fn optimize_single_frame(enc: &mut WebPAnimEncoder, webp_data: &mut Vec<u8>) -> WebPMuxError {
    let mut err;
    let mut frame = WebPMuxFrameInfo::default();
    let mut webp_data2 = Vec::new();
    let Some(mut mux) = webp_mux_create(webp_data) else {
        return WebPMuxError::BadData;
    };
    debug_assert!(enc.out_frame_count == 1);

    'end: {
        err = webp_mux_get_frame(&mux, 1, &mut frame);
        if err != WebPMuxError::Ok {
            break 'end;
        }
        if frame.id != WebPChunkId::Anmf {
            break 'end; // Non-animation: nothing to do.
        }
        if let Err(e) = webp_mux_get_canvas_size(&mux) {
            err = e;
            break 'end;
        }
        let Some(full_image) = frame_to_full_canvas(enc, &frame) else {
            err = WebPMuxError::BadData;
            break 'end;
        };
        err = webp_mux_set_image(&mut mux, &full_image);
        if err != WebPMuxError::Ok {
            break 'end;
        }
        err = webp_mux_assemble(&mut mux, &mut webp_data2);
        if err != WebPMuxError::Ok {
            break 'end;
        }

        if webp_data2.len() < webp_data.len() {
            // Pick 'webp_data2' if smaller.
            *webp_data = webp_data2;
        }
    }

    // End:
    err
}

/// Translation of `WebPAnimEncoderAssemble()`: the assembled data is in
/// `webp_data`.
pub(crate) fn webp_anim_encoder_assemble(
    enc: &mut WebPAnimEncoder,
    webp_data: &mut Vec<u8>,
) -> bool {
    mark_no_error(enc);

    if enc.in_frame_count == 0 {
        mark_error(enc, "ERROR: No frames to assemble");
        return false;
    }

    if !enc.got_null_frame && enc.in_frame_count > 1 && enc.count > 0 {
        // set duration of the last frame to be avg of durations of previous frames.
        let delta_time =
            (enc.prev_timestamp as u32).wrapping_sub(enc.first_timestamp as u32) as f64;
        let average_duration = (delta_time / (enc.in_frame_count - 1) as f64) as i32;
        if !increase_previous_duration(enc, average_duration) {
            return false;
        }
    }

    // Flush any remaining frames.
    enc.flush_count = enc.count;
    if !flush_frames(enc) {
        return false;
    }

    // Set definitive canvas size.
    let err = 'err: {
        let err = webp_mux_set_canvas_size(&mut enc.mux, enc.canvas_width, enc.canvas_height);
        if err != WebPMuxError::Ok {
            break 'err err;
        }

        let anim_params: WebPMuxAnimParams = enc.options.anim_params;
        let err = webp_mux_set_animation_params(&mut enc.mux, &anim_params);
        if err != WebPMuxError::Ok {
            break 'err err;
        }

        // Assemble into a WebP bitstream.
        let err = webp_mux_assemble(&mut enc.mux, webp_data);
        if err != WebPMuxError::Ok {
            break 'err err;
        }

        if enc.out_frame_count == 1 {
            let err = optimize_single_frame(enc, webp_data);
            if err != WebPMuxError::Ok {
                break 'err err;
            }
        }
        return true;
    };

    // Err:
    mark_error2(enc, "ERROR assembling WebP", err as i32);
    false
}
