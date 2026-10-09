// Rust translation of lib/jxl/dec_ans.h and lib/jxl/dec_ans.cc from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Library to decode the ANS population counts from the bit-stream and build
//! a decoding table from them, and the symbol reader (ANS or prefix codes,
//! with the hybrid integer configurations and LZ77).

use super::ans_common::{
    alias_lookup, create_flat_histogram, get_population_count_precision, init_alias_table,
    AliasEntry, ANS_LOG_TAB_SIZE, ANS_MAX_ALPHABET_SIZE, ANS_SIGNATURE, ANS_TAB_SIZE,
    PREFIX_MAX_BITS,
};
use super::base::{
    ceil_log2_nonzero_u64, floor_log2_nonzero_u32, jxl_failure, jxl_status, Status, StatusCode,
};
use super::dec_bit_reader::BitReader;
use super::dec_context_map::decode_context_map;
use super::dec_huffman::{HuffmanDecodingData, K_HUFFMAN_TABLE_BITS};
use super::fields::{bits_offset, bundle_init, bundle_read, val, Fields, Visitor};

// Experiments show that best performance is typically achieved for a
// split-exponent of 3 or 4. Trend seems to be that '4' is better
// for large-ish pictures, and '3' better for rather small-ish pictures.
// This is plausible - the more special symbols we have, the better
// statistics we need to get a benefit out of them.

// Our hybrid-encoding scheme has dedicated tokens for the smallest
// (1 << split_exponents) numbers, and for the rest
// encodes (number of bits) + (msb_in_token sub-leading binary digits) +
// (lsb_in_token lowest binary digits) in the token, with the remaining bits
// then being encoded as data.
// (See upstream's dec_ans.h for an example.)
/// Translation of `HybridUintConfig`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HybridUintConfig {
    pub split_exponent: u32,
    pub split_token: u32,
    pub msb_in_token: u32,
    pub lsb_in_token: u32,
}

impl HybridUintConfig {
    pub(crate) fn new(split_exponent: u32, msb_in_token: u32, lsb_in_token: u32) -> Self {
        HybridUintConfig {
            split_exponent,
            split_token: 1u32.wrapping_shl(split_exponent),
            msb_in_token,
            lsb_in_token,
        }
    }
}

impl Default for HybridUintConfig {
    fn default() -> Self {
        HybridUintConfig::new(4, 2, 0)
    }
}

/// Translation of `LZ77Params`.
#[derive(Clone, Debug)]
pub(crate) struct Lz77Params {
    pub enabled: bool,

    // Symbols above min_symbol use a special hybrid uint encoding and
    // represent a length, to be added to min_length.
    pub min_symbol: u32,
    pub min_length: u32,

    // Not serialized by VisitFields.
    pub length_uint_config: HybridUintConfig,

    pub nonserialized_distance_context: usize,
}

impl Default for Lz77Params {
    fn default() -> Self {
        let mut s = Lz77Params {
            enabled: false,
            min_symbol: 0,
            min_length: 0,
            length_uint_config: HybridUintConfig::new(0, 0, 0),
            nonserialized_distance_context: 0,
        };
        bundle_init(&mut s);
        s
    }
}

impl Fields for Lz77Params {
    fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
        visitor.bool_(false, &mut self.enabled)?;
        if !visitor.conditional(self.enabled) {
            return Ok(());
        }
        visitor.u32d(
            val(224),
            val(512),
            val(4096),
            bits_offset(15, 8),
            224,
            &mut self.min_symbol,
        )?;
        visitor.u32d(
            val(3),
            val(4),
            bits_offset(2, 5),
            bits_offset(8, 9),
            3,
            &mut self.min_length,
        )?;
        Ok(())
    }
}

