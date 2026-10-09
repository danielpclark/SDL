// Rust translation of lib/jxl/ac_strategy.h, lib/jxl/ac_strategy.cc and
// lib/jxl/coeff_order_fwd.h from libjxl (https://github.com/libjxl/libjxl, at
// the revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Defines the different kinds of transforms.
//! `AcStrategy` represents what transform should be used, and which
//! sub-block of that transform we are currently in. Note that DCT4x4 is
//! applied on all four 4x4 sub-blocks of an 8x8 block.
//! `AcStrategyImage` defines which strategy should be used for each 8x8
//! block of the image. The highest 4 bits represent the strategy to be used,
//! the lowest 4 represent the index of the block inside that strategy.

use super::base::{ceil_log2_nonzero_u64, jxl_failure, Status, StatusCode, K_BLOCK_DIM};
use super::image::{fill_image, fill_plane_rect, ImageB, Plane, Rect};

// --- coeff_order_fwd.h ---

/// Needs at least 16 bits. A 32-bit type speeds up DecodeAC by 2% at the
/// cost of more memory. Translation of `coeff_order_t`.
pub(crate) type CoeffOrderT = u32;

/// Maximum number of orders to be used. Note that this needs to be
/// multiplied by the number of channels. One per "size class" (plus one
/// extra for DCT8), shared between transforms of size XxY and of size YxX.
pub(crate) const K_NUM_ORDERS: u8 = 13;

/// DCT coefficients are laid out in such a way that the number of rows of
/// coefficients is always the smaller coordinate. Translation of
/// `CoefficientRows()`.
#[inline]
pub(crate) const fn coefficient_rows(rows: usize, columns: usize) -> usize {
    if rows < columns {
        rows
    } else {
        columns
    }
}

#[inline]
pub(crate) const fn coefficient_columns(rows: usize, columns: usize) -> usize {
    if rows < columns {
        columns
    } else {
        rows
    }
}

#[inline]
pub(crate) fn coefficient_layout(rows: &mut usize, columns: &mut usize) {
    let r = *rows;
    let c = *columns;
    *rows = coefficient_rows(r, c);
    *columns = coefficient_columns(r, c);
}

// --- ac_strategy.h ---

/// Raw strategy types. Translation of `AcStrategy::Type`.
pub(crate) mod acs_type {
    // Regular block size DCT
    pub(crate) const DCT: u8 = 0;
    // Encode pixels without transforming
    pub(crate) const IDENTITY: u8 = 1;
    // Use 2-by-2 DCT
    pub(crate) const DCT2X2: u8 = 2;
    // Use 4-by-4 DCT
    pub(crate) const DCT4X4: u8 = 3;
    // Use 16-by-16 DCT
    pub(crate) const DCT16X16: u8 = 4;
    // Use 32-by-32 DCT
    pub(crate) const DCT32X32: u8 = 5;
    // Use 16-by-8 DCT
    pub(crate) const DCT16X8: u8 = 6;
    // Use 8-by-16 DCT
    pub(crate) const DCT8X16: u8 = 7;
    // Use 32-by-8 DCT
    pub(crate) const DCT32X8: u8 = 8;
    // Use 8-by-32 DCT
    pub(crate) const DCT8X32: u8 = 9;
    // Use 32-by-16 DCT
    pub(crate) const DCT32X16: u8 = 10;
    // Use 16-by-32 DCT
    pub(crate) const DCT16X32: u8 = 11;
    // 4x8 and 8x4 DCT
    pub(crate) const DCT4X8: u8 = 12;
    pub(crate) const DCT8X4: u8 = 13;
    // Corner-DCT.
    pub(crate) const AFV0: u8 = 14;
    pub(crate) const AFV1: u8 = 15;
    pub(crate) const AFV2: u8 = 16;
    pub(crate) const AFV3: u8 = 17;
    // Larger DCTs
    pub(crate) const DCT64X64: u8 = 18;
    pub(crate) const DCT64X32: u8 = 19;
    pub(crate) const DCT32X64: u8 = 20;
    pub(crate) const DCT128X128: u8 = 21;
    pub(crate) const DCT128X64: u8 = 22;
    pub(crate) const DCT64X128: u8 = 23;
    pub(crate) const DCT256X256: u8 = 24;
    pub(crate) const DCT256X128: u8 = 25;
    pub(crate) const DCT128X256: u8 = 26;
    // Marker for num of valid strategies.
    pub(crate) const K_NUM_VALID_STRATEGIES: u8 = 27;
}

