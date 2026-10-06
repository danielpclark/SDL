// Rust translation of src/dsp/yuv.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), the YUV->RGB conversions.
// Copyright 2010 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! inline YUV<->RGB conversion function
//!
//! For the Y'CbCr to RGB conversion, the BT.601 specification reads:
//!   R = 1.164 * (Y-16) + 1.596 * (V-128)
//!   G = 1.164 * (Y-16) - 0.813 * (V-128) - 0.391 * (U-128)
//!   B = 1.164 * (Y-16)                   + 2.018 * (U-128)
//! where Y is in the [16,235] range, and U/V in the [16,240] range.
//!
//! The fixed-point implementation used here is:
//!  R = (19077 . y             + 26149 . v - 14234) >> 6
//!  G = (19077 . y -  6419 . u - 13320 . v +  8708) >> 6
//!  B = (19077 . y + 33050 . u             - 17685) >> 6
//! where the '.' operator is the mulhi_epu16 variant:
//!   a . b = ((a << 8) * b) >> 16
//! that preserves 8 bits of fractional precision before final descaling.
//!
//! (The RGB->YUV direction, the SSE2 helpers and yuv.c's point samplers,
//! which only the encoder and the decoder without fancy upsampling use,
//! are not translated.)

/// fixed-point precision for YUV->RGB. Translation of `YUV_FIX2`.
const YUV_FIX2: i32 = 6;
/// Translation of `YUV_MASK2`.
const YUV_MASK2: i32 = (256 << YUV_FIX2) - 1;

/// slower on x86 by ~7-8%, but bit-exact with the SSE2/NEON version.
/// _mm_mulhi_epu16 emulation. Translation of `MultHi()`.
fn mult_hi(v: i32, coeff: i32) -> i32 {
    (v * coeff) >> 8
}

/// Translation of `VP8Clip8()`.
fn vp8_clip8(v: i32) -> u8 {
    if (v & !YUV_MASK2) == 0 {
        (v >> YUV_FIX2) as u8
    } else if v < 0 {
        0
    } else {
        255
    }
}

/// Translation of `VP8YUVToR()`.
fn vp8_yuv_to_r(y: i32, v: i32) -> u8 {
    vp8_clip8(mult_hi(y, 19077) + mult_hi(v, 26149) - 14234)
}

/// Translation of `VP8YUVToG()`.
fn vp8_yuv_to_g(y: i32, u: i32, v: i32) -> u8 {
    vp8_clip8(mult_hi(y, 19077) - mult_hi(u, 6419) - mult_hi(v, 13320) + 8708)
}

/// Translation of `VP8YUVToB()`.
fn vp8_yuv_to_b(y: i32, u: i32) -> u8 {
    vp8_clip8(mult_hi(y, 19077) + mult_hi(u, 33050) - 17685)
}

/// Translation of `VP8YuvToRgb()`.
pub(crate) fn vp8_yuv_to_rgb(y: i32, u: i32, v: i32, rgb: &mut [u8]) {
    rgb[0] = vp8_yuv_to_r(y, v);
    rgb[1] = vp8_yuv_to_g(y, u, v);
    rgb[2] = vp8_yuv_to_b(y, u);
}

/// Translation of `VP8YuvToBgr()`.
pub(crate) fn vp8_yuv_to_bgr(y: i32, u: i32, v: i32, bgr: &mut [u8]) {
    bgr[0] = vp8_yuv_to_b(y, u);
    bgr[1] = vp8_yuv_to_g(y, u, v);
    bgr[2] = vp8_yuv_to_r(y, v);
}

//-----------------------------------------------------------------------------
// Alpha handling variants

/// Translation of `VP8YuvToArgb()`.
pub(crate) fn vp8_yuv_to_argb(y: i32, u: i32, v: i32, argb: &mut [u8]) {
    argb[0] = 0xff;
    vp8_yuv_to_rgb(y, u, v, &mut argb[1..]);
}

/// Translation of `VP8YuvToBgra()`.
pub(crate) fn vp8_yuv_to_bgra(y: i32, u: i32, v: i32, bgra: &mut [u8]) {
    vp8_yuv_to_bgr(y, u, v, bgra);
    bgra[3] = 0xff;
}

/// Translation of `VP8YuvToRgba()`.
pub(crate) fn vp8_yuv_to_rgba(y: i32, u: i32, v: i32, rgba: &mut [u8]) {
    vp8_yuv_to_rgb(y, u, v, rgba);
    rgba[3] = 0xff;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yuv_to_rgb_matches_bt601() {
        let mut rgb = [0u8; 3];
        // black and white
        vp8_yuv_to_rgb(16, 128, 128, &mut rgb);
        assert_eq!(rgb, [0, 0, 0]);
        vp8_yuv_to_rgb(235, 128, 128, &mut rgb);
        assert_eq!(rgb, [255, 255, 255]);
        // saturated primaries clip
        vp8_yuv_to_rgb(81, 90, 240, &mut rgb);
        assert!(rgb[0] > 250 && rgb[1] < 5 && rgb[2] < 5);
        let mut bgra = [0u8; 4];
        vp8_yuv_to_bgra(81, 90, 240, &mut bgra);
        assert_eq!([bgra[2], bgra[1], bgra[0], bgra[3]], [rgb[0], rgb[1], rgb[2], 255]);
    }
}