pub(crate) const K_WINDOW_SIZE: usize = 1 << 20;
pub(crate) const K_NUM_SPECIAL_DISTANCES: usize = 120;
// Table of special distance codes from WebP lossless.
pub(crate) const K_SPECIAL_DISTANCES: [[i8; 2]; K_NUM_SPECIAL_DISTANCES] = [
    [0, 1],
    [1, 0],
    [1, 1],
    [-1, 1],
    [0, 2],
    [2, 0],
    [1, 2],
    [-1, 2],
    [2, 1],
    [-2, 1],
    [2, 2],
    [-2, 2],
    [0, 3],
    [3, 0],
    [1, 3],
    [-1, 3],
    [3, 1],
    [-3, 1],
    [2, 3],
    [-2, 3],
    [3, 2],
    [-3, 2],
    [0, 4],
    [4, 0],
    [1, 4],
    [-1, 4],
    [4, 1],
    [-4, 1],
    [3, 3],
    [-3, 3],
    [2, 4],
    [-2, 4],
    [4, 2],
    [-4, 2],
    [0, 5],
    [3, 4],
    [-3, 4],
    [4, 3],
    [-4, 3],
    [5, 0],
    [1, 5],
    [-1, 5],
    [5, 1],
    [-5, 1],
    [2, 5],
    [-2, 5],
    [5, 2],
    [-5, 2],
    [4, 4],
    [-4, 4],
    [3, 5],
    [-3, 5],
    [5, 3],
    [-5, 3],
    [0, 6],
    [6, 0],
    [1, 6],
    [-1, 6],
    [6, 1],
    [-6, 1],
    [2, 6],
    [-2, 6],
    [6, 2],
    [-6, 2],
    [4, 5],
    [-4, 5],
    [5, 4],
    [-5, 4],
    [3, 6],
    [-3, 6],
    [6, 3],
    [-6, 3],
    [0, 7],
    [7, 0],
    [1, 7],
    [-1, 7],
    [5, 5],
    [-5, 5],
    [7, 1],
    [-7, 1],
    [4, 6],
    [-4, 6],
    [6, 4],
    [-6, 4],
    [2, 7],
    [-2, 7],
    [7, 2],
    [-7, 2],
    [3, 7],
    [-3, 7],
    [7, 3],
    [-7, 3],
    [5, 6],
    [-5, 6],
    [6, 5],
    [-6, 5],
    [8, 0],
    [4, 7],
    [-4, 7],
    [7, 4],
    [-7, 4],
    [8, 1],
    [8, 2],
    [6, 6],
    [-6, 6],
    [8, 3],
    [5, 7],
    [-5, 7],
    [7, 5],
    [-7, 5],
    [8, 4],
    [6, 7],
    [-6, 7],
    [7, 6],
    [-7, 6],
    [8, 5],
    [7, 7],
    [-7, 7],
    [8, 6],
    [8, 7],
];

/// Translation of `ANSCode`.
#[derive(Clone, Default, Debug)]
pub(crate) struct AnsCode {
    pub alias_tables: Vec<AliasEntry>,
    pub huffman_data: Vec<HuffmanDecodingData>,
    pub uint_config: Vec<HybridUintConfig>,
    pub degenerate_symbols: Vec<i32>,
    pub use_prefix_code: bool,
    pub log_alpha_size: u8, // for ANS.
    pub lz77: Lz77Params,
    // Maximum number of bits necessary to represent the result of a
    // ReadHybridUint call done with this ANSCode.
    pub max_num_bits: usize,
}

impl AnsCode {
    /// Translation of `ANSCode::UpdateMaxNumBits()`.
    fn update_max_num_bits(&mut self, ctx: usize, mut symbol: usize) {
        let mut cfg = &self.uint_config[ctx];
        // LZ77 symbols use a different uint config.
        if self.lz77.enabled
            && self.lz77.nonserialized_distance_context != ctx
            && symbol >= self.lz77.min_symbol as usize
        {
            symbol -= self.lz77.min_symbol as usize;
            cfg = &self.lz77.length_uint_config;
        }
        let split_token = cfg.split_token as usize;
        let msb_in_token = cfg.msb_in_token as usize;
        let lsb_in_token = cfg.lsb_in_token as usize;
        let split_exponent = cfg.split_exponent as usize;
        if symbol < split_token {
            self.max_num_bits = self.max_num_bits.max(split_exponent);
            return;
        }
        let n_extra_bits = (split_exponent as u32)
            .wrapping_sub((msb_in_token + lsb_in_token) as u32)
            .wrapping_add(((symbol - split_token) >> (msb_in_token + lsb_in_token)) as u32);
        let total_bits = msb_in_token + lsb_in_token + n_extra_bits as usize + 1;
        self.max_num_bits = self.max_num_bits.max(total_bits);
    }
}

/// Translation of `ANSSymbolReader::Checkpoint`.
#[allow(dead_code)]
pub(crate) struct Checkpoint {
    state: u32,
    num_to_copy: u32,
    copy_pos: u32,
    num_decoded: u32,
    lz77_window: Vec<u32>,
}

/// Translation of `ANSSymbolReader`.
pub(crate) struct AnsSymbolReader<'c> {
    alias_tables: &'c [AliasEntry], // not owned
    huffman_data: &'c [HuffmanDecodingData],
    use_prefix_code: bool,
    state: u32,
    configs: &'c [HybridUintConfig],
    log_alpha_size: u32,
    log_entry_size: u32,
    entry_size_minus_1: u32,

    // LZ77 structures and constants.
    lz77_window: Option<Vec<u32>>,
    num_decoded: u32,
    num_to_copy: u32,
    copy_pos: u32,
    lz77_ctx: u32,
    lz77_min_length: u32,
    lz77_threshold: u32, // bigger than any symbol.
    lz77_length_uint: HybridUintConfig,
    special_distances: [u32; K_NUM_SPECIAL_DISTANCES],
    num_special_distances: u32,
}

const K_WINDOW_MASK: usize = K_WINDOW_SIZE - 1;