/// Translation of `AcStrategy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AcStrategy {
    strategy: u8,
    is_first: bool,
}

impl AcStrategy {
    // Extremal values for the number of blocks/coefficients of a single strategy.
    pub(crate) const K_MAX_COEFF_BLOCKS: usize = 32;
    pub(crate) const K_MAX_BLOCK_DIM: usize = K_BLOCK_DIM * Self::K_MAX_COEFF_BLOCKS;
    // Maximum number of coefficients in a block. Guaranteed to be a multiple of
    // the vector size.
    pub(crate) const K_MAX_COEFF_AREA: usize = Self::K_MAX_BLOCK_DIM * Self::K_MAX_BLOCK_DIM;

    pub(crate) const K_NUM_VALID_STRATEGIES: u8 = acs_type::K_NUM_VALID_STRATEGIES;

    #[inline]
    pub(crate) const fn type_bit(t: u8) -> u32 {
        1u32 << t
    }

    // Returns true if this block is the first 8x8 block (i.e. top-left) of a
    // possibly multi-block strategy.
    #[inline]
    pub(crate) fn is_first_block(&self) -> bool {
        self.is_first
    }

    #[inline]
    pub(crate) fn is_multiblock(&self) -> bool {
        use acs_type::*;
        let bits: u32 = Self::type_bit(DCT16X16)
            | Self::type_bit(DCT32X32)
            | Self::type_bit(DCT16X8)
            | Self::type_bit(DCT8X16)
            | Self::type_bit(DCT32X8)
            | Self::type_bit(DCT8X32)
            | Self::type_bit(DCT16X32)
            | Self::type_bit(DCT32X16)
            | Self::type_bit(DCT32X64)
            | Self::type_bit(DCT64X32)
            | Self::type_bit(DCT64X64)
            | Self::type_bit(DCT64X128)
            | Self::type_bit(DCT128X64)
            | Self::type_bit(DCT128X128)
            | Self::type_bit(DCT128X256)
            | Self::type_bit(DCT256X128)
            | Self::type_bit(DCT256X256);
        ((1u32 << self.strategy) & bits) != 0
    }

    // Returns the raw strategy value. Should only be used for tokenization.
    #[inline]
    pub(crate) fn raw_strategy(&self) -> u8 {
        self.strategy
    }

    #[inline]
    pub(crate) fn strategy(&self) -> u8 {
        self.strategy
    }

    // Inverse check
    #[inline]
    pub(crate) const fn is_raw_strategy_valid(raw_strategy: i32) -> bool {
        raw_strategy < Self::K_NUM_VALID_STRATEGIES as i32 && raw_strategy >= 0
    }

    #[inline]
    pub(crate) fn from_raw_strategy(raw_strategy: u8) -> AcStrategy {
        debug_assert!(Self::is_raw_strategy_valid(raw_strategy as i32));
        AcStrategy {
            strategy: raw_strategy,
            is_first: true,
        }
    }

    // "Natural order" means the order of increasing of "anisotropic" frequency of
    // continuous version of DCT basis.
    pub(crate) fn compute_natural_coeff_order(&self, order: &mut [CoeffOrderT]) {
        coeff_order_and_lut::<false>(*self, order);
    }
    #[allow(dead_code)]
    pub(crate) fn compute_natural_coeff_order_lut(&self, lut: &mut [CoeffOrderT]) {
        coeff_order_and_lut::<true>(*self, lut);
    }

