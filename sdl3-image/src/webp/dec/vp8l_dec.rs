// Rust translation of src/dec/vp8l_dec.c and src/dec/vp8li_dec.h from
// libwebp (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2012 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! main entry for the decoder: the lossless (VP8L) decoder, for images and
//! for alpha planes.
//!
//! Not incremental (`incremental_` is 0: the state saving and restoring
//! are not translated), without rescaling and to RGB(A) output (the YUVA
//! export is not translated: SDL_image's loader doesn't ask for them).
//! Upstream's `pixels_` holds the decoded pixels and, after them, the argb
//! cache with its top-prediction row; here the pixels (32-bit, or 8-bit
//! for paletted alpha) and the cache are separate vectors, and the row
//! processing functions (`ProcessRows()`, `ExtractAlphaRows()`) are a
//! [`RowSink`] naming where the rows go.

use crate::webp::dec::alpha_dec::ALPHDecoder;
use crate::webp::dec::io_dec::WebPDecParams;
use crate::webp::dec::vp8_dec::VP8Io;
use crate::webp::dec::webp_dec::webp_io_init_from_options;
use crate::webp::decode::{
    VP8LImageTransformType, VP8StatusCode, WebPDecBuffer, WebpCspMode, HUFFMAN_CODES_PER_META_CODE,
    MAX_CACHE_BITS, NUM_DISTANCE_CODES, NUM_LENGTH_CODES, NUM_LITERAL_CODES, NUM_TRANSFORMS,
    VP8L_FRAME_HEADER_SIZE, VP8L_IMAGE_SIZE_BITS, VP8L_MAGIC_BYTE, VP8L_VERSION_BITS,
};
use crate::webp::dsp::alpha_processing::webp_extract_green;
use crate::webp::dsp::filters::webp_unfilter;
use crate::webp::dsp::lossless::{
    vp8l_add_pixels, vp8l_color_index_inverse_transform_alpha, vp8l_convert_from_bgra,
    vp8l_inverse_transform, vp8l_sub_sample_size,
};
use crate::webp::dsp::WebpFilterType;
use crate::webp::utils::bit_reader_utils::VP8LBitReader;
use crate::webp::utils::color_cache_utils::{
    vp8l_color_cache_clear, vp8l_color_cache_init, VP8LColorCache,
};
use crate::webp::utils::huffman_utils::{
    vp8l_build_huffman_table, vp8l_htree_groups_new, vp8l_huffman_tables_allocate,
    vp8l_huffman_tables_deallocate, HTreeGroup, HuffmanCode, HuffmanCode32, HuffmanTables,
    HUFFMAN_PACKED_BITS, HUFFMAN_PACKED_TABLE_SIZE, HUFFMAN_TABLE_BITS, HUFFMAN_TABLE_MASK,
    LENGTHS_TABLE_BITS, LENGTHS_TABLE_MASK,
};
use crate::webp::utils::{check_size_overflow, safe_alloc, WEBP_MAX_ALLOCABLE_MEMORY};

const NUM_ARGB_CACHE_ROWS: i32 = 16;

const K_CODE_LENGTH_LITERALS: i32 = 16;
const K_CODE_LENGTH_REPEAT_CODE: i32 = 16;
static K_CODE_LENGTH_EXTRA_BITS: [u8; 3] = [2, 3, 7];
static K_CODE_LENGTH_REPEAT_OFFSETS: [u8; 3] = [3, 3, 11];

// -----------------------------------------------------------------------------
//  Five Huffman codes are used at each meta code:
//  1. green + length prefix codes + color cache codes,
//  2. alpha,
//  3. red,
//  4. blue, and,
//  5. distance prefix codes.
// (Translation of `HuffIndex`.)
const GREEN: usize = 0;
const RED: usize = 1;
const BLUE: usize = 2;
const ALPHA: usize = 3;
const DIST: usize = 4;

static K_ALPHABET_SIZE: [u16; HUFFMAN_CODES_PER_META_CODE] = [
    (NUM_LITERAL_CODES + NUM_LENGTH_CODES) as u16,
    NUM_LITERAL_CODES as u16,
    NUM_LITERAL_CODES as u16,
    NUM_LITERAL_CODES as u16,
    NUM_DISTANCE_CODES as u16,
];

static K_LITERAL_MAP: [u8; HUFFMAN_CODES_PER_META_CODE] = [0, 1, 1, 1, 0];

const NUM_CODE_LENGTH_CODES: usize = 19;
static K_CODE_LENGTH_CODE_ORDER: [u8; NUM_CODE_LENGTH_CODES] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

const CODE_TO_PLANE_CODES: i32 = 120;
static K_CODE_TO_PLANE: [u8; CODE_TO_PLANE_CODES as usize] = [
    0x18, 0x07, 0x17, 0x19, 0x28, 0x06, 0x27, 0x29, 0x16, 0x1a, 0x26, 0x2a, 0x38, 0x05, 0x37, 0x39,
    0x15, 0x1b, 0x36, 0x3a, 0x25, 0x2b, 0x48, 0x04, 0x47, 0x49, 0x14, 0x1c, 0x35, 0x3b, 0x46, 0x4a,
    0x24, 0x2c, 0x58, 0x45, 0x4b, 0x34, 0x3c, 0x03, 0x57, 0x59, 0x13, 0x1d, 0x56, 0x5a, 0x23, 0x2d,
    0x44, 0x4c, 0x55, 0x5b, 0x33, 0x3d, 0x68, 0x02, 0x67, 0x69, 0x12, 0x1e, 0x66, 0x6a, 0x22, 0x2e,
    0x54, 0x5c, 0x43, 0x4d, 0x65, 0x6b, 0x32, 0x3e, 0x78, 0x01, 0x77, 0x79, 0x53, 0x5d, 0x11, 0x1f,
    0x64, 0x6c, 0x42, 0x4e, 0x76, 0x7a, 0x21, 0x2f, 0x75, 0x7b, 0x31, 0x3f, 0x63, 0x6d, 0x52, 0x5e,
    0x00, 0x74, 0x7c, 0x41, 0x4f, 0x10, 0x20, 0x62, 0x6e, 0x30, 0x73, 0x7d, 0x51, 0x5f, 0x40, 0x72,
    0x7e, 0x61, 0x6f, 0x50, 0x71, 0x7f, 0x60, 0x70,
];

// Memory needed for lookup tables of one Huffman tree group. Red, blue, alpha
// and distance alphabets are constant (256 for red, blue and alpha, 40 for
// distance) and lookup table sizes for them in worst case are 630 and 410
// respectively. Size of green alphabet depends on color cache size and is equal
// to 256 (green component values) + 24 (length prefix values)
// + color_cache_size (between 0 and 2048).
// All values computed for 8-bit first level lookup with Mark Adler's tool:
// https://github.com/madler/zlib/blob/v1.2.5/examples/enough.c
const FIXED_TABLE_SIZE: u16 = 630 * 3 + 410;
static K_TABLE_SIZE: [u16; 12] = [
    FIXED_TABLE_SIZE + 654,
    FIXED_TABLE_SIZE + 656,
    FIXED_TABLE_SIZE + 658,
    FIXED_TABLE_SIZE + 662,
    FIXED_TABLE_SIZE + 670,
    FIXED_TABLE_SIZE + 686,
    FIXED_TABLE_SIZE + 718,
    FIXED_TABLE_SIZE + 782,
    FIXED_TABLE_SIZE + 912,
    FIXED_TABLE_SIZE + 1168,
    FIXED_TABLE_SIZE + 1680,
    FIXED_TABLE_SIZE + 2704,
];

//------------------------------------------------------------------------------
// vp8li_dec.h

/// Translation of `VP8LDecodeState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum VP8LDecodeState {
    ReadData = 0,
    ReadHdr = 1,
    ReadDim = 2,
}

/// Translation of `VP8LTransform`.
#[derive(Default)]
pub(crate) struct VP8LTransform {
    /// transform type.
    pub(crate) type_: VP8LImageTransformType,
    /// subsampling bits defining transform window.
    pub(crate) bits: i32,
    /// transform window X index.
    pub(crate) xsize: i32,
    /// transform window Y index.
    pub(crate) ysize: i32,
    /// transform data.
    pub(crate) data: Vec<u32>,
}

/// Translation of `VP8LMetadata` (without the incremental decoder's saved
/// color cache).
#[derive(Default)]
pub(crate) struct VP8LMetadata {
    pub(crate) color_cache_size: i32,
    pub(crate) color_cache: VP8LColorCache,

