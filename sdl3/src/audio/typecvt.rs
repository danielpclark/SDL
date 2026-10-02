// Rust translation of src/audio/SDL_audiotypecvt.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Sample type conversions to and from native-endian `f32`.
//!
//! These are the scalar converters ("fallback scalar converters" upstream).
//! The SSE2/SSSE3/NEON versions upstream selects at runtime give the same
//! results for samples in the nominal range (they differ only in how they
//! clamp floats far outside [-1, 1]); the scalar loops here are written so
//! the compiler can vectorize them.
//!
//! Like their C counterparts, the converters work *in place*: the input
//! occupies the start of `buf` and the output is written to the start of
//! the same buffer. Conversions to a wider type walk backwards.

use super::format::{AudioFormat, AUDIO_MASK_BIG_ENDIAN};

/// 0x1p-31f. Translation of `DIVBY2147483648`.
#[allow(clippy::excessive_precision)] // (the exact decimal expansion upstream spells out)
const DIVBY2147483648: f32 = 0.000_000_000_465_661_287_307_739_257_812_5;

#[inline(always)]
fn put_f32(buf: &mut [u8], i: usize, v: f32) {
    buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
}

#[inline(always)]
fn get_f32(buf: &[u8], i: usize) -> f32 {
    f32::from_ne_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]])
}

/// Translation of `SDL_Convert_S8_to_F32_Scalar()`.
fn convert_s8_to_f32(buf: &mut [u8], num_samples: usize) {
    for i in (0..num_samples).rev() {
        /* 1) Construct a float in the range [65536.0, 65538.0)
         * 2) Shift the float range to [-1.0, 1.0) */
        let x = f32::from_bits(u32::from(buf[i]) ^ 0x4780_0080);
        put_f32(buf, i, x - 65537.0);
    }
}

/// Translation of `SDL_Convert_U8_to_F32_Scalar()`.
fn convert_u8_to_f32(buf: &mut [u8], num_samples: usize) {
    for i in (0..num_samples).rev() {
        /* 1) Construct a float in the range [65536.0, 65538.0)
         * 2) Shift the float range to [-1.0, 1.0) */
        let x = f32::from_bits(u32::from(buf[i]) ^ 0x4780_0000);
        put_f32(buf, i, x - 65537.0);
    }
}

/// Translation of `SDL_Convert_S16_to_F32_Scalar()`.
fn convert_s16_to_f32(buf: &mut [u8], num_samples: usize) {
    for i in (0..num_samples).rev() {
        /* 1) Construct a float in the range [256.0, 258.0)
         * 2) Shift the float range to [-1.0, 1.0) */
        let s = u16::from_ne_bytes([buf[i * 2], buf[i * 2 + 1]]);
        let x = f32::from_bits(u32::from(s) ^ 0x4380_8000);
        put_f32(buf, i, x - 257.0);
    }
}

/// Translation of `SDL_Convert_S32_to_F32_Scalar()`.
fn convert_s32_to_f32(buf: &mut [u8], num_samples: usize) {
    for i in (0..num_samples).rev() {
        let s = i32::from_ne_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]]);
        put_f32(buf, i, s as f32 * DIVBY2147483648);
    }
}

/// Create a bit-mask based on the sign-bit. Should optimize to a single
/// arithmetic-shift-right. Translation of `SIGNMASK()`.
#[inline(always)]
const fn signmask(x: u32) -> u32 {
    0u32.wrapping_sub(x >> 31)
}

/// Translation of `SDL_Convert_F32_to_S8_Scalar()`.
fn convert_f32_to_s8(buf: &mut [u8], num_samples: usize) {
    for i in 0..num_samples {
        /* 1) Shift the float range from [-1.0, 1.0] to [98303.0, 98305.0]
         * 2) Shift the integer range from [0x47BFFF80, 0x47C00080] to [-128, 128]
         * 3) Clamp the value to [-128, 127] */
        let x = get_f32(buf, i) + 98304.0;

        let mut y = x.to_bits().wrapping_sub(0x47C0_0000);
        let z = 0x7Fu32.wrapping_sub(y ^ signmask(y));
        y ^= z & signmask(z);

        buf[i] = (y & 0xFF) as u8;
    }
}

/// Translation of `SDL_Convert_F32_to_U8_Scalar()`.
fn convert_f32_to_u8(buf: &mut [u8], num_samples: usize) {
    for i in 0..num_samples {
        /* 1) Shift the float range from [-1.0, 1.0] to [98303.0, 98305.0]
         * 2) Shift the integer range from [0x47BFFF80, 0x47C00080] to [-128, 128]
         * 3) Clamp the value to [-128, 127]
         * 4) Shift the integer range from [-128, 127] to [0, 255] */
        let x = get_f32(buf, i) + 98304.0;

        let mut y = x.to_bits().wrapping_sub(0x47C0_0000);
        let z = 0x7Fu32.wrapping_sub(y ^ signmask(y));
        y = (y ^ 0x80) ^ (z & signmask(z));

        buf[i] = (y & 0xFF) as u8;
    }
}

