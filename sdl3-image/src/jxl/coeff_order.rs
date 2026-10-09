// Rust translation of lib/jxl/coeff_order.h, lib/jxl/coeff_order.cc and
// lib/jxl/lehmer_code.h from libjxl (https://github.com/libjxl/libjxl, at the
// revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Coefficient orders and permutations (Lehmer codes).

use super::ac_strategy::{AcStrategy, CoeffOrderT, K_NUM_ORDERS};
use super::base::{ceil_log2_nonzero_u64, jxl_failure, Status, K_DCT_BLOCK_SIZE};
use super::dec_ans::{decode_histograms, AnsCode, AnsSymbolReader, HybridUintConfig};
use super::dec_bit_reader::BitReader;

// --- lehmer_code.h ---

// Permutation <=> factorial base representation (Lehmer code).

pub(crate) type LehmerT = u32;

#[inline]
fn value_of_lowest_1_bit_u32(t: u32) -> u32 {
    t & t.wrapping_neg()
}

#[inline]
fn value_of_lowest_1_bit_usize(t: usize) -> usize {
    t & t.wrapping_neg()
}

/// Decodes the Lehmer code in code[0..n) into permutation[0..n).
/// temp must have 1 << CeilLog2(n) elements but need not be initialized.
/// Translation of `DecodeLehmerCode()`.
pub(crate) fn decode_lehmer_code(
    code: &[LehmerT],
    temp: &mut [u32],
    n: usize,
    permutation: &mut [CoeffOrderT],
) {
    debug_assert!(n != 0);
    let log2n = ceil_log2_nonzero_u64(n as u64);
    let padded_n = 1usize << log2n;

    for i in 0..padded_n {
        let i1 = (i + 1) as i32;
        temp[i] = value_of_lowest_1_bit_u32(i1 as u32);
    }

    for i in 0..n {
        let mut rank = code[i] + 1;

        // Extract i-th unused element via implicit order-statistics tree.
        let mut bit = padded_n;
        let mut next = 0usize;
        for _ in 0..=log2n {
            let cand = next + bit;
            bit >>= 1;
            if temp[cand - 1] < rank {
                next = cand;
                rank -= temp[cand - 1];
            }
        }

        permutation[i] = next as CoeffOrderT;

        // Mark as used
        next += 1;
        while next <= padded_n {
            temp[next - 1] = temp[next - 1].wrapping_sub(1);
            next += value_of_lowest_1_bit_usize(next);
        }
    }
}

// --- coeff_order.h ---

// Those offsets get multiplied by kDCTBlockSize.
pub(crate) const K_COEFF_ORDER_OFFSET: [usize; 40] = [
    0, 1, 2, 3, 4, 5, 6, 10, 14, 18, 34, 50, 66, 68, 70, 72, 76, 80, 84, 92, 100, 108, 172, 236,
    300, 332, 364, 396, 652, 908, 1164, 1292, 1420, 1548, 2572, 3596, 4620, 5132, 5644, 6156,
];

/// Translation of `CoeffOrderOffset()`.
#[inline]
pub(crate) const fn coeff_order_offset(order: usize, c: usize) -> usize {
    K_COEFF_ORDER_OFFSET[3 * order + c] * K_DCT_BLOCK_SIZE
}

pub(crate) const K_COEFF_ORDER_MAX_SIZE: usize =
    K_COEFF_ORDER_OFFSET[3 * K_NUM_ORDERS as usize] * K_DCT_BLOCK_SIZE;

// Mapping from AC strategy to order bucket. Strategies with different natural
// orders must have different buckets.
pub(crate) const K_STRATEGY_ORDER: [u8; 27] = [
    0, 1, 1, 1, 2, 3, 4, 4, 5, 5, 6, 6, 1, 1, 1, 1, 1, 1, 7, 8, 8, 9, 10, 10, 11, 12, 12,
];

pub(crate) const K_PERMUTATION_CONTEXTS: u32 = 8;

// --- coeff_order.cc ---

/// Translation of `CoeffOrderContext()`.
pub(crate) fn coeff_order_context(val: u32) -> u32 {
    // (HybridUintConfig(0, 0, 0).Encode(val, &token, &nbits, &bits))
    let config = HybridUintConfig::new(0, 0, 0);
    let token = if val < config.split_token {
        val
    } else {
        let n = super::base::floor_log2_nonzero_u32(val) as u32;
        config.split_token + (n - config.split_exponent)
    };
    token.min(K_PERMUTATION_CONTEXTS - 1)
}

