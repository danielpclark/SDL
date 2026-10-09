// Rust translation of lib/jxl/huffman_table.h and lib/jxl/huffman_table.cc
// from libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The two-level prefix code lookup tables.

use super::ans_common::PREFIX_MAX_BITS;

/// Translation of `HuffmanCode`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct HuffmanCode {
    pub bits: u8,   /* number of bits used for this symbol */
    pub value: u16, /* symbol value or table offset */
}

/* Returns reverse(reverse(key, len) + 1, len), where reverse(key, len) is the
bit-wise reversal of the len least significant bits of key. */
#[inline]
fn get_next_key(key: i32, len: usize) -> i32 {
    let mut step = (1u32 << (len - 1)) as i32;
    while key & step != 0 {
        step >>= 1;
    }
    (key & (step - 1)) + step
}

/* Stores code in table[0], table[step], table[2*step], ..., table[end] */
/* Assumes that end is an integer multiple of step */
#[inline]
fn replicate_value(
    table: &mut [HuffmanCode],
    base: usize,
    step: i32,
    mut end: i32,
    code: HuffmanCode,
) {
    loop {
        end -= step;
        table[base + end as usize] = code;
        if end <= 0 {
            break;
        }
    }
}

/* Returns the table width of the next 2nd level table. count is the histogram
of bit lengths for the remaining symbols, len is the code length of the next
processed symbol */
#[inline]
fn next_table_bit_size(count: &[u16; 16], mut len: usize, root_bits: i32) -> usize {
    let mut left = 1usize << (len - root_bits as usize);
    while len < PREFIX_MAX_BITS {
        if left <= count[len] as usize {
            break;
        }
        left -= count[len] as usize;
        len += 1;
        left <<= 1;
    }
    len - root_bits as usize
}

/* Builds Huffman lookup table assuming code lengths are in symbol order. */
/* Returns 0 in case of error (invalid tree or memory error), otherwise
populated size of table. */
/// Translation of `BuildHuffmanTable()`.
pub(crate) fn build_huffman_table(
    root_table: &mut [HuffmanCode],
    root_bits: i32,
    code_lengths: &[u8],
    code_lengths_size: usize,
    count: &mut [u16; 16],
) -> u32 {
    let mut code = HuffmanCode::default(); /* current table entry */
    let mut table: usize; /* next available space in table */
    let mut len: usize; /* current code length */
    let mut symbol: usize; /* symbol index in original or sorted table */
    let mut key: i32; /* reversed prefix code */
    let mut step: i32; /* step size to replicate values in current table */
    let mut low: i32; /* low bits for current root entry */
    let mask: i32; /* mask for low bits */
    let mut table_bits: usize; /* key length of current table */
    let mut table_size: i32; /* size of current table */
    let mut total_size: i32; /* sum of root table size and 2nd level table sizes */
    /* offsets in sorted table for each length */
    let mut offset = [0u16; PREFIX_MAX_BITS + 1];
    let mut max_length: usize = 1;

    if code_lengths_size > 1usize << PREFIX_MAX_BITS {
        return 0;
    }

    /* symbols sorted by code length */
    let mut sorted = vec![0u16; code_lengths_size];

    /* generate offsets into sorted symbol table by code length */
    {
        let mut sum: u16 = 0;
        len = 1;
        while len <= PREFIX_MAX_BITS {
            offset[len] = sum;
            if count[len] != 0 {
                sum = sum.wrapping_add(count[len]);
                max_length = len;
            }
            len += 1;
        }
    }

    /* sort symbols by length, by symbol order within each length */
    for symbol in 0..code_lengths_size {
        if code_lengths[symbol] != 0 {
            let l = code_lengths[symbol] as usize;
            sorted[offset[l] as usize] = symbol as u16;
            offset[l] = offset[l].wrapping_add(1);
        }
    }

    table = 0;
    table_bits = root_bits as usize;
    table_size = 1 << table_bits;
    total_size = table_size;

    /* special case code with only one value */
    if offset[PREFIX_MAX_BITS] == 1 {
        code.bits = 0;
        code.value = sorted[0];
        for k in 0..total_size {
            root_table[k as usize] = code;
        }
        return total_size as u32;
    }

    /* fill in root table */
    /* let's reduce the table size to a smaller size if possible, and */
    /* create the repetitions by memcpy if possible in the coming loop */
    if table_bits > max_length {
        table_bits = max_length;
        table_size = 1 << table_bits;
    }
    key = 0;
    symbol = 0;
    code.bits = 1;
    step = 2;
    loop {
        while count[code.bits as usize] != 0 {
            code.value = sorted[symbol];
            symbol += 1;
            replicate_value(root_table, table + key as usize, step, table_size, code);
            key = get_next_key(key, code.bits as usize);
            count[code.bits as usize] -= 1;
        }
        step <<= 1;
        code.bits += 1;
        if code.bits as usize > table_bits {
            break;
        }
    }

    /* if root_bits != table_bits we only created one fraction of the */
    /* table, and we need to replicate it now. */
    while total_size != table_size {
        let n = table_size as usize;
        root_table.copy_within(table..table + n, table + n);
        table_size <<= 1;
    }

    /* fill in 2nd level tables and add pointers to root table */
    mask = total_size - 1;
    low = -1;
    len = root_bits as usize + 1;
    step = 2;
    while len <= max_length {
        while count[len] != 0 {
            if (key & mask) != low {
                table += table_size as usize;
                table_bits = next_table_bit_size(count, len, root_bits);
                table_size = 1 << table_bits;
                total_size += table_size;
                low = key & mask;
                root_table[low as usize].bits = (table_bits + root_bits as usize) as u8;
                root_table[low as usize].value = (table as i32 - low) as u16;
            }
            code.bits = (len - root_bits as usize) as u8;
            code.value = sorted[symbol];
            symbol += 1;
            replicate_value(
                root_table,
                table + (key >> root_bits) as usize,
                step,
                table_size,
                code,
            );
            key = get_next_key(key, len);
            count[len] -= 1;
        }
        len += 1;
        step <<= 1;
    }

    total_size as u32
}
