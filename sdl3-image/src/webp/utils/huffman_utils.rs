// Rust translation of src/utils/huffman_utils.c and huffman_utils.h from
// libwebp (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Utilities for building and looking up Huffman trees.
//!
//! The chained segments of `HuffmanCode`s are a list of vectors here, and
//! a table pointer is a segment index and an offset in it.

use crate::webp::decode::{HUFFMAN_CODES_PER_META_CODE, MAX_ALLOWED_CODE_LENGTH};
use crate::webp::utils::safe_alloc;

pub(crate) const HUFFMAN_TABLE_BITS: i32 = 8;
pub(crate) const HUFFMAN_TABLE_MASK: u32 = (1 << HUFFMAN_TABLE_BITS) - 1;
pub(crate) const LENGTHS_TABLE_BITS: i32 = 7;
pub(crate) const LENGTHS_TABLE_MASK: u32 = (1 << LENGTHS_TABLE_BITS) - 1;

/// Huffman lookup table entry. Translation of `HuffmanCode`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct HuffmanCode {
    /// number of bits used for this symbol
    pub(crate) bits: u8,
    /// symbol value or table offset
    pub(crate) value: u16,
}

/// long version for holding 32b values. Translation of `HuffmanCode32`.
#[derive(Clone, Copy, Default)]
pub(crate) struct HuffmanCode32 {
    /// number of bits used for this symbol,
    /// or an impossible value if not a literal code.
    pub(crate) bits: i32,
    /// 32b packed ARGB value if literal,
    /// or non-literal symbol otherwise
    pub(crate) value: u32,
}

/// Contiguous memory segment of HuffmanCodes. Translation of
/// `HuffmanTablesSegment` (`start` and `size` are the vector).
pub(crate) struct HuffmanTablesSegment {
    pub(crate) start: Vec<HuffmanCode>,
    /// Where we are writing into the segment. Starts at 'start' and
    /// cannot go beyond 'start' + 'size'.
    pub(crate) curr_table: usize,
}

/// Chained memory segments of HuffmanCodes. Translation of
/// `HuffmanTables` (the chain is `segments`, the root first).
#[derive(Default)]
pub(crate) struct HuffmanTables {
    pub(crate) segments: Vec<HuffmanTablesSegment>,
    /// Currently processed segment. At first, this is 'root'.
    pub(crate) curr_segment: usize,
}

/// A table in a [`HuffmanTables`]: its segment and its offset there (a
/// `HuffmanCode*` upstream).
#[derive(Clone, Copy, Default)]
pub(crate) struct HuffmanTableRef {
    pub(crate) segment: u32,
    pub(crate) offset: u32,
}

impl HuffmanTables {
    /// The table `r` refers to, from its first entry to the end of its
    /// segment.
    pub(crate) fn table(&self, r: HuffmanTableRef) -> &[HuffmanCode] {
        &self.segments[r.segment as usize].start[r.offset as usize..]
    }

    /// Where the current segment is being written.
    pub(crate) fn curr_table(&self) -> HuffmanTableRef {
        HuffmanTableRef {
            segment: self.curr_segment as u32,
            offset: self.segments[self.curr_segment].curr_table as u32,
        }
    }
}

pub(crate) const HUFFMAN_PACKED_BITS: i32 = 6;
pub(crate) const HUFFMAN_PACKED_TABLE_SIZE: usize = 1 << HUFFMAN_PACKED_BITS;

/// Huffman table group.
/// Includes special handling for the following cases:
///  - is_trivial_literal: one common literal base for RED/BLUE/ALPHA (not GREEN)
///  - is_trivial_code: only 1 code (no bit is read from bitstream)
///  - use_packed_table: few enough literal symbols, so all the bit codes
///    can fit into a small look-up table packed_table[]
///
/// The common literal base, if applicable, is stored in 'literal_arb'.
/// Translation of `HTreeGroup`.
#[derive(Clone, Copy)]
pub(crate) struct HTreeGroup {
    pub(crate) htrees: [HuffmanTableRef; HUFFMAN_CODES_PER_META_CODE],
    /// True, if huffman trees for Red, Blue & Alpha Symbols are trivial
    /// (have a single code).
    pub(crate) is_trivial_literal: bool,
    /// If is_trivial_literal is true, this is the ARGB value of the pixel,
    /// with Green channel being set to zero.
    pub(crate) literal_arb: u32,
    /// true if is_trivial_literal with only one code
    pub(crate) is_trivial_code: bool,
    /// use packed table below for short literal code
    pub(crate) use_packed_table: bool,
    /// table mapping input bits to a packed values, or escape case to
    /// literal code
    pub(crate) packed_table: [HuffmanCode32; HUFFMAN_PACKED_TABLE_SIZE],
}

impl Default for HTreeGroup {
    fn default() -> Self {
        HTreeGroup {
            htrees: [HuffmanTableRef::default(); HUFFMAN_CODES_PER_META_CODE],
            is_trivial_literal: false,
            literal_arb: 0,
            is_trivial_code: false,
            use_packed_table: false,
            packed_table: [HuffmanCode32::default(); HUFFMAN_PACKED_TABLE_SIZE],
        }
    }
}

