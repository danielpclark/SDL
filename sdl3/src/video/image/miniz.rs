// Rust translation of the parts of src/video/miniz.h that SDL uses (the
// deflate compressor and the PNG writer, as configured by SDL_stb.c),
// from Simple DirectMedia Layer.
// miniz.c v1.15 - public domain deflate/inflate, zlib-subset, ZIP
// reading/writing/appending, PNG writing.
// Rich Geldreich <richgel99@gmail.com>, last updated Oct. 13, 2013.
// Implements RFC 1950: http://www.ietf.org/rfc/rfc1950.txt and RFC 1951:
// http://www.ietf.org/rfc/rfc1951.txt
// This is free and unencumbered software released into the public domain;
// see the notice in upstream's miniz.h.

//! The deflate compressor (`tdefl`) and `tdefl_write_image_to_png_file_in_memory_ex()`.
//!
//! SDL builds miniz with `MINIZ_USE_UNALIGNED_LOADS_AND_STORES 0`, so this
//! is the portable byte-at-a-time match finder and code emitter; the fast
//! level-1 path is never compiled. The compressor always writes through
//! its output callback (here, a growing `Vec`).

// stb's and miniz's constants are written with more digits than f32
// holds, and their loops index several arrays at once; both kept as written.
#![allow(
    clippy::excessive_precision,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

const LEN_SYM: [u16; 256] = [
    257, 258, 259, 260, 261, 262, 263, 264, 265, 265, 266, 266, 267, 267, 268, 268, 269, 269, 269,
    269, 270, 270, 270, 270, 271, 271, 271, 271, 272, 272, 272, 272, 273, 273, 273, 273, 273, 273,
    273, 273, 274, 274, 274, 274, 274, 274, 274, 274, 275, 275, 275, 275, 275, 275, 275, 275, 276,
    276, 276, 276, 276, 276, 276, 276, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277, 277,
    277, 277, 277, 277, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278, 278,
    278, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 279, 280, 280,
    280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 280, 281, 281, 281, 281, 281,
    281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281, 281,
    281, 281, 281, 281, 281, 281, 281, 281, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282,
    282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282, 282,
    282, 282, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283,
    283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 283, 284, 284, 284, 284,
    284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284, 284,
    284, 284, 284, 284, 284, 284, 284, 284, 285,
];

const LEN_EXTRA: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 0,
];

const SMALL_DIST_SYM: [u8; 512] = [
    0, 1, 2, 3, 4, 4, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 9,
    10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14,
    14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 14, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
];

const SMALL_DIST_EXTRA: [u8; 512] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
];

const LARGE_DIST_SYM: [u8; 128] = [
    0, 0, 18, 19, 20, 20, 21, 21, 22, 22, 22, 22, 23, 23, 23, 23, 24, 24, 24, 24, 24, 24, 24, 24,
    25, 25, 25, 25, 25, 25, 25, 25, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26, 26,
    27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 28, 28, 28, 28, 28, 28, 28, 28,
    28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29, 29,
    29, 29, 29, 29, 29, 29, 29, 29,
];

const LARGE_DIST_EXTRA: [u8; 128] = [
    0, 0, 8, 8, 9, 9, 9, 9, 10, 10, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11,
    11, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12,
    12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 13,
];

/// Translation of `mz_crc32()` (Karl Malbrain's compact CRC-32).
pub(crate) fn mz_crc32(crc: u32, buf: &[u8]) -> u32 {
    const S_CRC32: [u32; 16] = [
        0, 0x1db71064, 0x3b6e20c8, 0x26d930ac, 0x76dc4190, 0x6b6b51f4, 0x4db26158, 0x5005713c,
        0xedb88320, 0xf00f9344, 0xd6d6a3e8, 0xcb61b38c, 0x9b64c2b0, 0x86d3d2d4, 0xa00ae278,
        0xbdbdf21c,
    ];
    let mut crcu32 = !crc;
    for &b in buf {
        crcu32 = (crcu32 >> 4) ^ S_CRC32[((crcu32 & 0xF) ^ (b as u32 & 0xF)) as usize];
        crcu32 = (crcu32 >> 4) ^ S_CRC32[((crcu32 & 0xF) ^ (b as u32 >> 4)) as usize];
    }
    !crcu32
}

/// Translation of `mz_adler32()`.
pub(crate) fn mz_adler32(adler: u32, buf: &[u8]) -> u32 {
    let (mut s1, mut s2) = (adler & 0xffff, adler >> 16);
    let mut block_len = buf.len() % 5552;
    let mut ptr = buf;
    while !ptr.is_empty() {
        let (block, rest) = ptr.split_at(block_len);
        for &b in block {
            s1 += b as u32;
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
        ptr = rest;
        block_len = 5552;
    }
    (s2 << 16) + s1
}

// tdefl_init() compression flags (the low 12 bits are the number of probes)
#[allow(dead_code)] // (the probe count mask, for completeness)
const TDEFL_MAX_PROBES_MASK: u32 = 0xFFF;
const TDEFL_WRITE_ZLIB_HEADER: u32 = 0x01000;
const TDEFL_COMPUTE_ADLER32: u32 = 0x02000;
const TDEFL_GREEDY_PARSING_FLAG: u32 = 0x04000;
const TDEFL_NONDETERMINISTIC_PARSING_FLAG: u32 = 0x08000;
const TDEFL_RLE_MATCHES: u32 = 0x10000;
const TDEFL_FILTER_MATCHES: u32 = 0x20000;
const TDEFL_FORCE_ALL_STATIC_BLOCKS: u32 = 0x40000;
const TDEFL_FORCE_ALL_RAW_BLOCKS: u32 = 0x80000;

const TDEFL_MAX_HUFF_TABLES: usize = 3;
const TDEFL_MAX_HUFF_SYMBOLS_0: usize = 288;
const TDEFL_MAX_HUFF_SYMBOLS_1: usize = 32;
const TDEFL_MAX_HUFF_SYMBOLS_2: usize = 19;
const TDEFL_LZ_DICT_SIZE: usize = 32768;
const TDEFL_LZ_DICT_SIZE_MASK: u32 = (TDEFL_LZ_DICT_SIZE - 1) as u32;
const TDEFL_MIN_MATCH_LEN: u32 = 3;
const TDEFL_MAX_MATCH_LEN: u32 = 258;

// (TDEFL_LESS_MEMORY 0)
const TDEFL_LZ_CODE_BUF_SIZE: usize = 64 * 1024;
const TDEFL_OUT_BUF_SIZE: usize = (TDEFL_LZ_CODE_BUF_SIZE * 13) / 10;
const TDEFL_MAX_HUFF_SYMBOLS: usize = 288;
const TDEFL_LZ_HASH_BITS: u32 = 15;
const TDEFL_LZ_HASH_SHIFT: u32 = TDEFL_LZ_HASH_BITS.div_ceil(3);
const TDEFL_LZ_HASH_SIZE: usize = 1 << TDEFL_LZ_HASH_BITS;

/// Translation of `tdefl_status`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TdeflStatus {
    BadParam = -2,
    #[allow(dead_code)] // (the Vec output callback never fails)
    PutBufFailed = -1,
    Okay = 0,
    Done = 1,
}