impl<'c> AnsSymbolReader<'c> {
    /// An invalid symbol reader, to be overwritten. Translation of the
    /// default constructor.
    #[allow(dead_code)]
    pub(crate) fn empty() -> AnsSymbolReader<'static> {
        AnsSymbolReader {
            alias_tables: &[],
            huffman_data: &[],
            use_prefix_code: false,
            state: ANS_SIGNATURE << 16,
            configs: &[],
            log_alpha_size: 0,
            log_entry_size: 0,
            entry_size_minus_1: 0,
            lz77_window: None,
            num_decoded: 0,
            num_to_copy: 0,
            copy_pos: 0,
            lz77_ctx: 0,
            lz77_min_length: 0,
            lz77_threshold: 1 << 20,
            lz77_length_uint: HybridUintConfig::default(),
            special_distances: [0; K_NUM_SPECIAL_DISTANCES],
            num_special_distances: 0,
        }
    }

    pub(crate) fn new(code: &'c AnsCode, br: &mut BitReader<'_>, distance_multiplier: usize) -> Self {
        let mut r = AnsSymbolReader {
            alias_tables: &code.alias_tables,
            huffman_data: &code.huffman_data,
            use_prefix_code: code.use_prefix_code,
            state: ANS_SIGNATURE << 16,
            configs: &code.uint_config,
            log_alpha_size: 0,
            log_entry_size: 0,
            entry_size_minus_1: 0,
            lz77_window: None,
            num_decoded: 0,
            num_to_copy: 0,
            copy_pos: 0,
            lz77_ctx: 0,
            lz77_min_length: 0,
            lz77_threshold: 1 << 20,
            lz77_length_uint: HybridUintConfig::default(),
            special_distances: [0; K_NUM_SPECIAL_DISTANCES],
            num_special_distances: 0,
        };
        if !r.use_prefix_code {
            r.state = br.read_fixed_bits::<32>() as u32;
            r.log_alpha_size = code.log_alpha_size as u32;
            r.log_entry_size = ANS_LOG_TAB_SIZE.wrapping_sub(code.log_alpha_size as u32);
            r.entry_size_minus_1 = (1u32.wrapping_shl(r.log_entry_size)).wrapping_sub(1);
        } else {
            r.state = ANS_SIGNATURE << 16;
        }
        if !code.lz77.enabled {
            return r;
        }
        // a std::vector incurs unacceptable decoding speed loss because of
        // initialization.
        // (The window is zeroed here: upstream never reads its uninitialized
        // entries.)
        r.lz77_window = Some(vec![0u32; K_WINDOW_SIZE]);
        r.lz77_ctx = code.lz77.nonserialized_distance_context as u32;
        r.lz77_length_uint = code.lz77.length_uint_config;
        r.lz77_threshold = code.lz77.min_symbol;
        r.lz77_min_length = code.lz77.min_length;
        r.num_special_distances = if distance_multiplier == 0 {
            0
        } else {
            K_NUM_SPECIAL_DISTANCES as u32
        };
        for i in 0..r.num_special_distances as usize {
            let mut dist = K_SPECIAL_DISTANCES[i][0] as i32;
            dist = dist.wrapping_add(
                (distance_multiplier as i32).wrapping_mul(K_SPECIAL_DISTANCES[i][1] as i32),
            );
            if dist < 1 {
                dist = 1;
            }
            r.special_distances[i] = dist as u32;
        }
        r
    }

    #[inline]
    pub(crate) fn read_symbol_ans_without_refill(&mut self, histo_idx: usize, br: &mut BitReader<'_>) -> usize {
        let res = self.state & (ANS_TAB_SIZE - 1);

        let table = &self.alias_tables[histo_idx << self.log_alpha_size..];
        let symbol = alias_lookup(
            table,
            res as usize,
            self.log_entry_size as usize,
            self.entry_size_minus_1 as usize,
        );
        self.state = (symbol
            .freq
            .wrapping_mul((self.state >> ANS_LOG_TAB_SIZE) as usize)
            .wrapping_add(symbol.offset)) as u32;

        // Branchless version is about equally fast on SKX.
        let new_state = (self.state << 16) | br.peek_fixed_bits::<16>() as u32;
        let normalize = self.state < (1u32 << 16);
        self.state = if normalize { new_state } else { self.state };
        br.consume(if normalize { 16 } else { 0 });

        symbol.value
    }

    #[inline]
    pub(crate) fn read_symbol_huff_without_refill(&self, histo_idx: usize, br: &mut BitReader<'_>) -> usize {
        self.huffman_data[histo_idx].read_symbol(br) as usize
    }

    #[inline]
    pub(crate) fn read_symbol_without_refill(&mut self, histo_idx: usize, br: &mut BitReader<'_>) -> usize {
        // TODO(veluca): hoist if in hotter loops.
        if self.use_prefix_code {
            return self.read_symbol_huff_without_refill(histo_idx, br);
        }
        self.read_symbol_ans_without_refill(histo_idx, br)
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) fn read_symbol(&mut self, histo_idx: usize, br: &mut BitReader<'_>) -> usize {
        br.refill();
        self.read_symbol_without_refill(histo_idx, br)
    }

    pub(crate) fn check_ans_final_state(&self) -> bool {
        self.state == (ANS_SIGNATURE << 16)
    }

    /// Translation of `ReadHybridUintConfig()`.
    #[inline]
    pub(crate) fn read_hybrid_uint_config(
        config: &HybridUintConfig,
        mut token: usize,
        br: &mut BitReader<'_>,
    ) -> u32 {
        let split_token = config.split_token as usize;
        let msb_in_token = config.msb_in_token as usize;
        let lsb_in_token = config.lsb_in_token as usize;
        let split_exponent = config.split_exponent as usize;
        // Fast-track version of hybrid integer decoding.
        if token < split_token {
            return token as u32;
        }
        let mut nbits: u32 = (split_exponent as u32)
            .wrapping_sub((msb_in_token + lsb_in_token) as u32)
            .wrapping_add(((token - split_token) >> (msb_in_token + lsb_in_token)) as u32);
        // Max amount of bits for ReadBits is 32 and max valid left shift is 29
        // bits. However, for speed no error is propagated here, instead limit the
        // nbits size. If nbits > 29, the code stream is invalid, but no error is
        // returned.
        // Note that in most cases we will emit an error if the histogram allows
        // representing numbers that would cause invalid shifts, but we need to
        // keep this check as when LZ77 is enabled it might make sense to have an
        // histogram that could in principle cause invalid shifts.
        nbits &= 31;
        let low = (token & ((1usize << lsb_in_token) - 1)) as u32;
        token >>= lsb_in_token;
        let bits = br.peek_bits(nbits as usize) as usize;
        br.consume(nbits as usize);
        let ret = (((((1usize << msb_in_token) | (token & ((1usize << msb_in_token) - 1)))
            << nbits)
            | bits)
            << lsb_in_token)
            | low as usize;
        // TODO(eustas): mark BitReader as unhealthy if nbits > 29 or ret does not
        //               fit uint32_t
        ret as u32
    }

    /// Takes a *clustered* idx. Can only use if HuffRleOnly() is true.
    /// Translation of `ReadHybridUintClusteredHuffRleOnly()`.
    pub(crate) fn read_hybrid_uint_clustered_huff_rle_only(
        &mut self,
        ctx: usize,
        br: &mut BitReader<'_>,
        value: &mut u32,
        run: &mut u32,
    ) {
        br.refill(); // covers ReadSymbolWithoutRefill + PeekBits
        let token = self.read_symbol_huff_without_refill(ctx, br);
        if token >= self.lz77_threshold as usize {
            *run = Self::read_hybrid_uint_config(
                &self.lz77_length_uint,
                token - self.lz77_threshold as usize,
                br,
            )
            .wrapping_add(self.lz77_min_length)
            .wrapping_sub(1);
            return;
        }
        *value = Self::read_hybrid_uint_config(&self.configs[ctx], token, br);
    }

    /// Translation of `HuffRleOnly()`.
    pub(crate) fn huff_rle_only(&self) -> bool {
        if self.lz77_window.is_none() {
            return false;
        }
        if !self.use_prefix_code {
            return false;
        }
        for i in 0..K_HUFFMAN_TABLE_BITS {
            let t = &self.huffman_data[self.lz77_ctx as usize].table;
            if t[i].bits != 0 {
                return false;
            }
            if t[i].value != 1 {
                return false;
            }
        }
        if self.configs[self.lz77_ctx as usize].split_token > 1 {
            return false;
        }
        true
    }

    /// Takes a *clustered* idx. Translation of `ReadHybridUintClustered()`
    /// (its tail call written as a loop).
    pub(crate) fn read_hybrid_uint_clustered(&mut self, ctx: usize, br: &mut BitReader<'_>) -> usize {
        loop {
            if self.num_to_copy > 0 {
                let window = self.lz77_window.as_mut().unwrap();
                let ret = window[(self.copy_pos as usize) & K_WINDOW_MASK];
                self.copy_pos = self.copy_pos.wrapping_add(1);
                self.num_to_copy -= 1;
                window[(self.num_decoded as usize) & K_WINDOW_MASK] = ret;
                self.num_decoded = self.num_decoded.wrapping_add(1);
                return ret as usize;
            }
            br.refill(); // covers ReadSymbolWithoutRefill + PeekBits
            let token = self.read_symbol_without_refill(ctx, br);
            if token >= self.lz77_threshold as usize {
                self.num_to_copy = Self::read_hybrid_uint_config(
                    &self.lz77_length_uint,
                    token - self.lz77_threshold as usize,
                    br,
                )
                .wrapping_add(self.lz77_min_length);
                br.refill(); // covers ReadSymbolWithoutRefill + PeekBits
                // Distance code.
                let token = self.read_symbol_without_refill(self.lz77_ctx as usize, br);
                let mut distance = Self::read_hybrid_uint_config(
                    &self.configs[self.lz77_ctx as usize],
                    token,
                    br,
                ) as usize;
                if distance < self.num_special_distances as usize {
                    distance = self.special_distances[distance] as usize;
                } else {
                    distance = distance + 1 - self.num_special_distances as usize;
                }
                if distance > self.num_decoded as usize {
                    distance = self.num_decoded as usize;
                }
                if distance > K_WINDOW_SIZE {
                    distance = K_WINDOW_SIZE;
                }
                self.copy_pos = self.num_decoded.wrapping_sub(distance as u32);
                if distance == 0 {
                    // distance 0 -> num_decoded_ == copy_pos_ == 0
                    let to_fill = (self.num_to_copy as usize).min(K_WINDOW_SIZE);
                    if let Some(window) = self.lz77_window.as_mut() {
                        window[..to_fill].fill(0);
                    }
                }
                // TODO(eustas): overflow; mark BitReader as unhealthy
                if self.num_to_copy < self.lz77_min_length {
                    return 0;
                }
                continue; // will trigger a copy.
            }
            let ret = Self::read_hybrid_uint_config(&self.configs[ctx], token, br) as usize;
            if let Some(window) = self.lz77_window.as_mut() {
                window[(self.num_decoded as usize) & K_WINDOW_MASK] = ret as u32;
                self.num_decoded = self.num_decoded.wrapping_add(1);
            }
            return ret;
        }
    }

    /// Translation of `ReadHybridUint()`.
    #[inline]
    pub(crate) fn read_hybrid_uint(&mut self, ctx: usize, br: &mut BitReader<'_>, context_map: &[u8]) -> usize {
        self.read_hybrid_uint_clustered(context_map[ctx] as usize, br)
    }

    /// ctx is a *clustered* context!
    /// This function will modify the ANS state as if `count` symbols have
    /// been decoded. Translation of `IsSingleValueAndAdvance()`.
    pub(crate) fn is_single_value_and_advance(&mut self, ctx: usize, value: &mut u32, count: usize) -> bool {
        // TODO(veluca): No optimization for Huffman mode yet.
        if self.use_prefix_code {
            return false;
        }
        // TODO(eustas): propagate "degenerate_symbol" to simplify this method.
        let res = self.state & (ANS_TAB_SIZE - 1);
        let table = &self.alias_tables[ctx << self.log_alpha_size..];
        let symbol = alias_lookup(
            table,
            res as usize,
            self.log_entry_size as usize,
            self.entry_size_minus_1 as usize,
        );
        if symbol.freq != ANS_TAB_SIZE as usize {
            return false;
        }
        if self.configs[ctx].split_token as usize <= symbol.value {
            return false;
        }
        if symbol.value >= self.lz77_threshold as usize {
            return false;
        }
        *value = symbol.value as u32;
        if let Some(window) = self.lz77_window.as_mut() {
            for _ in 0..count {
                window[(self.num_decoded as usize) & K_WINDOW_MASK] = symbol.value as u32;
                self.num_decoded = self.num_decoded.wrapping_add(1);
            }
        }
        true
    }

    pub(crate) const K_MAX_CHECKPOINT_INTERVAL: usize = 512;

    /// Translation of `Save()`.
    #[allow(dead_code)]
    pub(crate) fn save(&self, checkpoint: &mut Checkpoint) {
        checkpoint.state = self.state;
        checkpoint.num_decoded = self.num_decoded;
        checkpoint.num_to_copy = self.num_to_copy;
        checkpoint.copy_pos = self.copy_pos;
        if let Some(window) = &self.lz77_window {
            checkpoint
                .lz77_window
                .resize(Self::K_MAX_CHECKPOINT_INTERVAL, 0);
            let win_start = (self.num_decoded as usize) & K_WINDOW_MASK;
            let win_end =
                (self.num_decoded as usize + Self::K_MAX_CHECKPOINT_INTERVAL) & K_WINDOW_MASK;
            if win_end > win_start {
                checkpoint.lz77_window[..win_end - win_start]
                    .copy_from_slice(&window[win_start..win_end]);
            } else {
                let n = K_WINDOW_SIZE - win_start;
                checkpoint.lz77_window[..n].copy_from_slice(&window[win_start..]);
                checkpoint.lz77_window[n..n + win_end].copy_from_slice(&window[..win_end]);
            }
        }
    }

    /// Translation of `Restore()`.
    #[allow(dead_code)]
    pub(crate) fn restore(&mut self, checkpoint: &Checkpoint) {
        self.state = checkpoint.state;
        self.num_decoded = checkpoint.num_decoded;
        self.num_to_copy = checkpoint.num_to_copy;
        self.copy_pos = checkpoint.copy_pos;
        if let Some(window) = self.lz77_window.as_mut() {
            let win_start = (self.num_decoded as usize) & K_WINDOW_MASK;
            let win_end =
                (self.num_decoded as usize + Self::K_MAX_CHECKPOINT_INTERVAL) & K_WINDOW_MASK;
            if win_end > win_start {
                window[win_start..win_end]
                    .copy_from_slice(&checkpoint.lz77_window[..win_end - win_start]);
            } else {
                let n = K_WINDOW_SIZE - win_start;
                window[win_start..].copy_from_slice(&checkpoint.lz77_window[..n]);
                window[..win_end].copy_from_slice(&checkpoint.lz77_window[n..n + win_end]);
            }
        }
    }
}

