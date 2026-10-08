// Rust translation of src/enc/histogram_enc.c and histogram_enc.h from
// libwebp (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it), with `VP8LHistogramAdd()` of
// src/dsp/lossless_enc.c.
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Models the histograms of literal and distance codes.
//!
//! A histogram set is a `Vec` of boxed histograms (`None` where upstream's
//! pointer is NULL). Upstream allocates them in one chunk and swaps
//! pointers around (a combined histogram and the scratch one trade
//! places); swapping the boxes does the same, and clearing the set gives
//! every slot a fresh zeroed histogram, as upstream's memset does. The
//! low-effort path (method 0) is not translated.

use crate::webp::decode::{NUM_DISTANCE_CODES, NUM_LENGTH_CODES, NUM_LITERAL_CODES};
use crate::webp::dsp::lossless_enc::{
    vp8l_bit_entropy_init, vp8l_bits_entropy_unrefined, vp8l_extra_cost, vp8l_extra_cost_combined,
    vp8l_get_combined_entropy_unrefined, vp8l_get_entropy_unrefined, vp8l_prefix_encode_bits,
    vp8l_sub_sample_size, VP8LBitEntropy, VP8LStreaks, VP8L_NON_TRIVIAL_SYM,
};
use crate::webp::enc::backward_references_enc::{PixOrCopy, VP8LBackwardRefs};

const NUM_LITERAL: usize = NUM_LITERAL_CODES as usize;
const NUM_DISTANCE: usize = NUM_DISTANCE_CODES as usize;
const NUM_LENGTH: usize = NUM_LENGTH_CODES as usize;

/// A simple container for histograms of data. Translation of
/// `VP8LHistogram`.
#[derive(Clone)]
pub(crate) struct VP8LHistogram {
    /// literal_ contains green literal, palette-code and
    /// copy-length-prefix histogram
    pub(crate) literal: Vec<u32>,
    pub(crate) red: [u32; NUM_LITERAL],
    pub(crate) blue: [u32; NUM_LITERAL],
    pub(crate) alpha: [u32; NUM_LITERAL],
    /// Backward reference prefix-code histogram.
    pub(crate) distance: [u32; NUM_DISTANCE],
    pub(crate) palette_code_bits: i32,
    /// True, if histograms for Red, Blue & Alpha literal symbols are
    /// single valued.
    pub(crate) trivial_symbol: u32,
    /// cached value of bit cost.
    pub(crate) bit_cost: f32,
    /// Cached values of dominant entropy costs: literal, red & blue.
    pub(crate) literal_cost: f32,
    pub(crate) red_cost: f32,
    pub(crate) blue_cost: f32,
    /// 5 for literal, red, blue, alpha, distance
    pub(crate) is_used: [u8; 5],
}

impl VP8LHistogram {
    /// Allocate and initialize histogram object with specified
    /// 'cache_bits', its arrays zeroed. Translation of
    /// `VP8LAllocateHistogram()` (whose arrays are uninitialized: every use
    /// clears or overwrites them first).
    pub(crate) fn new(cache_bits: i32) -> VP8LHistogram {
        VP8LHistogram {
            literal: vec![0; vp8l_histogram_num_codes(cache_bits) as usize],
            red: [0; NUM_LITERAL],
            blue: [0; NUM_LITERAL],
            alpha: [0; NUM_LITERAL],
            distance: [0; NUM_DISTANCE],
            palette_code_bits: cache_bits,
            trivial_symbol: 0,
            bit_cost: 0.0,
            literal_cost: 0.0,
            red_cost: 0.0,
            blue_cost: 0.0,
            is_used: [0; 5],
        }
    }
}

/// Collection of histograms with fixed capacity. Translation of
/// `VP8LHistogramSet`.
pub(crate) struct VP8LHistogramSet {
    /// number of slots currently in use
    pub(crate) size: i32,
    /// maximum capacity
    pub(crate) max_size: i32,
    pub(crate) histograms: Vec<Option<Box<VP8LHistogram>>>,
}

/// Translation of `VP8LHistogramNumCodes()`.
pub(crate) fn vp8l_histogram_num_codes(palette_code_bits: i32) -> i32 {
    NUM_LITERAL_CODES
        + NUM_LENGTH_CODES
        + if palette_code_bits > 0 {
            1 << palette_code_bits
        } else {
            0
        }
}

/// Number of partitions for the three dominant (literal, red and blue)
/// symbol costs.
const NUM_PARTITIONS: i32 = 4;
/// The size of the bin-hash corresponding to the three dominant costs.
const BIN_SIZE: i32 = NUM_PARTITIONS * NUM_PARTITIONS * NUM_PARTITIONS;
/// Maximum number of histograms allowed in greedy combining algorithm.
const MAX_HISTO_GREEDY: i32 = 100;

const MAX_BIT_COST: f32 = f32::MAX;

/// Translation of `HistogramClear()`.
fn histogram_clear(p: &mut VP8LHistogram) {
    let cache_bits = p.palette_code_bits;
    *p = VP8LHistogram::new(cache_bits);
}

/// Translation of `HistogramCopy()`.
fn histogram_copy(src: &VP8LHistogram, dst: &mut VP8LHistogram) {
    debug_assert!(src.palette_code_bits == dst.palette_code_bits);
    dst.clone_from(src);
}

/// Collect all the references into a histogram (without reset).
/// Translation of `VP8LHistogramStoreRefs()`.
pub(crate) fn vp8l_histogram_store_refs(refs: &VP8LBackwardRefs, histo: &mut VP8LHistogram) {
    for v in &refs.refs {
        vp8l_histogram_add_single_pix_or_copy(histo, v, None, 0);
    }
}

/// Create the histogram.
///
/// The input data is the PixOrCopy data, which models the literals, stop
/// codes and backward references (both distances and lengths).  Also: if
/// palette_code_bits is >= 0, initialize the histogram with this value.
/// Translation of `VP8LHistogramCreate()`.
pub(crate) fn vp8l_histogram_create(
    p: &mut VP8LHistogram,
    refs: &VP8LBackwardRefs,
    palette_code_bits: i32,
) {
    if palette_code_bits >= 0 {
        p.palette_code_bits = palette_code_bits;
    }
    histogram_clear(p);
    vp8l_histogram_store_refs(refs, p);
}

/// Allocate an array of pointer to histograms, allocated and initialized
/// using 'cache_bits'. Translation of `VP8LAllocateHistogramSet()`.
pub(crate) fn vp8l_allocate_histogram_set(size: i32, cache_bits: i32) -> VP8LHistogramSet {
    VP8LHistogramSet {
        size,
        max_size: size,
        histograms: (0..size)
            .map(|_| Some(Box::new(VP8LHistogram::new(cache_bits))))
            .collect(),
    }
}

/// Set the histograms in set to 0. Translation of
/// `VP8LHistogramSetClear()`.
pub(crate) fn vp8l_histogram_set_clear(set: &mut VP8LHistogramSet) {
    let cache_bits = set.histograms[0].as_ref().unwrap().palette_code_bits;
    let size = set.max_size;
    *set = vp8l_allocate_histogram_set(size, cache_bits);
}