/// Translation of `SDL_Convert_F32_to_S16_Scalar()`.
fn convert_f32_to_s16(buf: &mut [u8], num_samples: usize) {
    for i in 0..num_samples {
        /* 1) Shift the float range from [-1.0, 1.0] to [383.0, 385.0]
         * 2) Shift the integer range from [0x43BF8000, 0x43C08000] to [-32768, 32768]
         * 3) Clamp values outside the [-32768, 32767] range */
        let x = get_f32(buf, i) + 384.0;

        let mut y = x.to_bits().wrapping_sub(0x43C0_0000);
        let z = 0x7FFFu32.wrapping_sub(y ^ signmask(y));
        y ^= z & signmask(z);

        buf[i * 2..i * 2 + 2].copy_from_slice(&((y & 0xFFFF) as u16).to_ne_bytes());
    }
}

/// Translation of `SDL_Convert_F32_to_S32_Scalar()`.
fn convert_f32_to_s32(buf: &mut [u8], num_samples: usize) {
    for i in 0..num_samples {
        /* 1) Shift the float range from [-1.0, 1.0] to [-2147483648.0, 2147483648.0]
         * 2) Set values outside the [-2147483648.0, 2147483647.0] range to -2147483648.0
         * 3) Convert the float to an integer, and fixup values outside the valid range */
        let bits = get_f32(buf, i).to_bits();

        let y = bits.wrapping_add(0x0F80_0000);
        let mut z = y.wrapping_sub(0xCF00_0000);
        z &= signmask(y ^ z);
        let x = f32::from_bits(y.wrapping_sub(z));

        // (C's float-to-int conversion is only defined in range; the
        // fixup above keeps it there, so `as` behaves the same.)
        let v = (x as i32) ^ (signmask(z) as i32);
        buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
    }
}

/// Translation of `SDL_Convert_Swap16_Scalar()` (in place).
fn convert_swap16(buf: &mut [u8], num_samples: usize) {
    for s in buf[..num_samples * 2].chunks_exact_mut(2) {
        s.swap(0, 1);
    }
}

/// Translation of `SDL_Convert_Swap32_Scalar()` (in place).
fn convert_swap32(buf: &mut [u8], num_samples: usize) {
    for s in buf[..num_samples * 4].chunks_exact_mut(4) {
        s.reverse();
    }
}

/// Convert `num_samples` samples of `src_fmt` at the start of `buf` to
/// native `f32`, in place. Translation of `ConvertAudioToFloat()`.
pub(crate) fn convert_audio_to_float(buf: &mut [u8], num_samples: usize, src_fmt: AudioFormat) {
    match src_fmt {
        AudioFormat::S8 => convert_s8_to_f32(buf, num_samples),
        AudioFormat::U8 => convert_u8_to_f32(buf, num_samples),
        AudioFormat::S16 => convert_s16_to_f32(buf, num_samples),
        f if f.0 == AudioFormat::S16.0 ^ AUDIO_MASK_BIG_ENDIAN => {
            convert_swap16(buf, num_samples);
            convert_s16_to_f32(buf, num_samples);
        }
        AudioFormat::S32 => convert_s32_to_f32(buf, num_samples),
        f if f.0 == AudioFormat::S32.0 ^ AUDIO_MASK_BIG_ENDIAN => {
            convert_swap32(buf, num_samples);
            convert_s32_to_f32(buf, num_samples);
        }
        f if f.0 == AudioFormat::F32.0 ^ AUDIO_MASK_BIG_ENDIAN => convert_swap32(buf, num_samples),
        _ => crate::sdl_assert!(!"Unexpected audio format!"),
    }
}

/// Convert `num_samples` native `f32` samples at the start of `buf` to
/// `dst_fmt`, in place. Translation of `ConvertAudioFromFloat()`.
pub(crate) fn convert_audio_from_float(buf: &mut [u8], num_samples: usize, dst_fmt: AudioFormat) {
    match dst_fmt {
        AudioFormat::S8 => convert_f32_to_s8(buf, num_samples),
        AudioFormat::U8 => convert_f32_to_u8(buf, num_samples),
        AudioFormat::S16 => convert_f32_to_s16(buf, num_samples),
        f if f.0 == AudioFormat::S16.0 ^ AUDIO_MASK_BIG_ENDIAN => {
            convert_f32_to_s16(buf, num_samples);
            convert_swap16(buf, num_samples);
        }
        AudioFormat::S32 => convert_f32_to_s32(buf, num_samples),
        f if f.0 == AudioFormat::S32.0 ^ AUDIO_MASK_BIG_ENDIAN => {
            convert_f32_to_s32(buf, num_samples);
            convert_swap32(buf, num_samples);
        }
        f if f.0 == AudioFormat::F32.0 ^ AUDIO_MASK_BIG_ENDIAN => convert_swap32(buf, num_samples),
        _ => crate::sdl_assert!(!"Unexpected audio format!"),
    }
}

/// Byteswap `num_samples` samples of `bitsize` bits, in place.
/// Translation of `ConvertAudioSwapEndian()`.
pub(crate) fn convert_audio_swap_endian(buf: &mut [u8], num_samples: usize, bitsize: u32) {
    match bitsize {
        16 => convert_swap16(buf, num_samples),
        32 => convert_swap32(buf, num_samples),
        _ => crate::sdl_assert!(!"Unexpected audio format!"),
    }
}