    pub(crate) huffman_mask: i32,
    pub(crate) huffman_subsample_bits: i32,
    pub(crate) huffman_xsize: i32,
    pub(crate) huffman_image: Vec<u32>,
    pub(crate) num_htree_groups: i32,
    pub(crate) htree_groups: Vec<HTreeGroup>,
    pub(crate) huffman_tables: HuffmanTables,
}

/// Translation of `VP8LDecoder` (owning its `VP8Io`, upstream a pointer to
/// the caller's or the alpha decoder's).
pub(crate) struct VP8LDecoder<'a> {
    pub(crate) status: VP8StatusCode,
    pub(crate) state: VP8LDecodeState,
    pub(crate) io: VP8Io<'a>,

    /// Internal data: BGRA pixels.
    pub(crate) pixels: Vec<u32>,
    /// Internal data: the alpha indices of the 8-bit alpha decoding (a
    /// `uint8_t*` view of `pixels_` upstream).
    pub(crate) pixels8: Vec<u8>,
    /// Scratch buffer for temporary BGRA storage: the top-prediction row,
    /// then the cache rows (at `argb_cache_off`).
    pub(crate) argb_cache: Vec<u32>,
    pub(crate) argb_cache_off: usize,

    pub(crate) br: VP8LBitReader<'a>,

    pub(crate) width: i32,
    pub(crate) height: i32,
    /// last input row decoded so far.
    pub(crate) last_row: i32,
    /// last pixel decoded so far. However, it may not be transformed,
    /// scaled and color-converted yet.
    pub(crate) last_pixel: i32,
    /// last row output so far.
    pub(crate) last_out_row: i32,

    pub(crate) hdr: VP8LMetadata,

    pub(crate) next_transform: usize,
    pub(crate) transforms: [VP8LTransform; NUM_TRANSFORMS],
    /// or'd bitset storing the transforms types.
    pub(crate) transforms_seen: u32,
}

/// Where the decoded rows go: upstream's `ProcessRowsFunc` (NULL,
/// `ProcessRows()` into the output buffer, or `ExtractAlphaRows()` into
/// the alpha decoder's output).
pub(crate) enum RowSink<'s, 'o> {
    None,
    Rows(&'s mut WebPDecBuffer<'o>),
    Alpha(AlphaSink<'s>),
}

/// The alpha decoder's output for `ExtractAlphaRows()` and
/// `ExtractPalettedAlphaRows()`: its plane, filter and last output row
/// (`alph_dec->output_`, `filter_` and `prev_line_`).
pub(crate) struct AlphaSink<'s> {
    pub(crate) output: &'s mut [u8],
    pub(crate) filter: WebpFilterType,
    pub(crate) prev_line: &'s mut Option<usize>,
}

//------------------------------------------------------------------------------

/// Returns true if the next byte(s) in data is a VP8L signature.
/// Translation of `VP8LCheckSignature()`.
pub(crate) fn vp8l_check_signature(data: &[u8]) -> bool {
    data.len() >= VP8L_FRAME_HEADER_SIZE && data[0] as u32 == VP8L_MAGIC_BYTE && (data[4] >> 5) == 0
    // version
}

/// Translation of `ReadImageInfo()`.
fn read_image_info(br: &mut VP8LBitReader<'_>) -> Option<(i32, i32, bool)> {
    if br.read_bits(8) != VP8L_MAGIC_BYTE {
        return None;
    }
    let width = br.read_bits(VP8L_IMAGE_SIZE_BITS) as i32 + 1;
    let height = br.read_bits(VP8L_IMAGE_SIZE_BITS) as i32 + 1;
    let has_alpha = br.read_bits(1) != 0;
    if br.read_bits(VP8L_VERSION_BITS) != 0 {
        return None;
    }
    if br.eos {
        return None;
    }
    Some((width, height, has_alpha))
}

/// Validates the VP8L data-header and retrieves basic header information
/// viz width, height and alpha. Returns `None` in case of formatting
/// error. Translation of `VP8LGetInfo()`.
pub(crate) fn vp8l_get_info(data: &[u8]) -> Option<(i32, i32, bool)> {
    if data.len() < VP8L_FRAME_HEADER_SIZE {
        None // not enough data
    } else if !vp8l_check_signature(data) {
        None // bad signature
    } else {
        let mut br = VP8LBitReader::new(data);
        read_image_info(&mut br)
    }
}

//------------------------------------------------------------------------------

/// Translation of `GetCopyDistance()`.
fn get_copy_distance(distance_symbol: i32, br: &mut VP8LBitReader<'_>) -> i32 {
    if distance_symbol < 4 {
        return distance_symbol + 1;
    }
    let extra_bits = (distance_symbol - 2) >> 1;
    let offset = (2 + (distance_symbol & 1)) << extra_bits;
    offset + br.read_bits(extra_bits) as i32 + 1
}

/// Length and distance prefixes are encoded the same way.
/// Translation of `GetCopyLength()`.
fn get_copy_length(length_symbol: i32, br: &mut VP8LBitReader<'_>) -> i32 {
    get_copy_distance(length_symbol, br)
}

/// Translation of `PlaneCodeToDistance()`.
fn plane_code_to_distance(xsize: i32, plane_code: i32) -> i32 {
    if plane_code > CODE_TO_PLANE_CODES {
        plane_code - CODE_TO_PLANE_CODES
    } else {
        let dist_code = K_CODE_TO_PLANE[(plane_code - 1) as usize] as i32;
        let yoffset = dist_code >> 4;
        let xoffset = 8 - (dist_code & 0xf);
        let dist = yoffset * xsize + xoffset;
        if dist >= 1 {
            dist
        } else {
            1 // dist<1 can happen if xsize is very small
        }
    }
}

//------------------------------------------------------------------------------
// Decodes the next Huffman code from bit-stream.
// VP8LFillBitWindow(br) needs to be called at minimum every second call
// to ReadSymbol, in order to pre-fetch enough bits.

/// Translation of `ReadSymbol()`.
fn read_symbol(table: &[HuffmanCode], br: &mut VP8LBitReader<'_>) -> i32 {
    let mut val = br.prefetch_bits();
    let mut idx = (val & HUFFMAN_TABLE_MASK) as usize;
    let nbits = table[idx].bits as i32 - HUFFMAN_TABLE_BITS;
    if nbits > 0 {
        br.set_bit_pos(br.bit_pos + HUFFMAN_TABLE_BITS);
        val = br.prefetch_bits();
        idx += table[idx].value as usize;
        idx += (val & ((1 << nbits) - 1)) as usize;
    }
    br.set_bit_pos(br.bit_pos + table[idx].bits as i32);
    table[idx].value as i32
}

// Reads packed symbol depending on GREEN channel
/// something large enough (and a bit-mask)
const BITS_SPECIAL_MARKER: i32 = 0x100;
/// must be < NUM_LITERAL_CODES
const PACKED_NON_LITERAL_CODE: i32 = 0;

/// Translation of `ReadPackedSymbols()`.
fn read_packed_symbols(group: &HTreeGroup, br: &mut VP8LBitReader<'_>, dst: &mut u32) -> i32 {
    let val = br.prefetch_bits() & (HUFFMAN_PACKED_TABLE_SIZE as u32 - 1);
    let code = group.packed_table[val as usize];
    debug_assert!(group.use_packed_table);
    if code.bits < BITS_SPECIAL_MARKER {
        br.set_bit_pos(br.bit_pos + code.bits);
        *dst = code.value;
        PACKED_NON_LITERAL_CODE
    } else {
        br.set_bit_pos(br.bit_pos + code.bits - BITS_SPECIAL_MARKER);
        debug_assert!(code.value >= NUM_LITERAL_CODES as u32);
        code.value as i32
    }
}

/// Translation of `AccumulateHCode()`.
fn accumulate_hcode(hcode: HuffmanCode, shift: i32, huff: &mut HuffmanCode32) -> i32 {
    huff.bits += hcode.bits as i32;
    huff.value |= (hcode.value as u32) << shift;
    debug_assert!(huff.bits <= HUFFMAN_TABLE_BITS);
    hcode.bits as i32
}

