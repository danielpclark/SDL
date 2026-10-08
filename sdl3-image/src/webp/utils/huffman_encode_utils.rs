// Rust translation of src/utils/huffman_encode_utils.c and
// huffman_encode_utils.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2011 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Entropy encoding (Huffman) for webp lossless.
//!
//! A `HuffmanTreeCode`'s `code_lengths` and `codes` are `Vec`s here, where
//! upstream points them into one shared allocation.

use crate::webp::decode::MAX_ALLOWED_CODE_LENGTH;

/// Struct for holding the tree header in coded form. Translation of
/// `HuffmanTreeToken`.
#[derive(Clone, Copy, Default)]
pub(crate) struct HuffmanTreeToken {
    /// value (0..15) or escape code (16,17,18)
    pub(crate) code: u8,
    /// extra bits for escape codes
    pub(crate) extra_bits: u8,
}

/// Struct to represent the tree codes (depth and bits array). Translation
/// of `HuffmanTreeCode`.
#[derive(Clone, Default)]
pub(crate) struct HuffmanTreeCode {
    /// Number of symbols.
    pub(crate) num_symbols: i32,
    /// Code lengths of the symbols.
    pub(crate) code_lengths: Vec<u8>,
    /// Symbol Codes.
    pub(crate) codes: Vec<u16>,
}

/// Struct to represent the Huffman tree. Translation of `HuffmanTree`.
#[derive(Clone, Copy, Default)]
pub(crate) struct HuffmanTree {
    /// Symbol frequency.
    total_count: u32,
    /// Symbol value.
    value: i32,
    /// Index for the left sub-tree.
    pool_index_left: i32,
    /// Index for the right sub-tree.
    pool_index_right: i32,
}

// -----------------------------------------------------------------------------
// Util function to optimize the symbol map for RLE coding

/// Heuristics for selecting the stride ranges to collapse. Translation of
/// `ValuesShouldBeCollapsedToStrideAverage()`.
fn values_should_be_collapsed_to_stride_average(a: i32, b: i32) -> bool {
    (a - b).abs() < 4
}

/// Change the population counts in a way that the consequent
/// Huffman tree compression, especially its RLE-part, give smaller output.
/// Translation of `OptimizeHuffmanForRle()`.
fn optimize_huffman_for_rle(mut length: i32, good_for_rle: &mut [u8], counts: &mut [u32]) {
    // 1) Let's make the Huffman code more compatible with rle encoding.
    while length >= 0 {
        if length == 0 {
            return; // All zeros.
        }
        if counts[length as usize - 1] != 0 {
            // Now counts[0..length - 1] does not have trailing zeros.
            break;
        }
        length -= 1;
    }
    let length = length as usize;
    // 2) Let's mark all population counts that already can be encoded
    // with an rle code.
    {
        // Let's not spoil any of the existing good rle codes.
        // Mark any seq of 0's that is longer as 5 as a good_for_rle.
        // Mark any seq of non-0's that is longer as 7 as a good_for_rle.
        let mut symbol = counts[0];
        let mut stride = 0usize;
        for i in 0..length + 1 {
            if i == length || counts[i] != symbol {
                if (symbol == 0 && stride >= 5) || (symbol != 0 && stride >= 7) {
                    for k in 0..stride {
                        good_for_rle[i - k - 1] = 1;
                    }
                }
                stride = 1;
                if i != length {
                    symbol = counts[i];
                }
            } else {
                stride += 1;
            }
        }
    }
    // 3) Let's replace those population counts that lead to more rle codes.
    {
        let mut stride = 0u32;
        let mut limit = counts[0];
        let mut sum = 0u32;
        for i in 0..length + 1 {
            if i == length
                || good_for_rle[i] != 0
                || (i != 0 && good_for_rle[i - 1] != 0)
                || !values_should_be_collapsed_to_stride_average(counts[i] as i32, limit as i32)
            {
                if stride >= 4 || (stride >= 3 && sum == 0) {
                    // The stride must end, collapse what we have, if we have enough (4).
                    let mut count = (sum + stride / 2) / stride;
                    if count < 1 {
                        count = 1;
                    }
                    if sum == 0 {
                        // Don't make an all zeros stride to be upgraded to ones.
                        count = 0;
                    }
                    for k in 0..stride as usize {
                        // We don't want to change value at counts[i],
                        // that is already belonging to the next stride. Thus - 1.
                        counts[i - k - 1] = count;
                    }
                }
                stride = 0;
                sum = 0;
                if (i as i64) < length as i64 - 3 {
                    // All interesting strides have a count of at least 4,
                    // at least when non-zeros.
                    limit = (counts[i]
                        .wrapping_add(counts[i + 1])
                        .wrapping_add(counts[i + 2])
                        .wrapping_add(counts[i + 3])
                        .wrapping_add(2))
                        / 4;
                } else if i < length {
                    limit = counts[i];
                } else {
                    limit = 0;
                }
            }
            stride += 1;
            if i != length {
                sum = sum.wrapping_add(counts[i]);
                if stride >= 4 {
                    limit = (sum + stride / 2) / stride;
                }
            }
        }
    }
}