/// Translation of `tdefl_flush`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TdeflFlush {
    NoFlush = 0,
    #[allow(dead_code)] // (part of the flush interface; the PNG writer doesn't sync)
    SyncFlush = 2,
    FullFlush = 3,
    Finish = 4,
}

/// Translation of `tdefl_sym_freq`.
#[derive(Clone, Copy, Default)]
struct SymFreq {
    key: u16,
    sym_index: u16,
}

/// Translation of `tdefl_radix_sort_syms()`: the symbols sorted by key.
fn radix_sort_syms(syms0: &mut [SymFreq], syms1: &mut [SymFreq]) -> bool {
    let num_syms = syms0.len();
    let mut total_passes = 2usize;
    let mut hist = [0u32; 256 * 2];
    for s in syms0.iter() {
        let freq = s.key as usize;
        hist[freq & 0xFF] += 1;
        hist[256 + ((freq >> 8) & 0xFF)] += 1;
    }
    while total_passes > 1 && num_syms as u32 == hist[(total_passes - 1) * 256] {
        total_passes -= 1;
    }
    // (true: the result is in syms1)
    let mut in_second = false;
    for pass in 0..total_passes {
        let pass_shift = pass * 8;
        let p_hist = &hist[pass << 8..(pass << 8) + 256];
        let mut offsets = [0u32; 256];
        let mut cur_ofs = 0;
        for i in 0..256 {
            offsets[i] = cur_ofs;
            cur_ofs += p_hist[i];
        }
        let (cur, new): (&[SymFreq], &mut [SymFreq]) = if in_second {
            (&*syms1, &mut *syms0)
        } else {
            (&*syms0, &mut *syms1)
        };
        for s in cur {
            let o = &mut offsets[((s.key as usize) >> pass_shift) & 0xFF];
            new[*o as usize] = *s;
            *o += 1;
        }
        in_second = !in_second;
    }
    in_second
}

/// Translation of `tdefl_calculate_minimum_redundancy()`, originally written
/// by Alistair Moffat and Jyrki Katajainen (November 1996).
fn calculate_minimum_redundancy(a: &mut [SymFreq]) {
    let n = a.len() as i32;
    if n == 0 {
        return;
    } else if n == 1 {
        a[0].key = 1;
        return;
    }
    a[0].key = a[0].key.wrapping_add(a[1].key);
    let mut root = 0i32;
    let mut leaf = 2i32;
    let k = |a: &[SymFreq], i: i32| a[i as usize].key;
    for next in 1..n - 1 {
        if leaf >= n || k(a, root) < k(a, leaf) {
            a[next as usize].key = k(a, root);
            a[root as usize].key = next as u16;
            root += 1;
        } else {
            a[next as usize].key = k(a, leaf);
            leaf += 1;
        }
        if leaf >= n || (root < next && k(a, root) < k(a, leaf)) {
            a[next as usize].key = k(a, next).wrapping_add(k(a, root));
            a[root as usize].key = next as u16;
            root += 1;
        } else {
            a[next as usize].key = k(a, next).wrapping_add(k(a, leaf));
            leaf += 1;
        }
    }
    a[(n - 2) as usize].key = 0;
    let mut next = n - 3;
    while next >= 0 {
        a[next as usize].key = a[a[next as usize].key as usize].key + 1;
        next -= 1;
    }
    let mut avbl = 1i32;
    let mut used = 0i32;
    let mut dpth = 0i32;
    let mut root = n - 2;
    let mut next = n - 1;
    while avbl > 0 {
        while root >= 0 && a[root as usize].key as i32 == dpth {
            used += 1;
            root -= 1;
        }
        while avbl > used {
            a[next as usize].key = dpth as u16;
            next -= 1;
            avbl -= 1;
        }
        avbl = 2 * used;
        dpth += 1;
        used = 0;
    }
}

// Limits canonical Huffman code table's max code size.
const TDEFL_MAX_SUPPORTED_HUFF_CODESIZE: usize = 32;

/// Translation of `tdefl_huffman_enforce_max_code_size()`.
fn huffman_enforce_max_code_size(
    num_codes: &mut [i32],
    code_list_len: usize,
    max_code_size: usize,
) {
    if code_list_len <= 1 {
        return;
    }
    for i in max_code_size + 1..=TDEFL_MAX_SUPPORTED_HUFF_CODESIZE {
        num_codes[max_code_size] += num_codes[i];
    }
    let mut total = 0u32;
    for i in (1..=max_code_size).rev() {
        total = total.wrapping_add((num_codes[i] as u32) << (max_code_size - i));
    }
    while total != (1u32 << max_code_size) {
        num_codes[max_code_size] -= 1;
        for i in (1..max_code_size).rev() {
            if num_codes[i] != 0 {
                num_codes[i] -= 1;
                num_codes[i + 1] += 2;
                break;
            }
        }
        total = total.wrapping_sub(1);
    }
}

/// `MZ_MIN(a, b)` for the compressor's unsigned arithmetic.
fn mz_min(a: u32, b: u32) -> u32 {
    a.min(b)
}

/// tdefl's compression state structure. Translation of `tdefl_compressor`;
/// the pointers into its buffers are indices, and the output callback is
/// `tdefl_output_buffer_putter()` into `out`.
struct Compressor {
    out: Vec<u8>,
    flags: u32,
    max_probes: [u32; 2],
    greedy_parsing: bool,
    adler32: u32,
    lookahead_pos: u32,
    lookahead_size: u32,
    dict_size: u32,
    p_lz_code_buf: usize,
    p_lz_flags: usize,
    p_output_buf: usize,
    p_output_buf_end: usize,
    num_flags_left: u32,
    total_lz_bytes: u32,
    lz_code_buf_dict_pos: u32,
    bits_in: u32,
    bit_buffer: u32,
    saved_match_dist: u32,
    saved_match_len: u32,
    saved_lit: u32,
    output_flush_ofs: u32,
    output_flush_remaining: u32,
    finished: bool,
    block_index: u32,
    wants_to_finish: bool,
    prev_return_status: TdeflStatus,
    flush: TdeflFlush,
    src_pos: usize,
    src_buf_left: usize,
    dict: Box<[u8]>,
    huff_count: [[u16; TDEFL_MAX_HUFF_SYMBOLS]; TDEFL_MAX_HUFF_TABLES],
    huff_codes: [[u16; TDEFL_MAX_HUFF_SYMBOLS]; TDEFL_MAX_HUFF_TABLES],
    huff_code_sizes: [[u8; TDEFL_MAX_HUFF_SYMBOLS]; TDEFL_MAX_HUFF_TABLES],
    lz_code_buf: Box<[u8]>,
    next: Box<[u16]>,
    hash: Box<[u16]>,
    output_buf: Box<[u8]>,
}

