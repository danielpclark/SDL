// Rust translation of lib/jxl/chroma_from_luma.h and
// lib/jxl/chroma_from_luma.cc from libjxl (https://github.com/libjxl/libjxl,
// at the revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Chroma-from-luma, computed using heuristics to determine the best linear
//! model for the X and B channels from the Y channel.

use super::base::{div_ceil, jxl_failure, Status, StatusCode, K_BITS_PER_BYTE, K_BLOCK_DIM};
use super::dec_bit_reader::BitReader;
use super::fields::{bits_offset, f16_coder_read, u32_coder_read, val, U32Enc};
use super::image::{zero_fill_image, ImageSB};

// Tile is the rectangular grid of blocks that share color correlation
// parameters ("factor_x/b" such that residual_b = blue - Y * factor_b).
pub(crate) const K_COLOR_TILE_DIM: usize = 64;

pub(crate) const K_COLOR_TILE_DIM_IN_BLOCKS: usize = K_COLOR_TILE_DIM / K_BLOCK_DIM;

pub(crate) const K_DEFAULT_COLOR_FACTOR: u32 = 84;

// JPEG DCT coefficients are integers, therefore we represent DC factors as
// integers, too.
#[allow(dead_code)]
pub(crate) const K_CFL_FIXED_POINT_PRECISION: u8 = 11;

const K_COLOR_FACTOR_DIST: U32Enc =
    U32Enc::new(val(K_DEFAULT_COLOR_FACTOR), val(256), bits_offset(8, 2), bits_offset(16, 258));

// (opsin_params.h)
const K_Y_TO_B_RATIO: f32 = 1.0; // works better with 0.50017729543783418

/// Translation of `ColorCorrelationMap`.
#[derive(Clone, Debug)]
pub(crate) struct ColorCorrelationMap {
    pub ytox_map: ImageSB,
    pub ytob_map: ImageSB,

    dc_factors: [f32; 4],
    color_factor: u32,
    color_scale: f32,
    base_correlation_x: f32,
    base_correlation_b: f32,
    ytox_dc: i32,
    ytob_dc: i32,
}

impl Default for ColorCorrelationMap {
    fn default() -> Self {
        ColorCorrelationMap {
            ytox_map: ImageSB::empty(),
            ytob_map: ImageSB::empty(),
            dc_factors: [0.0; 4],
            color_factor: K_DEFAULT_COLOR_FACTOR,
            color_scale: 1.0f32 / K_DEFAULT_COLOR_FACTOR as f32,
            base_correlation_x: 0.0,
            base_correlation_b: K_Y_TO_B_RATIO,
            ytox_dc: 0,
            ytob_dc: 0,
        }
    }
}

impl ColorCorrelationMap {
    /// Translation of `ColorCorrelationMap(xsize, ysize, XYB)`.
    pub(crate) fn new(xsize: usize, ysize: usize, xyb: bool) -> Result<Self, StatusCode> {
        let mut m = ColorCorrelationMap {
            ytox_map: ImageSB::new(div_ceil(xsize, K_COLOR_TILE_DIM), div_ceil(ysize, K_COLOR_TILE_DIM))?,
            ytob_map: ImageSB::new(div_ceil(xsize, K_COLOR_TILE_DIM), div_ceil(ysize, K_COLOR_TILE_DIM))?,
            ..Default::default()
        };
        zero_fill_image(&mut m.ytox_map);
        zero_fill_image(&mut m.ytob_map);
        if !xyb {
            m.base_correlation_b = 0.0;
        }
        m.recompute_dc_factors();
        Ok(m)
    }

    #[inline]
    pub(crate) fn y_to_x_ratio(&self, x_factor: i32) -> f32 {
        self.base_correlation_x + x_factor as f32 * self.color_scale
    }

    #[inline]
    pub(crate) fn y_to_b_ratio(&self, b_factor: i32) -> f32 {
        self.base_correlation_b + b_factor as f32 * self.color_scale
    }

    /// Translation of `DecodeDC()`.
    pub(crate) fn decode_dc(&mut self, br: &mut BitReader<'_>) -> Status {
        if br.read_fixed_bits::<1>() == 1 {
            return Ok(());
        }
        self.set_color_factor(u32_coder_read(K_COLOR_FACTOR_DIST, br));
        f16_coder_read(br, &mut self.base_correlation_x)?;
        if self.base_correlation_x.abs() > 4.0f32 {
            return jxl_failure!("Base X correlation is out of range");
        }
        f16_coder_read(br, &mut self.base_correlation_b)?;
        if self.base_correlation_b.abs() > 4.0f32 {
            return jxl_failure!("Base B correlation is out of range");
        }
        self.ytox_dc = br.read_fixed_bits::<K_BITS_PER_BYTE>() as i32 + i8::MIN as i32;
        self.ytob_dc = br.read_fixed_bits::<K_BITS_PER_BYTE>() as i32 + i8::MIN as i32;
        self.recompute_dc_factors();
        Ok(())
    }

    fn set_color_factor(&mut self, factor: u32) {
        self.color_factor = factor;
        self.color_scale = 1.0f32 / self.color_factor as f32;
        self.recompute_dc_factors();
    }

    #[inline]
    pub(crate) fn dc_factors(&self) -> &[f32; 4] {
        &self.dc_factors
    }

    fn recompute_dc_factors(&mut self) {
        self.dc_factors[0] = self.y_to_x_ratio(self.ytox_dc);
        self.dc_factors[2] = self.y_to_b_ratio(self.ytob_dc);
    }
}