// Decodes a number in the range [0..255], by reading 1 - 11 bits.
#[inline]
fn decode_var_len_uint8(input: &mut BitReader<'_>) -> i32 {
    if input.read_fixed_bits::<1>() != 0 {
        let nbits = input.read_fixed_bits::<3>() as i32;
        if nbits == 0 {
            1
        } else {
            input.read_bits(nbits as usize) as i32 + (1 << nbits)
        }
    } else {
        0
    }
}

// Decodes a number in the range [0..65535], by reading 1 - 21 bits.
#[inline]
fn decode_var_len_uint16(input: &mut BitReader<'_>) -> i32 {
    if input.read_fixed_bits::<1>() != 0 {
        let nbits = input.read_fixed_bits::<4>() as i32;
        if nbits == 0 {
            1
        } else {
            input.read_bits(nbits as usize) as i32 + (1 << nbits)
        }
    } else {
        0
    }
}

/// Translation of `ReadHistogram()`.
fn read_histogram(precision_bits: i32, counts: &mut Vec<i32>, input: &mut BitReader<'_>) -> Status {
    let simple_code = input.read_bits(1) as i32;
    if simple_code == 1 {
        let mut symbols = [0i32; 2];
        let mut max_symbol = 0;
        let num_symbols = input.read_bits(1) as i32 + 1;
        for i in 0..num_symbols as usize {
            symbols[i] = decode_var_len_uint8(input);
            if symbols[i] > max_symbol {
                max_symbol = symbols[i];
            }
        }
        counts.resize(max_symbol as usize + 1, 0);
        if num_symbols == 1 {
            counts[symbols[0] as usize] = 1 << precision_bits;
        } else {
            if symbols[0] == symbols[1] {
                // corrupt data
                return jxl_failure!("corrupt data");
            }
            counts[symbols[0] as usize] = input.read_bits(precision_bits as usize) as i32;
            counts[symbols[1] as usize] = (1 << precision_bits) - counts[symbols[0] as usize];
        }
    } else {
        let is_flat = input.read_bits(1) as i32;
        if is_flat == 1 {
            let alphabet_size = decode_var_len_uint8(input) + 1;
            *counts = create_flat_histogram(alphabet_size, 1 << precision_bits);
            return Ok(());
        }

        let shift: u32;
        {
            // TODO(veluca): speed up reading with table lookups.
            let upper_bound_log = floor_log2_nonzero_u32(ANS_LOG_TAB_SIZE + 1) as i32;
            let mut log = 0;
            while log < upper_bound_log {
                if input.read_fixed_bits::<1>() == 0 {
                    break;
                }
                log += 1;
            }
            shift = ((input.read_bits(log as usize) as u32) | (1u32 << log)).wrapping_sub(1);
            if shift > ANS_LOG_TAB_SIZE + 1 {
                return jxl_failure!("Invalid shift value");
            }
        }

        let length = decode_var_len_uint8(input) + 3;
        counts.resize(length as usize, 0);
        let mut total_count: i32 = 0;

        const HUFF: [[u8; 2]; 128] = [
            [3, 10],
            [7, 12],
            [3, 7],
            [4, 3],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 5],
            [3, 10],
            [4, 4],
            [3, 7],
            [4, 1],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 2],
            [3, 10],
            [5, 0],
            [3, 7],
            [4, 3],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 5],
            [3, 10],
            [4, 4],
            [3, 7],
            [4, 1],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 2],
            [3, 10],
            [6, 11],
            [3, 7],
            [4, 3],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 5],
            [3, 10],
            [4, 4],
            [3, 7],
            [4, 1],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 2],
            [3, 10],
            [5, 0],
            [3, 7],
            [4, 3],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 5],
            [3, 10],
            [4, 4],
            [3, 7],
            [4, 1],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 2],
            [3, 10],
            [7, 13],
            [3, 7],
            [4, 3],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 5],
            [3, 10],
            [4, 4],
            [3, 7],
            [4, 1],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 2],
            [3, 10],
            [5, 0],
            [3, 7],
            [4, 3],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 5],
            [3, 10],
            [4, 4],
            [3, 7],
            [4, 1],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 2],
            [3, 10],
            [6, 11],
            [3, 7],
            [4, 3],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 5],
            [3, 10],
            [4, 4],
            [3, 7],
            [4, 1],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 2],
            [3, 10],
            [5, 0],
            [3, 7],
            [4, 3],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 5],
            [3, 10],
            [4, 4],
            [3, 7],
            [4, 1],
            [3, 6],
            [3, 8],
            [3, 9],
            [4, 2],
        ];

        let n = counts.len();
        let mut logcounts = vec![0i32; n];
        let mut omit_log: i32 = -1;
        let mut omit_pos: i32 = -1;
        // This array remembers which symbols have an RLE length.
        let mut same = vec![0i32; n];
        let mut i = 0usize;
        while i < n {
            input.refill(); // for PeekFixedBits + Advance
            let idx = input.peek_fixed_bits::<7>() as usize;
            input.consume(HUFF[idx][0] as usize);
            logcounts[i] = HUFF[idx][1] as i32;
            // The RLE symbol.
            if logcounts[i] == ANS_LOG_TAB_SIZE as i32 + 1 {
                let rle_length = decode_var_len_uint8(input);
                same[i] = rle_length + 5;
                i += rle_length as usize + 3;
                i += 1;
                continue;
            }
            if logcounts[i] > omit_log {
                omit_log = logcounts[i];
                omit_pos = i as i32;
            }
            i += 1;
        }
        // Invalid input, e.g. due to invalid usage of RLE.
        if omit_pos < 0 {
            return jxl_failure!("Invalid histogram.");
        }
        // FIXME (upstream): this compares with ANS_TAB_SIZE + 1 where the RLE
        // symbol is ANS_LOG_TAB_SIZE + 1, so it never matches.
        if (omit_pos as usize) + 1 < n && logcounts[omit_pos as usize + 1] == ANS_TAB_SIZE as i32 + 1
        {
            return jxl_failure!("Invalid histogram.");
        }
        let mut prev: i32 = 0;
        let mut numsame: i32 = 0;
        for i in 0..n {
            if same[i] != 0 {
                // RLE sequence, let this loop output the same count for the next
                // iterations.
                numsame = same[i] - 1;
                prev = if i > 0 { counts[i - 1] } else { 0 };
            }
            if numsame > 0 {
                counts[i] = prev;
                numsame -= 1;
            } else {
                let code = logcounts[i];
                // omit_pos may not be negative at this point (checked before).
                if i == omit_pos as usize || code == 0 {
                    continue;
                } else if code == 1 {
                    counts[i] = 1;
                } else {
                    let bitcount = get_population_count_precision((code - 1) as u32, shift) as i32;
                    counts[i] = (1 << (code - 1))
                        + ((input.read_bits(bitcount as usize) as i32) << (code - 1 - bitcount));
                }
            }
            total_count = total_count.wrapping_add(counts[i]);
        }
        counts[omit_pos as usize] = (1 << precision_bits) - total_count;
        if counts[omit_pos as usize] <= 0 {
            // The histogram we've read sums to more than total_count (including at
            // least 1 for the omitted value).
            return jxl_failure!("Invalid histogram count.");
        }
    }
    Ok(())
}

