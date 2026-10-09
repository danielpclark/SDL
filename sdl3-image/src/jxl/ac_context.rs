// Rust translation of lib/jxl/ac_context.h and the decoder part of
// lib/jxl/entropy_coder.h/.cc from libjxl (https://github.com/libjxl/libjxl,
// at the revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Block contexts of the VarDCT AC coefficients.

use super::ac_strategy::K_NUM_ORDERS;
use super::base::{jxl_failure, unpack_signed, Status};
use super::dec_bit_reader::BitReader;
use super::dec_context_map::decode_context_map;
use super::fields::{bits, bits_offset, u32_coder_read, U32Enc};

// Block context used for scanning order, number of non-zeros, AC coefficients.
// Equal to the channel.
#[allow(dead_code)]
pub(crate) const K_DCT_ORDER_CONTEXT_START: u32 = 0;

// The number of predicted nonzeros goes from 0 to 1008. We use
// ceil(log2(predicted+1)) as a context for the number of nonzeros, so from 0 to
// 10, inclusive.
pub(crate) const K_NON_ZERO_BUCKETS: u32 = 37;

pub(crate) const K_COEFF_FREQ_CONTEXT: [u16; 64] = [
    0xBAD, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 15, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21,
    22, 22, 23, 23, 23, 23, 24, 24, 24, 24, 25, 25, 25, 25, 26, 26, 26, 26, 27, 27, 27, 27, 28, 28, 28, 28, 29, 29,
    29, 29, 30, 30, 30, 30,
];

pub(crate) const K_COEFF_NUM_NONZERO_CONTEXT: [u16; 64] = [
    0xBAD, 0, 31, 62, 62, 93, 93, 93, 93, 123, 123, 123, 123, 152, 152, 152, 152, 152, 152, 152, 152, 180, 180, 180,
    180, 180, 180, 180, 180, 180, 180, 180, 180, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206,
    206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206,
];

// Supremum of ZeroDensityContext(x, y) + 1, when x + y < 64.
pub(crate) const K_ZERO_DENSITY_CONTEXT_COUNT: usize = 458;
// Supremum of ZeroDensityContext(x, y) + 1.
pub(crate) const K_ZERO_DENSITY_CONTEXT_LIMIT: usize = 474;

/* This function is used for entropy-sources pre-clustering.
 *
 * Ideally, each combination of |nonzeros_left| and |k| should go to its own
 * bucket; but it implies (64 * 63 / 2) == 2016 buckets. If there is other
 * dimension (e.g. block context), then number of primary clusters becomes too
 * big.
 *
 * To solve this problem, |nonzeros_left| and |k| values are clustered. It is
 * known that their sum is at most 64, consequently, the total number buckets
 * is at most A(64) * B(64).
 */
// TODO(user): investigate, why disabling pre-clustering makes entropy code
// less dense. Perhaps we would need to add HQ clustering algorithm that would
// be able to squeeze better by spending more CPU cycles.
/// Translation of `ZeroDensityContext()`.
#[inline]
pub(crate) fn zero_density_context(
    nonzeros_left: usize,
    k: usize,
    covered_blocks: usize,
    log2_covered_blocks: usize,
    prev: usize,
) -> usize {
    debug_assert!((1usize << log2_covered_blocks) == covered_blocks);
    let nonzeros_left = (nonzeros_left + covered_blocks - 1) >> log2_covered_blocks;
    let k = k >> log2_covered_blocks;
    // Asserting nonzeros_left + k < 65 here causes crashes in debug mode with
    // invalid input, since the (hot) decoding loop does not check this condition.
    // As no out-of-bound memory reads are issued even if that condition is
    // broken, we check this simpler condition which holds anyway. The decoder
    // will still mark a file in which that condition happens as not valid at the
    // end of the decoding loop, as `nzeros` will not be `0`.
    (K_COEFF_NUM_NONZERO_CONTEXT[nonzeros_left] as usize + K_COEFF_FREQ_CONTEXT[k] as usize) * 2 + prev
}

/// Translation of `BlockCtxMap`.
#[derive(Clone, Debug)]
pub(crate) struct BlockCtxMap {
    pub dc_thresholds: [Vec<i32>; 3],
    pub qf_thresholds: Vec<u32>,
    pub ctx_map: Vec<u8>,
    pub num_ctxs: usize,
    pub num_dc_ctxs: usize,
}

impl Default for BlockCtxMap {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockCtxMap {
    const K_DEFAULT_CTX_MAP: [u8; 39] = [
        // Default ctx map clusters all the large transforms together.
        0, 1, 2, 2, 3, 3, 4, 5, 6, 6, 6, 6, 6, //
        7, 8, 9, 9, 10, 11, 12, 13, 14, 14, 14, 14, 14, //
        7, 8, 9, 9, 10, 11, 12, 13, 14, 14, 14, 14, 14, //
    ];