/// Removes the histogram 'i' from 'set' by setting it to NULL.
/// Translation of `HistogramSetRemoveHistogram()`.
fn histogram_set_remove_histogram(set: &mut VP8LHistogramSet, i: usize, num_used: &mut i32) {
    debug_assert!(set.histograms[i].is_some());
    set.histograms[i] = None;
    *num_used -= 1;
    // If we remove the last valid one, shrink until the next valid one.
    if i as i32 == set.size - 1 {
        while set.size >= 1 && set.histograms[set.size as usize - 1].is_none() {
            set.size -= 1;
        }
    }
}

// -----------------------------------------------------------------------------

/// Accumulate a token 'v' into a histogram. Translation of
/// `VP8LHistogramAddSinglePixOrCopy()`.
pub(crate) fn vp8l_histogram_add_single_pix_or_copy(
    histo: &mut VP8LHistogram,
    v: &PixOrCopy,
    distance_modifier: Option<fn(i32, i32) -> i32>,
    distance_modifier_arg0: i32,
) {
    if v.is_literal() {
        histo.alpha[v.literal(3) as usize] += 1;
        histo.red[v.literal(2) as usize] += 1;
        histo.literal[v.literal(1) as usize] += 1;
        histo.blue[v.literal(0) as usize] += 1;
    } else if v.is_cache_idx() {
        let literal_ix = NUM_LITERAL + NUM_LENGTH + v.cache_idx() as usize;
        debug_assert!(histo.palette_code_bits != 0);
        histo.literal[literal_ix] += 1;
    } else {
        let (code, _) = vp8l_prefix_encode_bits(v.length() as i32);
        histo.literal[NUM_LITERAL + code as usize] += 1;
        let (code, _) = match distance_modifier {
            None => vp8l_prefix_encode_bits(v.distance() as i32),
            Some(modifier) => {
                vp8l_prefix_encode_bits(modifier(distance_modifier_arg0, v.distance() as i32))
            }
        };
        histo.distance[code as usize] += 1;
    }
}

// -----------------------------------------------------------------------------
// Entropy-related functions.

/// Translation of `BitsEntropyRefine()`.
fn bits_entropy_refine(entropy: &VP8LBitEntropy) -> f32 {
    let mix;
    if entropy.nonzeros < 5 {
        if entropy.nonzeros <= 1 {
            return 0.0;
        }
        // Two symbols, they will be 0 and 1 in a Huffman code.
        // Let's mix in a bit of entropy to favor good clustering when
        // distributions of these are combined.
        if entropy.nonzeros == 2 {
            return 0.99f32 * entropy.sum as f32 + 0.01f32 * entropy.entropy;
        }
        // No matter what the entropy says, we cannot be better than min_limit
        // with Huffman coding. I am mixing a bit of entropy into the
        // min_limit since it produces much better (~0.5 %) compression results
        // perhaps because of better entropy clustering.
        if entropy.nonzeros == 3 {
            mix = 0.95f32;
        } else {
            mix = 0.7f32; // nonzeros == 4.
        }
    } else {
        mix = 0.627f32;
    }

    {
        let mut min_limit = 2.0f32 * entropy.sum as f32 - entropy.max_val as f32;
        min_limit = mix * min_limit + (1.0f32 - mix) * entropy.entropy;
        if entropy.entropy < min_limit {
            min_limit
        } else {
            entropy.entropy
        }
    }
}

/// Returns the entropy for the symbols in the input array. Translation of
/// `VP8LBitsEntropy()`.
pub(crate) fn vp8l_bits_entropy(array: &[u32], n: usize) -> f32 {
    let mut entropy = VP8LBitEntropy::default();
    vp8l_bits_entropy_unrefined(array, n, &mut entropy);

    bits_entropy_refine(&entropy)
}

/// Translation of `InitialHuffmanCost()`.
fn initial_huffman_cost() -> f32 {
    // Small bias because Huffman code length is typically not stored in
    // full length.
    const K_HUFFMAN_CODE_OF_HUFFMAN_CODE_SIZE: i32 = 19 * 3; // CODE_LENGTH_CODES * 3
    const K_SMALL_BIAS: f32 = 9.1;
    K_HUFFMAN_CODE_OF_HUFFMAN_CODE_SIZE as f32 - K_SMALL_BIAS
}

/// Finalize the Huffman cost based on streak numbers and length type (<3
/// or >=3). Translation of `FinalHuffmanCost()`.
fn final_huffman_cost(stats: &VP8LStreaks) -> f32 {
    // The constants in this function are experimental and got rounded from
    // their original values in 1/8 when switched to 1/1024.
    let mut retval = initial_huffman_cost();
    // Second coefficient: Many zeros in the histogram are covered efficiently
    // by a run-length encode. Originally 2/8.
    retval += stats.counts[0] as f32 * 1.5625f32 + 0.234375f32 * stats.streaks[0][1] as f32;
    // Second coefficient: Constant values are encoded less efficiently, but still
    // RLE'ed. Originally 6/8.
    retval += stats.counts[1] as f32 * 2.578125f32 + 0.703125f32 * stats.streaks[1][1] as f32;
    // 0s are usually encoded more efficiently than non-0s.
    // Originally 15/8.
    retval += 1.796875f32 * stats.streaks[0][0] as f32;
    // Originally 26/8.
    retval += 3.28125f32 * stats.streaks[1][0] as f32;
    retval
}

/// Get the symbol entropy for the distribution 'population'.
/// Set 'trivial_sym', if there's only one symbol present in the
/// distribution. Translation of `PopulationCost()`.
fn population_cost(
    population: &[u32],
    length: usize,
    trivial_sym: Option<&mut u32>,
    is_used: &mut u8,
) -> f32 {
    let mut bit_entropy = VP8LBitEntropy::default();
    let mut stats = VP8LStreaks::default();
    vp8l_get_entropy_unrefined(population, length, &mut bit_entropy, &mut stats);
    if let Some(trivial_sym) = trivial_sym {
        *trivial_sym = if bit_entropy.nonzeros == 1 {
            bit_entropy.nonzero_code
        } else {
            VP8L_NON_TRIVIAL_SYM
        };
    }
    // The histogram is used if there is at least one non-zero streak.
    *is_used = (stats.streaks[1][0] != 0 || stats.streaks[1][1] != 0) as u8;

    bits_entropy_refine(&bit_entropy) + final_huffman_cost(&stats)
}