impl Compressor {
    /// Translation of `tdefl_init()` with `tdefl_output_buffer_putter()`.
    fn new(out: Vec<u8>, flags: u32) -> Box<Compressor> {
        let mut d = Box::new(Compressor {
            out,
            flags,
            max_probes: [
                1 + (flags & 0xFFF).div_ceil(3),
                1 + ((flags & 0xFFF) >> 2).div_ceil(3),
            ],
            greedy_parsing: (flags & TDEFL_GREEDY_PARSING_FLAG) != 0,
            adler32: 1,
            lookahead_pos: 0,
            lookahead_size: 0,
            dict_size: 0,
            p_lz_code_buf: 1,
            p_lz_flags: 0,
            p_output_buf: 0,
            p_output_buf_end: 0,
            num_flags_left: 8,
            total_lz_bytes: 0,
            lz_code_buf_dict_pos: 0,
            bits_in: 0,
            bit_buffer: 0,
            saved_match_dist: 0,
            saved_match_len: 0,
            saved_lit: 0,
            output_flush_ofs: 0,
            output_flush_remaining: 0,
            finished: false,
            block_index: 0,
            wants_to_finish: false,
            prev_return_status: TdeflStatus::Okay,
            flush: TdeflFlush::NoFlush,
            src_pos: 0,
            src_buf_left: 0,
            dict: vec![0; TDEFL_LZ_DICT_SIZE + TDEFL_MAX_MATCH_LEN as usize - 1].into_boxed_slice(),
            huff_count: [[0; TDEFL_MAX_HUFF_SYMBOLS]; TDEFL_MAX_HUFF_TABLES],
            huff_codes: [[0; TDEFL_MAX_HUFF_SYMBOLS]; TDEFL_MAX_HUFF_TABLES],
            huff_code_sizes: [[0; TDEFL_MAX_HUFF_SYMBOLS]; TDEFL_MAX_HUFF_TABLES],
            lz_code_buf: vec![0; TDEFL_LZ_CODE_BUF_SIZE].into_boxed_slice(),
            next: vec![0; TDEFL_LZ_DICT_SIZE].into_boxed_slice(),
            hash: vec![0; TDEFL_LZ_HASH_SIZE].into_boxed_slice(),
            output_buf: vec![0; TDEFL_OUT_BUF_SIZE].into_boxed_slice(),
        });
        // (TDEFL_NONDETERMINISTIC_PARSING_FLAG would skip clearing m_hash,
        // which is always zeroed here)
        let _ = flags & TDEFL_NONDETERMINISTIC_PARSING_FLAG;
        d.huff_count[0][..TDEFL_MAX_HUFF_SYMBOLS_0].fill(0);
        d.huff_count[1][..TDEFL_MAX_HUFF_SYMBOLS_1].fill(0);
        d
    }

    /// Translation of `TDEFL_PUT_BITS()`.
    fn put_bits(&mut self, bits: u32, len: u32) {
        crate::sdl_assert!(bits <= ((1u32 << len) - 1));
        self.bit_buffer |= bits << self.bits_in;
        self.bits_in += len;
        while self.bits_in >= 8 {
            if self.p_output_buf < self.p_output_buf_end {
                self.output_buf[self.p_output_buf] = self.bit_buffer as u8;
                self.p_output_buf += 1;
            }
            self.bit_buffer >>= 8;
            self.bits_in -= 8;
        }
    }

    /// Translation of `tdefl_optimize_huffman_table()`.
    fn optimize_huffman_table(
        &mut self,
        table_num: usize,
        table_len: usize,
        code_size_limit: usize,
        static_table: bool,
    ) {
        let mut num_codes = [0i32; 1 + TDEFL_MAX_SUPPORTED_HUFF_CODESIZE];
        let mut next_code = [0u32; TDEFL_MAX_SUPPORTED_HUFF_CODESIZE + 1];
        if static_table {
            for i in 0..table_len {
                num_codes[self.huff_code_sizes[table_num][i] as usize] += 1;
            }
        } else {
            let mut syms0 = [SymFreq::default(); TDEFL_MAX_HUFF_SYMBOLS];
            let mut syms1 = [SymFreq::default(); TDEFL_MAX_HUFF_SYMBOLS];
            let mut num_used_syms = 0;
            for i in 0..table_len {
                let count = self.huff_count[table_num][i];
                if count != 0 {
                    syms0[num_used_syms] = SymFreq {
                        key: count,
                        sym_index: i as u16,
                    };
                    num_used_syms += 1;
                }
            }

            let in_second =
                radix_sort_syms(&mut syms0[..num_used_syms], &mut syms1[..num_used_syms]);
            let p_syms = if in_second {
                &mut syms1[..num_used_syms]
            } else {
                &mut syms0[..num_used_syms]
            };
            calculate_minimum_redundancy(p_syms);

            for s in p_syms.iter() {
                num_codes[s.key as usize] += 1;
            }

            huffman_enforce_max_code_size(&mut num_codes, num_used_syms, code_size_limit);

            self.huff_code_sizes[table_num].fill(0);
            self.huff_codes[table_num].fill(0);
            let mut j = num_used_syms;
            for i in 1..=code_size_limit {
                for _ in 0..num_codes[i].max(0) {
                    j -= 1;
                    self.huff_code_sizes[table_num][p_syms[j].sym_index as usize] = i as u8;
                }
            }
        }

        next_code[1] = 0;
        let mut j = 0u32;
        for i in 2..=code_size_limit {
            j = (j.wrapping_add(num_codes[i - 1] as u32)) << 1;
            next_code[i] = j;
        }

        for i in 0..table_len {
            let code_size = self.huff_code_sizes[table_num][i] as usize;
            if code_size == 0 {
                continue;
            }
            let mut code = next_code[code_size];
            next_code[code_size] += 1;
            let mut rev_code = 0u32;
            for _ in 0..code_size {
                rev_code = (rev_code << 1) | (code & 1);
                code >>= 1;
            }
            self.huff_codes[table_num][i] = rev_code as u16;
        }
    }

