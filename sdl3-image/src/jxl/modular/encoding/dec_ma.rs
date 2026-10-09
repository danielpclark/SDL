// Rust translation of lib/jxl/modular/encoding/ma_common.h,
// lib/jxl/modular/encoding/dec_ma.h and lib/jxl/modular/encoding/dec_ma.cc
// from libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The meta-adaptive (MA) tree decoder.

use super::super::super::base::{jxl_failure, unpack_signed, Status};
use super::super::super::dec_ans::{decode_histograms, AnsCode, AnsSymbolReader};
use super::super::super::dec_bit_reader::BitReader;
use super::super::modular_image::PixelType;
use super::super::options::{Predictor, PropertyVal, K_NUM_MODULAR_PREDICTORS};

// --- ma_common.h ---

pub(crate) const K_SPLIT_VAL_CONTEXT: usize = 0;
pub(crate) const K_PROPERTY_CONTEXT: usize = 1;
pub(crate) const K_PREDICTOR_CONTEXT: usize = 2;
pub(crate) const K_OFFSET_CONTEXT: usize = 3;
pub(crate) const K_MULTIPLIER_LOG_CONTEXT: usize = 4;
pub(crate) const K_MULTIPLIER_BITS_CONTEXT: usize = 5;

pub(crate) const K_NUM_TREE_CONTEXTS: usize = 6;

pub(crate) const K_MAX_TREE_SIZE: usize = 1 << 22;

// --- dec_ma.h ---

/// inner nodes. Translation of `PropertyDecisionNode`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PropertyDecisionNode {
    pub splitval: PropertyVal,
    pub property: i16, // -1: leaf node, lchild points to leaf node
    pub lchild: u32,
    pub rchild: u32,
    pub predictor: Predictor,
    pub predictor_offset: i64,
    pub multiplier: u32,
}

pub(crate) type Tree = Vec<PropertyDecisionNode>;

// --- dec_ma.cc ---

/// Translation of `ValidateTree()` (its range table allocated fallibly).
fn validate_tree(tree: &Tree) -> Status {
    let mut num_properties: i32 = 0;
    for node in tree {
        if node.property as i32 >= num_properties {
            num_properties = node.property as i32 + 1;
        }
    }
    let mut height = vec![0i32; tree.len()];
    let n = (num_properties as usize).checked_mul(tree.len());
    let Some(n) = n else {
        return jxl_failure!("out of memory");
    };
    let mut property_ranges: Vec<(PixelType, PixelType)> = Vec::new();
    if property_ranges.try_reserve_exact(n).is_err() {
        return jxl_failure!("out of memory");
    }
    property_ranges.resize(n, (0, 0));
    for i in 0..num_properties as usize {
        property_ranges[i].0 = PixelType::MIN;
        property_ranges[i].1 = PixelType::MAX;
    }
    const K_HEIGHT_LIMIT: i32 = 2048;
    let np = num_properties as usize;
    for i in 0..tree.len() {
        if height[i] > K_HEIGHT_LIMIT {
            return jxl_failure!("Tree too tall");
        }
        if tree[i].property == -1 {
            continue;
        }
        let lc = tree[i].lchild as usize;
        let rc = tree[i].rchild as usize;
        height[lc] = height[i] + 1;
        height[rc] = height[i] + 1;
        for p in 0..np {
            if p == tree[i].property as usize {
                let l = property_ranges[i * np + p].0;
                let u = property_ranges[i * np + p].1;
                let val = tree[i].splitval;
                if l > val || u <= val {
                    return jxl_failure!("Invalid tree");
                }
                property_ranges[lc * np + p] = (val + 1, u);
                property_ranges[rc * np + p] = (l, val);
            } else {
                property_ranges[lc * np + p] = property_ranges[i * np + p];
                property_ranges[rc * np + p] = property_ranges[i * np + p];
            }
        }
    }
    Ok(())
}

/// Translation of the inner `DecodeTree()`.
fn decode_tree_inner(
    br: &mut BitReader<'_>,
    reader: &mut AnsSymbolReader<'_>,
    context_map: &[u8],
    tree: &mut Tree,
    tree_size_limit: usize,
) -> Status {
    let mut leaf_id: usize = 0;
    let mut to_decode: usize = 1;
    tree.clear();
    while to_decode > 0 {
        if !br.all_reads_within_bounds() {
            return jxl_failure!("out of bounds");
        }
        if tree.len() > tree_size_limit {
            return jxl_failure!("Tree is too large");
        }
        to_decode -= 1;
        let prop1 = reader.read_hybrid_uint(K_PROPERTY_CONTEXT, br, context_map) as u32;
        if prop1 > 256 {
            return jxl_failure!("Invalid tree property value");
        }
        let property = prop1 as i32 - 1;
        if property == -1 {
            let predictor = reader.read_hybrid_uint(K_PREDICTOR_CONTEXT, br, context_map);
            if predictor >= K_NUM_MODULAR_PREDICTORS {
                return jxl_failure!("Invalid predictor");
            }
            let predictor_offset =
                unpack_signed(reader.read_hybrid_uint(K_OFFSET_CONTEXT, br, context_map)) as i64;
            let mul_log = reader.read_hybrid_uint(K_MULTIPLIER_LOG_CONTEXT, br, context_map) as u32;
            if mul_log >= 31 {
                return jxl_failure!("Invalid multiplier logarithm");
            }
            let mul_bits = reader.read_hybrid_uint(K_MULTIPLIER_BITS_CONTEXT, br, context_map) as u32;
            if mul_bits.wrapping_add(1) >= 1u32 << (31 - mul_log) {
                return jxl_failure!("Invalid multiplier");
            }
            let multiplier = mul_bits.wrapping_add(1) << mul_log;
            tree.push(PropertyDecisionNode {
                splitval: 0,
                property: -1,
                lchild: leaf_id as u32,
                rchild: 0,
                predictor: Predictor::from_u32(predictor as u32),
                predictor_offset,
                multiplier,
            });
            leaf_id += 1;
            continue;
        }
        let splitval = unpack_signed(reader.read_hybrid_uint(K_SPLIT_VAL_CONTEXT, br, context_map)) as i32;
        let lchild = (tree.len() + to_decode + 1) as u32;
        let rchild = (tree.len() + to_decode + 2) as u32;
        tree.push(PropertyDecisionNode {
            splitval,
            property: property as i16,
            lchild,
            rchild,
            predictor: Predictor::Zero,
            predictor_offset: 0,
            multiplier: 1,
        });
        to_decode += 2;
    }
    validate_tree(tree)
}

/// Translation of `DecodeTree()`.
pub(crate) fn decode_tree(br: &mut BitReader<'_>, tree: &mut Tree, tree_size_limit: usize) -> Status {
    let mut tree_context_map: Vec<u8> = Vec::new();
    let mut tree_code = AnsCode::default();
    decode_histograms(br, K_NUM_TREE_CONTEXTS, &mut tree_code, &mut tree_context_map, false)?;
    // TODO(eustas): investigate more infinite tree cases.
    if tree_code.degenerate_symbols[tree_context_map[K_PROPERTY_CONTEXT] as usize] > 0 {
        return jxl_failure!("Infinite tree");
    }
    let mut reader = AnsSymbolReader::new(&tree_code, br, 0);
    decode_tree_inner(
        br,
        &mut reader,
        &tree_context_map,
        tree,
        tree_size_limit.min(K_MAX_TREE_SIZE),
    )?;
    if !reader.check_ans_final_state() {
        return jxl_failure!("ANS decode final state failed");
    }
    Ok(())
}