/// Translation of `BuildPackedTable()`.
fn build_packed_table(htree_group: &mut HTreeGroup, tables: &HuffmanTables) {
    for code in 0..HUFFMAN_PACKED_TABLE_SIZE as u32 {
        let mut bits = code;
        let htree = |i: usize, b: u32| tables.table(htree_group.htrees[i])[b as usize];
        let hcode = htree(GREEN, bits);
        let mut huff = HuffmanCode32::default();
        if hcode.value as i32 >= NUM_LITERAL_CODES {
            huff.bits = hcode.bits as i32 + BITS_SPECIAL_MARKER;
            huff.value = hcode.value as u32;
        } else {
            huff.bits = 0;
            huff.value = 0;
            bits >>= accumulate_hcode(hcode, 8, &mut huff);
            bits >>= accumulate_hcode(htree(RED, bits), 16, &mut huff);
            bits >>= accumulate_hcode(htree(BLUE, bits), 0, &mut huff);
            bits >>= accumulate_hcode(htree(ALPHA, bits), 24, &mut huff);
            let _ = bits;
        }
        htree_group.packed_table[code as usize] = huff;
    }
}

/// Translation of `ReadHuffmanCodeLengths()`.
fn read_huffman_code_lengths(
    dec: &mut VP8LDecoder<'_>,
    code_length_code_lengths: &[i32; NUM_CODE_LENGTH_CODES],
    num_symbols: i32,
    code_lengths: &mut [i32],
) -> bool {
    let mut ok = false;
    let br = &mut dec.br;
    let mut prev_code_len = crate::webp::decode::DEFAULT_CODE_LENGTH;
    let mut tables = HuffmanTables::default();

    'end: {
        if !vp8l_huffman_tables_allocate(1 << LENGTHS_TABLE_BITS, &mut tables)
            || vp8l_build_huffman_table(
                Some(&mut tables),
                LENGTHS_TABLE_BITS,
                code_length_code_lengths,
            ) == 0
        {
            break 'end;
        }

        let mut max_symbol;
        if br.read_bits(1) != 0 {
            // use length
            let length_nbits = 2 + 2 * br.read_bits(3) as i32;
            max_symbol = 2 + br.read_bits(length_nbits) as i32;
            if max_symbol > num_symbols {
                break 'end;
            }
        } else {
            max_symbol = num_symbols;
        }

        let mut symbol = 0;
        while symbol < num_symbols {
            if max_symbol == 0 {
                break;
            }
            max_symbol -= 1;
            br.fill_bit_window();
            let p = tables.segments[tables.curr_segment].start
                [(br.prefetch_bits() & LENGTHS_TABLE_MASK) as usize];
            br.set_bit_pos(br.bit_pos + p.bits as i32);
            let code_len = p.value as i32;
            if code_len < K_CODE_LENGTH_LITERALS {
                code_lengths[symbol as usize] = code_len;
                symbol += 1;
                if code_len != 0 {
                    prev_code_len = code_len;
                }
            } else {
                let use_prev = code_len == K_CODE_LENGTH_REPEAT_CODE;
                let slot = (code_len - K_CODE_LENGTH_LITERALS) as usize;
                let extra_bits = K_CODE_LENGTH_EXTRA_BITS[slot] as i32;
                let repeat_offset = K_CODE_LENGTH_REPEAT_OFFSETS[slot] as i32;
                let mut repeat = br.read_bits(extra_bits) as i32 + repeat_offset;
                if symbol + repeat > num_symbols {
                    break 'end;
                } else {
                    let length = if use_prev { prev_code_len } else { 0 };
                    while repeat > 0 {
                        repeat -= 1;
                        code_lengths[symbol as usize] = length;
                        symbol += 1;
                    }
                }
            }
        }
        ok = true;
    }

    vp8l_huffman_tables_deallocate(&mut tables);
    if !ok {
        dec.status = VP8StatusCode::BitstreamError;
    }
    ok
}

/// 'code_lengths' is pre-allocated temporary buffer, used for creating
/// Huffman tree. Translation of `ReadHuffmanCode()`.
fn read_huffman_code(
    alphabet_size: i32,
    dec: &mut VP8LDecoder<'_>,
    code_lengths: &mut [i32],
    table: Option<&mut HuffmanTables>,
) -> i32 {
    let mut ok;
    let mut size = 0;
    let br = &mut dec.br;
    let simple_code = br.read_bits(1) != 0;

    code_lengths[..alphabet_size as usize].fill(0);

    if simple_code {
        // Read symbols, codes & code lengths directly.
        let num_symbols = br.read_bits(1) + 1;
        let first_symbol_len_code = br.read_bits(1);
        // The first code is either 1 bit or 8 bit code.
        let mut symbol = br.read_bits(if first_symbol_len_code == 0 { 1 } else { 8 });
        code_lengths[symbol as usize] = 1;
        // The second code (if present), is always 8 bits long.
        if num_symbols == 2 {
            symbol = br.read_bits(8);
            code_lengths[symbol as usize] = 1;
        }
        ok = true;
    } else {
        // Decode Huffman-coded code lengths.
        let mut code_length_code_lengths = [0i32; NUM_CODE_LENGTH_CODES];
        let num_codes = br.read_bits(4) as usize + 4;
        if num_codes > NUM_CODE_LENGTH_CODES {
            dec.status = VP8StatusCode::BitstreamError;
            return 0;
        }

        for &order in &K_CODE_LENGTH_CODE_ORDER[..num_codes] {
            code_length_code_lengths[order as usize] = br.read_bits(3) as i32;
        }
        ok = read_huffman_code_lengths(dec, &code_length_code_lengths, alphabet_size, code_lengths);
    }

    ok = ok && !dec.br.eos;
    if ok {
        size = vp8l_build_huffman_table(
            table,
            HUFFMAN_TABLE_BITS,
            &code_lengths[..alphabet_size as usize],
        );
    }
    if !ok || size == 0 {
        dec.status = VP8StatusCode::BitstreamError;
        return 0;
    }
    size
}