/// Translation of `DecodeANSCodes()`.
fn decode_ans_codes(
    num_histograms: usize,
    max_alphabet_size: usize,
    input: &mut BitReader<'_>,
    result: &mut AnsCode,
) -> Status {
    result.degenerate_symbols.resize(num_histograms, -1);
    if result.use_prefix_code {
        debug_assert!(max_alphabet_size <= 1 << PREFIX_MAX_BITS);
        result
            .huffman_data
            .resize(num_histograms, HuffmanDecodingData::default());
        let mut alphabet_sizes = vec![0u16; num_histograms];
        for c in 0..num_histograms {
            alphabet_sizes[c] = (decode_var_len_uint16(input) + 1) as u16;
            if alphabet_sizes[c] as usize > max_alphabet_size {
                return jxl_failure!("Alphabet size is too long");
            }
        }
        for c in 0..num_histograms {
            if alphabet_sizes[c] > 1 {
                if !result.huffman_data[c].read_from_bit_stream(alphabet_sizes[c] as usize, input) {
                    if !input.all_reads_within_bounds() {
                        return jxl_status!(
                            StatusCode::NotEnoughBytes,
                            "Not enough bytes for huffman code"
                        );
                    }
                    return jxl_failure!("Invalid huffman tree number");
                }
            } else {
                // 0-bit codes does not require extension tables.
                result.huffman_data[c].table.clear();
                result.huffman_data[c]
                    .table
                    .resize(1 << K_HUFFMAN_TABLE_BITS, Default::default());
            }
            for k in 0..result.huffman_data[c].table.len() {
                let h = result.huffman_data[c].table[k];
                if h.bits as usize <= K_HUFFMAN_TABLE_BITS {
                    result.update_max_num_bits(c, h.value as usize);
                }
            }
        }
    } else {
        debug_assert!(max_alphabet_size <= ANS_MAX_ALPHABET_SIZE);
        let per = 1usize << result.log_alpha_size;
        result.alias_tables = vec![AliasEntry::default(); num_histograms * per];
        for c in 0..num_histograms {
            let mut counts: Vec<i32> = Vec::new();
            if read_histogram(ANS_LOG_TAB_SIZE as i32, &mut counts, input).is_err() {
                return jxl_failure!("Invalid histogram bitstream.");
            }
            if counts.len() > max_alphabet_size {
                return jxl_failure!("Alphabet size is too long");
            }
            while counts.last() == Some(&0) {
                counts.pop();
            }
            for s in 0..counts.len() {
                if counts[s] != 0 {
                    result.update_max_num_bits(c, s);
                }
            }
            // InitAliasTable "fixes" empty counts to contain degenerate "0" symbol.
            let mut degenerate_symbol: i32 = if counts.is_empty() {
                0
            } else {
                counts.len() as i32 - 1
            };
            for s in 0..degenerate_symbol.max(0) as usize {
                if counts[s] != 0 {
                    degenerate_symbol = -1;
                    break;
                }
            }
            result.degenerate_symbols[c] = degenerate_symbol;
            let log_alpha_size = result.log_alpha_size as usize;
            init_alias_table(
                counts,
                ANS_TAB_SIZE,
                log_alpha_size,
                &mut result.alias_tables[c * per..(c + 1) * per],
            );
        }
    }
    Ok(())
}

