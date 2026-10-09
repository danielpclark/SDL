// Rust translation of lib/jxl/ans_params.h, lib/jxl/ans_common.h and
// lib/jxl/ans_common.cc from libjxl (https://github.com/libjxl/libjxl, at the
// revision SDL_image's external/libjxl pins: libjxl 0.7.3 with SDL's
// patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Common parameters that are needed for both the ANS entropy encoding and
//! decoding methods, and the alias tables.

// TODO(veluca): decide if 12 is the best constant here (valid range is up to
// 16). This requires recomputing the Huffman tables in {enc,dec}_ans.cc
// 14 gives a 0.2% improvement at d1 and makes d8 slightly worse. This is
// likely not worth the increase in encoder complexity.
pub(crate) const ANS_LOG_TAB_SIZE: u32 = 12;
pub(crate) const ANS_TAB_SIZE: u32 = 1 << ANS_LOG_TAB_SIZE;
#[allow(dead_code)]
pub(crate) const ANS_TAB_MASK: u32 = ANS_TAB_SIZE - 1;

// Largest possible symbol to be encoded by either ANS or prefix coding.
#[allow(dead_code)]
pub(crate) const PREFIX_MAX_ALPHABET_SIZE: usize = 4096;
pub(crate) const ANS_MAX_ALPHABET_SIZE: usize = 256;

// Max number of bits for prefix coding.
pub(crate) const PREFIX_MAX_BITS: usize = 15;

pub(crate) const ANS_SIGNATURE: u32 = 0x13; // Initial state, used as CRC.

/// Returns the precision (number of bits) that should be used to store
/// a histogram count such that Log2Floor(count) == logcount. Translation of
/// `GetPopulationCountPrecision()`.
#[inline]
pub(crate) fn get_population_count_precision(logcount: u32, shift: u32) -> u32 {
    let r = (logcount as i32).min(shift as i32 - ((ANS_LOG_TAB_SIZE - logcount) >> 1) as i32);
    if r < 0 {
        return 0;
    }
    r as u32
}

/// Returns a histogram where the counts are positive, differ by at most 1,
/// and add up to total_count. The bigger counts (if any) are at the beginning
/// of the histogram. Translation of `CreateFlatHistogram()`.
pub(crate) fn create_flat_histogram(length: i32, total_count: i32) -> Vec<i32> {
    debug_assert!(length > 0);
    debug_assert!(length <= total_count);
    let count = total_count / length;
    let mut result = vec![count; length as usize];
    let rem_counts = total_count % length;
    for r in result.iter_mut().take(rem_counts as usize) {
        *r += 1;
    }
    result
}

/// An alias table implements a mapping from the [0, ANS_TAB_SIZE) range into
/// the [0, ANS_MAX_ALPHABET_SIZE) range, satisfying the following conditions:
/// - each symbol occurs as many times as specified by any valid distribution
///   of frequencies of the symbols. A valid distribution here is an array of
///   ANS_MAX_ALPHABET_SIZE that contains numbers in the range [0, ANS_TAB_SIZE],
///   and whose sum is ANS_TAB_SIZE.
/// - lookups can be done in constant time, and also return how many smaller
///   input values map into the same symbol, according to some well-defined order
///   of input values.
/// - the space used by the alias table is given by a small constant times the
///   index of the largest symbol with nonzero probability in the distribution.
///
/// Each of the entries in the table covers a range of `entry_size` values in the
/// [0, ANS_TAB_SIZE) range; consecutive entries represent consecutive
/// sub-ranges. In the range covered by entry `i`, the first `cutoff` values map
/// to symbol `i`, while the others map to symbol `right_value`.
///
/// TODO(veluca): consider making the order used for computing offsets easier to
/// define - it is currently defined by the algorithm to compute the alias table.
/// Beware of breaking the implicit assumption that symbols that come after the
/// cutoff value should have an offset at least as big as the cutoff.
///
/// Translation of `AliasTable::Entry`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct AliasEntry {
    pub cutoff: u8,      // < kEntrySizeMinus1 when used by ANS.
    pub right_value: u8, // < alphabet size.
    pub freq0: u16,

    // Only used if `greater` (see Lookup)
    pub offsets1: u16,        // <= ANS_TAB_SIZE
    pub freq1_xor_freq0: u16, // for branchless ternary in Lookup
}

/// Translation of `AliasTable::Symbol`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AliasSymbol {
    pub value: usize,
    pub offset: usize,
    pub freq: usize,
}

/// Dividing `value` by `entry_size` determines `i`, the entry which is
/// responsible for the input. If the remainder is below `cutoff`, then the
/// mapped symbol is `i`; since `offsets[0]` stores the number of occurrences
/// of `i` "before" the start of this entry, the offset of the input will be
/// `offsets[0] + remainder`. If the remainder is above cutoff, the mapped
/// symbol is `right_value`; since `offsets[1]` stores the number of
/// occurrences of `right_value` "before" this entry, minus the `cutoff` value,
/// the input offset is then `remainder + offsets[1]`. Translation of
/// `AliasTable::Lookup()`.
#[inline]
pub(crate) fn alias_lookup(
    table: &[AliasEntry],
    value: usize,
    log_entry_size: usize,
    entry_size_minus_1: usize,
) -> AliasSymbol {
    let i = value >> log_entry_size;
    let pos = value & entry_size_minus_1;

    let e = &table[i];
    let cutoff = e.cutoff as usize;
    let right_value = e.right_value as usize;
    let freq0 = e.freq0 as usize;

    let greater = pos >= cutoff;

    let offsets1_or_0 = if greater { e.offsets1 as usize } else { 0 };
    let freq1_xor_freq0_or_0 = if greater {
        e.freq1_xor_freq0 as usize
    } else {
        0
    };

    AliasSymbol {
        value: if greater { right_value } else { i },
        offset: offsets1_or_0 + pos,
        freq: freq0 ^ freq1_xor_freq0_or_0, // = greater ? freq1 : freq0
    }
}