/// Translation of `ReadHuffmanCodes()`.
fn read_huffman_codes(
    dec: &mut VP8LDecoder<'_>,
    xsize: i32,
    ysize: i32,
    color_cache_bits: i32,
    allow_recursion: bool,
) -> bool {
    let mut huffman_image: Vec<u32> = Vec::new();
    let mut num_htree_groups: i32 = 1;
    let mut num_htree_groups_max: i32 = 1;
    let mut max_alphabet_size: i32 = 0;
    let table_size = K_TABLE_SIZE[color_cache_bits as usize] as i32;
    let mut mapping: Option<Vec<i32>> = None;
    let mut ok = false;

    // Check the table has been 0 initialized (through InitMetadata).
    debug_assert!(dec.hdr.huffman_tables.segments.is_empty());

    'error: {
        if allow_recursion && dec.br.read_bits(1) != 0 {
            // use meta Huffman codes.
            let huffman_precision = dec.br.read_bits(3) as i32 + 2;
            let huffman_xsize = vp8l_sub_sample_size(xsize as u32, huffman_precision as u32) as i32;
            let huffman_ysize = vp8l_sub_sample_size(ysize as u32, huffman_precision as u32) as i32;
            let huffman_pixs = (huffman_xsize * huffman_ysize) as usize;
            if !decode_image_stream(
                huffman_xsize,
                huffman_ysize,
                false,
                dec,
                Some(&mut huffman_image),
            ) {
                break 'error;
            }
            dec.hdr.huffman_subsample_bits = huffman_precision;
            for pixel in &mut huffman_image[..huffman_pixs] {
                // The huffman data is stored in red and green bytes.
                let group = ((*pixel >> 8) & 0xffff) as i32;
                *pixel = group as u32;
                if group >= num_htree_groups_max {
                    num_htree_groups_max = group + 1;
                }
            }
            // Check the validity of num_htree_groups_max. If it seems too big, use a
            // smaller value for later. This will prevent big memory allocations to end
            // up with a bad bitstream anyway.
            // The value of 1000 is totally arbitrary. We know that num_htree_groups_max
            // is smaller than (1 << 16) and should be smaller than the number of pixels
            // (though the format allows it to be bigger).
            if num_htree_groups_max > 1000 || num_htree_groups_max > xsize * ysize {
                // Create a mapping from the used indices to the minimal set of used
                // values [0, num_htree_groups)
                // -1 means a value is unmapped, and therefore unused in the Huffman
                // image.
                let Some(mut m) = safe_alloc(num_htree_groups_max as u64, -1i32) else {
                    dec.status = VP8StatusCode::OutOfMemory;
                    break 'error;
                };
                num_htree_groups = 0;
                for pixel in &mut huffman_image[..huffman_pixs] {
                    // Get the current mapping for the group and remap the Huffman image.
                    let mapped_group = &mut m[*pixel as usize];
                    if *mapped_group == -1 {
                        *mapped_group = num_htree_groups;
                        num_htree_groups += 1;
                    }
                    *pixel = *mapped_group as u32;
                }
                mapping = Some(m);
            } else {
                num_htree_groups = num_htree_groups_max;
            }
        }

        if dec.br.eos {
            break 'error;
        }

        // Find maximum alphabet size for the htree group.
        for (j, &size) in K_ALPHABET_SIZE.iter().enumerate() {
            let mut alphabet_size = size as i32;
            if j == 0 && color_cache_bits > 0 {
                alphabet_size += 1 << color_cache_bits;
            }
            if max_alphabet_size < alphabet_size {
                max_alphabet_size = alphabet_size;
            }
        }

        let code_lengths = safe_alloc(max_alphabet_size as u64, 0i32);
        let groups = vp8l_htree_groups_new(num_htree_groups);

        let (Some(mut code_lengths), Some(groups)) = (code_lengths, groups) else {
            dec.status = VP8StatusCode::OutOfMemory;
            break 'error;
        };
        let mut htree_groups: Vec<HTreeGroup> = groups;
        if !vp8l_huffman_tables_allocate(num_htree_groups * table_size, &mut dec.hdr.huffman_tables)
        {
            dec.status = VP8StatusCode::OutOfMemory;
            break 'error;
        }

        for i in 0..num_htree_groups_max as usize {
            // If the index "i" is unused in the Huffman image, just make sure the
            // coefficients are valid but do not store them.
            if mapping.as_ref().is_some_and(|m| m[i] == -1) {
                for (j, &size) in K_ALPHABET_SIZE.iter().enumerate() {
                    let mut alphabet_size = size as i32;
                    if j == 0 && color_cache_bits > 0 {
                        alphabet_size += 1 << color_cache_bits;
                    }
                    // Passing in NULL so that nothing gets filled.
                    if read_huffman_code(alphabet_size, dec, &mut code_lengths, None) == 0 {
                        break 'error;
                    }
                }
            } else {
                let g = match &mapping {
                    None => i,
                    Some(m) => m[i] as usize,
                };
                let mut total_size = 0;
                let mut is_trivial_literal = true;
                let mut max_bits = 0;
                for (j, &size) in K_ALPHABET_SIZE.iter().enumerate() {
                    let mut alphabet_size = size as i32;
                    if j == 0 && color_cache_bits > 0 {
                        alphabet_size += 1 << color_cache_bits;
                    }
                    let mut tables = std::mem::take(&mut dec.hdr.huffman_tables);
                    let size =
                        read_huffman_code(alphabet_size, dec, &mut code_lengths, Some(&mut tables));
                    dec.hdr.huffman_tables = tables;
                    let tables = &mut dec.hdr.huffman_tables;
                    htree_groups[g].htrees[j] = tables.curr_table();
                    if size == 0 {
                        break 'error;
                    }
                    let first = tables.table(htree_groups[g].htrees[j])[0];
                    if is_trivial_literal && K_LITERAL_MAP[j] == 1 {
                        is_trivial_literal = first.bits == 0;
                    }
                    total_size += first.bits as i32;
                    let curr = tables.curr_segment;
                    tables.segments[curr].curr_table += size as usize;
                    if j <= ALPHA {
                        let mut local_max_bits = code_lengths[0];
                        for &len in &code_lengths[1..alphabet_size as usize] {
                            if len > local_max_bits {
                                local_max_bits = len;
                            }
                        }
                        max_bits += local_max_bits;
                    }
                }
                let tables = &dec.hdr.huffman_tables;
                let htree_group = &mut htree_groups[g];
                htree_group.is_trivial_literal = is_trivial_literal;
                htree_group.is_trivial_code = false;
                if is_trivial_literal {
                    let first = |j: usize| tables.table(htree_group.htrees[j])[0].value as u32;
                    let red = first(RED);
                    let blue = first(BLUE);
                    let alpha = first(ALPHA);
                    let green = first(GREEN);
                    htree_group.literal_arb = (alpha << 24) | (red << 16) | blue;
                    if total_size == 0 && green < NUM_LITERAL_CODES as u32 {
                        htree_group.is_trivial_code = true;
                        htree_group.literal_arb |= green << 8;
                    }
                }
                htree_group.use_packed_table =
                    !htree_group.is_trivial_code && (max_bits < HUFFMAN_PACKED_BITS);
                if htree_group.use_packed_table {
                    build_packed_table(htree_group, tables);
                }
            }
        }
        ok = true;

        // All OK. Finalize pointers.
        dec.hdr.huffman_image = std::mem::take(&mut huffman_image);
        dec.hdr.num_htree_groups = num_htree_groups;
        dec.hdr.htree_groups = std::mem::take(&mut htree_groups);
    }

    if !ok {
        vp8l_huffman_tables_deallocate(&mut dec.hdr.huffman_tables);
    }
    ok
}

//------------------------------------------------------------------------------
// Emit rows without any scaling.

/// Translation of `EmitRows()`: `mb_h` rows of `mb_w` pixels at `row_in`
/// in `cache` (`in_stride` pixels apart) to `out` at `out_off`.
#[allow(clippy::too_many_arguments)]
fn emit_rows(
    colorspace: WebpCspMode,
    cache: &[u32],
    mut row_in: usize,
    in_stride: usize,
    mb_w: i32,
    mb_h: i32,
    out: &mut [u8],
    mut out_off: usize,
    out_stride: usize,
) -> i32 {
    let mut lines = mb_h;
    while lines > 0 {
        lines -= 1;
        vp8l_convert_from_bgra(
            &cache[row_in..],
            mb_w as usize,
            colorspace,
            &mut out[out_off..],
        );
        row_in += in_stride;
        out_off += out_stride;
    }
    mb_h // Num rows out == num rows in.
}

//------------------------------------------------------------------------------
// Cropping.

/// Sets io->mb_y, io->mb_h & io->mb_w according to start row, end row and
/// crop options. Also updates the input data pointer, so that it points to
/// the start of the cropped window. Note that pixels are in ARGB format
/// even if 'in_data' is uint8_t*. Returns true if the crop window is not
/// empty. Translation of `SetCropWindow()` (`in_data` and `pixel_stride`
/// in pixels).
fn set_crop_window(
    io: &mut VP8Io<'_>,
    mut y_start: i32,
    mut y_end: i32,
    in_data: &mut usize,
    pixel_stride: usize,
) -> bool {
    debug_assert!(y_start < y_end);
    debug_assert!(io.crop_left < io.crop_right);
    if y_end > io.crop_bottom {
        y_end = io.crop_bottom; // make sure we don't overflow on last row.
    }
    if y_start < io.crop_top {
        let delta = io.crop_top - y_start;
        y_start = io.crop_top;
        *in_data += delta as usize * pixel_stride;
    }
    if y_start >= y_end {
        return false; // Crop window is empty.
    }

    *in_data += io.crop_left as usize;

    io.mb_y = y_start - io.crop_top;
    io.mb_w = io.crop_right - io.crop_left;
    io.mb_h = y_end - y_start;
    true // Non-empty crop window.
}

//------------------------------------------------------------------------------

/// Translation of `GetMetaIndex()`.
fn get_meta_index(image: &[u32], xsize: i32, bits: i32, x: i32, y: i32) -> usize {
    if bits == 0 {
        return 0;
    }
    image[(xsize * (y >> bits) + (x >> bits)) as usize] as usize
}

/// Translation of `GetHtreeGroupForPos()`: the group's index.
fn get_htree_group_for_pos(hdr: &VP8LMetadata, x: i32, y: i32) -> usize {
    let meta_index = get_meta_index(
        &hdr.huffman_image,
        hdr.huffman_xsize,
        hdr.huffman_subsample_bits,
        x,
        y,
    );
    debug_assert!((meta_index as i32) < hdr.num_htree_groups);
    meta_index
}

//------------------------------------------------------------------------------
// Main loop, with custom row-processing function

/// Translation of `ApplyInverseTransforms()`: the rows (from `rows`, the
/// decoded pixels) into the argb cache, transformed there.
fn apply_inverse_transforms(
    dec: &mut VP8LDecoder<'_>,
    start_row: i32,
    num_rows: i32,
    rows: &[u32],
) {
    let cache_pixs = (dec.width * num_rows) as usize;
    let end_row = start_row + num_rows;
    let rows_out = dec.argb_cache_off;

    // (the input rows are copied to the output first, each transform then
    // working in place; with no transform, this is upstream's plain copy)
    dec.argb_cache[rows_out..rows_out + cache_pixs].copy_from_slice(&rows[..cache_pixs]);
    // Inverse transforms.
    let mut n = dec.next_transform;
    while n > 0 {
        n -= 1;
        let transform = &dec.transforms[n];
        vp8l_inverse_transform(transform, start_row, end_row, &mut dec.argb_cache, rows_out);
    }
}