    /// Translation of `tdefl_start_dynamic_block()`.
    #[allow(unused_assignments)] // (the RLE macros reset their counters, as upstream's do)
    fn start_dynamic_block(&mut self) {
        const SWIZZLE: [u8; 19] = [
            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
        ];
        let mut code_sizes_to_pack = [0u8; TDEFL_MAX_HUFF_SYMBOLS_0 + TDEFL_MAX_HUFF_SYMBOLS_1];
        let mut packed_code_sizes = [0u8; TDEFL_MAX_HUFF_SYMBOLS_0 + TDEFL_MAX_HUFF_SYMBOLS_1];
        let mut prev_code_size = 0xFFu8;

        self.huff_count[0][256] = 1;

        self.optimize_huffman_table(0, TDEFL_MAX_HUFF_SYMBOLS_0, 15, false);
        self.optimize_huffman_table(1, TDEFL_MAX_HUFF_SYMBOLS_1, 15, false);

        let mut num_lit_codes = 286;
        while num_lit_codes > 257 {
            if self.huff_code_sizes[0][num_lit_codes - 1] != 0 {
                break;
            }
            num_lit_codes -= 1;
        }
        let mut num_dist_codes = 30;
        while num_dist_codes > 1 {
            if self.huff_code_sizes[1][num_dist_codes - 1] != 0 {
                break;
            }
            num_dist_codes -= 1;
        }

        code_sizes_to_pack[..num_lit_codes]
            .copy_from_slice(&self.huff_code_sizes[0][..num_lit_codes]);
        code_sizes_to_pack[num_lit_codes..num_lit_codes + num_dist_codes]
            .copy_from_slice(&self.huff_code_sizes[1][..num_dist_codes]);
        let total_code_sizes_to_pack = num_lit_codes + num_dist_codes;
        let mut num_packed_code_sizes = 0usize;
        let mut rle_z_count = 0u32;
        let mut rle_repeat_count = 0u32;

        self.huff_count[2][..TDEFL_MAX_HUFF_SYMBOLS_2].fill(0);

        macro_rules! rle_prev_code_size {
            () => {
                if rle_repeat_count != 0 {
                    if rle_repeat_count < 3 {
                        self.huff_count[2][prev_code_size as usize] = self.huff_count[2]
                            [prev_code_size as usize]
                            .wrapping_add(rle_repeat_count as u16);
                        while rle_repeat_count != 0 {
                            rle_repeat_count -= 1;
                            packed_code_sizes[num_packed_code_sizes] = prev_code_size;
                            num_packed_code_sizes += 1;
                        }
                    } else {
                        self.huff_count[2][16] = self.huff_count[2][16].wrapping_add(1);
                        packed_code_sizes[num_packed_code_sizes] = 16;
                        packed_code_sizes[num_packed_code_sizes + 1] = (rle_repeat_count - 3) as u8;
                        num_packed_code_sizes += 2;
                    }
                    rle_repeat_count = 0;
                }
            };
        }
        macro_rules! rle_zero_code_size {
            () => {
                if rle_z_count != 0 {
                    if rle_z_count < 3 {
                        self.huff_count[2][0] =
                            self.huff_count[2][0].wrapping_add(rle_z_count as u16);
                        while rle_z_count != 0 {
                            rle_z_count -= 1;
                            packed_code_sizes[num_packed_code_sizes] = 0;
                            num_packed_code_sizes += 1;
                        }
                    } else if rle_z_count <= 10 {
                        self.huff_count[2][17] = self.huff_count[2][17].wrapping_add(1);
                        packed_code_sizes[num_packed_code_sizes] = 17;
                        packed_code_sizes[num_packed_code_sizes + 1] = (rle_z_count - 3) as u8;
                        num_packed_code_sizes += 2;
                    } else {
                        self.huff_count[2][18] = self.huff_count[2][18].wrapping_add(1);
                        packed_code_sizes[num_packed_code_sizes] = 18;
                        packed_code_sizes[num_packed_code_sizes + 1] = (rle_z_count - 11) as u8;
                        num_packed_code_sizes += 2;
                    }
                    rle_z_count = 0;
                }
            };
        }

        for &code_size in &code_sizes_to_pack[..total_code_sizes_to_pack] {
            if code_size == 0 {
                rle_prev_code_size!();
                rle_z_count += 1;
                if rle_z_count == 138 {
                    rle_zero_code_size!();
                }
            } else {
                rle_zero_code_size!();
                if code_size != prev_code_size {
                    rle_prev_code_size!();
                    self.huff_count[2][code_size as usize] =
                        self.huff_count[2][code_size as usize].wrapping_add(1);
                    packed_code_sizes[num_packed_code_sizes] = code_size;
                    num_packed_code_sizes += 1;
                } else {
                    rle_repeat_count += 1;
                    if rle_repeat_count == 6 {
                        rle_prev_code_size!();
                    }
                }
            }
            prev_code_size = code_size;
        }
        if rle_repeat_count != 0 {
            rle_prev_code_size!();
        } else {
            rle_zero_code_size!();
        }

        self.optimize_huffman_table(2, TDEFL_MAX_HUFF_SYMBOLS_2, 7, false);

        self.put_bits(2, 2);

        self.put_bits(num_lit_codes as u32 - 257, 5);
        self.put_bits(num_dist_codes as u32 - 1, 5);

        let mut num_bit_lengths = 18i32;
        while num_bit_lengths >= 0 {
            if self.huff_code_sizes[2][SWIZZLE[num_bit_lengths as usize] as usize] != 0 {
                break;
            }
            num_bit_lengths -= 1;
        }
        let num_bit_lengths = 4.max(num_bit_lengths + 1) as usize;
        self.put_bits(num_bit_lengths as u32 - 4, 4);
        for &s in &SWIZZLE[..num_bit_lengths] {
            self.put_bits(self.huff_code_sizes[2][s as usize] as u32, 3);
        }

        let mut packed_code_sizes_index = 0;
        while packed_code_sizes_index < num_packed_code_sizes {
            let code = packed_code_sizes[packed_code_sizes_index] as usize;
            packed_code_sizes_index += 1;
            crate::sdl_assert!(code < TDEFL_MAX_HUFF_SYMBOLS_2);
            self.put_bits(
                self.huff_codes[2][code] as u32,
                self.huff_code_sizes[2][code] as u32,
            );
            if code >= 16 {
                self.put_bits(
                    packed_code_sizes[packed_code_sizes_index] as u32,
                    [2, 3, 7][code - 16],
                );
                packed_code_sizes_index += 1;
            }
        }
    }

    /// Translation of `tdefl_start_static_block()`.
    fn start_static_block(&mut self) {
        let p = &mut self.huff_code_sizes[0];
        p[..=143].fill(8);
        p[144..=255].fill(9);
        p[256..=279].fill(7);
        p[280..=287].fill(8);

        self.huff_code_sizes[1][..32].fill(5);

        self.optimize_huffman_table(0, 288, 15, true);
        self.optimize_huffman_table(1, 32, 15, true);

        self.put_bits(1, 2);
    }