/// Computes an alias table for a given distribution. Translation of
/// `InitAliasTable()`.
///
/// First, all trailing non-occuring symbols are removed from the
/// distribution; if this leaves the distribution empty, a dummy symbol with
/// max weight is added. This ensures that the resulting distribution sums to
/// total table size. Then, `entry_size` is chosen to be the largest power of
/// two so that `table_size` = ANS_TAB_SIZE/`entry_size` is at least as big as
/// the distribution size. (See upstream's ans_common.cc for the details of the
/// construction.)
pub(crate) fn init_alias_table(
    mut distribution: Vec<i32>,
    range: u32,
    log_alpha_size: usize,
    a: &mut [AliasEntry],
) {
    while distribution.last() == Some(&0) {
        distribution.pop();
    }
    // Ensure that a valid table is always returned, even for an empty
    // alphabet. Otherwise, a specially-crafted stream might crash the
    // decoder.
    if distribution.is_empty() {
        distribution.push(range as i32);
    }
    let table_size = 1usize << log_alpha_size;
    debug_assert!(distribution.iter().sum::<i32>() as u32 == range);
    // range must be a power of two
    debug_assert!((range & (range - 1)) == 0);
    debug_assert!(distribution.len() <= table_size);
    debug_assert!(table_size as u32 <= range);
    let entry_size = range >> log_alpha_size; // this is exact
                                              // Special case for single-symbol distributions, that ensures that the state
                                              // does not change when decoding from such a distribution. Note that, since we
                                              // hardcode offset0 == 0, it is not straightforward (if at all possible) to
                                              // fix the general case to produce this result.
    for sym in 0..distribution.len() {
        if distribution[sym] == ANS_TAB_SIZE as i32 {
            for i in 0..table_size {
                a[i].right_value = sym as u8;
                a[i].cutoff = 0;
                a[i].offsets1 = (entry_size as usize * i) as u16;
                a[i].freq0 = 0;
                a[i].freq1_xor_freq0 = ANS_TAB_SIZE as u16;
            }
            return;
        }
    }
    let mut underfull_posn: Vec<u32> = Vec::new();
    let mut overfull_posn: Vec<u32> = Vec::new();
    let mut cutoffs = vec![0u32; 1 << log_alpha_size];
    // Initialize entries.
    for i in 0..distribution.len() {
        cutoffs[i] = distribution[i] as u32;
        if cutoffs[i] > entry_size {
            overfull_posn.push(i as u32);
        } else if cutoffs[i] < entry_size {
            underfull_posn.push(i as u32);
        }
    }
    for i in distribution.len()..table_size {
        cutoffs[i] = 0;
        underfull_posn.push(i as u32);
    }
    // Reassign overflow/underflow values.
    while let Some(overfull_i) = overfull_posn.pop() {
        let Some(underfull_i) = underfull_posn.pop() else {
            // JXL_ASSERT(!underfull_posn.empty());
            debug_assert!(false);
            return;
        };
        let (overfull_i, underfull_i) = (overfull_i as usize, underfull_i as usize);
        let underfull_by = entry_size.wrapping_sub(cutoffs[underfull_i]);
        cutoffs[overfull_i] = cutoffs[overfull_i].wrapping_sub(underfull_by);
        // overfull positions have their original symbols
        a[underfull_i].right_value = overfull_i as u8;
        a[underfull_i].offsets1 = cutoffs[overfull_i] as u16;
        // Slots in the right part of entry underfull_i were taken from the end
        // of the symbols in entry overfull_i.
        if cutoffs[overfull_i] < entry_size {
            underfull_posn.push(overfull_i as u32);
        } else if cutoffs[overfull_i] > entry_size {
            overfull_posn.push(overfull_i as u32);
        }
    }
    for i in 0..table_size {
        // cutoffs[i] is properly initialized but the clang-analyzer doesn't infer
        // it since it is partially initialized across two for-loops.
        if cutoffs[i] == entry_size {
            a[i].right_value = i as u8;
            a[i].offsets1 = 0;
            a[i].cutoff = 0;
        } else {
            // Note that, if cutoff is not equal to entry_size,
            // a[i].offsets1 was initialized with (overfull cutoff) -
            // (entry_size - a[i].cutoff). Thus, subtracting
            // a[i].cutoff cannot make it negative.
            a[i].offsets1 = a[i].offsets1.wrapping_sub(cutoffs[i] as u16);
            a[i].cutoff = cutoffs[i] as u8;
        }
        let freq0 = if i < distribution.len() {
            distribution[i] as usize
        } else {
            0
        };
        let i1 = a[i].right_value as usize;
        let freq1 = if i1 < distribution.len() {
            distribution[i1] as usize
        } else {
            0
        };
        a[i].freq0 = freq0 as u16;
        a[i].freq1_xor_freq0 = (freq1 ^ freq0) as u16;
    }
}