    pub(crate) fn new() -> Self {
        let ctx_map = Self::K_DEFAULT_CTX_MAP.to_vec();
        let num_ctxs = *ctx_map.iter().max().unwrap_or(&0) as usize + 1;
        BlockCtxMap {
            dc_thresholds: [Vec::new(), Vec::new(), Vec::new()],
            qf_thresholds: Vec::new(),
            ctx_map,
            num_ctxs,
            num_dc_ctxs: 1,
        }
    }

    /// Translation of `Context()`.
    #[inline]
    pub(crate) fn context(&self, dc_idx: i32, qf: u32, ord: usize, c: usize) -> usize {
        let mut qf_idx = 0usize;
        for &t in &self.qf_thresholds {
            if qf > t {
                qf_idx += 1;
            }
        }
        let mut idx = if c < 2 { c ^ 1 } else { 2 };
        idx = idx * K_NUM_ORDERS as usize + ord;
        idx = idx * (self.qf_thresholds.len() + 1) + qf_idx;
        idx = idx * self.num_dc_ctxs + dc_idx as usize;
        self.ctx_map[idx] as usize
    }

    // Non-zero context is based on number of non-zeros and block context.
    // For better clustering, contexts with same number of non-zeros are grouped.
    /// Translation of `ZeroDensityContextsOffset()`.
    #[inline]
    pub(crate) fn zero_density_contexts_offset(&self, block_ctx: u32) -> u32 {
        self.num_ctxs as u32 * K_NON_ZERO_BUCKETS + K_ZERO_DENSITY_CONTEXT_COUNT as u32 * block_ctx
    }

    // Context map for AC coefficients consists of 2 blocks:
    //  |num_ctxs x                : context for number of non-zeros in the block
    //   kNonZeroBuckets|            computed from block context and predicted
    //                               value (based top and left values)
    //  |num_ctxs x                : context for AC coefficient symbols,
    //   kZeroDensityContextCount|   computed from block context,
    //                               number of non-zeros left and
    //                               index in scan order
    /// Translation of `NumACContexts()`.
    pub(crate) fn num_ac_contexts(&self) -> u32 {
        self.num_ctxs as u32 * (K_NON_ZERO_BUCKETS + K_ZERO_DENSITY_CONTEXT_COUNT as u32)
    }

    // Non-zero context is based on number of non-zeros and block context.
    // For better clustering, contexts with same number of non-zeros are grouped.
    /// Translation of `NonZeroContext()`.
    #[inline]
    pub(crate) fn non_zero_context(&self, mut non_zeros: u32, block_ctx: u32) -> u32 {
        if non_zeros >= 64 {
            non_zeros = 64;
        }
        let ctx = if non_zeros < 8 { non_zeros } else { 4 + non_zeros / 2 };
        ctx * self.num_ctxs as u32 + block_ctx
    }
}

// --- entropy_coder.h ---

const K_DC_THRESHOLD_DIST: U32Enc =
    U32Enc::new(bits(4), bits_offset(8, 16), bits_offset(16, 272), bits_offset(32, 65808));

const K_QF_THRESHOLD_DIST: U32Enc = U32Enc::new(bits(2), bits_offset(3, 4), bits_offset(5, 12), bits_offset(8, 44));

/// Translation of `DecodeBlockCtxMap()`.
pub(crate) fn decode_block_ctx_map(br: &mut BitReader<'_>, block_ctx_map: &mut BlockCtxMap) -> Status {
    let is_default = br.read_fixed_bits::<1>() != 0;
    if is_default {
        *block_ctx_map = BlockCtxMap::new();
        return Ok(());
    }
    block_ctx_map.num_dc_ctxs = 1;
    for j in 0..3 {
        let n = br.read_fixed_bits::<4>() as usize;
        block_ctx_map.dc_thresholds[j].resize(n, 0);
        block_ctx_map.num_dc_ctxs *= n + 1;
        for i in block_ctx_map.dc_thresholds[j].iter_mut() {
            *i = unpack_signed(u32_coder_read(K_DC_THRESHOLD_DIST, br) as usize) as i32;
        }
    }
    let n = br.read_fixed_bits::<4>() as usize;
    block_ctx_map.qf_thresholds.resize(n, 0);
    for i in block_ctx_map.qf_thresholds.iter_mut() {
        *i = u32_coder_read(K_QF_THRESHOLD_DIST, br).wrapping_add(1);
    }

    if block_ctx_map.num_dc_ctxs * (block_ctx_map.qf_thresholds.len() + 1) > 64 {
        return jxl_failure!("Invalid block context map: too big");
    }

    let size = 3 * K_NUM_ORDERS as usize * block_ctx_map.num_dc_ctxs * (block_ctx_map.qf_thresholds.len() + 1);
    block_ctx_map.ctx_map.resize(size, 0);
    decode_context_map(&mut block_ctx_map.ctx_map, &mut block_ctx_map.num_ctxs, br)?;
    if block_ctx_map.num_ctxs > 16 {
        return jxl_failure!("Invalid block context map: too many distinct contexts");
    }
    Ok(())
}