/// trivial_at_end is 1 if the two histograms only have one element that is
/// non-zero: both the zero-th one, or both the last one. Translation of
/// `GetCombinedEntropy()`.
fn get_combined_entropy(
    x: &[u32],
    y: &[u32],
    length: usize,
    is_x_used: bool,
    is_y_used: bool,
    trivial_at_end: bool,
) -> f32 {
    let mut stats = VP8LStreaks::default();
    if trivial_at_end {
        // This configuration is due to palettization that transforms an indexed
        // pixel into 0xff000000 | (pixel << 8) in VP8LBundleColorMap.
        // BitsEntropyRefine is 0 for histograms with only one non-zero value.
        // Only FinalHuffmanCost needs to be evaluated.
        // Deal with the non-zero value at index 0 or length-1.
        stats.streaks[1][0] = 1;
        // Deal with the following/previous zero streak.
        stats.counts[0] = 1;
        stats.streaks[0][1] = length as i32 - 1;
        final_huffman_cost(&stats)
    } else {
        let mut bit_entropy = VP8LBitEntropy::default();
        if is_x_used {
            if is_y_used {
                vp8l_get_combined_entropy_unrefined(x, y, length, &mut bit_entropy, &mut stats);
            } else {
                vp8l_get_entropy_unrefined(x, length, &mut bit_entropy, &mut stats);
            }
        } else if is_y_used {
            vp8l_get_entropy_unrefined(y, length, &mut bit_entropy, &mut stats);
        } else {
            stats.counts[0] = 1;
            stats.streaks[0][(length > 3) as usize] = length as i32;
            vp8l_bit_entropy_init(&mut bit_entropy);
        }

        bits_entropy_refine(&bit_entropy) + final_huffman_cost(&stats)
    }
}

/// Estimate how many bits the combined entropy of literals and distance
/// approximately maps to. Translation of `VP8LHistogramEstimateBits()`.
pub(crate) fn vp8l_histogram_estimate_bits(p: &mut VP8LHistogram) -> f32 {
    let num_codes = vp8l_histogram_num_codes(p.palette_code_bits) as usize;
    population_cost(&p.literal, num_codes, None, &mut p.is_used[0])
        + population_cost(&p.red, NUM_LITERAL, None, &mut p.is_used[1])
        + population_cost(&p.blue, NUM_LITERAL, None, &mut p.is_used[2])
        + population_cost(&p.alpha, NUM_LITERAL, None, &mut p.is_used[3])
        + population_cost(&p.distance, NUM_DISTANCE, None, &mut p.is_used[4])
        + vp8l_extra_cost(&p.literal[NUM_LITERAL..], NUM_LENGTH)
        + vp8l_extra_cost(&p.distance, NUM_DISTANCE)
}

// -----------------------------------------------------------------------------
// Various histogram combine/cost-eval functions

/// Translation of `GetCombinedHistogramEntropy()`.
fn get_combined_histogram_entropy(
    a: &VP8LHistogram,
    b: &VP8LHistogram,
    cost_threshold: f32,
    cost: &mut f32,
) -> bool {
    let palette_code_bits = a.palette_code_bits;
    let mut trivial_at_end = false;
    debug_assert!(a.palette_code_bits == b.palette_code_bits);
    *cost += get_combined_entropy(
        &a.literal,
        &b.literal,
        vp8l_histogram_num_codes(palette_code_bits) as usize,
        a.is_used[0] != 0,
        b.is_used[0] != 0,
        false,
    );
    *cost += vp8l_extra_cost_combined(
        &a.literal[NUM_LITERAL..],
        &b.literal[NUM_LITERAL..],
        NUM_LENGTH,
    );
    if *cost > cost_threshold {
        return false;
    }

    if a.trivial_symbol != VP8L_NON_TRIVIAL_SYM && a.trivial_symbol == b.trivial_symbol {
        // A, R and B are all 0 or 0xff.
        let color_a = (a.trivial_symbol >> 24) & 0xff;
        let color_r = (a.trivial_symbol >> 16) & 0xff;
        let color_b = a.trivial_symbol & 0xff;
        if (color_a == 0 || color_a == 0xff)
            && (color_r == 0 || color_r == 0xff)
            && (color_b == 0 || color_b == 0xff)
        {
            trivial_at_end = true;
        }
    }

    *cost += get_combined_entropy(
        &a.red,
        &b.red,
        NUM_LITERAL,
        a.is_used[1] != 0,
        b.is_used[1] != 0,
        trivial_at_end,
    );
    if *cost > cost_threshold {
        return false;
    }

    *cost += get_combined_entropy(
        &a.blue,
        &b.blue,
        NUM_LITERAL,
        a.is_used[2] != 0,
        b.is_used[2] != 0,
        trivial_at_end,
    );
    if *cost > cost_threshold {
        return false;
    }

    *cost += get_combined_entropy(
        &a.alpha,
        &b.alpha,
        NUM_LITERAL,
        a.is_used[3] != 0,
        b.is_used[3] != 0,
        trivial_at_end,
    );
    if *cost > cost_threshold {
        return false;
    }

    *cost += get_combined_entropy(
        &a.distance,
        &b.distance,
        NUM_DISTANCE,
        a.is_used[4] != 0,
        b.is_used[4] != 0,
        false,
    );
    *cost += vp8l_extra_cost_combined(&a.distance, &b.distance, NUM_DISTANCE);
    if *cost > cost_threshold {
        return false;
    }

    true
}

/// The `ADD()` of `VP8LHistogramAdd()`.
fn add_array(a: &[u32], a_used: u8, b: &[u32], b_used: u8, out: &mut [u32], len: usize) {
    if a_used != 0 {
        if b_used != 0 {
            for i in 0..len {
                out[i] = a[i].wrapping_add(b[i]);
            }
        } else {
            out[..len].copy_from_slice(&a[..len]);
        }
    } else if b_used != 0 {
        out[..len].copy_from_slice(&b[..len]);
    } else {
        out[..len].fill(0);
    }
}

/// The `ADD_EQ()` of `VP8LHistogramAdd()`.
fn add_eq_array(a: &[u32], a_used: u8, out_used: u8, out: &mut [u32], len: usize) {
    if a_used != 0 {
        if out_used != 0 {
            for i in 0..len {
                out[i] = out[i].wrapping_add(a[i]);
            }
        } else {
            out[..len].copy_from_slice(&a[..len]);
        }
    }
}

/// Adds two histograms ('a' and 'b'), stores the result in 'out'.
/// Translation of `VP8LHistogramAdd()` (of dsp/lossless_enc.c) for
/// `b != out`.
fn vp8l_histogram_add(a: &VP8LHistogram, b: &VP8LHistogram, out: &mut VP8LHistogram) {
    let literal_size = vp8l_histogram_num_codes(a.palette_code_bits) as usize;
    debug_assert!(a.palette_code_bits == b.palette_code_bits);

    add_array(
        &a.literal,
        a.is_used[0],
        &b.literal,
        b.is_used[0],
        &mut out.literal,
        literal_size,
    );
    add_array(
        &a.red,
        a.is_used[1],
        &b.red,
        b.is_used[1],
        &mut out.red,
        NUM_LITERAL,
    );
    add_array(
        &a.blue,
        a.is_used[2],
        &b.blue,
        b.is_used[2],
        &mut out.blue,
        NUM_LITERAL,
    );
    add_array(
        &a.alpha,
        a.is_used[3],
        &b.alpha,
        b.is_used[3],
        &mut out.alpha,
        NUM_LITERAL,
    );
    add_array(
        &a.distance,
        a.is_used[4],
        &b.distance,
        b.is_used[4],
        &mut out.distance,
        NUM_DISTANCE,
    );
    for i in 0..5 {
        out.is_used[i] = a.is_used[i] | b.is_used[i];
    }
}

