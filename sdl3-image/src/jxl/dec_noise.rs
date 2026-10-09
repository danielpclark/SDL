// Rust translation of lib/jxl/dec_noise.h, lib/jxl/dec_noise.cc,
// lib/jxl/noise.h and lib/jxl/xorshift128plus-inl.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Noise synthesis: the parameters and the random planes.

use super::base::Status;
use super::dec_bit_reader::BitReader;
use super::image::{ImageF, Rect};

// --- noise.h ---

pub(crate) const K_NOISE_PRECISION: f32 = (1 << 10) as f32;

/// Translation of `NoiseParams`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NoiseParams {
    // LUT index is an intensity of pixel / mean intensity of patch
    pub lut: [f32; NoiseParams::K_NUM_NOISE_POINTS],
}

impl NoiseParams {
    pub(crate) const K_NUM_NOISE_POINTS: usize = 8;

    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        for i in self.lut.iter_mut() {
            *i = 0.0;
        }
    }

    pub(crate) fn has_any(&self) -> bool {
        for &i in &self.lut {
            if i.abs() > 1e-3f32 {
                return true;
            }
        }
        false
    }
}

// --- xorshift128plus-inl.h ---

/// Adapted from https://github.com/vpxyz/xorshift/blob/master/xorshift128plus/
/// (MIT-license). Translation of `Xorshift128Plus` (the scalar loop).
struct Xorshift128Plus {
    s0: [u64; Xorshift128Plus::N],
    s1: [u64; Xorshift128Plus::N],
}

impl Xorshift128Plus {
    // 8 independent generators (= single iteration for AVX-512)
    const N: usize = 8;

    fn new(seed1: u32, seed2: u32, seed3: u32, seed4: u32) -> Self {
        let mut s0 = [0u64; Self::N];
        let mut s1 = [0u64; Self::N];
        // Init state using SplitMix64 generator
        s0[0] = Self::split_mix64(
            (((seed1 as u64) << 32).wrapping_add(seed2 as u64)).wrapping_add(0x9E3779B97F4A7C15),
        );
        s1[0] = Self::split_mix64(
            (((seed3 as u64) << 32).wrapping_add(seed4 as u64)).wrapping_add(0x9E3779B97F4A7C15),
        );
        for i in 1..Self::N {
            s0[i] = Self::split_mix64(s0[i - 1]);
            s1[i] = Self::split_mix64(s1[i - 1]);
        }
        Xorshift128Plus { s0, s1 }
    }

    fn fill(&mut self, random_bits: &mut [u64; Self::N]) {
        for i in 0..Self::N {
            let mut s1 = self.s0[i];
            let s0 = self.s1[i];
            let bits = s1.wrapping_add(s0); // b, c
            self.s0[i] = s0;
            s1 ^= s1 << 23;
            random_bits[i] = bits;
            s1 ^= s0 ^ (s1 >> 18) ^ (s0 >> 5);
            self.s1[i] = s1;
        }
    }

    fn split_mix64(mut z: u64) -> u64 {
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

// --- dec_noise.cc ---

/// Converts one vector's worth of random bits to floats in [1, 2).
/// NOTE: as the convolution kernel sums to 0, it doesn't matter if inputs
/// are in [0, 1) or in [1, 2). Translation of `BitsToFloat()`.
#[inline]
fn bits_to_float(bits: u32) -> f32 {
    // 1.0 + 23 random mantissa bits = [1, 2)
    f32::from_bits((bits >> 9) | 0x3F800000)
}

/// The 32-bit word `i` of a batch of 64-bit words (little-endian).
#[inline]
fn batch_word(batch: &[u64; Xorshift128Plus::N], i: usize) -> u32 {
    let w = batch[i / 2];
    if i % 2 == 0 {
        w as u32
    } else {
        (w >> 32) as u32
    }
}

/// Translation of `RandomImage()`.
fn random_image(rng: &mut Xorshift128Plus, rect: &Rect, noise: &mut ImageF) {
    let xsize = rect.xsize();
    let ysize = rect.ysize();

    // May exceed the vector size, hence we have two loops over x below.
    const K_FLOATS_PER_BATCH: usize = Xorshift128Plus::N * 8 / 4;
    let mut batch = [0u64; Xorshift128Plus::N];

    for y in 0..ysize {
        let row = rect.row(noise, y);

        let mut x = 0usize;
        // Only entire batches (avoids exceeding the image padding).
        while x + K_FLOATS_PER_BATCH <= xsize {
            rng.fill(&mut batch);
            for i in 0..K_FLOATS_PER_BATCH {
                row[x + i] = bits_to_float(batch_word(&batch, i));
            }
            x += K_FLOATS_PER_BATCH;
        }

        // Any remaining pixels, rounded up to vectors (safe due to padding).
        rng.fill(&mut batch);
        let mut batch_pos = 0usize; // < kFloatsPerBatch
        while x < xsize {
            row[x] = bits_to_float(batch_word(&batch, batch_pos));
            batch_pos += 1;
            x += 1;
        }
    }
}

/// Generates a random image for each of the 3 planes. Translation of
/// `Random3Planes()`.
pub(crate) fn random_3_planes(
    visible_frame_index: usize,
    nonvisible_frame_index: usize,
    x0: usize,
    y0: usize,
    planes: [(&mut ImageF, Rect); 3],
) {
    let mut rng = Xorshift128Plus::new(
        visible_frame_index as u32,
        nonvisible_frame_index as u32,
        x0 as u32,
        y0 as u32,
    );
    for (plane, rect) in planes {
        random_image(&mut rng, &rect, plane);
    }
}

/// Translation of `DecodeFloatParam()`.
fn decode_float_param(precision: f32, val: &mut f32, br: &mut BitReader<'_>) {
    let absval_quant = br.read_fixed_bits::<10>() as i32;
    *val = absval_quant as f32 / precision;
}

/// Translation of `DecodeNoise()`.
pub(crate) fn decode_noise(br: &mut BitReader<'_>, noise_params: &mut NoiseParams) -> Status {
    for i in noise_params.lut.iter_mut() {
        decode_float_param(K_NOISE_PRECISION, i, br);
    }
    Ok(())
}
