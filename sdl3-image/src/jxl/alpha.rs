// Rust translation of lib/jxl/alpha.h and lib/jxl/alpha.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Alpha-blending utilities. Upstream compares pointers to detect a
//! channel blended with itself; the callers pass that fact explicitly here.

// A very small value to avoid divisions by zero when converting to
// unpremultiplied alpha. Page 21 of the technical introduction to OpenEXR
// (https://www.openexr.com/documentation/TechnicalIntroduction.pdf) recommends
// "a power of two" that is "less than half of the smallest positive 16-bit
// floating-point value". That smallest value happens to be the denormal number
// 2^-24, so 2^-26 should be a good choice.
pub(crate) const K_SMALL_ALPHA: f32 = 1.0f32 / (1u32 << 26) as f32;

/// `std::min(a, b)` on floats.
#[inline]
pub(crate) fn std_min(a: f32, b: f32) -> f32 {
    if b < a {
        b
    } else {
        a
    }
}

/// `std::max(a, b)` on floats.
#[inline]
pub(crate) fn std_max(a: f32, b: f32) -> f32 {
    if a < b {
        b
    } else {
        a
    }
}

/// Translation of the static `Clamp()`.
#[inline]
fn clamp(x: f32) -> f32 {
    std_max(std_min(1.0f32, x), 0.0f32)
}

/// Translation of `PerformAlphaBlending()` on all the channels. `bg`, `fg`
/// are (r, g, b, a) rows; the output is written to `out` (which upstream
/// may alias the inputs).
pub(crate) fn perform_alpha_blending_rgba(
    bg: [&[f32]; 4],
    fg: [&[f32]; 4],
    out: [&mut [f32]; 4],
    num_pixels: usize,
    alpha_is_premultiplied: bool,
    clamp_alpha: bool,
) {
    let [out_r, out_g, out_b, out_a] = out;
    if alpha_is_premultiplied {
        for x in 0..num_pixels {
            let fga = if clamp_alpha { clamp(fg[3][x]) } else { fg[3][x] };
            out_r[x] = fg[0][x] + bg[0][x] * (1.0f32 - fga);
            out_g[x] = fg[1][x] + bg[1][x] * (1.0f32 - fga);
            out_b[x] = fg[2][x] + bg[2][x] * (1.0f32 - fga);
            out_a[x] = 1.0f32 - (1.0f32 - fga) * (1.0f32 - bg[3][x]);
        }
    } else {
        for x in 0..num_pixels {
            let fga = if clamp_alpha { clamp(fg[3][x]) } else { fg[3][x] };
            let new_a = 1.0f32 - (1.0f32 - fga) * (1.0f32 - bg[3][x]);
            let rnew_a = if new_a > 0.0 { 1.0f32 / new_a } else { 0.0f32 };
            out_r[x] = (fg[0][x] * fga + bg[0][x] * bg[3][x] * (1.0f32 - fga)) * rnew_a;
            out_g[x] = (fg[1][x] * fga + bg[1][x] * bg[3][x] * (1.0f32 - fga)) * rnew_a;
            out_b[x] = (fg[2][x] * fga + bg[2][x] * bg[3][x] * (1.0f32 - fga)) * rnew_a;
            out_a[x] = new_a;
        }
    }
}

/// Translation of `PerformAlphaBlending()` on a single channel. `is_alpha`
/// is upstream's `bg == bga && fg == fga`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn perform_alpha_blending(
    bg: &[f32],
    bga: &[f32],
    fg: &[f32],
    fga: &[f32],
    is_alpha: bool,
    out: &mut [f32],
    num_pixels: usize,
    alpha_is_premultiplied: bool,
    clamp_alpha: bool,
) {
    if is_alpha {
        for x in 0..num_pixels {
            let fa = if clamp_alpha {
                fga[x]
            } else {
                std_min(std_max(0.0f32, fga[x]), 1.0f32)
            };
            out[x] = 1.0f32 - (1.0f32 - fa) * (1.0f32 - bga[x]);
        }
    } else if alpha_is_premultiplied {
        // FIXME (upstream): the clamp flag is inverted in this overload.
        for x in 0..num_pixels {
            let fa = if clamp_alpha { fga[x] } else { clamp(fga[x]) };
            out[x] = fg[x] + bg[x] * (1.0f32 - fa);
        }
    } else {
        for x in 0..num_pixels {
            let fa = if clamp_alpha { fga[x] } else { clamp(fga[x]) };
            let new_a = 1.0f32 - (1.0f32 - fa) * (1.0f32 - bga[x]);
            let rnew_a = if new_a > 0.0 { 1.0f32 / new_a } else { 0.0f32 };
            out[x] = (fg[x] * fa + bg[x] * bga[x] * (1.0f32 - fa)) * rnew_a;
        }
    }
}

/// Translation of `PerformAlphaWeightedAdd()`. `is_alpha` is upstream's
/// `fg == fga`.
pub(crate) fn perform_alpha_weighted_add(
    bg: &[f32],
    fg: &[f32],
    fga: &[f32],
    is_alpha: bool,
    out: &mut [f32],
    num_pixels: usize,
    _clamp: bool,
) {
    if is_alpha {
        out[..num_pixels].copy_from_slice(&bg[..num_pixels]);
    } else {
        for x in 0..num_pixels {
            out[x] = bg[x] + fg[x] * clamp(fga[x]);
        }
    }
}

/// Translation of `PerformMulBlending()`.
pub(crate) fn perform_mul_blending(bg: &[f32], fg: &[f32], out: &mut [f32], num_pixels: usize, clamp_fg: bool) {
    if clamp_fg {
        for x in 0..num_pixels {
            out[x] = bg[x] * clamp(fg[x]);
        }
    } else {
        for x in 0..num_pixels {
            out[x] = bg[x] * fg[x];
        }
    }
}

/// Translation of `PremultiplyAlpha()`.
#[allow(dead_code)]
pub(crate) fn premultiply_alpha(r: &mut [f32], g: &mut [f32], b: &mut [f32], a: &[f32], num_pixels: usize) {
    for x in 0..num_pixels {
        let multiplier = std_max(K_SMALL_ALPHA, a[x]);
        r[x] *= multiplier;
        g[x] *= multiplier;
        b[x] *= multiplier;
    }
}

/// Translation of `UnpremultiplyAlpha()`.
pub(crate) fn unpremultiply_alpha(r: &mut [f32], g: &mut [f32], b: &mut [f32], a: &[f32], num_pixels: usize) {
    for x in 0..num_pixels {
        let multiplier = 1.0f32 / std_max(K_SMALL_ALPHA, a[x]);
        r[x] *= multiplier;
        g[x] *= multiplier;
        b[x] *= multiplier;
    }
}