/// Processes (transforms, scales & color-converts) the rows decoded after
/// the last call. Translation of `ProcessRows()`.
fn process_rows(
    dec: &mut VP8LDecoder<'_>,
    pixels: &[u32],
    row: i32,
    output: &mut WebPDecBuffer<'_>,
) {
    let rows = (dec.width * dec.last_row) as usize;
    let num_rows = row - dec.last_row;

    debug_assert!(row <= dec.io.crop_bottom);
    // We can't process more than NUM_ARGB_CACHE_ROWS at a time (that's the size
    // of argb_cache_), but we currently don't need more than that.
    debug_assert!(num_rows <= NUM_ARGB_CACHE_ROWS);
    if num_rows > 0 {
        // Emit output.
        let mut rows_data = dec.argb_cache_off;
        let in_stride = dec.io.width as usize; // in unit of RGBA
        apply_inverse_transforms(dec, dec.last_row, num_rows, &pixels[rows..]);
        let last_row = dec.last_row;
        if !set_crop_window(&mut dec.io, last_row, row, &mut rows_data, in_stride) {
            // Nothing to output (this time).
        } else {
            // (WebPIsRGBMode(output->colorspace): the YUVA export is not translated)
            let stride = output.rgba.stride as usize;
            let rgba = dec.last_out_row as usize * stride;
            let num_rows_out = emit_rows(
                output.colorspace,
                &dec.argb_cache,
                rows_data,
                in_stride,
                dec.io.mb_w,
                dec.io.mb_h,
                output.rgba.rgba,
                rgba,
                stride,
            );
            // Update 'last_out_row_'.
            dec.last_out_row += num_rows_out;
            debug_assert!(dec.last_out_row <= output.height);
        }
    }

    // Update 'last_row_'.
    dec.last_row = row;
    debug_assert!(dec.last_row <= dec.height);
}

/// Row-processing for the special case when alpha data contains only one
/// transform (color indexing), and trivial non-green literals.
/// Translation of `Is8bOptimizable()`.
fn is_8b_optimizable(hdr: &VP8LMetadata) -> bool {
    if hdr.color_cache_size > 0 {
        return false;
    }
    // When the Huffman tree contains only one symbol, we can skip the
    // call to ReadSymbol() for red/blue/alpha channels.
    for group in &hdr.htree_groups[..hdr.num_htree_groups as usize] {
        let htrees = &group.htrees;
        let tables = &hdr.huffman_tables;
        if tables.table(htrees[RED])[0].bits > 0 {
            return false;
        }
        if tables.table(htrees[BLUE])[0].bits > 0 {
            return false;
        }
        if tables.table(htrees[ALPHA])[0].bits > 0 {
            return false;
        }
    }
    true
}

/// Translation of `AlphaApplyFilter()`.
fn alpha_apply_filter(
    sink: &mut AlphaSink<'_>,
    first_row: i32,
    last_row: i32,
    mut out: usize,
    stride: usize,
) {
    if sink.filter != WebpFilterType::None {
        let mut prev_line = *sink.prev_line;
        for _ in first_row..last_row {
            webp_unfilter(sink.filter, sink.output, prev_line, out, stride);
            prev_line = Some(out);
            out += stride;
        }
        *sink.prev_line = prev_line;
    }
}

/// Translation of `ExtractPalettedAlphaRows()`.
fn extract_paletted_alpha_rows(
    dec: &mut VP8LDecoder<'_>,
    pixels8: &[u8],
    last_row: i32,
    sink: &mut AlphaSink<'_>,
) {
    // For vertical and gradient filtering, we need to decode the part above the
    // crop_top row, in order to have the correct spatial predictors.
    let top_row =
        if sink.filter == WebpFilterType::None || sink.filter == WebpFilterType::Horizontal {
            dec.io.crop_top
        } else {
            dec.last_row
        };
    let first_row = if dec.last_row < top_row {
        top_row
    } else {
        dec.last_row
    };
    debug_assert!(last_row <= dec.io.crop_bottom);
    if last_row > first_row {
        // Special method for paletted alpha data. We only process the cropped area.
        let width = dec.io.width as usize;
        let out = width * first_row as usize;
        let input = dec.width as usize * first_row as usize;
        let transform = &dec.transforms[0];
        debug_assert!(dec.next_transform == 1);
        debug_assert!(transform.type_ == VP8LImageTransformType::ColorIndexingTransform);
        vp8l_color_index_inverse_transform_alpha(
            transform,
            first_row,
            last_row,
            &pixels8[input..],
            &mut sink.output[out..],
        );
        alpha_apply_filter(sink, first_row, last_row, out, width);
    }
    dec.last_row = last_row;
    dec.last_out_row = last_row;
}

//------------------------------------------------------------------------------
// Helper functions for fast pattern copy (8b and 32b)

/// Translation of `CopyBlock8b()`: upstream's pattern copies (for
/// distances of 1, 2 and 4) and its `memcpy()` give what this forward
/// copy gives.
fn copy_block_8b(data: &mut [u8], dst: usize, dist: usize, length: usize) {
    let src = dst - dist;
    if dist >= length {
        // no overlap -> use memcpy()
        data.copy_within(src..src + length, dst);
    } else {
        for i in 0..length {
            data[dst + i] = data[src + i];
        }
    }
}

/// Translation of `CopyBlock32b()` (see [`copy_block_8b`]).
fn copy_block_32b(data: &mut [u32], dst: usize, dist: usize, length: usize) {
    let src = dst - dist;
    if dist >= length {
        // no overlap
        data.copy_within(src..src + length, dst);
    } else {
        for i in 0..length {
            data[dst + i] = data[src + i];
        }
    }
}

//------------------------------------------------------------------------------

/// Translation of `DecodeAlphaData()`.
fn decode_alpha_data(
    dec: &mut VP8LDecoder<'_>,
    data: &mut [u8],
    width: i32,
    height: i32,
    last_row: i32,
    sink: &mut AlphaSink<'_>,
) -> bool {
    let mut ok = true;
    let mut row = dec.last_pixel / width;
    let mut col = dec.last_pixel % width;
    let mut pos = dec.last_pixel; // current position
    let end = width * height; // End of data
    let last = width * last_row; // Last pixel to decode
    let len_code_limit = NUM_LITERAL_CODES + NUM_LENGTH_CODES;
    let mask = dec.hdr.huffman_mask;
    let mut htree_group = if pos < last {
        get_htree_group_for_pos(&dec.hdr, col, row)
    } else {
        0
    };
    debug_assert!(pos <= end);
    debug_assert!(last_row <= height);
    debug_assert!(is_8b_optimizable(&dec.hdr));

    'end: {
        while !dec.br.eos && pos < last {
            // Only update when changing tile.
            if (col & mask) == 0 {
                htree_group = get_htree_group_for_pos(&dec.hdr, col, row);
            }
            dec.br.fill_bit_window();
            let group = &dec.hdr.htree_groups[htree_group];
            let tables = &dec.hdr.huffman_tables;
            let code = read_symbol(tables.table(group.htrees[GREEN]), &mut dec.br);
            if code < NUM_LITERAL_CODES {
                // Literal
                data[pos as usize] = code as u8;
                pos += 1;
                col += 1;
                if col >= width {
                    col = 0;
                    row += 1;
                    if row <= last_row && (row % NUM_ARGB_CACHE_ROWS == 0) {
                        extract_paletted_alpha_rows(dec, data, row, sink);
                    }
                }
            } else if code < len_code_limit {
                // Backward reference
                let length_sym = code - NUM_LITERAL_CODES;
                let length = get_copy_length(length_sym, &mut dec.br);
                let dist_symbol = read_symbol(tables.table(group.htrees[DIST]), &mut dec.br);
                dec.br.fill_bit_window();
                let dist_code = get_copy_distance(dist_symbol, &mut dec.br);
                let dist = plane_code_to_distance(width, dist_code);
                if pos >= dist && end - pos >= length {
                    copy_block_8b(data, pos as usize, dist as usize, length as usize);
                } else {
                    ok = false;
                    break 'end;
                }
                pos += length;
                col += length;
                while col >= width {
                    col -= width;
                    row += 1;
                    if row <= last_row && (row % NUM_ARGB_CACHE_ROWS == 0) {
                        extract_paletted_alpha_rows(dec, data, row, sink);
                    }
                }
                if pos < last && (col & mask) != 0 {
                    htree_group = get_htree_group_for_pos(&dec.hdr, col, row);
                }
            } else {
                // Not reached
                ok = false;
                break 'end;
            }
            dec.br.eos = dec.br.is_end_of_stream();
        }
        // Process the remaining rows corresponding to last row-block.
        extract_paletted_alpha_rows(dec, data, if row > last_row { last_row } else { row }, sink);
    }

    dec.br.eos = dec.br.is_end_of_stream();
    if !ok || (dec.br.eos && pos < end) {
        ok = false;
        dec.status = if dec.br.eos {
            VP8StatusCode::Suspended
        } else {
            VP8StatusCode::BitstreamError
        };
    } else {
        dec.last_pixel = pos;
    }
    ok
}