// Huffman data read via DecodeImageStream is represented in two (red and green)
// bytes.
const MAX_HTREE_GROUPS: i32 = 0x10000;

/// Creates the instance of HTreeGroup with specified number of
/// tree-groups. Translation of `VP8LHtreeGroupsNew()`.
pub(crate) fn vp8l_htree_groups_new(num_htree_groups: i32) -> Option<Vec<HTreeGroup>> {
    let htree_groups = safe_alloc(num_htree_groups as u64, HTreeGroup::default())?;
    debug_assert!(num_htree_groups <= MAX_HTREE_GROUPS);
    Some(htree_groups)
}

/// Returns reverse(reverse(key, len) + 1, len), where reverse(key, len) is
/// the bit-wise reversal of the len least significant bits of key.
/// Translation of `GetNextKey()`.
fn get_next_key(key: u32, len: i32) -> u32 {
    let mut step: u32 = 1 << (len - 1);
    while key & step != 0 {
        step >>= 1;
    }
    if step != 0 {
        (key & (step - 1)) + step
    } else {
        key
    }
}

/// Stores code in table[0], table[step], table[2*step], ..., table[end].
/// Assumes that end is an integer multiple of step. Translation of
/// `ReplicateValue()`.
fn replicate_value(table: &mut [HuffmanCode], step: i32, mut end: i32, code: HuffmanCode) {
    debug_assert!(end % step == 0);
    loop {
        end -= step;
        table[end as usize] = code;
        if end <= 0 {
            break;
        }
    }
}

/// Returns the table width of the next 2nd level table. count is the
/// histogram of bit lengths for the remaining symbols, len is the code
/// length of the next processed symbol. Translation of `NextTableBitSize()`.
fn next_table_bit_size(count: &[i32], mut len: i32, root_bits: i32) -> i32 {
    let mut left = 1 << (len - root_bits);
    while len < MAX_ALLOWED_CODE_LENGTH {
        left -= count[len as usize];
        if left <= 0 {
            break;
        }
        len += 1;
        left <<= 1;
    }
    len - root_bits
}

/// sorted[code_lengths_size] is a pre-allocated array for sorting symbols
/// by code length. Translation of `BuildHuffmanTable()` (with
/// `root_table` and `sorted` both `None`, it only computes the size).
fn build_huffman_table(
    mut root_table: Option<&mut [HuffmanCode]>,
    root_bits: i32,
    code_lengths: &[i32],
    mut sorted: Option<&mut [u16]>,
) -> i32 {
    let code_lengths_size = code_lengths.len();
    // next available space in table
    let mut table: usize = 0;
    // total size root table + 2nd level table
    let mut total_size: i32 = 1 << root_bits;
    // number of codes of each length:
    let mut count = [0i32; MAX_ALLOWED_CODE_LENGTH as usize + 1];
    // offsets in sorted table for each length:
    let mut offset = [0i32; MAX_ALLOWED_CODE_LENGTH as usize + 1];

    debug_assert!(code_lengths_size != 0);
    debug_assert!(root_table.is_some() == sorted.is_some());
    debug_assert!(root_bits > 0);

    // Build histogram of code lengths.
    for &len in code_lengths {
        if len > MAX_ALLOWED_CODE_LENGTH {
            return 0;
        }
        count[len as usize] += 1;
    }

    // Error, all code lengths are zeros.
    if count[0] as usize == code_lengths_size {
        return 0;
    }

    // Generate offsets into sorted symbol table by code length.
    offset[1] = 0;
    for len in 1..MAX_ALLOWED_CODE_LENGTH as usize {
        if count[len] > (1 << len) {
            return 0;
        }
        offset[len + 1] = offset[len] + count[len];
    }

    // Sort symbols by length, by symbol order within each length.
    for (symbol, &symbol_code_length) in code_lengths.iter().enumerate() {
        if symbol_code_length > 0 {
            let o = &mut offset[symbol_code_length as usize];
            if let Some(sorted) = sorted.as_deref_mut() {
                sorted[*o as usize] = symbol as u16;
            }
            *o += 1;
        }
    }

    // Special case code with only one value.
    if offset[MAX_ALLOWED_CODE_LENGTH as usize] == 1 {
        if let (Some(root), Some(sorted)) = (root_table, sorted) {
            let code = HuffmanCode {
                bits: 0,
                value: sorted[0],
            };
            replicate_value(&mut root[table..], 1, total_size, code);
        }
        return total_size;
    }

    {
        // step size to replicate values in current table
        let mut step: i32;
        // low bits for current root entry
        let mut low: u32 = 0xffffffff;
        // mask for low bits
        let mask: u32 = (total_size - 1) as u32;
        // reversed prefix code
        let mut key: u32 = 0;
        // number of Huffman tree nodes
        let mut num_nodes: i32 = 1;
        // number of open branches in current tree level
        let mut num_open: i32 = 1;
        // key length of current table
        let mut table_bits = root_bits;
        // size of current table
        let mut table_size: i32 = 1 << table_bits;
        let mut symbol = 0usize;
        // Fill in root table.
        let mut len = 1;
        step = 2;
        while len <= root_bits {
            num_open <<= 1;
            num_nodes += num_open;
            num_open -= count[len as usize];
            if num_open < 0 {
                return 0;
            }
            if let (Some(root), Some(sorted)) = (root_table.as_deref_mut(), sorted.as_deref()) {
                while count[len as usize] > 0 {
                    let code = HuffmanCode {
                        bits: len as u8,
                        value: sorted[symbol],
                    };
                    symbol += 1;
                    replicate_value(&mut root[table + key as usize..], step, table_size, code);
                    key = get_next_key(key, len);
                    count[len as usize] -= 1;
                }
            }
            len += 1;
            step <<= 1;
        }

        // Fill in 2nd level tables and add pointers to root table.
        len = root_bits + 1;
        step = 2;
        while len <= MAX_ALLOWED_CODE_LENGTH {
            num_open <<= 1;
            num_nodes += num_open;
            num_open -= count[len as usize];
            if num_open < 0 {
                return 0;
            }
            while count[len as usize] > 0 {
                if (key & mask) != low {
                    if root_table.is_some() {
                        table += table_size as usize;
                    }
                    table_bits = next_table_bit_size(&count, len, root_bits);
                    table_size = 1 << table_bits;
                    total_size += table_size;
                    low = key & mask;
                    if let Some(root) = root_table.as_deref_mut() {
                        root[low as usize].bits = (table_bits + root_bits) as u8;
                        root[low as usize].value = (table - low as usize) as u16;
                    }
                }
                if let (Some(root), Some(sorted)) =
                    (root_table.as_deref_mut(), sorted.as_deref())
                {
                    let code = HuffmanCode {
                        bits: (len - root_bits) as u8,
                        value: sorted[symbol],
                    };
                    symbol += 1;
                    replicate_value(
                        &mut root[table + (key >> root_bits) as usize..],
                        step,
                        table_size,
                        code,
                    );
                }
                key = get_next_key(key, len);
                count[len as usize] -= 1;
            }
            len += 1;
            step <<= 1;
        }

        // Check if tree is full.
        if num_nodes != 2 * offset[MAX_ALLOWED_CODE_LENGTH as usize] - 1 {
            return 0;
        }
    }

    total_size
}