/// `VP8LHistogramAdd()` for `b == out`.
fn vp8l_histogram_add_eq(a: &VP8LHistogram, out: &mut VP8LHistogram) {
    let literal_size = vp8l_histogram_num_codes(a.palette_code_bits) as usize;
    debug_assert!(a.palette_code_bits == out.palette_code_bits);

    add_eq_array(
        &a.literal,
        a.is_used[0],
        out.is_used[0],
        &mut out.literal,
        literal_size,
    );
    add_eq_array(
        &a.red,
        a.is_used[1],
        out.is_used[1],
        &mut out.red,
        NUM_LITERAL,
    );
    add_eq_array(
        &a.blue,
        a.is_used[2],
        out.is_used[2],
        &mut out.blue,
        NUM_LITERAL,
    );
    add_eq_array(
        &a.alpha,
        a.is_used[3],
        out.is_used[3],
        &mut out.alpha,
        NUM_LITERAL,
    );
    add_eq_array(
        &a.distance,
        a.is_used[4],
        out.is_used[4],
        &mut out.distance,
        NUM_DISTANCE,
    );
    for i in 0..5 {
        out.is_used[i] |= a.is_used[i];
    }
}

/// Translation of `HistogramAdd()` (`b != out`).
fn histogram_add(a: &VP8LHistogram, b: &VP8LHistogram, out: &mut VP8LHistogram) {
    vp8l_histogram_add(a, b, out);
    out.trivial_symbol = if a.trivial_symbol == b.trivial_symbol {
        a.trivial_symbol
    } else {
        VP8L_NON_TRIVIAL_SYM
    };
}

/// Translation of `HistogramAdd()` with `b == out`.
fn histogram_add_eq(a: &VP8LHistogram, out: &mut VP8LHistogram) {
    let b_trivial_symbol = out.trivial_symbol;
    vp8l_histogram_add_eq(a, out);
    out.trivial_symbol = if a.trivial_symbol == b_trivial_symbol {
        a.trivial_symbol
    } else {
        VP8L_NON_TRIVIAL_SYM
    };
}

/// Performs out = a + b, computing the cost C(a+b) - C(a) - C(b) while
/// comparing to the threshold value 'cost_threshold'. The score returned
/// is Score = C(a+b) - C(a) - C(b), where C(a) + C(b) is known and fixed.
/// Since the previous score passed is 'cost_threshold', we only need to
/// compare the partial cost against 'cost_threshold + C(a) + C(b)' to
/// possibly bail-out early. Translation of `HistogramAddEval()`.
fn histogram_add_eval(
    a: &VP8LHistogram,
    b: &VP8LHistogram,
    out: &mut VP8LHistogram,
    mut cost_threshold: f32,
) -> f32 {
    let mut cost = 0.0f32;
    let sum_cost = a.bit_cost + b.bit_cost;
    cost_threshold += sum_cost;

    if get_combined_histogram_entropy(a, b, cost_threshold, &mut cost) {
        histogram_add(a, b, out);
        out.bit_cost = cost;
        out.palette_code_bits = a.palette_code_bits;
    }

    cost - sum_cost
}

/// Same as HistogramAddEval(), except that the resulting histogram
/// is not stored. Only the cost C(a+b) - C(a) is evaluated. We omit
/// the term C(b) which is constant over all the evaluations. Translation
/// of `HistogramAddThresh()`.
fn histogram_add_thresh(a: &VP8LHistogram, b: &VP8LHistogram, cost_threshold: f32) -> f32 {
    let mut cost = -a.bit_cost;
    get_combined_histogram_entropy(a, b, cost_threshold, &mut cost);
    cost
}

// -----------------------------------------------------------------------------

/// The structure to keep track of cost range for the three dominant
/// entropy symbols. Translation of `DominantCostRange`.
struct DominantCostRange {
    literal_max: f32,
    literal_min: f32,
    red_max: f32,
    red_min: f32,
    blue_max: f32,
    blue_min: f32,
}

/// Translation of `DominantCostRangeInit()`.
fn dominant_cost_range_init() -> DominantCostRange {
    DominantCostRange {
        literal_max: 0.0,
        literal_min: MAX_BIT_COST,
        red_max: 0.0,
        red_min: MAX_BIT_COST,
        blue_max: 0.0,
        blue_min: MAX_BIT_COST,
    }
}

/// Translation of `UpdateDominantCostRange()`.
fn update_dominant_cost_range(h: &VP8LHistogram, c: &mut DominantCostRange) {
    if c.literal_max < h.literal_cost {
        c.literal_max = h.literal_cost;
    }
    if c.literal_min > h.literal_cost {
        c.literal_min = h.literal_cost;
    }
    if c.red_max < h.red_cost {
        c.red_max = h.red_cost;
    }
    if c.red_min > h.red_cost {
        c.red_min = h.red_cost;
    }
    if c.blue_max < h.blue_cost {
        c.blue_max = h.blue_cost;
    }
    if c.blue_min > h.blue_cost {
        c.blue_min = h.blue_cost;
    }
}

/// Translation of `UpdateHistogramCost()`.
fn update_histogram_cost(h: &mut VP8LHistogram) {
    let mut alpha_sym = 0u32;
    let mut red_sym = 0u32;
    let mut blue_sym = 0u32;
    let alpha_cost = population_cost(
        &h.alpha,
        NUM_LITERAL,
        Some(&mut alpha_sym),
        &mut h.is_used[3],
    );
    let distance_cost = population_cost(&h.distance, NUM_DISTANCE, None, &mut h.is_used[4])
        + vp8l_extra_cost(&h.distance, NUM_DISTANCE);
    let num_codes = vp8l_histogram_num_codes(h.palette_code_bits) as usize;
    h.literal_cost = population_cost(&h.literal, num_codes, None, &mut h.is_used[0])
        + vp8l_extra_cost(&h.literal[NUM_LITERAL..], NUM_LENGTH);
    h.red_cost = population_cost(&h.red, NUM_LITERAL, Some(&mut red_sym), &mut h.is_used[1]);
    h.blue_cost = population_cost(&h.blue, NUM_LITERAL, Some(&mut blue_sym), &mut h.is_used[2]);
    h.bit_cost = h.literal_cost + h.red_cost + h.blue_cost + alpha_cost + distance_cost;
    if (alpha_sym | red_sym | blue_sym) == VP8L_NON_TRIVIAL_SYM {
        h.trivial_symbol = VP8L_NON_TRIVIAL_SYM;
    } else {
        h.trivial_symbol = (alpha_sym << 24) | (red_sym << 16) | blue_sym;
    }
}

/// Translation of `GetBinIdForEntropy()`.
fn get_bin_id_for_entropy(min: f32, max: f32, val: f32) -> i32 {
    let range = max - min;
    if range > 0.0 {
        let delta = val - min;
        ((NUM_PARTITIONS as f64 - 1e-6) * delta as f64 / range as f64) as i32
    } else {
        0
    }
}