/// Translation of `DecodeUintConfig()`.
fn decode_uint_config(
    log_alpha_size: usize,
    uint_config: &mut HybridUintConfig,
    br: &mut BitReader<'_>,
) -> Status {
    br.refill();
    let split_exponent = br.read_bits(ceil_log2_nonzero_u64(log_alpha_size as u64 + 1)) as usize;
    let mut msb_in_token = 0usize;
    let mut lsb_in_token = 0usize;
    if split_exponent != log_alpha_size {
        // otherwise, msb/lsb don't matter.
        let mut nbits = ceil_log2_nonzero_u64(split_exponent as u64 + 1);
        msb_in_token = br.read_bits(nbits) as usize;
        if msb_in_token > split_exponent {
            // This could be invalid here already and we need to check this before
            // we use its value to read more bits.
            return jxl_failure!("Invalid HybridUintConfig");
        }
        nbits = ceil_log2_nonzero_u64((split_exponent - msb_in_token) as u64 + 1);
        lsb_in_token = br.read_bits(nbits) as usize;
    }
    if lsb_in_token + msb_in_token > split_exponent {
        return jxl_failure!("Invalid HybridUintConfig");
    }
    *uint_config = HybridUintConfig::new(
        split_exponent as u32,
        msb_in_token as u32,
        lsb_in_token as u32,
    );
    Ok(())
}