/// A comparer function for two Huffman trees: sorts first by 'total count'
/// (more comes first), and then by 'value' (more comes first). Translation
/// of `CompareHuffmanTrees()`.
fn compare_huffman_trees(t1: &HuffmanTree, t2: &HuffmanTree) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if t1.total_count > t2.total_count {
        Ordering::Less
    } else if t1.total_count < t2.total_count {
        Ordering::Greater
    } else {
        debug_assert!(t1.value != t2.value);
        if t1.value < t2.value {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
}

/// Translation of `SetBitDepths()`.
fn set_bit_depths(tree: &HuffmanTree, pool: &[HuffmanTree], bit_depths: &mut [u8], level: i32) {
    if tree.pool_index_left >= 0 {
        set_bit_depths(
            &pool[tree.pool_index_left as usize],
            pool,
            bit_depths,
            level + 1,
        );
        set_bit_depths(
            &pool[tree.pool_index_right as usize],
            pool,
            bit_depths,
            level + 1,
        );
    } else {
        bit_depths[tree.value as usize] = level as u8;
    }
}

/// Create an optimal Huffman tree.
///
/// (data,length): population counts.
/// tree_limit: maximum bit depth (inclusive) of the codes.
/// bit_depths[]: how many bits are used for the symbol.
///
/// Returns 0 when an error has occurred.
///
/// The catch here is that the tree cannot be arbitrarily deep
///
/// count_limit is the value that is to be faked as the minimum value
/// and this minimum value is raised until the tree matches the
/// maximum length requirement.
///
/// This algorithm is not of excellent performance for very long data
/// blocks, especially when population counts are longer than
/// 2**tree_limit, but we are not planning to use this with extremely long
/// blocks.
///
/// See https://en.wikipedia.org/wiki/Huffman_coding
///
/// Translation of `GenerateOptimalTree()` (`tree` is the tree and its pool,
/// `tree[tree_size_orig..]`).
fn generate_optimal_tree(
    histogram: &[u32],
    histogram_size: usize,
    tree: &mut [HuffmanTree],
    tree_depth_limit: i32,
    bit_depths: &mut [u8],
) {
    let mut tree_size_orig = 0usize;

    for &h in &histogram[..histogram_size] {
        if h != 0 {
            tree_size_orig += 1;
        }
    }

    if tree_size_orig == 0 {
        // pretty optimal already!
        return;
    }

    let (tree, tree_pool) = tree.split_at_mut(tree_size_orig);

    // For block sizes with less than 64k symbols we never need to do a
    // second iteration of this loop.
    // If we actually start running inside this loop a lot, we would perhaps
    // be better off with the Katajainen algorithm.
    debug_assert!(tree_size_orig <= (1 << (tree_depth_limit - 1)));
    let mut count_min = 1u32;
    loop {
        let mut tree_size = tree_size_orig;
        // We need to pack the Huffman tree in tree_depth_limit bits.
        // So, we try by faking histogram entries to be at least 'count_min'.
        let mut idx = 0;
        for (j, &h) in histogram[..histogram_size].iter().enumerate() {
            if h != 0 {
                let count = if h < count_min { count_min } else { h };
                tree[idx].total_count = count;
                tree[idx].value = j as i32;
                tree[idx].pool_index_left = -1;
                tree[idx].pool_index_right = -1;
                idx += 1;
            }
        }

        // Build the Huffman tree.
        // (the values are distinct: any sort gives qsort()'s order)
        tree[..tree_size].sort_by(compare_huffman_trees);

        if tree_size > 1 {
            // Normal case.
            let mut tree_pool_size = 0usize;
            while tree_size > 1 {
                // Finish when we have only one root.
                tree_pool[tree_pool_size] = tree[tree_size - 1];
                tree_pool_size += 1;
                tree_pool[tree_pool_size] = tree[tree_size - 2];
                tree_pool_size += 1;
                let count = tree_pool[tree_pool_size - 1]
                    .total_count
                    .wrapping_add(tree_pool[tree_pool_size - 2].total_count);
                tree_size -= 2;
                {
                    // Search for the insertion point.
                    let mut k = 0;
                    while k < tree_size {
                        if tree[k].total_count <= count {
                            break;
                        }
                        k += 1;
                    }
                    tree.copy_within(k..tree_size, k + 1);
                    tree[k].total_count = count;
                    tree[k].value = -1;

                    tree[k].pool_index_left = tree_pool_size as i32 - 1;
                    tree[k].pool_index_right = tree_pool_size as i32 - 2;
                    tree_size += 1;
                }
            }
            set_bit_depths(&tree[0], tree_pool, bit_depths, 0);
        } else if tree_size == 1 {
            // Trivial case: only one element.
            bit_depths[tree[0].value as usize] = 1;
        }

        {
            // Test if this Huffman tree satisfies our 'tree_depth_limit' criteria.
            let mut max_depth = bit_depths[0] as i32;
            for &d in &bit_depths[1..histogram_size] {
                if max_depth < d as i32 {
                    max_depth = d as i32;
                }
            }
            if max_depth <= tree_depth_limit {
                break;
            }
        }
        count_min = count_min.wrapping_mul(2);
    }
}

// -----------------------------------------------------------------------------
// Coding of the Huffman tree values

/// Translation of `CodeRepeatedValues()`: the tokens are appended to
/// `tokens` at `pos`, which is returned advanced.
fn code_repeated_values(
    mut repetitions: i32,
    tokens: &mut [HuffmanTreeToken],
    mut pos: usize,
    value: i32,
    prev_value: i32,
) -> usize {
    debug_assert!(value <= MAX_ALLOWED_CODE_LENGTH);
    if value != prev_value {
        tokens[pos].code = value as u8;
        tokens[pos].extra_bits = 0;
        pos += 1;
        repetitions -= 1;
    }
    while repetitions >= 1 {
        if repetitions < 3 {
            for _ in 0..repetitions {
                tokens[pos].code = value as u8;
                tokens[pos].extra_bits = 0;
                pos += 1;
            }
            break;
        } else if repetitions < 7 {
            tokens[pos].code = 16;
            tokens[pos].extra_bits = (repetitions - 3) as u8;
            pos += 1;
            break;
        } else {
            tokens[pos].code = 16;
            tokens[pos].extra_bits = 3;
            pos += 1;
            repetitions -= 6;
        }
    }
    pos
}

/// Translation of `CodeRepeatedZeros()`.
fn code_repeated_zeros(
    mut repetitions: i32,
    tokens: &mut [HuffmanTreeToken],
    mut pos: usize,
) -> usize {
    while repetitions >= 1 {
        if repetitions < 3 {
            for _ in 0..repetitions {
                tokens[pos].code = 0; // 0-value
                tokens[pos].extra_bits = 0;
                pos += 1;
            }
            break;
        } else if repetitions < 11 {
            tokens[pos].code = 17;
            tokens[pos].extra_bits = (repetitions - 3) as u8;
            pos += 1;
            break;
        } else if repetitions < 139 {
            tokens[pos].code = 18;
            tokens[pos].extra_bits = (repetitions - 11) as u8;
            pos += 1;
            break;
        } else {
            tokens[pos].code = 18;
            tokens[pos].extra_bits = 0x7f; // 138 repeated 0s
            pos += 1;
            repetitions -= 138;
        }
    }
    pos
}

/// Turn the Huffman tree into a token sequence.
/// Returns the number of tokens used. Translation of
/// `VP8LCreateCompressedHuffmanTree()`.
pub(crate) fn vp8l_create_compressed_huffman_tree(
    tree: &HuffmanTreeCode,
    tokens: &mut [HuffmanTreeToken],
    max_tokens: usize,
) -> usize {
    let depth_size = tree.num_symbols as usize;
    let mut prev_value = 8; // 8 is the initial value for rle.
    let mut i = 0;
    let mut pos = 0;
    while i < depth_size {
        let value = tree.code_lengths[i] as i32;
        let mut k = i + 1;
        while k < depth_size && tree.code_lengths[k] as i32 == value {
            k += 1;
        }
        let runs = k - i;
        if value == 0 {
            pos = code_repeated_zeros(runs as i32, tokens, pos);
        } else {
            pos = code_repeated_values(runs as i32, tokens, pos, value, prev_value);
            prev_value = value;
        }
        i += runs;
        debug_assert!(pos <= max_tokens);
    }
    pos
}

// -----------------------------------------------------------------------------

/// Pre-reversed 4-bit values.
const K_REVERSED_BITS: [u8; 16] = [
    0x0, 0x8, 0x4, 0xc, 0x2, 0xa, 0x6, 0xe, 0x1, 0x9, 0x5, 0xd, 0x3, 0xb, 0x7, 0xf,
];

/// Translation of `ReverseBits()`.
fn reverse_bits(num_bits: i32, mut bits: u32) -> u32 {
    let mut retval = 0u32;
    let mut i = 0;
    while i < num_bits {
        i += 4;
        retval |=
            (K_REVERSED_BITS[(bits & 0xf) as usize] as u32) << (MAX_ALLOWED_CODE_LENGTH + 1 - i);
        bits >>= 4;
    }
    retval >>= MAX_ALLOWED_CODE_LENGTH + 1 - num_bits;
    retval
}

/// Get the actual bit values for a tree of bit depths. Translation of
/// `ConvertBitDepthsToSymbols()`.
fn convert_bit_depths_to_symbols(tree: &mut HuffmanTreeCode) {
    // 0 bit-depth means that the symbol does not exist.
    let mut next_code = [0u32; MAX_ALLOWED_CODE_LENGTH as usize + 1];
    let mut depth_count = [0i32; MAX_ALLOWED_CODE_LENGTH as usize + 1];

    let len = tree.num_symbols as usize;
    for i in 0..len {
        let code_length = tree.code_lengths[i] as usize;
        debug_assert!(code_length <= MAX_ALLOWED_CODE_LENGTH as usize);
        depth_count[code_length] += 1;
    }
    depth_count[0] = 0; // ignore unused symbol
    next_code[0] = 0;
    {
        let mut code = 0u32;
        for i in 1..=MAX_ALLOWED_CODE_LENGTH as usize {
            code = (code + depth_count[i - 1] as u32) << 1;
            next_code[i] = code;
        }
    }
    for i in 0..len {
        let code_length = tree.code_lengths[i] as usize;
        tree.codes[i] = reverse_bits(code_length as i32, next_code[code_length]) as u16;
        next_code[code_length] = next_code[code_length].wrapping_add(1);
    }
}

// -----------------------------------------------------------------------------
// Main entry point

/// Create an optimized tree, and tokenize it.
/// 'buf_rle' and 'huff_tree' are pre-allocated and the 'tree' is the
/// constructed huffman code tree. Translation of `VP8LCreateHuffmanTree()`.
pub(crate) fn vp8l_create_huffman_tree(
    histogram: &mut [u32],
    tree_depth_limit: i32,
    buf_rle: &mut [u8],
    huff_tree: &mut [HuffmanTree],
    huff_code: &mut HuffmanTreeCode,
) {
    let num_symbols = huff_code.num_symbols as usize;
    buf_rle[..num_symbols].fill(0);
    optimize_huffman_for_rle(num_symbols as i32, buf_rle, histogram);
    generate_optimal_tree(
        histogram,
        num_symbols,
        huff_tree,
        tree_depth_limit,
        &mut huff_code.code_lengths,
    );
    // Create the actual bit codes for the bit lengths.
    convert_bit_depths_to_symbols(huff_code);
}