/// Builds Huffman lookup table assuming code lengths are in symbol order.
/// Returns built table size or 0 in case of error (invalid tree or memory
/// error). Translation of `VP8LBuildHuffmanTable()` (the C stack-or-heap
/// `sorted` buffer is a vector).
pub(crate) fn vp8l_build_huffman_table(
    root_table: Option<&mut HuffmanTables>,
    root_bits: i32,
    code_lengths: &[i32],
) -> i32 {
    let total_size = build_huffman_table(None, root_bits, code_lengths, None);
    let Some(root_table) = root_table else {
        return total_size;
    };
    if total_size == 0 {
        return total_size;
    }

    let curr = &root_table.segments[root_table.curr_segment];
    if curr.curr_table + total_size as usize >= curr.start.len() {
        // If 'root_table' does not have enough memory, allocate a new segment.
        // The available part of root_table->curr_segment is left unused because we
        // need a contiguous buffer.
        let segment_size = curr.start.len() as i32;
        // Fill the new segment.
        // We need at least 'total_size' but if that value is small, it is better to
        // allocate a big chunk to prevent more allocations later. 'segment_size' is
        // therefore chosen (any other arbitrary value could be chosen).
        let size = if total_size > segment_size {
            total_size
        } else {
            segment_size
        };
        let Some(start) = safe_alloc(size as u64, HuffmanCode::default()) else {
            return 0;
        };
        // Point to the new segment.
        root_table.segments.push(HuffmanTablesSegment {
            start,
            curr_table: 0,
        });
        root_table.curr_segment = root_table.segments.len() - 1;
    }
    let mut sorted = vec![0u16; code_lengths.len()];
    let curr = &mut root_table.segments[root_table.curr_segment];
    build_huffman_table(
        Some(&mut curr.start[curr.curr_table..]),
        root_bits,
        code_lengths,
        Some(&mut sorted),
    );
    total_size
}

/// Allocates a HuffmanTables with 'size' contiguous HuffmanCodes. Returns
/// false on memory allocation error. Translation of
/// `VP8LHuffmanTablesAllocate()`.
pub(crate) fn vp8l_huffman_tables_allocate(size: i32, huffman_tables: &mut HuffmanTables) -> bool {
    // Have 'segment' point to the first segment for now, 'root'.
    huffman_tables.segments.clear();
    huffman_tables.curr_segment = 0;
    // Allocate root.
    let Some(start) = safe_alloc(size as u64, HuffmanCode::default()) else {
        return false;
    };
    huffman_tables.segments.push(HuffmanTablesSegment {
        start,
        curr_table: 0,
    });
    true
}

/// Translation of `VP8LHuffmanTablesDeallocate()`.
pub(crate) fn vp8l_huffman_tables_deallocate(huffman_tables: &mut HuffmanTables) {
    huffman_tables.segments.clear();
    huffman_tables.curr_segment = 0;
}
