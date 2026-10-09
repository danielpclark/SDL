// Rust translation of lib/jxl/dec_huffman.h and lib/jxl/dec_huffman.cc from
// libjxl (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The prefix code (Huffman) decoder.

use super::ans_common::PREFIX_MAX_BITS;
use super::base::floor_log2_nonzero_u64;
use super::dec_bit_reader::BitReader;
use super::huffman_table::{build_huffman_table, HuffmanCode};

pub(crate) const K_HUFFMAN_TABLE_BITS: usize = 8;

/// Translation of `HuffmanDecodingData`.
#[derive(Clone, Default, Debug)]
pub(crate) struct HuffmanDecodingData {
    pub table: Vec<HuffmanCode>,
}

const K_CODE_LENGTH_CODES: usize = 18;
const K_CODE_LENGTH_CODE_ORDER: [u8; K_CODE_LENGTH_CODES] =
    [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const K_DEFAULT_CODE_LENGTH: u8 = 8;
const K_CODE_LENGTH_REPEAT_CODE: u8 = 16;

/// Translation of `ReadHuffmanCodeLengths()`.
fn read_huffman_code_lengths(
    code_length_code_lengths: &[u8; K_CODE_LENGTH_CODES],
    num_symbols: i32,
    code_lengths: &mut [u8],
    br: &mut BitReader<'_>,
) -> bool {
    let mut symbol: i32 = 0;
    let mut prev_code_len: u8 = K_DEFAULT_CODE_LENGTH;
    let mut repeat: i32 = 0;
    let mut repeat_code_len: u8 = 0;
    let mut space: i32 = 32768;
    let mut table = [HuffmanCode::default(); 32];

    let mut counts = [0u16; 16];
    for i in 0..K_CODE_LENGTH_CODES {
        counts[code_length_code_lengths[i] as usize] += 1;
    }
    if build_huffman_table(
        &mut table,
        5,
        code_length_code_lengths,
        K_CODE_LENGTH_CODES,
        &mut counts,
    ) == 0
    {
        return false;
    }

    while symbol < num_symbols && space > 0 {
        br.refill();
        let p = &table[br.peek_fixed_bits::<5>() as usize];
        br.consume(p.bits as usize);
        let code_len = p.value as u8;
        if code_len < K_CODE_LENGTH_REPEAT_CODE {
            repeat = 0;
            code_lengths[symbol as usize] = code_len;
            symbol += 1;
            if code_len != 0 {
                prev_code_len = code_len;
                space = (space as u32).wrapping_sub(32768u32 >> code_len) as i32;
            }
        } else {
            let extra_bits = code_len as i32 - 14;
            let mut new_len: u8 = 0;
            if code_len == K_CODE_LENGTH_REPEAT_CODE {
                new_len = prev_code_len;
            }
            if repeat_code_len != new_len {
                repeat = 0;
                repeat_code_len = new_len;
            }
            let old_repeat = repeat;
            if repeat > 0 {
                repeat -= 2;
                repeat <<= extra_bits;
            }
            repeat += br.read_bits(extra_bits as usize) as i32 + 3;
            let repeat_delta = repeat - old_repeat;
            if symbol + repeat_delta > num_symbols {
                return false;
            }
            code_lengths[symbol as usize..(symbol + repeat_delta) as usize].fill(repeat_code_len);
            symbol += repeat_delta;
            if repeat_code_len != 0 {
                space -= repeat_delta << (15 - repeat_code_len as i32);
            }
        }
    }
    if space != 0 {
        return false;
    }
    code_lengths[symbol as usize..num_symbols as usize].fill(0);
    true
}

/// Translation of `ReadSimpleCode()`.
#[inline]
fn read_simple_code(
    alphabet_size: usize,
    br: &mut BitReader<'_>,
    table: &mut [HuffmanCode],
) -> bool {
    let max_bits = if alphabet_size > 1 {
        floor_log2_nonzero_u64(alphabet_size as u64 - 1) + 1
    } else {
        0
    };

    let mut num_symbols = br.read_fixed_bits::<2>() as usize + 1;

    let mut symbols = [0u16; 4];
    for i in 0..num_symbols {
        let symbol = br.read_bits(max_bits) as u16;
        if symbol as usize >= alphabet_size {
            return false;
        }
        symbols[i] = symbol;
    }

    for i in 0..num_symbols - 1 {
        for j in i + 1..num_symbols {
            if symbols[i] == symbols[j] {
                return false;
            }
        }
    }

    // 4 symbols have to option to encode.
    if num_symbols == 4 {
        num_symbols += br.read_fixed_bits::<1>() as usize;
    }

    let hc = |bits: u8, value: u16| HuffmanCode { bits, value };

    let mut table_size = 1usize;
    match num_symbols {
        1 => {
            table[0] = hc(0, symbols[0]);
        }
        2 => {
            if symbols[0] > symbols[1] {
                symbols.swap(0, 1);
            }
            table[0] = hc(1, symbols[0]);
            table[1] = hc(1, symbols[1]);
            table_size = 2;
        }
        3 => {
            if symbols[1] > symbols[2] {
                symbols.swap(1, 2);
            }
            table[0] = hc(1, symbols[0]);
            table[2] = hc(1, symbols[0]);
            table[1] = hc(2, symbols[1]);
            table[3] = hc(2, symbols[2]);
            table_size = 4;
        }
        4 => {
            for i in 0..3 {
                for j in i + 1..4 {
                    if symbols[i] > symbols[j] {
                        symbols.swap(i, j);
                    }
                }
            }
            table[0] = hc(2, symbols[0]);
            table[2] = hc(2, symbols[1]);
            table[1] = hc(2, symbols[2]);
            table[3] = hc(2, symbols[3]);
            table_size = 4;
        }
        5 => {
            if symbols[2] > symbols[3] {
                symbols.swap(2, 3);
            }
            table[0] = hc(1, symbols[0]);
            table[1] = hc(2, symbols[1]);
            table[2] = hc(1, symbols[0]);
            table[3] = hc(3, symbols[2]);
            table[4] = hc(1, symbols[0]);
            table[5] = hc(2, symbols[1]);
            table[6] = hc(1, symbols[0]);
            table[7] = hc(3, symbols[3]);
            table_size = 8;
        }
        _ => {
            // Unreachable.
            return false;
        }
    }

    let goal_size = 1usize << K_HUFFMAN_TABLE_BITS;
    while table_size != goal_size {
        table.copy_within(0..table_size, table_size);
        table_size <<= 1;
    }

    true
}

impl HuffmanDecodingData {
    /// Decodes the Huffman code lengths from the bit-stream and fills in the
    /// pre-allocated table with the corresponding 2-level Huffman decoding
    /// table. Returns false if the Huffman code lengths can not de decoded.
    /// Translation of `ReadFromBitStream()`.
    pub(crate) fn read_from_bit_stream(
        &mut self,
        alphabet_size: usize,
        br: &mut BitReader<'_>,
    ) -> bool {
        if alphabet_size > (1 << PREFIX_MAX_BITS) {
            return false;
        }

        /* simple_code_or_skip is used as follows:
        1 for simple code;
        0 for no skipping, 2 skips 2 code lengths, 3 skips 3 code lengths */
        let simple_code_or_skip = br.read_fixed_bits::<2>() as u32;
        if simple_code_or_skip == 1 {
            self.table
                .resize(1 << K_HUFFMAN_TABLE_BITS, HuffmanCode::default());
            return read_simple_code(alphabet_size, br, &mut self.table);
        }

        let mut code_lengths = vec![0u8; alphabet_size];
        let mut code_length_code_lengths = [0u8; K_CODE_LENGTH_CODES];
        let mut space: i32 = 32;
        let mut num_codes = 0;
        /* Static Huffman code for the code length code lengths */
        const HUFF: [(u8, u16); 16] = [
            (2, 0),
            (2, 4),
            (2, 3),
            (3, 2),
            (2, 0),
            (2, 4),
            (2, 3),
            (4, 1),
            (2, 0),
            (2, 4),
            (2, 3),
            (3, 2),
            (2, 0),
            (2, 4),
            (2, 3),
            (4, 5),
        ];
        let mut i = simple_code_or_skip as usize;
        while i < K_CODE_LENGTH_CODES && space > 0 {
            let code_len_idx = K_CODE_LENGTH_CODE_ORDER[i] as usize;
            br.refill();
            let p = HUFF[br.peek_fixed_bits::<4>() as usize];
            br.consume(p.0 as usize);
            let v = p.1 as u8;
            code_length_code_lengths[code_len_idx] = v;
            if v != 0 {
                space = (space as u32).wrapping_sub(32u32 >> v) as i32;
                num_codes += 1;
            }
            i += 1;
        }
        let ok = (num_codes == 1 || space == 0)
            && read_huffman_code_lengths(
                &code_length_code_lengths,
                alphabet_size as i32,
                &mut code_lengths,
                br,
            );

        if !ok {
            return false;
        }
        let mut counts = [0u16; 16];
        for &l in code_lengths.iter() {
            counts[l as usize] += 1;
        }
        self.table
            .resize(alphabet_size + 376, HuffmanCode::default());
        let table_size = build_huffman_table(
            &mut self.table,
            K_HUFFMAN_TABLE_BITS as i32,
            &code_lengths,
            alphabet_size,
            &mut counts,
        );
        self.table.truncate(table_size as usize);
        table_size > 0
    }

    /// Decodes the next Huffman coded symbol from the bit-stream.
    /// Translation of `ReadSymbol()`.
    #[inline]
    pub(crate) fn read_symbol(&self, br: &mut BitReader<'_>) -> u16 {
        let mut table = br.peek_bits(K_HUFFMAN_TABLE_BITS) as usize;
        let mut n_bits = self.table[table].bits as usize;
        if n_bits > K_HUFFMAN_TABLE_BITS {
            br.consume(K_HUFFMAN_TABLE_BITS);
            n_bits -= K_HUFFMAN_TABLE_BITS;
            table += self.table[table].value as usize;
            table += br.peek_bits(n_bits) as usize;
        }
        br.consume(self.table[table].bits as usize);
        self.table[table].value
    }
}