    /// Translation of `tdefl_compress_lz_codes()` (the portable version).
    fn compress_lz_codes(&mut self) -> bool {
        const MZ_BITMASKS: [u32; 17] = [
            0x0000, 0x0001, 0x0003, 0x0007, 0x000F, 0x001F, 0x003F, 0x007F, 0x00FF, 0x01FF, 0x03FF,
            0x07FF, 0x0FFF, 0x1FFF, 0x3FFF, 0x7FFF, 0xFFFF,
        ];
        let mut flags = 1u32;
        let mut p = 0usize;
        while p < self.p_lz_code_buf {
            if flags == 1 {
                flags = self.lz_code_buf[p] as u32 | 0x100;
                p += 1;
            }
            if flags & 1 != 0 {
                let match_len = self.lz_code_buf[p] as usize;
                let match_dist =
                    self.lz_code_buf[p + 1] as usize | ((self.lz_code_buf[p + 2] as usize) << 8);
                p += 3;

                let len_sym = LEN_SYM[match_len] as usize;
                crate::sdl_assert!(self.huff_code_sizes[0][len_sym] != 0);
                self.put_bits(
                    self.huff_codes[0][len_sym] as u32,
                    self.huff_code_sizes[0][len_sym] as u32,
                );
                self.put_bits(
                    match_len as u32 & MZ_BITMASKS[LEN_EXTRA[match_len] as usize],
                    LEN_EXTRA[match_len] as u32,
                );

                let (sym, num_extra_bits) = if match_dist < 512 {
                    (
                        SMALL_DIST_SYM[match_dist] as usize,
                        SMALL_DIST_EXTRA[match_dist] as usize,
                    )
                } else {
                    (
                        LARGE_DIST_SYM[match_dist >> 8] as usize,
                        LARGE_DIST_EXTRA[match_dist >> 8] as usize,
                    )
                };
                crate::sdl_assert!(self.huff_code_sizes[1][sym] != 0);
                self.put_bits(
                    self.huff_codes[1][sym] as u32,
                    self.huff_code_sizes[1][sym] as u32,
                );
                self.put_bits(
                    match_dist as u32 & MZ_BITMASKS[num_extra_bits],
                    num_extra_bits as u32,
                );
            } else {
                let lit = self.lz_code_buf[p] as usize;
                p += 1;
                crate::sdl_assert!(self.huff_code_sizes[0][lit] != 0);
                self.put_bits(
                    self.huff_codes[0][lit] as u32,
                    self.huff_code_sizes[0][lit] as u32,
                );
            }
            flags >>= 1;
        }

        self.put_bits(
            self.huff_codes[0][256] as u32,
            self.huff_code_sizes[0][256] as u32,
        );

        self.p_output_buf < self.p_output_buf_end
    }

    /// Translation of `tdefl_compress_block()`.
    fn compress_block(&mut self, static_block: bool) -> bool {
        if static_block {
            self.start_static_block();
        } else {
            self.start_dynamic_block();
        }
        self.compress_lz_codes()
    }

    /// Translation of `tdefl_flush_block()`.
    fn flush_block(&mut self, flush: TdeflFlush) -> i32 {
        let mut comp_block_succeeded = false;
        let use_raw_block = (self.flags & TDEFL_FORCE_ALL_RAW_BLOCKS) != 0
            && self.lookahead_pos.wrapping_sub(self.lz_code_buf_dict_pos) <= self.dict_size;
        // (output always goes through the internal buffer and the callback)
        let p_output_buf_start = 0;

        self.p_output_buf = p_output_buf_start;
        self.p_output_buf_end = self.p_output_buf + TDEFL_OUT_BUF_SIZE - 16;

        crate::sdl_assert!(self.output_flush_remaining == 0);
        self.output_flush_ofs = 0;
        self.output_flush_remaining = 0;

        self.lz_code_buf[self.p_lz_flags] =
            ((self.lz_code_buf[self.p_lz_flags] as u32) >> self.num_flags_left) as u8;
        if self.num_flags_left == 8 {
            self.p_lz_code_buf -= 1;
        }

        if (self.flags & TDEFL_WRITE_ZLIB_HEADER) != 0 && self.block_index == 0 {
            self.put_bits(0x78, 8);
            self.put_bits(0x01, 8);
        }

        self.put_bits((flush == TdeflFlush::Finish) as u32, 1);

        let saved_output_buf = self.p_output_buf;
        let saved_bit_buf = self.bit_buffer;
        let saved_bits_in = self.bits_in;

        if !use_raw_block {
            comp_block_succeeded = self.compress_block(
                (self.flags & TDEFL_FORCE_ALL_STATIC_BLOCKS) != 0 || self.total_lz_bytes < 48,
            );
        }

        // If the block gets expanded, forget the current contents of the output buffer and send a raw block instead.
        if (use_raw_block
            || (self.total_lz_bytes != 0
                && (self.p_output_buf - saved_output_buf + 1) as u32 >= self.total_lz_bytes))
            && self.lookahead_pos.wrapping_sub(self.lz_code_buf_dict_pos) <= self.dict_size
        {
            self.p_output_buf = saved_output_buf;
            self.bit_buffer = saved_bit_buf;
            self.bits_in = saved_bits_in;
            self.put_bits(0, 2);
            if self.bits_in != 0 {
                self.put_bits(0, 8 - self.bits_in);
            }
            for _ in 0..2 {
                self.put_bits(self.total_lz_bytes & 0xFFFF, 16);
                self.total_lz_bytes ^= 0xFFFF;
            }
            for i in 0..self.total_lz_bytes {
                let b = self.dict[((self.lz_code_buf_dict_pos.wrapping_add(i))
                    & TDEFL_LZ_DICT_SIZE_MASK) as usize];
                self.put_bits(b as u32, 8);
            }
        }
        // Check for the extremely unlikely (if not impossible) case of the compressed block not fitting into the output buffer when using dynamic codes.
        else if !comp_block_succeeded {
            self.p_output_buf = saved_output_buf;
            self.bit_buffer = saved_bit_buf;
            self.bits_in = saved_bits_in;
            self.compress_block(true);
        }

        if flush != TdeflFlush::NoFlush {
            if flush == TdeflFlush::Finish {
                if self.bits_in != 0 {
                    self.put_bits(0, 8 - self.bits_in);
                }
                if (self.flags & TDEFL_WRITE_ZLIB_HEADER) != 0 {
                    let mut a = self.adler32;
                    for _ in 0..4 {
                        self.put_bits((a >> 24) & 0xFF, 8);
                        a <<= 8;
                    }
                }
            } else {
                let mut z = 0u32;
                self.put_bits(0, 3);
                if self.bits_in != 0 {
                    self.put_bits(0, 8 - self.bits_in);
                }
                for _ in 0..2 {
                    self.put_bits(z & 0xFFFF, 16);
                    z ^= 0xFFFF;
                }
            }
        }

        crate::sdl_assert!(self.p_output_buf < self.p_output_buf_end);

        self.huff_count[0][..TDEFL_MAX_HUFF_SYMBOLS_0].fill(0);
        self.huff_count[1][..TDEFL_MAX_HUFF_SYMBOLS_1].fill(0);

        self.p_lz_code_buf = 1;
        self.p_lz_flags = 0;
        self.num_flags_left = 8;
        self.lz_code_buf_dict_pos = self.lz_code_buf_dict_pos.wrapping_add(self.total_lz_bytes);
        self.total_lz_bytes = 0;
        self.block_index += 1;

        let n = self.p_output_buf - p_output_buf_start;
        if n != 0 {
            // tdefl_output_buffer_putter()
            let (out, buf) = (&mut self.out, &self.output_buf);
            out.extend_from_slice(&buf[..n]);
        }

        self.output_flush_remaining as i32
    }