/// Translation of `GetHistoBinIndex()`.
fn get_histo_bin_index(h: &VP8LHistogram, c: &DominantCostRange, low_effort: bool) -> i32 {
    let mut bin_id = get_bin_id_for_entropy(c.literal_min, c.literal_max, h.literal_cost);
    debug_assert!(bin_id < NUM_PARTITIONS);
    if !low_effort {
        bin_id = bin_id * NUM_PARTITIONS + get_bin_id_for_entropy(c.red_min, c.red_max, h.red_cost);
        bin_id =
            bin_id * NUM_PARTITIONS + get_bin_id_for_entropy(c.blue_min, c.blue_max, h.blue_cost);
        debug_assert!(bin_id < BIN_SIZE);
    }
    bin_id
}

/// Construct the histograms from backward references. Translation of
/// `HistogramBuild()`.
fn histogram_build(
    xsize: i32,
    histo_bits: i32,
    backward_refs: &VP8LBackwardRefs,
    image_histo: &mut VP8LHistogramSet,
) {
    let mut x = 0;
    let mut y = 0;
    let histo_xsize = vp8l_sub_sample_size(xsize as u32, histo_bits as u32) as i32;
    debug_assert!(histo_bits > 0);
    vp8l_histogram_set_clear(image_histo);
    for v in &backward_refs.refs {
        let ix = ((y >> histo_bits) * histo_xsize + (x >> histo_bits)) as usize;
        vp8l_histogram_add_single_pix_or_copy(
            image_histo.histograms[ix].as_mut().unwrap(),
            v,
            None,
            0,
        );
        x += v.length() as i32;
        while x >= xsize {
            x -= xsize;
            y += 1;
        }
    }
}

/// Copies the histograms and computes its bit_cost.
const K_INVALID_HISTOGRAM_SYMBOL: u16 = 0xffff;

/// Translation of `HistogramCopyAndAnalyze()`.
fn histogram_copy_and_analyze(
    orig_histo: &mut VP8LHistogramSet,
    image_histo: &mut VP8LHistogramSet,
    num_used: &mut i32,
    histogram_symbols: &mut [u16],
) {
    let mut num_used_orig = *num_used;
    let mut cluster_id = 0;
    debug_assert!(image_histo.max_size == orig_histo.max_size);
    for i in 0..orig_histo.max_size as usize {
        let is_empty = {
            let histo = orig_histo.histograms[i].as_mut().unwrap();
            update_histogram_cost(histo);
            histo.is_used.iter().all(|&u| u == 0)
        };

        // Skip the histogram if it is completely empty, which can happen for tiles
        // with no information (when they are skipped because of LZ77).
        if is_empty {
            // The first histogram is always used. If an histogram is empty, we set
            // its id to be the same as the previous one: this will improve
            // compressibility for later LZ77.
            debug_assert!(i > 0);
            histogram_set_remove_histogram(image_histo, i, num_used);
            histogram_set_remove_histogram(orig_histo, i, &mut num_used_orig);
            histogram_symbols[i] = K_INVALID_HISTOGRAM_SYMBOL;
        } else {
            // Copy histograms from orig_histo[] to image_histo[].
            histogram_copy(
                orig_histo.histograms[i].as_ref().unwrap(),
                image_histo.histograms[i].as_mut().unwrap(),
            );
            histogram_symbols[i] = cluster_id;
            cluster_id += 1;
            debug_assert!(cluster_id as i32 <= image_histo.max_size);
        }
    }
}

/// Partition histograms to different entropy bins for three dominant
/// (literal, red and blue) symbol costs and compute the histogram
/// aggregate bit_cost. Translation of `HistogramAnalyzeEntropyBin()`.
fn histogram_analyze_entropy_bin(
    image_histo: &VP8LHistogramSet,
    bin_map: &mut [u16],
    low_effort: bool,
) {
    let histo_size = image_histo.size as usize;
    let mut cost_range = dominant_cost_range_init();

    // Analyze the dominant (literal, red and blue) entropy costs.
    for h in image_histo.histograms[..histo_size].iter().flatten() {
        update_dominant_cost_range(h, &mut cost_range);
    }

    // bin-hash histograms on three of the dominant (literal, red and blue)
    // symbol costs and store the resulting bin_id for each histogram.
    for i in 0..histo_size {
        // bin_map[i] is not set to a special value as its use will later be guarded
        // by another (histograms[i] == NULL).
        let Some(h) = &image_histo.histograms[i] else {
            continue;
        };
        bin_map[i] = get_histo_bin_index(h, &cost_range, low_effort) as u16;
    }
}

/// Merges some histograms with same bin_id together if it's advantageous.
/// Sets the remaining histograms to NULL. Translation of
/// `HistogramCombineEntropyBin()` (without its low-effort path).
#[allow(clippy::too_many_arguments)]
fn histogram_combine_entropy_bin(
    image_histo: &mut VP8LHistogramSet,
    num_used: &mut i32,
    clusters: &[u16],
    cluster_mappings: &mut [u16],
    cur_combo: &mut Box<VP8LHistogram>,
    bin_map: &[u16],
    num_bins: i32,
    combine_cost_factor: f32,
) {
    #[derive(Clone, Copy)]
    struct BinInfo {
        /// position of the histogram that accumulates all histograms with
        /// the same bin_id
        first: i16,
        /// number of combine failures per bin_id
        num_combine_failures: u16,
    }
    let mut bin_info = [BinInfo {
        first: -1,
        num_combine_failures: 0,
    }; BIN_SIZE as usize];

    debug_assert!(num_bins <= BIN_SIZE);

    // By default, a cluster matches itself.
    for idx in 0..*num_used as usize {
        cluster_mappings[idx] = idx as u16;
    }
    let mut idx = 0;
    while (idx as i32) < image_histo.size {
        if image_histo.histograms[idx].is_none() {
            idx += 1;
            continue;
        }
        let bin_id = bin_map[idx] as usize;
        let first = bin_info[bin_id].first;
        if first == -1 {
            bin_info[bin_id].first = idx as i16;
        } else {
            let first = first as usize;
            // try to merge #idx into #first (both share the same bin_id)
            let h_idx = image_histo.histograms[idx].as_ref().unwrap();
            let h_first = image_histo.histograms[first].as_ref().unwrap();
            let bit_cost = h_idx.bit_cost;
            let bit_cost_thresh = -bit_cost * combine_cost_factor;
            let curr_cost_diff = histogram_add_eval(h_first, h_idx, cur_combo, bit_cost_thresh);
            if curr_cost_diff < bit_cost_thresh {
                // Try to merge two histograms only if the combo is a trivial one or
                // the two candidate histograms are already non-trivial.
                // For some images, 'try_combine' turns out to be false for a lot of
                // histogram pairs. In that case, we fallback to combining
                // histograms as usual to avoid increasing the header size.
                let try_combine = (cur_combo.trivial_symbol != VP8L_NON_TRIVIAL_SYM)
                    || ((h_idx.trivial_symbol == VP8L_NON_TRIVIAL_SYM)
                        && (h_first.trivial_symbol == VP8L_NON_TRIVIAL_SYM));
                let max_combine_failures = 32;
                if try_combine || bin_info[bin_id].num_combine_failures >= max_combine_failures {
                    // move the (better) merged histogram to its final slot
                    std::mem::swap(cur_combo, image_histo.histograms[first].as_mut().unwrap());
                    histogram_set_remove_histogram(image_histo, idx, num_used);
                    cluster_mappings[clusters[idx] as usize] = clusters[first];
                } else {
                    bin_info[bin_id].num_combine_failures += 1;
                }
            }
        }
        idx += 1;
    }
}