/// Translation of `DecodeImageData()`: decodes `data` (upstream's
/// `pixels_` when `sink` processes rows, else a sub-image's buffer).
fn decode_image_data(
    dec: &mut VP8LDecoder<'_>,
    data: &mut [u32],
    width: i32,
    height: i32,
    last_row: i32,
    sink: &mut RowSink<'_, '_>,
) -> bool {
    let mut row = dec.last_pixel / width;
    let mut col = dec.last_pixel % width;
    let mut src = dec.last_pixel as usize;
    let mut last_cached = src;
    let src_end = (width * height) as usize; // End of data
    let src_last = (width * last_row) as usize; // Last pixel to decode
    let len_code_limit = NUM_LITERAL_CODES + NUM_LENGTH_CODES;
    let color_cache_limit = len_code_limit + dec.hdr.color_cache_size;
    let has_color_cache = dec.hdr.color_cache_size > 0;
    let mask = dec.hdr.huffman_mask;
    let mut htree_group = if src < src_last {
        get_htree_group_for_pos(&dec.hdr, col, row)
    } else {
        0
    };
    debug_assert!(dec.last_row < last_row);
    debug_assert!(src_last <= src_end);

    /// Translation of `process_func(dec, row)`.
    fn process(dec: &mut VP8LDecoder<'_>, data: &[u32], row: i32, sink: &mut RowSink<'_, '_>) {
        match sink {
            RowSink::None => {}
            RowSink::Rows(output) => process_rows(dec, data, row, output),
            RowSink::Alpha(alpha) => extract_alpha_rows(dec, data, row, alpha),
        }
    }
    let has_process = !matches!(sink, RowSink::None);

    'error: {
        while src < src_last {
            // Only update when changing tile. Note we could use this test:
            // if "((((prev_col ^ col) | prev_row ^ row)) > mask)" -> tile changed
            // but that's actually slower and needs storing the previous col/row.
            if (col & mask) == 0 {
                htree_group = get_htree_group_for_pos(&dec.hdr, col, row);
            }
            let group = &dec.hdr.htree_groups[htree_group];
            let tables = &dec.hdr.huffman_tables;
            let mut advance_by_one = false;
            let mut code = 0;
            if group.is_trivial_code {
                data[src] = group.literal_arb;
                advance_by_one = true;
            } else {
                dec.br.fill_bit_window();
                if group.use_packed_table {
                    code = read_packed_symbols(group, &mut dec.br, &mut data[src]);
                    if dec.br.is_end_of_stream() {
                        break;
                    }
                    if code == PACKED_NON_LITERAL_CODE {
                        advance_by_one = true;
                    }
                } else {
                    code = read_symbol(tables.table(group.htrees[GREEN]), &mut dec.br);
                }
            }
            if !advance_by_one {
                if dec.br.is_end_of_stream() {
                    break;
                }
                if code < NUM_LITERAL_CODES {
                    // Literal
                    if group.is_trivial_literal {
                        data[src] = group.literal_arb | ((code as u32) << 8);
                    } else {
                        let red = read_symbol(tables.table(group.htrees[RED]), &mut dec.br);
                        dec.br.fill_bit_window();
                        let blue = read_symbol(tables.table(group.htrees[BLUE]), &mut dec.br);
                        let alpha = read_symbol(tables.table(group.htrees[ALPHA]), &mut dec.br);
                        if dec.br.is_end_of_stream() {
                            break;
                        }
                        data[src] = ((alpha as u32) << 24)
                            | ((red as u32) << 16)
                            | ((code as u32) << 8)
                            | blue as u32;
                    }
                    advance_by_one = true;
                } else if code < len_code_limit {
                    // Backward reference
                    let length_sym = code - NUM_LITERAL_CODES;
                    let length = get_copy_length(length_sym, &mut dec.br) as usize;
                    let dist_symbol = read_symbol(tables.table(group.htrees[DIST]), &mut dec.br);
                    dec.br.fill_bit_window();
                    let dist_code = get_copy_distance(dist_symbol, &mut dec.br);
                    let dist = plane_code_to_distance(width, dist_code) as usize;

                    if dec.br.is_end_of_stream() {
                        break;
                    }
                    if src < dist || src_end - src < length {
                        break 'error;
                    } else {
                        copy_block_32b(data, src, dist, length);
                    }
                    src += length;
                    col += length as i32;
                    while col >= width {
                        col -= width;
                        row += 1;
                        if has_process && row <= last_row && (row % NUM_ARGB_CACHE_ROWS == 0) {
                            process(dec, data, row, sink);
                        }
                    }
                    // Because of the check done above (before 'src' was incremented by
                    // 'length'), the following holds true.
                    debug_assert!(src <= src_end);
                    if (col & mask) != 0 {
                        htree_group = get_htree_group_for_pos(&dec.hdr, col, row);
                    }
                    if has_color_cache {
                        while last_cached < src {
                            dec.hdr.color_cache.insert(data[last_cached]);
                            last_cached += 1;
                        }
                    }
                } else if code < color_cache_limit {
                    // Color cache
                    let key = (code - len_code_limit) as u32;
                    debug_assert!(has_color_cache);
                    while last_cached < src {
                        dec.hdr.color_cache.insert(data[last_cached]);
                        last_cached += 1;
                    }
                    data[src] = dec.hdr.color_cache.lookup(key);
                    advance_by_one = true;
                } else {
                    // Not reached
                    break 'error;
                }
            }
            if advance_by_one {
                // AdvanceByOne:
                src += 1;
                col += 1;
                if col >= width {
                    col = 0;
                    row += 1;
                    if has_process && row <= last_row && (row % NUM_ARGB_CACHE_ROWS == 0) {
                        process(dec, data, row, sink);
                    }
                    if has_color_cache {
                        while last_cached < src {
                            dec.hdr.color_cache.insert(data[last_cached]);
                            last_cached += 1;
                        }
                    }
                }
            }
        }

        dec.br.eos = dec.br.is_end_of_stream();
        // (not incremental: past the end of buffer (eos_=1), this is a real
        // bitstream error)
        if !dec.br.eos {
            // Process the remaining rows corresponding to last row-block.
            if has_process {
                process(dec, data, if row > last_row { last_row } else { row }, sink);
            }
            dec.status = VP8StatusCode::Ok;
            dec.last_pixel = src as i32; // end-of-scan marker
        } else {
            // if not incremental, and we are past the end of buffer (eos_=1), then this
            // is a real bitstream error.
            break 'error;
        }
        return true;
    }

    dec.status = VP8StatusCode::BitstreamError;
    false
}

// -----------------------------------------------------------------------------
// VP8LTransform

/// Translation of `ClearTransform()`.
fn clear_transform(transform: &mut VP8LTransform) {
    transform.data = Vec::new();
}

/// For security reason, we need to remap the color map to span the total
/// possible bundled values, and not just the num_colors. Translation of
/// `ExpandColorMap()`.
fn expand_color_map(num_colors: i32, transform: &mut VP8LTransform) -> bool {
    let final_num_colors = 1 << (8 >> transform.bits);
    let Some(mut new_color_map) = safe_alloc(final_num_colors as u64, 0u32) else {
        return false;
    };
    let data = &transform.data;
    new_color_map[0] = data[0];
    for i in 1..num_colors as usize {
        // Equivalent to VP8LAddPixels(), on a byte-basis.
        new_color_map[i] = vp8l_add_pixels(data[i], new_color_map[i - 1]);
    }
    // (the rest is the black tail: zeroes)
    transform.data = new_color_map;
    true
}