    /// Translation of `tdefl_find_match()` (the portable version).
    fn find_match(
        &self,
        lookahead_pos: u32,
        max_dist: u32,
        max_match_len: u32,
        p_match_dist: &mut u32,
        p_match_len: &mut u32,
    ) {
        let dict = &self.dict;
        let pos = (lookahead_pos & TDEFL_LZ_DICT_SIZE_MASK) as usize;
        let mut match_len = *p_match_len;
        let mut probe_pos = pos;
        let mut num_probes_left = self.max_probes[(match_len >= 32) as usize];
        let mut c0 = dict[pos + match_len as usize];
        let mut c1 = dict[pos + match_len as usize - 1];
        crate::sdl_assert!(max_match_len <= TDEFL_MAX_MATCH_LEN);
        if max_match_len <= match_len {
            return;
        }
        loop {
            let dist;
            'probe: loop {
                num_probes_left = num_probes_left.wrapping_sub(1);
                if num_probes_left == 0 {
                    return;
                }
                for _ in 0..3 {
                    // TDEFL_PROBE
                    let next_probe_pos = self.next[probe_pos] as u32;
                    if next_probe_pos == 0 {
                        return;
                    }
                    let d = lookahead_pos.wrapping_sub(next_probe_pos) as u16 as u32;
                    if d > max_dist {
                        return;
                    }
                    probe_pos = (next_probe_pos & TDEFL_LZ_DICT_SIZE_MASK) as usize;
                    if dict[probe_pos + match_len as usize] == c0
                        && dict[probe_pos + match_len as usize - 1] == c1
                    {
                        dist = d;
                        break 'probe;
                    }
                }
            }
            if dist == 0 {
                break;
            }
            let mut probe_len = 0;
            while probe_len < max_match_len {
                if dict[pos + probe_len as usize] != dict[probe_pos + probe_len as usize] {
                    break;
                }
                probe_len += 1;
            }
            if probe_len > match_len {
                *p_match_dist = dist;
                match_len = probe_len;
                *p_match_len = match_len;
                if match_len == max_match_len {
                    return;
                }
                c0 = dict[pos + match_len as usize];
                c1 = dict[pos + match_len as usize - 1];
            }
        }
    }

    /// Translation of `tdefl_record_literal()`.
    fn record_literal(&mut self, lit: u8) {
        self.total_lz_bytes += 1;
        self.lz_code_buf[self.p_lz_code_buf] = lit;
        self.p_lz_code_buf += 1;
        self.lz_code_buf[self.p_lz_flags] >>= 1;
        self.num_flags_left -= 1;
        if self.num_flags_left == 0 {
            self.num_flags_left = 8;
            self.p_lz_flags = self.p_lz_code_buf;
            self.p_lz_code_buf += 1;
        }
        self.huff_count[0][lit as usize] = self.huff_count[0][lit as usize].wrapping_add(1);
    }

    /// Translation of `tdefl_record_match()`.
    fn record_match(&mut self, match_len: u32, mut match_dist: u32) {
        crate::sdl_assert!(
            match_len >= TDEFL_MIN_MATCH_LEN
                && match_dist >= 1
                && match_dist <= TDEFL_LZ_DICT_SIZE as u32
        );

        self.total_lz_bytes += match_len;

        self.lz_code_buf[self.p_lz_code_buf] = (match_len - TDEFL_MIN_MATCH_LEN) as u8;

        match_dist -= 1;
        self.lz_code_buf[self.p_lz_code_buf + 1] = (match_dist & 0xFF) as u8;
        self.lz_code_buf[self.p_lz_code_buf + 2] = (match_dist >> 8) as u8;
        self.p_lz_code_buf += 3;

        self.lz_code_buf[self.p_lz_flags] = (self.lz_code_buf[self.p_lz_flags] >> 1) | 0x80;
        self.num_flags_left -= 1;
        if self.num_flags_left == 0 {
            self.num_flags_left = 8;
            self.p_lz_flags = self.p_lz_code_buf;
            self.p_lz_code_buf += 1;
        }

        let s0 = SMALL_DIST_SYM[(match_dist & 511) as usize] as usize;
        let s1 = LARGE_DIST_SYM[((match_dist >> 8) & 127) as usize] as usize;
        let sym = if match_dist < 512 { s0 } else { s1 };
        self.huff_count[1][sym] = self.huff_count[1][sym].wrapping_add(1);

        if match_len >= TDEFL_MIN_MATCH_LEN {
            let l = LEN_SYM[(match_len - TDEFL_MIN_MATCH_LEN) as usize] as usize;
            self.huff_count[0][l] = self.huff_count[0][l].wrapping_add(1);
        }
    }

    /// Translation of `tdefl_compress_normal()`.
    fn compress_normal(&mut self, src: &[u8]) -> bool {
        let mut p_src = self.src_pos;
        let mut src_buf_left = self.src_buf_left;
        let flush = self.flush;

        while src_buf_left != 0 || (flush != TdeflFlush::NoFlush && self.lookahead_size != 0) {
            // Update dictionary and hash chains. Keeps the lookahead size equal to TDEFL_MAX_MATCH_LEN.
            if (self.lookahead_size + self.dict_size) >= (TDEFL_MIN_MATCH_LEN - 1) {
                let mut dst_pos = (self.lookahead_pos.wrapping_add(self.lookahead_size))
                    & TDEFL_LZ_DICT_SIZE_MASK;
                let mut ins_pos = self
                    .lookahead_pos
                    .wrapping_add(self.lookahead_size)
                    .wrapping_sub(2);
                let mut hash = ((self.dict[(ins_pos & TDEFL_LZ_DICT_SIZE_MASK) as usize] as u32)
                    << TDEFL_LZ_HASH_SHIFT)
                    ^ self.dict[((ins_pos.wrapping_add(1)) & TDEFL_LZ_DICT_SIZE_MASK) as usize]
                        as u32;
                let num_bytes_to_process =
                    src_buf_left.min((TDEFL_MAX_MATCH_LEN - self.lookahead_size) as usize);
                let p_src_end = p_src + num_bytes_to_process;
                src_buf_left -= num_bytes_to_process;
                self.lookahead_size += num_bytes_to_process as u32;
                while p_src != p_src_end {
                    let c = src[p_src];
                    p_src += 1;
                    self.dict[dst_pos as usize] = c;
                    if dst_pos < (TDEFL_MAX_MATCH_LEN - 1) {
                        self.dict[TDEFL_LZ_DICT_SIZE + dst_pos as usize] = c;
                    }
                    hash = ((hash << TDEFL_LZ_HASH_SHIFT) ^ c as u32)
                        & (TDEFL_LZ_HASH_SIZE as u32 - 1);
                    self.next[(ins_pos & TDEFL_LZ_DICT_SIZE_MASK) as usize] =
                        self.hash[hash as usize];
                    self.hash[hash as usize] = ins_pos as u16;
                    dst_pos = (dst_pos + 1) & TDEFL_LZ_DICT_SIZE_MASK;
                    ins_pos = ins_pos.wrapping_add(1);
                }
            } else {
                while src_buf_left != 0 && self.lookahead_size < TDEFL_MAX_MATCH_LEN {
                    let c = src[p_src];
                    p_src += 1;
                    let dst_pos = (self.lookahead_pos.wrapping_add(self.lookahead_size))
                        & TDEFL_LZ_DICT_SIZE_MASK;
                    src_buf_left -= 1;
                    self.dict[dst_pos as usize] = c;
                    if dst_pos < (TDEFL_MAX_MATCH_LEN - 1) {
                        self.dict[TDEFL_LZ_DICT_SIZE + dst_pos as usize] = c;
                    }
                    self.lookahead_size += 1;
                    if (self.lookahead_size + self.dict_size) >= TDEFL_MIN_MATCH_LEN {
                        let ins_pos = self
                            .lookahead_pos
                            .wrapping_add(self.lookahead_size - 1)
                            .wrapping_sub(2);
                        let hash = (((self.dict[(ins_pos & TDEFL_LZ_DICT_SIZE_MASK) as usize]
                            as u32)
                            << (TDEFL_LZ_HASH_SHIFT * 2))
                            ^ ((self.dict
                                [((ins_pos.wrapping_add(1)) & TDEFL_LZ_DICT_SIZE_MASK) as usize]
                                as u32)
                                << TDEFL_LZ_HASH_SHIFT)
                            ^ c as u32)
                            & (TDEFL_LZ_HASH_SIZE as u32 - 1);
                        self.next[(ins_pos & TDEFL_LZ_DICT_SIZE_MASK) as usize] =
                            self.hash[hash as usize];
                        self.hash[hash as usize] = ins_pos as u16;
                    }
                }
            }
            self.dict_size = mz_min(
                TDEFL_LZ_DICT_SIZE as u32 - self.lookahead_size,
                self.dict_size,
            );
            if flush == TdeflFlush::NoFlush && self.lookahead_size < TDEFL_MAX_MATCH_LEN {
                break;
            }

            // Simple lazy/greedy parsing state machine.
            let mut len_to_move = 1u32;
            let mut cur_match_dist = 0u32;
            let mut cur_match_len = if self.saved_match_len != 0 {
                self.saved_match_len
            } else {
                TDEFL_MIN_MATCH_LEN - 1
            };
            let cur_pos = self.lookahead_pos & TDEFL_LZ_DICT_SIZE_MASK;
            if (self.flags & (TDEFL_RLE_MATCHES | TDEFL_FORCE_ALL_RAW_BLOCKS)) != 0 {
                if self.dict_size != 0 && (self.flags & TDEFL_FORCE_ALL_RAW_BLOCKS) == 0 {
                    let c = self.dict[(cur_pos.wrapping_sub(1) & TDEFL_LZ_DICT_SIZE_MASK) as usize];
                    cur_match_len = 0;
                    while cur_match_len < self.lookahead_size {
                        if self.dict[(cur_pos + cur_match_len) as usize] != c {
                            break;
                        }
                        cur_match_len += 1;
                    }
                    if cur_match_len < TDEFL_MIN_MATCH_LEN {
                        cur_match_len = 0;
                    } else {
                        cur_match_dist = 1;
                    }
                }
            } else {
                self.find_match(
                    self.lookahead_pos,
                    self.dict_size,
                    self.lookahead_size,
                    &mut cur_match_dist,
                    &mut cur_match_len,
                );
            }
            if (cur_match_len == TDEFL_MIN_MATCH_LEN && cur_match_dist >= 8 * 1024)
                || cur_pos == cur_match_dist
                || ((self.flags & TDEFL_FILTER_MATCHES) != 0 && cur_match_len <= 5)
            {
                cur_match_dist = 0;
                cur_match_len = 0;
            }
            let dict_last = self.dict.len() - 1;
            if self.saved_match_len != 0 {
                if cur_match_len > self.saved_match_len {
                    self.record_literal(self.saved_lit as u8);
                    if cur_match_len >= 128 {
                        self.record_match(cur_match_len, cur_match_dist);
                        self.saved_match_len = 0;
                        len_to_move = cur_match_len;
                    } else {
                        self.saved_lit = self.dict[cur_pos as usize] as u32;
                        self.saved_match_dist = cur_match_dist;
                        self.saved_match_len = cur_match_len;
                    }
                } else {
                    self.record_match(self.saved_match_len, self.saved_match_dist);
                    len_to_move = self.saved_match_len - 1;
                    self.saved_match_len = 0;
                }
            } else if cur_match_dist == 0 {
                self.record_literal(self.dict[(cur_pos as usize).min(dict_last)]);
            } else if self.greedy_parsing
                || (self.flags & TDEFL_RLE_MATCHES) != 0
                || cur_match_len >= 128
            {
                self.record_match(cur_match_len, cur_match_dist);
                len_to_move = cur_match_len;
            } else {
                self.saved_lit = self.dict[(cur_pos as usize).min(dict_last)] as u32;
                self.saved_match_dist = cur_match_dist;
                self.saved_match_len = cur_match_len;
            }
            // Move the lookahead forward by len_to_move bytes.
            self.lookahead_pos = self.lookahead_pos.wrapping_add(len_to_move);
            crate::sdl_assert!(self.lookahead_size >= len_to_move);
            self.lookahead_size -= len_to_move;
            self.dict_size = mz_min(self.dict_size + len_to_move, TDEFL_LZ_DICT_SIZE as u32);
            // Check if it's time to flush the current LZ codes to the internal output buffer.
            if self.p_lz_code_buf > TDEFL_LZ_CODE_BUF_SIZE - 8
                || (self.total_lz_bytes > 31 * 1024
                    && ((((self.p_lz_code_buf as u32) * 115) >> 7) >= self.total_lz_bytes
                        || (self.flags & TDEFL_FORCE_ALL_RAW_BLOCKS) != 0))
            {
                self.src_pos = p_src;
                self.src_buf_left = src_buf_left;
                let n = self.flush_block(TdeflFlush::NoFlush);
                if n != 0 {
                    return n >= 0;
                }
            }
        }

        self.src_pos = p_src;
        self.src_buf_left = src_buf_left;
        true
    }

    /// Translation of `tdefl_flush_output_buffer()` (with the output
    /// callback, there's no caller buffer to flush into).
    fn flush_output_buffer(&mut self) -> TdeflStatus {
        if self.finished && self.output_flush_remaining == 0 {
            TdeflStatus::Done
        } else {
            TdeflStatus::Okay
        }
    }

    /// Translation of `tdefl_compress_buffer()` (`tdefl_compress()` with the
    /// output callback): compress all of `input`.
    fn compress_buffer(&mut self, input: &[u8], flush: TdeflFlush) -> TdeflStatus {
        self.src_pos = 0;
        self.src_buf_left = input.len();
        self.flush = flush;

        if self.prev_return_status != TdeflStatus::Okay
            || (self.wants_to_finish && flush != TdeflFlush::Finish)
        {
            self.prev_return_status = TdeflStatus::BadParam;
            return self.prev_return_status;
        }
        self.wants_to_finish |= flush == TdeflFlush::Finish;

        if self.output_flush_remaining != 0 || self.finished {
            self.prev_return_status = self.flush_output_buffer();
            return self.prev_return_status;
        }

        // (tdefl_compress_fast() needs unaligned loads, which SDL disables)
        if !self.compress_normal(input) {
            return self.prev_return_status;
        }

        if (self.flags & (TDEFL_WRITE_ZLIB_HEADER | TDEFL_COMPUTE_ADLER32)) != 0
            && !input.is_empty()
        {
            self.adler32 = mz_adler32(self.adler32, &input[..self.src_pos]);
        }

        if flush != TdeflFlush::NoFlush
            && self.lookahead_size == 0
            && self.src_buf_left == 0
            && self.output_flush_remaining == 0
        {
            if self.flush_block(flush) < 0 {
                return self.prev_return_status;
            }
            self.finished = flush == TdeflFlush::Finish;
            if flush == TdeflFlush::FullFlush {
                self.hash.fill(0);
                self.next.fill(0);
                self.dict_size = 0;
            }
        }

        self.prev_return_status = self.flush_output_buffer();
        self.prev_return_status
    }
}