/// Implement a Lehmer random number generator with a multiplicative
/// constant of 48271 and a modulo constant of 2^31 - 1. Translation of
/// `MyRand()`.
fn my_rand(seed: &mut u32) -> u32 {
    *seed = ((*seed as u64 * 48271) % 2147483647) as u32;
    debug_assert!(*seed > 0);
    *seed
}

// -----------------------------------------------------------------------------
// Histogram pairs priority queue

/// Pair of histograms. Negative idx1 value means that pair is out-of-date.
/// Translation of `HistogramPair`.
#[derive(Clone, Copy, Default)]
struct HistogramPair {
    idx1: i32,
    idx2: i32,
    cost_diff: f32,
    cost_combo: f32,
}

/// Translation of `HistoQueue`.
struct HistoQueue {
    queue: Vec<HistogramPair>,
    size: usize,
    max_size: usize,
}

impl HistoQueue {
    /// Translation of `HistoQueueInit()`.
    fn new(max_size: usize) -> HistoQueue {
        // We allocate max_size + 1 because the last element at index "size" is
        // used as temporary data (and it could be up to max_size).
        HistoQueue {
            size: 0,
            max_size,
            queue: vec![HistogramPair::default(); max_size + 1],
        }
    }

    /// Pop a specific pair in the queue by replacing it with the last one
    /// and shrinking the queue. Translation of `HistoQueuePopPair()`.
    fn pop_pair(&mut self, pair: usize) {
        debug_assert!(pair < self.size);
        debug_assert!(self.size > 0);
        self.queue[pair] = self.queue[self.size - 1];
        self.size -= 1;
    }

    /// Check whether a pair in the queue should be updated as head or not.
    /// Translation of `HistoQueueUpdateHead()`.
    fn update_head(&mut self, pair: usize) {
        debug_assert!(self.queue[pair].cost_diff < 0.0);
        debug_assert!(pair < self.size);
        debug_assert!(self.size > 0);
        if self.queue[pair].cost_diff < self.queue[0].cost_diff {
            // Replace the best pair.
            self.queue.swap(0, pair);
        }
    }

    /// Create a pair from indices "idx1" and "idx2" provided its cost
    /// is inferior to "threshold", a negative entropy.
    /// It returns the cost of the pair, or 0. if it superior to threshold.
    /// Translation of `HistoQueuePush()`.
    fn push(
        &mut self,
        histograms: &[Option<Box<VP8LHistogram>>],
        mut idx1: i32,
        mut idx2: i32,
        threshold: f32,
    ) -> f32 {
        // Stop here if the queue is full.
        if self.size == self.max_size {
            return 0.0;
        }
        debug_assert!(threshold <= 0.0);
        if idx1 > idx2 {
            std::mem::swap(&mut idx1, &mut idx2);
        }
        let mut pair = HistogramPair {
            idx1,
            idx2,
            ..HistogramPair::default()
        };
        let h1 = histograms[idx1 as usize].as_ref().unwrap();
        let h2 = histograms[idx2 as usize].as_ref().unwrap();

        histo_queue_update_pair(h1, h2, threshold, &mut pair);

        // Do not even consider the pair if it does not improve the entropy.
        if pair.cost_diff >= threshold {
            return 0.0;
        }

        self.queue[self.size] = pair;
        self.size += 1;
        self.update_head(self.size - 1);

        pair.cost_diff
    }
}

/// Update the cost diff and combo of a pair of histograms. This needs to
/// be called when the the histograms have been merged with a third one.
/// Translation of `HistoQueueUpdatePair()`.
fn histo_queue_update_pair(
    h1: &VP8LHistogram,
    h2: &VP8LHistogram,
    threshold: f32,
    pair: &mut HistogramPair,
) {
    let sum_cost = h1.bit_cost + h2.bit_cost;
    pair.cost_combo = 0.0;
    get_combined_histogram_entropy(h1, h2, sum_cost + threshold, &mut pair.cost_combo);
    pair.cost_diff = pair.cost_combo - sum_cost;
}

/// The `HistogramAdd(histograms[idx2], histograms[idx1], histograms[idx1])`
/// of the combiners.
fn histogram_add_into(histograms: &mut [Option<Box<VP8LHistogram>>], src: usize, dst: usize) {
    let a = histograms[src].take().unwrap();
    histogram_add_eq(&a, histograms[dst].as_mut().unwrap());
    histograms[src] = Some(a);
}

// -----------------------------------------------------------------------------

/// Combines histograms by continuously choosing the one with the highest
/// cost reduction. Translation of `HistogramCombineGreedy()`.
fn histogram_combine_greedy(image_histo: &mut VP8LHistogramSet, num_used: &mut i32) -> bool {
    let image_histo_size = image_histo.size as usize;

    // image_histo_size^2 for the queue size is safe. If you look at
    // HistogramCombineGreedy, and imagine that UpdateQueueFront always pushes
    // data to the queue, you insert at most:
    // - image_histo_size*(image_histo_size-1)/2 (the first two for loops)
    // - image_histo_size - 1 in the last for loop at the first iteration of
    //   the while loop, image_histo_size - 2 at the second iteration ...
    //   therefore image_histo_size*(image_histo_size-1)/2 overall too
    let mut histo_queue = HistoQueue::new(image_histo_size * image_histo_size);

    for i in 0..image_histo_size {
        if image_histo.histograms[i].is_none() {
            continue;
        }
        for j in i + 1..image_histo_size {
            // Initialize queue.
            if image_histo.histograms[j].is_none() {
                continue;
            }
            histo_queue.push(&image_histo.histograms, i as i32, j as i32, 0.0);
        }
    }

    while histo_queue.size > 0 {
        let idx1 = histo_queue.queue[0].idx1;
        let idx2 = histo_queue.queue[0].idx2;
        histogram_add_into(&mut image_histo.histograms, idx2 as usize, idx1 as usize);
        image_histo.histograms[idx1 as usize]
            .as_mut()
            .unwrap()
            .bit_cost = histo_queue.queue[0].cost_combo;

        // Remove merged histogram.
        histogram_set_remove_histogram(image_histo, idx2 as usize, num_used);

        // Remove pairs intersecting the just combined best pair.
        let mut i = 0;
        while i < histo_queue.size {
            let p = histo_queue.queue[i];
            if p.idx1 == idx1 || p.idx2 == idx1 || p.idx1 == idx2 || p.idx2 == idx2 {
                histo_queue.pop_pair(i);
            } else {
                histo_queue.update_head(i);
                i += 1;
            }
        }

        // Push new pairs formed with combined histogram to the queue.
        for i in 0..image_histo.size as usize {
            if i as i32 == idx1 || image_histo.histograms[i].is_none() {
                continue;
            }
            histo_queue.push(&image_histo.histograms, idx1, i as i32, 0.0);
        }
    }

    true
}