    // Number of 8x8 blocks that this strategy will cover. 0 for non-top-left
    // blocks inside a multi-block transform.
    #[inline]
    pub(crate) fn covered_blocks_x(&self) -> usize {
        const K_LUT: [u8; 27] = [
            1, 1, 1, 1, 2, 4, 1, 2, 1, 4, 2, 4, 1, 1, 1, 1, 1, 1, 8, 4, 8, 16, 8, 16, 32, 16, 32,
        ];
        K_LUT[self.strategy as usize] as usize
    }

    #[inline]
    pub(crate) fn covered_blocks_y(&self) -> usize {
        const K_LUT: [u8; 27] = [
            1, 1, 1, 1, 2, 4, 2, 1, 4, 1, 4, 2, 1, 1, 1, 1, 1, 1, 8, 8, 4, 16, 16, 8, 32, 32, 16,
        ];
        K_LUT[self.strategy as usize] as usize
    }

    #[inline]
    pub(crate) fn log2_covered_blocks(&self) -> usize {
        const K_LUT: [u8; 27] = [
            0, 0, 0, 0, 2, 4, 1, 1, 2, 2, 3, 3, 0, 0, 0, 0, 0, 0, 6, 5, 5, 8, 7, 7, 10, 9, 9,
        ];
        K_LUT[self.strategy as usize] as usize
    }
}

/// Class to use a certain row of the AC strategy. Translation of
/// `AcStrategyRow`.
#[derive(Clone, Copy)]
pub(crate) struct AcStrategyRow<'a> {
    row: &'a [u8],
}

impl AcStrategyRow<'_> {
    #[inline]
    pub(crate) fn get(&self, x: usize) -> AcStrategy {
        AcStrategy {
            strategy: self.row[x] >> 1,
            is_first: self.row[x] & 1 != 0,
        }
    }
}

/// Translation of `AcStrategyImage`.
#[derive(Clone, Debug, Default)]
pub(crate) struct AcStrategyImage {
    layers: ImageB,
}

impl AcStrategyImage {
    // A value that does not represent a valid combined AC strategy
    // value. Used as a sentinel.
    const INVALID: u8 = 0xFF;

    pub(crate) fn new(xsize: usize, ysize: usize) -> Result<Self, StatusCode> {
        Ok(AcStrategyImage {
            layers: Plane::new(xsize, ysize)?,
        })
    }

    pub(crate) fn fill_dct8_rect(&mut self, rect: &Rect) {
        fill_plane_rect((acs_type::DCT << 1) | 1, &mut self.layers, rect);
    }
    pub(crate) fn fill_dct8(&mut self) {
        let r = Rect::from_plane(&self.layers);
        self.fill_dct8_rect(&r);
    }

    pub(crate) fn fill_invalid(&mut self) {
        fill_image(Self::INVALID, &mut self.layers);
    }

    /// Translation of `SetNoBoundsCheck()` (the caller checked the bounds).
    pub(crate) fn set_no_bounds_check(&mut self, x: usize, y: usize, type_: u8, check: bool) -> Status {
        let acs = AcStrategy::from_raw_strategy(type_);
        let stride = self.layers.pixels_per_row();
        let row = self.layers.data_mut();
        for iy in 0..acs.covered_blocks_y() {
            for ix in 0..acs.covered_blocks_x() {
                let pos = (y + iy) * stride + x + ix;
                if check && row[pos] != Self::INVALID {
                    return jxl_failure!("Invalid AC strategy: block overlap");
                }
                row[pos] = (type_ << 1) | if (iy | ix) == 0 { 1 } else { 0 };
            }
        }
        Ok(())
    }

    /// Translation of `Set()`.
    #[allow(dead_code)]
    pub(crate) fn set(&mut self, x: usize, y: usize, type_: u8) -> Status {
        let acs = AcStrategy::from_raw_strategy(type_);
        if y + acs.covered_blocks_y() > self.layers.ysize() || x + acs.covered_blocks_x() > self.layers.xsize() {
            return jxl_failure!("AC strategy out of bounds");
        }
        self.set_no_bounds_check(x, y, type_, false)
    }