/// Translation of `ReadPermutation()`.
fn read_permutation(
    skip: usize,
    size: usize,
    order: Option<&mut [CoeffOrderT]>,
    br: &mut BitReader<'_>,
    reader: &mut AnsSymbolReader<'_>,
    context_map: &[u8],
) -> Status {
    let mut lehmer: Vec<LehmerT> = vec![0; size];
    // temp space needs to be as large as the next power of 2, so doubling the
    // allocated size is enough.
    let mut temp: Vec<u32> = vec![0; size * 2];
    let end = (reader.read_hybrid_uint(coeff_order_context(size as u32) as usize, br, context_map)
        as u32)
        .wrapping_add(skip as u32);
    if end as usize > size {
        return jxl_failure!("Invalid permutation size");
    }
    let mut last: u32 = 0;
    for i in skip..end as usize {
        lehmer[i] =
            reader.read_hybrid_uint(coeff_order_context(last) as usize, br, context_map) as u32;
        last = lehmer[i];
        if lehmer[i] as usize + i >= size {
            return jxl_failure!("Invalid lehmer code");
        }
    }
    let Some(order) = order else {
        return Ok(());
    };
    decode_lehmer_code(&lehmer, &mut temp, size, order);
    Ok(())
}

/// Translation of `DecodePermutation()`.
pub(crate) fn decode_permutation(
    skip: usize,
    size: usize,
    order: Option<&mut [CoeffOrderT]>,
    br: &mut BitReader<'_>,
) -> Status {
    let mut context_map: Vec<u8> = Vec::new();
    let mut code = AnsCode::default();
    decode_histograms(
        br,
        K_PERMUTATION_CONTEXTS as usize,
        &mut code,
        &mut context_map,
        false,
    )?;
    let mut reader = AnsSymbolReader::new(&code, br, 0);
    read_permutation(skip, size, order, br, &mut reader, &context_map)?;
    if !reader.check_ans_final_state() {
        return jxl_failure!("Invalid ANS stream");
    }
    Ok(())
}

/// Translation of `DecodeCoeffOrder()`.
fn decode_coeff_order(
    acs: AcStrategy,
    order: Option<&mut [CoeffOrderT]>,
    br: &mut BitReader<'_>,
    reader: &mut AnsSymbolReader<'_>,
    natural_order: &[CoeffOrderT],
    context_map: &[u8],
) -> Status {
    let llf = acs.covered_blocks_x() * acs.covered_blocks_y();
    let size = K_DCT_BLOCK_SIZE * llf;

    match order {
        None => read_permutation(llf, size, None, br, reader, context_map),
        Some(order) => {
            read_permutation(llf, size, Some(&mut *order), br, reader, context_map)?;
            for k in 0..size {
                order[k] = natural_order[order[k] as usize];
            }
            Ok(())
        }
    }
}

/// Translation of `DecodeCoeffOrders()`.
pub(crate) fn decode_coeff_orders(
    used_orders: u16,
    used_acs: u32,
    order: &mut [CoeffOrderT],
    br: &mut BitReader<'_>,
) -> Status {
    let mut computed: u16 = 0;
    let mut context_map: Vec<u8> = Vec::new();
    let mut code = AnsCode::default();
    let mut natural_order: Vec<CoeffOrderT> = Vec::new();
    // Bitstream does not have histograms if no coefficient order is used.
    if used_orders != 0 {
        decode_histograms(
            br,
            K_PERMUTATION_CONTEXTS as usize,
            &mut code,
            &mut context_map,
            false,
        )?;
    }
    let mut reader = if used_orders != 0 {
        Some(AnsSymbolReader::new(&code, br, 0))
    } else {
        None
    };
    let mut acs_mask: u32 = 0;
    for o in 0..AcStrategy::K_NUM_VALID_STRATEGIES {
        if (used_acs & (1 << o)) == 0 {
            continue;
        }
        acs_mask |= 1 << K_STRATEGY_ORDER[o as usize];
    }
    for o in 0..AcStrategy::K_NUM_VALID_STRATEGIES {
        let ord = K_STRATEGY_ORDER[o as usize];
        if computed & (1 << ord) != 0 {
            continue;
        }
        computed |= 1 << ord;
        let acs = AcStrategy::from_raw_strategy(o);
        let used = (acs_mask & (1 << ord)) != 0;

        let llf = acs.covered_blocks_x() * acs.covered_blocks_y();
        let size = K_DCT_BLOCK_SIZE * llf;

        if used || (used_orders & (1 << ord)) != 0 {
            if natural_order.len() < size {
                natural_order.resize(size, 0);
            }
            acs.compute_natural_coeff_order(&mut natural_order);
        }

        if (used_orders & (1 << ord)) == 0 {
            // No need to set the default order if no ACS uses this order.
            if used {
                for c in 0..3 {
                    let off = coeff_order_offset(ord as usize, c);
                    order[off..off + size].copy_from_slice(&natural_order[..size]);
                }
            }
        } else {
            for c in 0..3 {
                let off = coeff_order_offset(ord as usize, c);
                let dest = if used {
                    Some(&mut order[off..off + size])
                } else {
                    None
                };
                let reader = reader.as_mut().expect("reader for used orders");
                decode_coeff_order(acs, dest, br, reader, &natural_order, &context_map)?;
            }
        }
    }
    if used_orders != 0 && !reader.as_ref().is_some_and(|r| r.check_ans_final_state()) {
        return jxl_failure!("Invalid ANS stream");
    }
    Ok(())
}