/// Perform histogram aggregation using a stochastic approach.
/// 'do_greedy' is set to 1 if a greedy approach needs to be performed
/// afterwards, 0 otherwise. Translation of `HistogramCombineStochastic()`.
fn histogram_combine_stochastic(
    image_histo: &mut VP8LHistogramSet,
    num_used: &mut i32,
    min_cluster_size: i32,
    do_greedy: &mut bool,
) -> bool {
    let mut seed = 1u32;
    let mut tries_with_no_success = 0;
    let outer_iters = *num_used;
    let num_tries_no_success = outer_iters / 2;
    // Priority queue of histogram pairs. Its size of 'kHistoQueueSize'
    // impacts the quality of the compression and the speed: the smaller the
    // faster but the worse for the compression.
    let k_histo_queue_size = 9;

    if *num_used < min_cluster_size {
        *do_greedy = true;
        return true;
    }

    // mapping from an index in image_histo with no NULL histogram to the full
    // blown image_histo.
    let mut mappings = vec![0i32; *num_used as usize];
    let mut histo_queue = HistoQueue::new(k_histo_queue_size);
    // Fill the initial mapping.
    {
        let mut j = 0;
        for iter in 0..image_histo.size as usize {
            if image_histo.histograms[iter].is_none() {
                continue;
            }
            mappings[j] = iter as i32;
            j += 1;
        }
        debug_assert!(j as i32 == *num_used);
    }

    // Collapse similar histograms in 'image_histo'.
    let mut iter = 0;
    while iter < outer_iters && *num_used >= min_cluster_size && {
        tries_with_no_success += 1;
        tries_with_no_success < num_tries_no_success
    } {
        let mut best_cost = if histo_queue.size == 0 {
            0.0f32
        } else {
            histo_queue.queue[0].cost_diff
        };
        let rand_range = ((*num_used - 1) * *num_used) as u32;
        // (*num_used) / 2 was chosen empirically. Less means faster but worse
        // compression.
        let num_tries = *num_used / 2;

        // Pick random samples.
        let mut j = 0;
        while *num_used >= 2 && j < num_tries {
            // Choose two different histograms at random and try to combine them.
            let tmp = my_rand(&mut seed) % rand_range;
            let idx1 = tmp / (*num_used - 1) as u32;
            let mut idx2 = tmp % (*num_used - 1) as u32;
            if idx2 >= idx1 {
                idx2 += 1;
            }
            let idx1 = mappings[idx1 as usize];
            let idx2 = mappings[idx2 as usize];

            // Calculate cost reduction on combination.
            let curr_cost = histo_queue.push(&image_histo.histograms, idx1, idx2, best_cost);
            if curr_cost < 0.0 {
                // found a better pair?
                best_cost = curr_cost;
                // Empty the queue if we reached full capacity.
                if histo_queue.size == histo_queue.max_size {
                    break;
                }
            }
            j += 1;
        }
        if histo_queue.size == 0 {
            iter += 1;
            continue;
        }

        // Get the best histograms.
        let best_idx1 = histo_queue.queue[0].idx1;
        let best_idx2 = histo_queue.queue[0].idx2;
        debug_assert!(best_idx1 < best_idx2);
        // Pop best_idx2 from mappings.
        let mapping_index = mappings[..*num_used as usize]
            .binary_search(&best_idx2)
            .expect("best_idx2 is mapped");
        mappings.copy_within(mapping_index + 1..*num_used as usize, mapping_index);
        // Merge the histograms and remove best_idx2 from the queue.
        histogram_add_into(
            &mut image_histo.histograms,
            best_idx2 as usize,
            best_idx1 as usize,
        );
        image_histo.histograms[best_idx1 as usize]
            .as_mut()
            .unwrap()
            .bit_cost = histo_queue.queue[0].cost_combo;
        histogram_set_remove_histogram(image_histo, best_idx2 as usize, num_used);
        // Parse the queue and update each pair that deals with best_idx1,
        // best_idx2 or image_histo_size.
        let mut j = 0;
        while j < histo_queue.size {
            let mut p = histo_queue.queue[j];
            let is_idx1_best = p.idx1 == best_idx1 || p.idx1 == best_idx2;
            let is_idx2_best = p.idx2 == best_idx1 || p.idx2 == best_idx2;
            let mut do_eval = false;
            // The front pair could have been duplicated by a random pick so
            // check for it all the time nevertheless.
            if is_idx1_best && is_idx2_best {
                histo_queue.pop_pair(j);
                continue;
            }
            // Any pair containing one of the two best indices should only refer to
            // best_idx1. Its cost should also be updated.
            if is_idx1_best {
                p.idx1 = best_idx1;
                do_eval = true;
            } else if is_idx2_best {
                p.idx2 = best_idx1;
                do_eval = true;
            }
            // Make sure the index order is respected.
            if p.idx1 > p.idx2 {
                std::mem::swap(&mut p.idx1, &mut p.idx2);
            }
            if do_eval {
                // Re-evaluate the cost of an updated pair.
                let h1 = image_histo.histograms[p.idx1 as usize].as_ref().unwrap();
                let h2 = image_histo.histograms[p.idx2 as usize].as_ref().unwrap();
                histo_queue_update_pair(h1, h2, 0.0, &mut p);
                if p.cost_diff >= 0.0 {
                    histo_queue.queue[j] = p;
                    histo_queue.pop_pair(j);
                    continue;
                }
            }
            histo_queue.queue[j] = p;
            histo_queue.update_head(j);
            j += 1;
        }
        tries_with_no_success = 0;
        iter += 1;
    }
    *do_greedy = *num_used <= min_cluster_size;
    true
}

// -----------------------------------------------------------------------------
// Histogram refinement

/// Find the best 'out' histogram for each of the 'in' histograms.
/// At call-time, 'out' contains the histograms of the clusters.
/// Note: we assume that out[]->bit_cost_ is already up-to-date.
/// Translation of `HistogramRemap()`.
fn histogram_remap(input: &VP8LHistogramSet, out: &mut VP8LHistogramSet, symbols: &mut [u16]) {
    let in_size = out.max_size as usize;
    let out_size = out.size as usize;
    if out_size > 1 {
        for i in 0..in_size {
            let mut best_out = 0;
            let mut best_bits = MAX_BIT_COST;
            let Some(in_histo) = &input.histograms[i] else {
                // Arbitrarily set to the previous value if unused to help future LZ77.
                symbols[i] = symbols[i - 1];
                continue;
            };
            for k in 0..out_size {
                let cur_bits =
                    histogram_add_thresh(out.histograms[k].as_ref().unwrap(), in_histo, best_bits);
                if k == 0 || cur_bits < best_bits {
                    best_bits = cur_bits;
                    best_out = k;
                }
            }
            symbols[i] = best_out as u16;
        }
    } else {
        debug_assert!(out_size == 1);
        symbols[..in_size].fill(0);
    }

    // Recompute each out based on raw and symbols.
    vp8l_histogram_set_clear(out);
    out.size = out_size as i32;

    for i in 0..in_size {
        let Some(in_histo) = &input.histograms[i] else {
            continue;
        };
        let idx = symbols[i] as usize;
        histogram_add_eq(in_histo, out.histograms[idx].as_mut().unwrap());
    }
}