/// Translation of `DecodeUintConfigs()`.
pub(crate) fn decode_uint_configs(
    log_alpha_size: usize,
    uint_config: &mut [HybridUintConfig],
    br: &mut BitReader<'_>,
) -> Status {
    // TODO(veluca): RLE?
    for c in uint_config.iter_mut() {
        decode_uint_config(log_alpha_size, c, br)?;
    }
    Ok(())
}

/// Translation of `DecodeHistograms()`.
pub(crate) fn decode_histograms(
    br: &mut BitReader<'_>,
    mut num_contexts: usize,
    code: &mut AnsCode,
    context_map: &mut Vec<u8>,
    disallow_lz77: bool,
) -> Status {
    bundle_read(br, &mut code.lz77)?;
    if code.lz77.enabled {
        num_contexts += 1;
        decode_uint_config(/*log_alpha_size=*/ 8, &mut code.lz77.length_uint_config, br)?;
    }
    if code.lz77.enabled && disallow_lz77 {
        return jxl_failure!("Using LZ77 when explicitly disallowed");
    }
    let mut num_histograms = 1usize;
    context_map.resize(num_contexts, 0);
    if num_contexts > 1 {
        decode_context_map(context_map, &mut num_histograms, br)?;
    }
    code.lz77.nonserialized_distance_context = *context_map.last().unwrap_or(&0) as usize;
    code.use_prefix_code = br.read_fixed_bits::<1>() != 0;
    if code.use_prefix_code {
        code.log_alpha_size = PREFIX_MAX_BITS as u8;
    } else {
        code.log_alpha_size = br.read_fixed_bits::<2>() as u8 + 5;
    }
    code.uint_config
        .resize(num_histograms, HybridUintConfig::default());
    decode_uint_configs(code.log_alpha_size as usize, &mut code.uint_config, br)?;
    let max_alphabet_size = 1usize << code.log_alpha_size;
    decode_ans_codes(num_histograms, max_alphabet_size, br, code)?;
    // When using LZ77, flat codes might result in valid codestreams with
    // histograms that potentially allow very large bit counts.
    // TODO(veluca): in principle, a valid codestream might contain a histogram
    // that could allow very large numbers of bits that is never used during ANS
    // decoding. There's no benefit to doing that, though.
    if !code.lz77.enabled && code.max_num_bits > 32 {
        // Just emit a warning as there are many opportunities for false positives.
        // JXL_WARNING("Histogram can represent numbers that are too large");
    }
    Ok(())
}