    pub(crate) fn is_valid(&self, x: usize, y: usize) -> bool {
        self.layers.data()[y * self.layers.pixels_per_row() + x] != Self::INVALID
    }

    pub(crate) fn const_row(&self, y: usize, x_prefix: usize) -> AcStrategyRow<'_> {
        AcStrategyRow {
            row: &self.layers.row(y)[x_prefix..],
        }
    }

    pub(crate) fn const_row_rect(&self, rect: &Rect, y: usize) -> AcStrategyRow<'_> {
        self.const_row(rect.y0() + y, rect.x0())
    }

    pub(crate) fn pixels_per_row(&self) -> usize {
        self.layers.pixels_per_row()
    }

    pub(crate) fn xsize(&self) -> usize {
        self.layers.xsize()
    }
    pub(crate) fn ysize(&self) -> usize {
        self.layers.ysize()
    }

    // Count the number of blocks of a given type.
    #[allow(dead_code)]
    pub(crate) fn count_blocks(&self, type_: u8) -> usize {
        let mut ret = 0;
        for y in 0..self.layers.ysize() {
            let row = self.layers.row(y);
            for &v in &row[..self.layers.xsize()] {
                if v == ((type_ << 1) | 1) {
                    ret += 1;
                }
            }
        }
        ret
    }
}

// --- ac_strategy.cc ---

/// Tries to generalize zig-zag order to non-square blocks. Surprisingly, in
/// square block frequency along the (i + j == const) diagonals is roughly
/// the same. For historical reasons, consecutive diagonals are traversed in
/// alternating directions - so called "zig-zag" (or "snake") order.
/// Translation of `CoeffOrderAndLut<is_lut>()`.
fn coeff_order_and_lut<const IS_LUT: bool>(acs: AcStrategy, out: &mut [CoeffOrderT]) {
    let mut cx = acs.covered_blocks_x();
    let mut cy = acs.covered_blocks_y();
    coefficient_layout(&mut cy, &mut cx);

    // CoefficientLayout ensures cx >= cy.
    // We compute the zigzag order for a cx x cx block, then discard all the
    // lines that are not multiple of the ratio between cx and cy.
    let xs = cx / cy;
    let xsm = xs - 1;
    let xss = ceil_log2_nonzero_u64(xs as u64);
    // First half of the block
    let mut cur = cx * cy;
    for i in 0..cx * K_BLOCK_DIM {
        for j in 0..=i {
            let mut x = j;
            let mut y = i - j;
            if i % 2 != 0 {
                std::mem::swap(&mut x, &mut y);
            }
            if (y & xsm) != 0 {
                continue;
            }
            y >>= xss;
            let val;
            if x < cx && y < cy {
                val = y * cx + x;
            } else {
                val = cur;
                cur += 1;
            }
            if IS_LUT {
                out[y * cx * K_BLOCK_DIM + x] = val as CoeffOrderT;
            } else {
                out[val] = (y * cx * K_BLOCK_DIM + x) as CoeffOrderT;
            }
        }
    }
    // Second half
    let mut ip = cx * K_BLOCK_DIM - 1;
    while ip > 0 {
        let i = ip - 1;
        for j in 0..=i {
            let mut x = cx * K_BLOCK_DIM - 1 - (i - j);
            let mut y = cx * K_BLOCK_DIM - 1 - j;
            if i % 2 != 0 {
                std::mem::swap(&mut x, &mut y);
            }
            if (y & xsm) != 0 {
                continue;
            }
            y >>= xss;
            let val = cur;
            cur += 1;
            if IS_LUT {
                out[y * cx * K_BLOCK_DIM + x] = val as CoeffOrderT;
            } else {
                out[val] = (y * cx * K_BLOCK_DIM + x) as CoeffOrderT;
            }
        }
        ip -= 1;
    }
}