/// Translation of `ReadTransform()`.
fn read_transform(xsize: &mut i32, ysize: i32, dec: &mut VP8LDecoder<'_>) -> bool {
    let mut ok = true;
    let t = dec.next_transform;
    let type_ = match dec.br.read_bits(2) {
        0 => VP8LImageTransformType::PredictorTransform,
        1 => VP8LImageTransformType::CrossColorTransform,
        2 => VP8LImageTransformType::SubtractGreenTransform,
        _ => VP8LImageTransformType::ColorIndexingTransform,
    };

    // Each transform type can only be present once in the stream.
    if dec.transforms_seen & (1 << type_ as u32) != 0 {
        return false; // Already there, let's not accept the second same transform.
    }
    dec.transforms_seen |= 1 << type_ as u32;

    {
        let transform = &mut dec.transforms[t];
        transform.type_ = type_;
        transform.xsize = *xsize;
        transform.ysize = ysize;
        transform.data = Vec::new();
    }
    dec.next_transform += 1;
    debug_assert!(dec.next_transform <= NUM_TRANSFORMS);

    match type_ {
        VP8LImageTransformType::PredictorTransform
        | VP8LImageTransformType::CrossColorTransform => {
            let bits = dec.br.read_bits(3) as i32 + 2;
            dec.transforms[t].bits = bits;
            let (txsize, tysize) = (dec.transforms[t].xsize, dec.transforms[t].ysize);
            let mut data = Vec::new();
            ok = decode_image_stream(
                vp8l_sub_sample_size(txsize as u32, bits as u32) as i32,
                vp8l_sub_sample_size(tysize as u32, bits as u32) as i32,
                false,
                dec,
                Some(&mut data),
            );
            dec.transforms[t].data = data;
        }
        VP8LImageTransformType::ColorIndexingTransform => {
            let num_colors = dec.br.read_bits(8) as i32 + 1;
            let bits = if num_colors > 16 {
                0
            } else if num_colors > 4 {
                1
            } else if num_colors > 2 {
                2
            } else {
                3
            };
            *xsize = vp8l_sub_sample_size(dec.transforms[t].xsize as u32, bits as u32) as i32;
            dec.transforms[t].bits = bits;
            let mut data = Vec::new();
            ok = decode_image_stream(num_colors, 1, false, dec, Some(&mut data));
            dec.transforms[t].data = data;
            ok = ok && expand_color_map(num_colors, &mut dec.transforms[t]);
        }
        VP8LImageTransformType::SubtractGreenTransform => {}
    }

    ok
}

// -----------------------------------------------------------------------------
// VP8LMetadata

/// Translation of `ClearMetadata()` (and `InitMetadata()`).
fn clear_metadata(hdr: &mut VP8LMetadata) {
    vp8l_huffman_tables_deallocate(&mut hdr.huffman_tables);
    vp8l_color_cache_clear(&mut hdr.color_cache);
    *hdr = VP8LMetadata::default();
}

// -----------------------------------------------------------------------------
// VP8LDecoder

/// Allocates and initialize a new lossless decoder instance. Translation
/// of `VP8LNew()`.
pub(crate) fn vp8l_new<'a>() -> VP8LDecoder<'a> {
    VP8LDecoder {
        status: VP8StatusCode::Ok,
        state: VP8LDecodeState::ReadDim,
        io: VP8Io::default(),
        pixels: Vec::new(),
        pixels8: Vec::new(),
        argb_cache: Vec::new(),
        argb_cache_off: 0,
        br: VP8LBitReader::default(),
        width: 0,
        height: 0,
        last_row: 0,
        last_pixel: 0,
        last_out_row: 0,
        hdr: VP8LMetadata::default(),
        next_transform: 0,
        transforms: Default::default(),
        transforms_seen: 0,
    }
}

/// Resets the decoder in its initial state, reclaiming memory. Preserves
/// the dec->status_ value. Translation of `VP8LClear()`.
pub(crate) fn vp8l_clear(dec: &mut VP8LDecoder<'_>) {
    clear_metadata(&mut dec.hdr);

    dec.pixels = Vec::new();
    dec.pixels8 = Vec::new();
    dec.argb_cache = Vec::new();
    for transform in &mut dec.transforms[..dec.next_transform] {
        clear_transform(transform);
    }
    dec.next_transform = 0;
    dec.transforms_seen = 0;
}

/// Translation of `UpdateDecoder()`.
fn update_decoder(dec: &mut VP8LDecoder<'_>, width: i32, height: i32) {
    let hdr = &mut dec.hdr;
    let num_bits = hdr.huffman_subsample_bits;
    dec.width = width;
    dec.height = height;

    hdr.huffman_xsize = vp8l_sub_sample_size(width as u32, num_bits as u32) as i32;
    hdr.huffman_mask = if num_bits == 0 {
        !0
    } else {
        (1 << num_bits) - 1
    };
}

/// Translation of `DecodeImageStream()`.
fn decode_image_stream(
    xsize: i32,
    ysize: i32,
    is_level0: bool,
    dec: &mut VP8LDecoder<'_>,
    decoded_data: Option<&mut Vec<u32>>,
) -> bool {
    let mut ok = true;
    let mut transform_xsize = xsize;
    let transform_ysize = ysize;
    let mut data: Vec<u32> = Vec::new();
    let mut color_cache_bits = 0;

    'end: {
        // Read the transforms (may recurse).
        if is_level0 {
            while ok && dec.br.read_bits(1) != 0 {
                ok = read_transform(&mut transform_xsize, transform_ysize, dec);
            }
        }

        // Color cache
        if ok && dec.br.read_bits(1) != 0 {
            color_cache_bits = dec.br.read_bits(4) as i32;
            ok = (1..=MAX_CACHE_BITS).contains(&color_cache_bits);
            if !ok {
                dec.status = VP8StatusCode::BitstreamError;
                break 'end;
            }
        }

        // Read the Huffman codes (may recurse).
        ok = ok
            && read_huffman_codes(
                dec,
                transform_xsize,
                transform_ysize,
                color_cache_bits,
                is_level0,
            );
        if !ok {
            dec.status = VP8StatusCode::BitstreamError;
            break 'end;
        }

        // Finish setting up the color-cache
        if color_cache_bits > 0 {
            dec.hdr.color_cache_size = 1 << color_cache_bits;
            if !vp8l_color_cache_init(&mut dec.hdr.color_cache, color_cache_bits) {
                dec.status = VP8StatusCode::OutOfMemory;
                ok = false;
                break 'end;
            }
        } else {
            dec.hdr.color_cache_size = 0;
        }
        update_decoder(dec, transform_xsize, transform_ysize);

        if is_level0 {
            // level 0 complete
            dec.state = VP8LDecodeState::ReadHdr;
            break 'end;
        }

        {
            let total_size = transform_xsize as u64 * transform_ysize as u64;
            match safe_alloc(total_size, 0u32) {
                Some(d) => data = d,
                None => {
                    dec.status = VP8StatusCode::OutOfMemory;
                    ok = false;
                    break 'end;
                }
            }
        }

        // Use the Huffman trees to decode the LZ77 encoded data.
        ok = decode_image_data(
            dec,
            &mut data,
            transform_xsize,
            transform_ysize,
            transform_ysize,
            &mut RowSink::None,
        );
        ok = ok && !dec.br.eos;
    }

    if !ok {
        clear_metadata(&mut dec.hdr);
    } else {
        if let Some(decoded_data) = decoded_data {
            *decoded_data = data;
        } else {
            // We allocate image data in this function only for transforms. At level 0
            // (that is: not the transforms), we shouldn't have allocated anything.
            debug_assert!(data.is_empty());
            debug_assert!(is_level0);
        }
        dec.last_pixel = 0; // Reset for future DECODE_DATA_FUNC() calls.
        if !is_level0 {
            clear_metadata(&mut dec.hdr); // Clean up temporary data behind.
        }
    }
    ok
}

//------------------------------------------------------------------------------
/// Allocate internal buffers dec->pixels_ and dec->argb_cache_.
/// Translation of `AllocateInternalBuffers32b()`.
fn allocate_internal_buffers_32b(dec: &mut VP8LDecoder<'_>, final_width: i32) -> bool {
    let num_pixels = dec.width as u64 * dec.height as u64;
    // Scratch buffer corresponding to top-prediction row for transforming the
    // first row in the row-blocks. Not needed for paletted alpha.
    let cache_top_pixels = final_width as u16 as u64;
    // Scratch buffer for temporary BGRA storage. Not needed for paletted alpha.
    let cache_pixels = final_width as u64 * NUM_ARGB_CACHE_ROWS as u64;
    let total_num_pixels = num_pixels + cache_top_pixels + cache_pixels;

    debug_assert!(dec.width <= final_width);
    // (WebPSafeMalloc(total_num_pixels, sizeof(uint32_t))'s size check, then
    // the two vectors)
    let fits = total_num_pixels <= WEBP_MAX_ALLOCABLE_MEMORY / 4
        && check_size_overflow(total_num_pixels * 4);
    let buffers = if fits {
        safe_alloc(num_pixels, 0u32)
            .and_then(|p| Some((p, safe_alloc(cache_top_pixels + cache_pixels, 0u32)?)))
    } else {
        None
    };
    let Some((pixels, argb_cache)) = buffers else {
        dec.status = VP8StatusCode::OutOfMemory;
        return false;
    };
    dec.pixels = pixels;
    dec.argb_cache = argb_cache;
    dec.argb_cache_off = cache_top_pixels as usize;
    true
}

