// Rust translation of lib/jxl/quantizer.h and lib/jxl/quantizer.cc from
// libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Quantizer: the global and per-block quantization scales. (Upstream the
//! quantizer points to the dequantization matrices; here they are passed to
//! the methods that read them. The encoder-side methods are not translated.)

use super::base::Status;
use super::dec_bit_reader::BitReader;
use super::fields::{bits_offset, bundle_init, bundle_read, val, Fields, Visitor};
use super::quant_weights::DequantMatrices;

const K_GLOBAL_SCALE_DENOM: i32 = 1 << 16;

// zero-biases for quantizing channels X, Y, B
const K_ZERO_BIAS_DEFAULT: [f32; 3] = [0.5, 0.5, 0.5];

// Returns adjusted version of a quantized integer, such that its value is
// closer to the expected value of the original.
// The residuals of AC coefficients that we quantize are not uniformly
// distributed. Numerical experiments show that they have a distribution with
// the "shape" of 1/(1+x^2) [up to some coefficients]. This means that the
// expected value of a coefficient that gets quantized to x will not be x
// itself, but (at least with reasonable approximation):
// - 0 if x is 0
// - x * biases[c] if x is 1 or -1
// - x - biases[3]/x otherwise
// This follows from computing the distribution of the quantization bias, which
// can be approximated fairly well by <constant>/x when |x| is at least two.
#[allow(dead_code)]
pub(crate) const K_BIAS_NUMERATOR: f32 = 0.145;

pub(crate) const K_DEFAULT_QUANT_BIAS: [f32; 4] = [
    1.0 - 0.05465007330715401,
    1.0 - 0.07005449891748593,
    1.0 - 0.049935103337343655,
    0.145,
];

const K_DEFAULT_QUANT: i32 = 64;

/// Translation of `Quantizer`.
#[derive(Clone, Debug)]
pub(crate) struct Quantizer {
    mul_dc: [f32; 4],
    inv_mul_dc: [f32; 4],

    // These are serialized:
    global_scale: i32,
    quant_dc: i32,

    // These are derived from global_scale_:
    inv_global_scale: f32,
    global_scale_float: f32, // reciprocal of inv_global_scale_
    inv_quant_dc: f32,

    #[allow(dead_code)]
    zero_bias: [f32; 3],
}

impl Quantizer {
    pub(crate) const K_QUANT_MAX: i32 = 256;

    /// Translation of `Quantizer(dequant)`.
    pub(crate) fn new(dequant: &DequantMatrices) -> Self {
        Self::with_params(dequant, K_DEFAULT_QUANT, K_GLOBAL_SCALE_DENOM / K_DEFAULT_QUANT)
    }

    /// Translation of `Quantizer(dequant, quant_dc, global_scale)`.
    fn with_params(dequant: &DequantMatrices, quant_dc: i32, global_scale: i32) -> Self {
        let mut q = Quantizer {
            mul_dc: [0.0; 4],
            inv_mul_dc: [0.0; 4],
            global_scale,
            quant_dc,
            inv_global_scale: 0.0,
            global_scale_float: 0.0,
            inv_quant_dc: 0.0,
            zero_bias: K_ZERO_BIAS_DEFAULT,
        };
        q.recompute_from_global_scale(dequant);
        q.inv_quant_dc = q.inv_global_scale / q.quant_dc as f32;
        q
    }

    /// Recomputes other derived fields after global_scale_ has changed.
    /// Translation of `RecomputeFromGlobalScale()`.
    fn recompute_from_global_scale(&mut self, dequant: &DequantMatrices) {
        self.global_scale_float = (self.global_scale as f64 * (1.0 / K_GLOBAL_SCALE_DENOM as f64)) as f32;
        self.inv_global_scale = (1.0 * K_GLOBAL_SCALE_DENOM as f64 / self.global_scale as f64) as f32;
        self.inv_quant_dc = self.inv_global_scale / self.quant_dc as f32;
        for c in 0..3 {
            self.mul_dc[c] = self.get_dc_step(c, dequant);
            self.inv_mul_dc[c] = self.get_inv_dc_step(c, dequant);
        }
    }

    /// Returns scaling factor such that Scale() * (RawDC() or
    /// RawQuantField()) pixels yields the same float values returned by
    /// GetQuantField.
    #[inline]
    pub(crate) fn scale(&self) -> f32 {
        self.global_scale_float
    }

    /// Reciprocal of Scale().
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn inv_global_scale(&self) -> f32 {
        self.inv_global_scale
    }

    /// Dequantize by multiplying with this times dequant_matrix.
    #[inline]
    pub(crate) fn inv_quant_ac(&self, quant: i32) -> f32 {
        self.inv_global_scale / quant as f32
    }

    /// Translation of `Decode()`.
    pub(crate) fn decode(&mut self, reader: &mut BitReader<'_>, dequant: &DequantMatrices) -> Status {
        let mut params = QuantizerParams::new();
        bundle_read(reader, &mut params)?;
        self.global_scale = params.global_scale as i32;
        self.quant_dc = params.quant_dc as i32;
        self.recompute_from_global_scale(dequant);
        Ok(())
    }

    /// Calculates DC quantization step.
    #[inline]
    fn get_dc_step(&self, c: usize, dequant: &DequantMatrices) -> f32 {
        self.inv_quant_dc * dequant.dc_quant(c)
    }
    #[inline]
    fn get_inv_dc_step(&self, c: usize, dequant: &DequantMatrices) -> f32 {
        dequant.inv_dc_quant(c) * (self.global_scale_float * self.quant_dc as f32)
    }

    #[inline]
    pub(crate) fn mul_dc(&self) -> &[f32; 4] {
        &self.mul_dc
    }
}

/// Translation of `QuantizerParams`.
#[derive(Clone, Debug, Default)]
struct QuantizerParams {
    global_scale: u32,
    quant_dc: u32,
}

impl QuantizerParams {
    fn new() -> Self {
        let mut p = QuantizerParams::default();
        bundle_init(&mut p);
        p
    }
}

impl Fields for QuantizerParams {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visitor.u32d(
            bits_offset(11, 1),
            bits_offset(11, 2049),
            bits_offset(12, 4097),
            bits_offset(16, 8193),
            1,
            &mut self.global_scale,
        )?;
        visitor.u32d(
            val(16),
            bits_offset(5, 1),
            bits_offset(8, 1),
            bits_offset(16, 1),
            1,
            &mut self.quant_dc,
        )?;
        Ok(())
    }
}
