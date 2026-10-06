// Rust translation of src/dsp/alpha_processing.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the functions the decoder needs.
// Copyright 2013 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Utilities for processing transparent channel: copying the decoded alpha
//! plane into the output, and extracting it from the lossless decoder's
//! green channel. (The premultiplication functions are for the
//! premultiplied colorspaces SDL_image doesn't ask for.)

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

/// Translation of `ExtractGreen_C()` (`WebPExtractGreen`).
pub(crate) fn webp_extract_green(argb: &[u32], alpha: &mut [u8], size: usize) {
    for i in 0..size {
        alpha[i] = (argb[i] >> 8) as u8;
    }
}