/// Translation of `AllocateInternalBuffers8b()`.
fn allocate_internal_buffers_8b(dec: &mut VP8LDecoder<'_>) -> bool {
    let total_num_pixels = dec.width as u64 * dec.height as u64;
    dec.argb_cache = Vec::new(); // for soundness
    match safe_alloc(total_num_pixels, 0u8) {
        Some(p) => {
            dec.pixels8 = p;
            true
        }
        None => {
            dec.status = VP8StatusCode::OutOfMemory;
            false
        }
    }
}

//------------------------------------------------------------------------------

/// Special row-processing that only stores the alpha data.
/// Translation of `ExtractAlphaRows()`.
fn extract_alpha_rows(
    dec: &mut VP8LDecoder<'_>,
    pixels: &[u32],
    last_row: i32,
    sink: &mut AlphaSink<'_>,
) {
    let mut cur_row = dec.last_row;
    let mut num_rows = last_row - cur_row;
    let mut input = (dec.width * cur_row) as usize;

    debug_assert!(last_row <= dec.io.crop_bottom);
    while num_rows > 0 {
        let num_rows_to_process = if num_rows > NUM_ARGB_CACHE_ROWS {
            NUM_ARGB_CACHE_ROWS
        } else {
            num_rows
        };
        // Extract alpha (which is stored in the green plane).
        let width = dec.io.width as usize; // the final width (!= dec->width_)
        let cache_pixs = width * num_rows_to_process as usize;
        let dst = width * cur_row as usize;
        apply_inverse_transforms(dec, cur_row, num_rows_to_process, &pixels[input..]);
        webp_extract_green(
            &dec.argb_cache[dec.argb_cache_off..],
            &mut sink.output[dst..],
            cache_pixs,
        );
        alpha_apply_filter(sink, cur_row, cur_row + num_rows_to_process, dst, width);
        num_rows -= num_rows_to_process;
        input += num_rows_to_process as usize * dec.width as usize;
        cur_row += num_rows_to_process;
    }
    debug_assert!(cur_row == last_row);
    dec.last_row = last_row;
    dec.last_out_row = last_row;
}

/// Decodes image header for alpha data stored using lossless compression.
/// Returns false in case of error. Translation of
/// `VP8LDecodeAlphaHeader()`.
pub(crate) fn vp8l_decode_alpha_header<'a>(alph_dec: &mut ALPHDecoder<'a>, data: &'a [u8]) -> bool {
    let mut dec = vp8l_new();

    dec.width = alph_dec.width;
    dec.height = alph_dec.height;
    dec.io = alph_dec.io;
    dec.io.width = alph_dec.width;
    dec.io.height = alph_dec.height;

    dec.status = VP8StatusCode::Ok;
    dec.br = VP8LBitReader::new(data);

    if !decode_image_stream(alph_dec.width, alph_dec.height, true, &mut dec, None) {
        return false;
    }

    // Special case: if alpha data uses only the color indexing transform and
    // doesn't use color cache (a frequent case), we will use DecodeAlphaData()
    // method that only needs allocation of 1 byte per pixel (alpha channel).
    let ok = if dec.next_transform == 1
        && dec.transforms[0].type_ == VP8LImageTransformType::ColorIndexingTransform
        && is_8b_optimizable(&dec.hdr)
    {
        alph_dec.use_8b_decode = true;
        allocate_internal_buffers_8b(&mut dec)
    } else {
        // Allocate internal buffers (note that dec->width_ may have changed here).
        alph_dec.use_8b_decode = false;
        allocate_internal_buffers_32b(&mut dec, alph_dec.width)
    };

    if !ok {
        return false;
    }

    // Only set here, once we are sure it is valid (to avoid thread races).
    alph_dec.vp8l_dec = Some(Box::new(dec));
    true
}

/// Decodes *at least* 'last_row' rows of alpha. If some of the initial
/// rows are already decoded in previous call(s), it will resume decoding
/// from where it was paused. Returns false in case of bitstream error.
/// Translation of `VP8LDecodeAlphaImageStream()` (the alpha decoder's
/// output is `output`).
pub(crate) fn vp8l_decode_alpha_image_stream(
    alph_dec: &mut ALPHDecoder<'_>,
    last_row: i32,
    output: &mut [u8],
) -> bool {
    let use_8b_decode = alph_dec.use_8b_decode;
    let filter = alph_dec.filter;
    let Some(dec) = alph_dec.vp8l_dec.as_mut() else {
        return false;
    };
    let mut sink = AlphaSink {
        output,
        filter,
        prev_line: &mut alph_dec.prev_line,
    };
    debug_assert!(last_row <= dec.height);

    if dec.last_row >= last_row {
        return true; // done
    }

    // Decode (with special row processing).
    let (width, height) = (dec.width, dec.height);
    if use_8b_decode {
        let mut pixels8 = std::mem::take(&mut dec.pixels8);
        let ok = decode_alpha_data(dec, &mut pixels8, width, height, last_row, &mut sink);
        dec.pixels8 = pixels8;
        ok
    } else {
        let mut pixels = std::mem::take(&mut dec.pixels);
        let mut sink = RowSink::Alpha(sink);
        let ok = decode_image_data(dec, &mut pixels, width, height, last_row, &mut sink);
        dec.pixels = pixels;
        ok
    }
}

//------------------------------------------------------------------------------

/// Decodes the image header. Returns false in case of error. Translation
/// of `VP8LDecodeHeader()` (the stream is `dec.io.data`).
pub(crate) fn vp8l_decode_header(dec: &mut VP8LDecoder<'_>) -> bool {
    dec.status = VP8StatusCode::Ok;
    dec.br = VP8LBitReader::new(dec.io.data);
    'error: {
        let Some((width, height, _has_alpha)) = read_image_info(&mut dec.br) else {
            dec.status = VP8StatusCode::BitstreamError;
            break 'error;
        };
        dec.state = VP8LDecodeState::ReadDim;
        dec.io.width = width;
        dec.io.height = height;

        if !decode_image_stream(width, height, true, dec, None) {
            break 'error;
        }
        return true;
    }

    vp8l_clear(dec);
    debug_assert!(dec.status != VP8StatusCode::Ok);
    false
}

/// Decodes an image. It's required to decode the lossless header before
/// calling this function. Returns false in case of error, with updated
/// dec->status_. Translation of `VP8LDecodeImage()` (`params` is
/// `io->opaque`).
pub(crate) fn vp8l_decode_image(dec: &mut VP8LDecoder<'_>, params: &mut WebPDecParams<'_>) -> bool {
    debug_assert!(!dec.hdr.huffman_tables.segments.is_empty());
    debug_assert!(!dec.hdr.htree_groups.is_empty());
    debug_assert!(dec.hdr.num_htree_groups > 0);

    'err: {
        // Initialization.
        if dec.state != VP8LDecodeState::ReadData {
            if !webp_io_init_from_options(&mut dec.io, WebpCspMode::Bgra) {
                dec.status = VP8StatusCode::InvalidParam;
                break 'err;
            }

            let width = dec.io.width;
            if !allocate_internal_buffers_32b(dec, width) {
                break 'err;
            }

            // (no rescaling, no premultiplied or YUV output)
            dec.state = VP8LDecodeState::ReadData;
        }

        // Decode.
        let (width, height, crop_bottom) = (dec.width, dec.height, dec.io.crop_bottom);
        let mut pixels = std::mem::take(&mut dec.pixels);
        let mut sink = RowSink::Rows(&mut params.output);
        let ok = decode_image_data(dec, &mut pixels, width, height, crop_bottom, &mut sink);
        dec.pixels = pixels;
        if !ok {
            break 'err;
        }

        params.last_y = dec.last_out_row;
        return true;
    }

    vp8l_clear(dec);
    debug_assert!(dec.status != VP8StatusCode::Ok);
    false
}
