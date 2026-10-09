// Rust translation of lib/jxl/blending.h and lib/jxl/blending.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Blending of frames and patches.

use super::alpha::{perform_alpha_blending, perform_alpha_blending_rgba, perform_alpha_weighted_add, perform_mul_blending};
use super::dec_patch_dictionary::{PatchBlendMode, PatchBlending};
use super::frame_header::{BlendMode, FrameHeader, FrameType};
use super::image_metadata::{ExtraChannel, ExtraChannelInfo};

/// Translation of `NeedsBlending()` (on the frame header of the shared
/// state).
pub(crate) fn needs_blending(frame_header: &FrameHeader) -> bool {
    if !(frame_header.frame_type == FrameType::RegularFrame || frame_header.frame_type == FrameType::SkipProgressive) {
        return false;
    }
    let info = &frame_header.blending_info;
    let mut replace_all = info.mode == BlendMode::Replace;
    for ec_i in &frame_header.extra_channel_blending_info {
        if ec_i.mode != BlendMode::Replace {
            replace_all = false;
        }
    }
    // Replace the full frame: nothing to do.
    if !frame_header.custom_size_or_origin && replace_all {
        return false;
    }
    true
}

/// Translation of `PerformBlending()`. `bg[c]` and `fg[c]` are the rows of
/// channel `c` starting at upstream's `x0`; the blended `xsize` pixels of
/// each channel are returned (upstream writes them to `out[c] + x0`, which
/// may alias `bg` or `fg`).
pub(crate) fn perform_blending(
    bg: &[&[f32]],
    fg: &[&[f32]],
    xsize: usize,
    color_blending: &PatchBlending,
    ec_blending: &[PatchBlending],
    extra_channel_info: &[ExtraChannelInfo],
) -> Vec<Vec<f32>> {
    let mut has_alpha = false;
    let num_ec = extra_channel_info.len();
    for eci in extra_channel_info {
        if eci.type_ == ExtraChannel::Alpha {
            has_alpha = true;
            break;
        }
    }
    let mut tmp: Vec<Vec<f32>> = vec![vec![0f32; xsize]; 3 + num_ec];
    // Blend extra channels first so that we use the pre-blending alpha.
    for i in 0..num_ec {
        let eb = &ec_blending[i];
        let alpha = eb.alpha_channel as usize;
        match eb.mode {
            PatchBlendMode::Add => {
                for x in 0..xsize {
                    tmp[3 + i][x] = bg[3 + i][x] + fg[3 + i][x];
                }
            }
            PatchBlendMode::BlendAbove => {
                let is_premultiplied = extra_channel_info[alpha].alpha_associated;
                perform_alpha_blending(
                    bg[3 + i],
                    bg[3 + alpha],
                    fg[3 + i],
                    fg[3 + alpha],
                    i == alpha,
                    &mut tmp[3 + i],
                    xsize,
                    is_premultiplied,
                    eb.clamp,
                );
            }
            PatchBlendMode::BlendBelow => {
                let is_premultiplied = extra_channel_info[alpha].alpha_associated;
                perform_alpha_blending(
                    fg[3 + i],
                    fg[3 + alpha],
                    bg[3 + i],
                    bg[3 + alpha],
                    i == alpha,
                    &mut tmp[3 + i],
                    xsize,
                    is_premultiplied,
                    eb.clamp,
                );
            }
            PatchBlendMode::AlphaWeightedAddAbove => {
                perform_alpha_weighted_add(
                    bg[3 + i],
                    fg[3 + i],
                    fg[3 + alpha],
                    i == alpha,
                    &mut tmp[3 + i],
                    xsize,
                    eb.clamp,
                );
            }
            PatchBlendMode::AlphaWeightedAddBelow => {
                perform_alpha_weighted_add(
                    fg[3 + i],
                    bg[3 + i],
                    bg[3 + alpha],
                    i == alpha,
                    &mut tmp[3 + i],
                    xsize,
                    eb.clamp,
                );
            }
            PatchBlendMode::Mul => {
                perform_mul_blending(bg[3 + i], fg[3 + i], &mut tmp[3 + i], xsize, eb.clamp);
            }
            PatchBlendMode::Replace => {
                tmp[3 + i].copy_from_slice(&fg[3 + i][..xsize]);
            }
            PatchBlendMode::None => {
                if xsize != 0 {
                    tmp[3 + i].copy_from_slice(&bg[3 + i][..xsize]);
                }
            }
        }
    }
    let alpha = color_blending.alpha_channel as usize;

    let mode = color_blending.mode;
    if mode == PatchBlendMode::Add
        || (mode == PatchBlendMode::AlphaWeightedAddAbove && !has_alpha)
        || (mode == PatchBlendMode::AlphaWeightedAddBelow && !has_alpha)
    {
        for p in 0..3 {
            for x in 0..xsize {
                tmp[p][x] = bg[p][x] + fg[p][x];
            }
        }
    } else if mode == PatchBlendMode::BlendAbove
        // blend without alpha is just replace
        && has_alpha
    {
        let is_premultiplied = extra_channel_info[alpha].alpha_associated;
        let (t_rgb, t_ec) = tmp.split_at_mut(3);
        let [t0, t1, t2] = t_rgb else { unreachable!() };
        perform_alpha_blending_rgba(
            [bg[0], bg[1], bg[2], bg[3 + alpha]],
            [fg[0], fg[1], fg[2], fg[3 + alpha]],
            [t0, t1, t2, &mut t_ec[alpha]],
            xsize,
            is_premultiplied,
            color_blending.clamp,
        );
    } else if mode == PatchBlendMode::BlendBelow
        // blend without alpha is just replace
        && has_alpha
    {
        let is_premultiplied = extra_channel_info[alpha].alpha_associated;
        let (t_rgb, t_ec) = tmp.split_at_mut(3);
        let [t0, t1, t2] = t_rgb else { unreachable!() };
        perform_alpha_blending_rgba(
            [fg[0], fg[1], fg[2], fg[3 + alpha]],
            [bg[0], bg[1], bg[2], bg[3 + alpha]],
            [t0, t1, t2, &mut t_ec[alpha]],
            xsize,
            is_premultiplied,
            color_blending.clamp,
        );
    } else if mode == PatchBlendMode::AlphaWeightedAddAbove {
        debug_assert!(has_alpha);
        for c in 0..3 {
            perform_alpha_weighted_add(bg[c], fg[c], fg[3 + alpha], false, &mut tmp[c], xsize, color_blending.clamp);
        }
    } else if mode == PatchBlendMode::AlphaWeightedAddBelow {
        debug_assert!(has_alpha);
        for c in 0..3 {
            perform_alpha_weighted_add(fg[c], bg[c], bg[3 + alpha], false, &mut tmp[c], xsize, color_blending.clamp);
        }
    } else if mode == PatchBlendMode::Mul {
        for p in 0..3 {
            perform_mul_blending(bg[p], fg[p], &mut tmp[p], xsize, color_blending.clamp);
        }
    } else if mode == PatchBlendMode::Replace || mode == PatchBlendMode::BlendAbove || mode == PatchBlendMode::BlendBelow {
        // kReplace
        for p in 0..3 {
            tmp[p].copy_from_slice(&fg[p][..xsize]);
        }
    } else {
        // kNone
        for p in 0..3 {
            tmp[p].copy_from_slice(&bg[p][..xsize]);
        }
    }
    tmp
}
