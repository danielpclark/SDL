// Rust translation of src/dsp/alpha_processing.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the functions the decoder and encoder need.
// Copyright 2013 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Utilities for processing transparent channel: copying the decoded alpha
//! plane into the output, and extracting it from the lossless decoder's
//! green channel; for the encoder, extracting it from RGBA rows, putting it
//! in the green channel, looking for transparency and replacing
//! transparent pixels. (The
//! premultiplication functions are for the premultiplied colorspaces
//! SDL_image doesn't ask for.)

/// Translation of `DispatchAlpha_C()` (`WebPDispatchAlpha`): the alpha
/// rows into every fourth byte of the destination rows. Returns true if
/// the alpha values are not all 0xff.
#[allow(clippy::too_many_arguments)]
pub(crate) fn webp_dispatch_alpha(
    alpha: &[u8],
    mut alpha_off: usize,
    alpha_stride: usize,
    width: usize,
    height: usize,
    dst: &mut [u8],
    mut dst_off: usize,
    dst_stride: usize,
) -> bool {
    let mut alpha_mask: u32 = 0xff;
    for _ in 0..height {
        for i in 0..width {
            let alpha_value = alpha[alpha_off + i] as u32;
            dst[dst_off + 4 * i] = alpha_value as u8;
            alpha_mask &= alpha_value;
        }
        alpha_off += alpha_stride;
        dst_off += dst_stride;
    }

    alpha_mask != 0xff
}

/// Translation of `DispatchAlphaToGreen_C()` (`WebPDispatchAlphaToGreen`).
pub(crate) fn webp_dispatch_alpha_to_green(
    alpha: &[u8],
    alpha_stride: usize,
    width: usize,
    height: usize,
    dst: &mut [u32],
    dst_stride: usize,
) {
    for j in 0..height {
        for i in 0..width {
            dst[j * dst_stride + i] = (alpha[j * alpha_stride + i] as u32) << 8;
            // leave A/R/B channels zero'd.
        }
    }
}

/// Translation of `ExtractGreen_C()` (`WebPExtractGreen`).
pub(crate) fn webp_extract_green(argb: &[u32], alpha: &mut [u8], size: usize) {
    for i in 0..size {
        alpha[i] = (argb[i] >> 8) as u8;
    }
}

/// Translation of `ExtractAlpha_C()` (`WebPExtractAlpha`): every fourth
/// byte of the `argb` rows (from `argb_off`) to the `alpha` rows. Returns
/// true if the alpha values are all 0xff.
#[allow(clippy::too_many_arguments)]
pub(crate) fn webp_extract_alpha(
    argb: &[u8],
    mut argb_off: usize,
    argb_stride: usize,
    width: usize,
    height: usize,
    alpha: &mut [u8],
    mut alpha_off: usize,
    alpha_stride: usize,
) -> bool {
    let mut alpha_mask = 0xffu8;

    for _ in 0..height {
        for i in 0..width {
            let alpha_value = argb[argb_off + 4 * i];
            alpha[alpha_off + i] = alpha_value;
            alpha_mask &= alpha_value;
        }
        argb_off += argb_stride;
        alpha_off += alpha_stride;
    }
    alpha_mask == 0xff
}

/// Translation of `HasAlpha8b_C()` (`WebPHasAlpha8b`): whether any of the
/// `length` bytes isn't 0xff.
pub(crate) fn webp_has_alpha_8b(src: &[u8], length: usize) -> bool {
    src[..length].iter().any(|&a| a != 0xff)
}

/// Translation of `HasAlpha32b_C()` (`WebPHasAlpha32b`): whether any of the
/// `length` bytes 4 apart isn't 0xff.
pub(crate) fn webp_has_alpha_32b(src: &[u8], length: usize) -> bool {
    (0..length).any(|i| src[4 * i] != 0xff)
}

/// Translation of `AlphaReplace_C()` (`WebPAlphaReplace`): the transparent
/// pixels become `color`.
pub(crate) fn webp_alpha_replace(src: &mut [u32], color: u32) {
    for p in src {
        if (*p >> 24) == 0 {
            *p = color;
        }
    }
}