/// Compress an image to a PNG file in memory: `num_chans` 1 to 4 bytes per
/// pixel, rows `bpl` bytes apart, with optional `PLTE` and `tRNS` chunks.
/// `level` ranges from 0 to 10; `flip` writes the rows bottom up.
/// Translation of `tdefl_write_image_to_png_file_in_memory_ex()`, a
/// modification of Alex Evans' simple PNG writer (2011).
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_image_to_png_file_in_memory_ex(
    image: &[u8],
    w: i32,
    h: i32,
    num_chans: i32,
    bpl: i32,
    level: u32,
    flip: bool,
    plte: &[u8],
    trns: &[u8],
) -> Option<Vec<u8>> {
    // Using a local copy of this array here in case MINIZ_NO_ZLIB_APIS was defined.
    const S_TDEFL_PNG_NUM_PROBES: [u32; 11] = [0, 1, 6, 32, 16, 32, 128, 256, 512, 768, 1500];
    let z = [0u8; 1];
    let mut capacity = 57 + 64.max((1 + w * num_chans) * h) as usize;
    if !plte.is_empty() {
        capacity += 12 + plte.len();
    }
    if !trns.is_empty() {
        capacity += 12 + trns.len();
    }
    let mut out = Vec::with_capacity(capacity);
    // write header
    {
        const CHANS: [u8; 5] = [0x00, 0x00, 0x04, 0x02, 0x06];

        let mut pnghdr: [u8; 33] = [
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        // FIXME (upstream): only the low 16 bits of the width and height
        // are written, so images 65536 or more pixels across come out wrong.
        pnghdr[18] = (w >> 8) as u8;
        pnghdr[19] = w as u8;

        pnghdr[22] = (h >> 8) as u8;
        pnghdr[23] = h as u8;

        if num_chans == 1 && !plte.is_empty() {
            pnghdr[25] = 3;
        } else {
            pnghdr[25] = CHANS[num_chans as usize];
        }

        let mut c = mz_crc32(0, &pnghdr[12..29]);
        for i in 0..4 {
            pnghdr[29 + i] = (c >> 24) as u8;
            c <<= 8;
        }
        out.extend_from_slice(&pnghdr);
    }
    // write PLTE and tRNS chunks
    for (chunk, data) in [(b"PLTE", plte), (b"tRNS", trns)] {
        if data.is_empty() {
            continue;
        }
        let size = data.len() as u32;
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(chunk);
        let data_start = out.len();
        out.extend_from_slice(data);
        out.extend_from_slice(&[0; 4]);
        let mut c = mz_crc32(0, &out[data_start - 4..data_start + data.len()]);
        let end = out.len();
        for i in 0..4 {
            out[end - 4 + i] = (c >> 24) as u8;
            c <<= 8;
        }
    }
    // write IDAT chunk
    out.extend_from_slice(b"\0\0\0\0IDAT");
    let data_start = out.len();
    // compress image data
    let mut comp = Compressor::new(
        out,
        S_TDEFL_PNG_NUM_PROBES[level.min(10) as usize] | TDEFL_WRITE_ZLIB_HEADER,
    );
    let row_len = (w * num_chans) as usize;
    for y in 0..h {
        comp.compress_buffer(&z, TdeflFlush::NoFlush);
        let row = if flip { h - 1 - y } else { y } as usize * bpl as usize;
        comp.compress_buffer(&image[row..row + row_len], TdeflFlush::NoFlush);
    }
    if comp.compress_buffer(&[], TdeflFlush::Finish) != TdeflStatus::Done {
        return None;
    }
    let mut out = std::mem::take(&mut comp.out);
    // write IDAT size
    let data_size = out.len() - data_start;
    out[data_start - 8..data_start - 4].copy_from_slice(&(data_size as u32).to_be_bytes());
    // write footer (IDAT CRC-32, followed by IEND chunk)
    out.extend_from_slice(b"\0\0\0\0\0\0\0\0IEND\xae\x42\x60\x82");
    let mut c = mz_crc32(0, &out[data_start - 4..data_start + data_size]);
    let end = out.len();
    for i in 0..4 {
        out[end - 16 + i] = (c >> 24) as u8;
        c <<= 8;
    }
    // compute final size of file, grab compressed data buffer and return
    Some(out)
}