/// Translation of `GetCombineCostFactor()`.
fn get_combine_cost_factor(histo_size: i32, quality: i32) -> f32 {
    let mut combine_cost_factor = 0.16f32;
    if quality < 90 {
        if histo_size > 256 {
            combine_cost_factor /= 2.0;
        }
        if histo_size > 512 {
            combine_cost_factor /= 2.0;
        }
        if histo_size > 1024 {
            combine_cost_factor /= 2.0;
        }
        if quality <= 50 {
            combine_cost_factor /= 2.0;
        }
    }
    combine_cost_factor
}

/// Given a HistogramSet 'set', the mapping of clusters 'cluster_mapping'
/// and the current assignment of the cells in 'symbols', merge the
/// clusters and assign the smallest possible clusters values. Translation
/// of `OptimizeHistogramSymbols()`.
fn optimize_histogram_symbols(
    set: &VP8LHistogramSet,
    cluster_mappings: &mut [u16],
    num_clusters: usize,
    cluster_mappings_tmp: &mut [u16],
    symbols: &mut [u16],
) {
    let mut do_continue = true;
    // First, assign the lowest cluster to each pixel.
    while do_continue {
        do_continue = false;
        for i in 0..num_clusters {
            let mut k = cluster_mappings[i] as usize;
            while k != cluster_mappings[k] as usize {
                cluster_mappings[k] = cluster_mappings[cluster_mappings[k] as usize];
                k = cluster_mappings[k] as usize;
            }
            if k != cluster_mappings[i] as usize {
                do_continue = true;
                cluster_mappings[i] = k as u16;
            }
        }
    }
    // Create a mapping from a cluster id to its minimal version.
    let mut cluster_max = 0u16;
    cluster_mappings_tmp[..set.max_size as usize].fill(0);
    debug_assert!(cluster_mappings[0] == 0);
    // Re-map the ids.
    for i in 0..set.max_size as usize {
        if symbols[i] == K_INVALID_HISTOGRAM_SYMBOL {
            continue;
        }
        let cluster = cluster_mappings[symbols[i] as usize] as usize;
        debug_assert!((symbols[i] as usize) < num_clusters);
        if cluster > 0 && cluster_mappings_tmp[cluster] == 0 {
            cluster_max += 1;
            cluster_mappings_tmp[cluster] = cluster_max;
        }
        symbols[i] = cluster_mappings_tmp[cluster];
    }

    // Make sure all cluster values are used.
    if cfg!(debug_assertions) {
        let mut cluster_max = 0;
        for &s in &symbols[..set.max_size as usize] {
            if s == K_INVALID_HISTOGRAM_SYMBOL {
                continue;
            }
            if s <= cluster_max {
                continue;
            }
            cluster_max += 1;
            debug_assert!(s == cluster_max);
        }
    }
}

/// Translation of `RemoveEmptyHistograms()`.
fn remove_empty_histograms(image_histo: &mut VP8LHistogramSet) {
    let mut size = 0;
    for i in 0..image_histo.size as usize {
        if image_histo.histograms[i].is_none() {
            continue;
        }
        let h = image_histo.histograms[i].take();
        image_histo.histograms[size] = h;
        size += 1;
    }
    image_histo.size = size as i32;
}

/// Builds the histogram image. Returns false in case of error.
/// Translation of `VP8LGetHistoImageSymbols()` (without its low-effort
/// path and progress report).
#[allow(clippy::too_many_arguments)]
pub(crate) fn vp8l_get_histo_image_symbols(
    xsize: i32,
    ysize: i32,
    refs: &VP8LBackwardRefs,
    quality: i32,
    low_effort: bool,
    histogram_bits: i32,
    cache_bits: i32,
    image_histo: &mut VP8LHistogramSet,
    tmp_histo: &mut Box<VP8LHistogram>,
    histogram_symbols: &mut [u16],
) -> bool {
    debug_assert!(!low_effort, "the low-effort mode is not translated");
    let histo_xsize = if histogram_bits != 0 {
        vp8l_sub_sample_size(xsize as u32, histogram_bits as u32) as i32
    } else {
        1
    };
    let histo_ysize = if histogram_bits != 0 {
        vp8l_sub_sample_size(ysize as u32, histogram_bits as u32) as i32
    } else {
        1
    };
    let image_histo_raw_size = histo_xsize * histo_ysize;
    let mut orig_histo = vp8l_allocate_histogram_set(image_histo_raw_size, cache_bits);
    // Don't attempt linear bin-partition heuristic for
    // histograms of small sizes (as bin_map will be very sparse) and
    // maximum quality q==100 (to preserve the compression gains at that level).
    let entropy_combine_num_bins = BIN_SIZE;
    let mut map_tmp_mem = vec![0u16; 2 * image_histo_raw_size as usize];
    let (map_tmp, cluster_mappings) = map_tmp_mem.split_at_mut(image_histo_raw_size as usize);
    let mut num_used = image_histo_raw_size;

    // Construct the histograms from backward references.
    histogram_build(xsize, histogram_bits, refs, &mut orig_histo);
    // Copies the histograms and computes its bit_cost.
    // histogram_symbols is optimized
    histogram_copy_and_analyze(
        &mut orig_histo,
        image_histo,
        &mut num_used,
        histogram_symbols,
    );

    let entropy_combine = (num_used > entropy_combine_num_bins * 2) && (quality < 100);

    if entropy_combine {
        let combine_cost_factor = get_combine_cost_factor(image_histo_raw_size, quality);
        let num_clusters = num_used as usize;

        histogram_analyze_entropy_bin(image_histo, map_tmp, low_effort);
        // Collapse histograms with similar entropy.
        histogram_combine_entropy_bin(
            image_histo,
            &mut num_used,
            histogram_symbols,
            cluster_mappings,
            tmp_histo,
            map_tmp,
            entropy_combine_num_bins,
            combine_cost_factor,
        );
        optimize_histogram_symbols(
            image_histo,
            cluster_mappings,
            num_clusters,
            map_tmp,
            histogram_symbols,
        );
    }

    // Don't combine the histograms using stochastic and greedy heuristics for
    // low-effort compression mode.
    {
        let x = quality as f32 / 100.0f32;
        // cubic ramp between 1 and MAX_HISTO_GREEDY:
        let threshold_size = (1.0f32 + (x * x * x) * (MAX_HISTO_GREEDY - 1) as f32) as i32;
        let mut do_greedy = false;
        if !histogram_combine_stochastic(image_histo, &mut num_used, threshold_size, &mut do_greedy)
        {
            return false;
        }
        if do_greedy {
            remove_empty_histograms(image_histo);
            if !histogram_combine_greedy(image_histo, &mut num_used) {
                return false;
            }
        }
    }

    // Find the optimal map from original histograms to the final ones.
    remove_empty_histograms(image_histo);
    histogram_remap(&orig_histo, image_histo, histogram_symbols);

    true
}
