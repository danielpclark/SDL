// Rust translation of src/tiny_jpeg.h from SDL_image.
// tiny_jpeg.h - Tiny JPEG Encoder - Sergio Gonzalez
//
// This software is in the public domain. Where that dedication is not
// recognized, you are granted a perpetual, irrevocable license to copy and
// modify this file as you see fit.
//
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Tiny JPEG Encoder
//!
//! This is a readable and simple single-header JPEG encoder.
//!
//! Features
//!  - Implements Baseline DCT JPEG compression.
//!  - No dynamic allocations.
//!
//! This library is coded in the spirit of the stb libraries and mostly follows
//! the stb guidelines.
//!
//! ==== Thanks ====
//!
//!  AssociationSirius (Bug reports)
//!  Bernard van Gastel (Thread-safe defaults, BSD compilation)
//!
//! As SDL_image builds it: `tje_encode_with_func()` only, the fast DCT
//! (`TJE_USE_FAST_DCT`), `floorf()` from SDL's libm.

// tiny_jpeg's constants are written with more digits than f32 holds, and
// its loops index several arrays at once; both kept as written.
#![allow(
    clippy::approx_constant,
    clippy::excessive_precision,
    clippy::int_plus_one,
    clippy::needless_range_loop
)]

use sdl3::stdlib::math::floorf;

// Only use zero for debugging and/or inspection.
// (TJE_USE_FAST_DCT is 1: the slow DCT isn't translated.)

const TJEI_BUFFER_SIZE: usize = 1024;

/// The output callback. Translation of `tje_write_func` (with the
/// `context` captured by the closure).
pub(crate) type WriteFunc<'f> = &'f mut dyn FnMut(&[u8]);

/// Translation of `TJEState`.
struct TjeState<'f> {
    // Huffman data.
    ehuffsize: [[u8; 257]; 4],
    ehuffcode: [[u16; 256]; 4],
    ht_bits: [&'static [u8]; 4],
    ht_vals: [&'static [u8]; 4],

    // Cuantization tables.
    qt_luma: [u8; 64],
    qt_chroma: [u8; 64],

    // fwrite by default. User-defined when using tje_encode_with_func.
    write_context: WriteFunc<'f>,

    // Buffered output. Big performance win when using the usual stdlib implementations.
    output_buffer_count: usize,
    output_buffer: [u8; TJEI_BUFFER_SIZE],
}

// ============================================================
// Table definitions.
//
// The spec defines tjei_default reasonably good quantization matrices and huffman
// specification tables.
//
//
// Instead of hard-coding the final huffman table, we only hard-code the table
// spec suggested by the specification, and then derive the full table from
// there.  This is only for didactic purposes but it might be useful if there
// ever is the case that we need to swap huffman tables from various sources.
// ============================================================

// K.1 - suggested luminance QT
static TJEI_DEFAULT_QT_LUMA_FROM_SPEC: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, //
    12, 12, 14, 19, 26, 58, 60, 55, //
    14, 13, 16, 24, 40, 57, 69, 56, //
    14, 17, 22, 29, 51, 87, 80, 62, //
    18, 22, 37, 56, 68, 109, 103, 77, //
    24, 35, 55, 64, 81, 104, 113, 92, //
    49, 64, 78, 87, 103, 121, 120, 101, //
    72, 92, 95, 98, 112, 100, 103, 99, //
];

// Unused: tjei_default_qt_chroma_from_spec (K.1 - suggested chrominance QT)

static TJEI_DEFAULT_QT_CHROMA_FROM_PAPER: [u8; 64] = [
    // Example QT from JPEG paper
    16, 12, 14, 14, 18, 24, 49, 72, //
    11, 10, 16, 24, 40, 51, 61, 12, //
    13, 17, 22, 35, 64, 92, 14, 16, //
    22, 37, 55, 78, 95, 19, 24, 29, //
    56, 64, 87, 98, 26, 40, 51, 68, //
    81, 103, 112, 58, 57, 87, 109, 104, //
    121, 100, 60, 69, 80, 103, 113, 120, //
    103, 55, 56, 62, 77, 92, 101, 99, //
];

// == Procedure to 'deflate' the huffman tree: JPEG spec, C.2

// Number of 16 bit values for every code length. (K.3.3.1)
static TJEI_DEFAULT_HT_LUMA_DC_LEN: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
// values
static TJEI_DEFAULT_HT_LUMA_DC: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

// Number of 16 bit values for every code length. (K.3.3.1)
static TJEI_DEFAULT_HT_CHROMA_DC_LEN: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
// values
static TJEI_DEFAULT_HT_CHROMA_DC: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

// Same as above, but AC coefficients.
static TJEI_DEFAULT_HT_LUMA_AC_LEN: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
static TJEI_DEFAULT_HT_LUMA_AC: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
    0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5,
    0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];

static TJEI_DEFAULT_HT_CHROMA_AC_LEN: [u8; 16] =
    [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
static TJEI_DEFAULT_HT_CHROMA_AC: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0,
    0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26,
    0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5,
    0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3,
    0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA,
    0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8,
    0xF9, 0xFA,
];

// ============================================================
// Code
// ============================================================

// Zig-zag order:
static TJEI_ZIG_ZAG: [u8; 64] = [
    0, 1, 5, 6, 14, 15, 27, 28, //
    2, 4, 7, 13, 16, 26, 29, 42, //
    3, 8, 12, 17, 25, 30, 41, 43, //
    9, 11, 18, 24, 31, 40, 44, 53, //
    10, 19, 23, 32, 39, 45, 52, 54, //
    20, 22, 33, 38, 46, 51, 55, 60, //
    21, 34, 37, 47, 50, 56, 59, 61, //
    35, 36, 48, 49, 57, 58, 62, 63, //
];

// Memory order as big endian.
// (tjei_be_word(): the structs below are written as big-endian bytes
// directly.)

// ============================================================
// The following structs exist only for code clarity, debugability, and
// readability. They are used when writing to disk, but it is useful to have
// 1-packed-structs to document how the format works, and to inspect memory
// while developing.
// (Here they are built as byte arrays in the same packed layout.)
// ============================================================

static TJEIK_JFIF_ID: &[u8; 5] = b"JFIF\0";
static TJEIK_COM_STR: &[u8] = b"Created by Tiny JPEG Encoder";

/// The size of the packed `TJEJPEGHeader`.
const TJE_JPEG_HEADER_SIZE: usize = 20;

impl TjeState<'_> {
    /// Translation of `tjei_write()`.
    fn write(&mut self, data: &[u8]) {
        let to_write = data.len();

        // Cap to the buffer available size and copy memory.
        let capped_count = to_write.min(TJEI_BUFFER_SIZE - 1 - self.output_buffer_count);

        self.output_buffer[self.output_buffer_count..self.output_buffer_count + capped_count]
            .copy_from_slice(&data[..capped_count]);
        self.output_buffer_count += capped_count;

        debug_assert!(self.output_buffer_count <= TJEI_BUFFER_SIZE - 1);

        // Flush the buffer.
        if self.output_buffer_count == TJEI_BUFFER_SIZE - 1 {
            (self.write_context)(&self.output_buffer[..self.output_buffer_count]);
            self.output_buffer_count = 0;
        }

        // Recursively calling ourselves with the rest of the buffer.
        if capped_count < to_write {
            self.write(&data[capped_count..]);
        }
    }

    /// Translation of `tjei_write_DQT()`.
    fn write_dqt(&mut self, matrix: &[u8; 64], id: u8) {
        let dqt: u16 = 0xffdb;
        let len: u16 = 0x0043; // 2(len) + 1(id) + 64(matrix) = 67 = 0x43
        let precision_and_id = id; // 0x0000 8 bits | 0x00id
        self.write(&dqt.to_be_bytes());
        self.write(&len.to_be_bytes());
        debug_assert!(id < 4);
        self.write(&[precision_and_id]);
        // Write matrix
        self.write(matrix);
    }

    /// Translation of `tjei_write_DHT()`.
    fn write_dht(&mut self, matrix_len: &[u8], matrix_val: &[u8], ht_class: u8, id: u8) {
        let mut num_values = 0usize;

        for i in 0..16 {
            num_values += matrix_len[i] as usize;
        }
        debug_assert!(num_values <= 0xffff);

        let dht: u16 = 0xffc4;
        // 2(len) + 1(Tc|th) + 16 (num lengths) + ?? (num values)
        let len: u16 = 2 + 1 + 16 + num_values as u16;
        debug_assert!(id < 4);
        let tc_th = (ht_class << 4) | id;

        self.write(&dht.to_be_bytes());
        self.write(&len.to_be_bytes());
        self.write(&[tc_th]);
        self.write(&matrix_len[..16]);
        self.write(&matrix_val[..num_values]);
    }
}

const TJEI_DC: u8 = 0;
const TJEI_AC: u8 = 1;

// ============================================================
//  Huffman deflation code.
// ============================================================

/// Returns all code sizes from the BITS specification (JPEG C.3)
/// Translation of `tjei_huff_get_code_lengths()`.
fn huff_get_code_lengths(huffsize: &mut [u8; 257], bits: &[u8]) {
    let mut k = 0;
    for i in 0..16 {
        for _ in 0..bits[i] {
            huffsize[k] = (i + 1) as u8;
            k += 1;
        }
        huffsize[k] = 0;
    }
}

/// Fills out the prefixes for each code. Translation of `tjei_huff_get_codes()`.
fn huff_get_codes(codes: &mut [u16; 256], huffsize: &[u8; 257], count: i64) {
    let mut code: u16 = 0;
    let mut k = 0usize;
    let mut sz = huffsize[0];
    loop {
        loop {
            debug_assert!((k as i64) < count);
            codes[k] = code;
            k += 1;
            code = code.wrapping_add(1);
            if huffsize[k] != sz {
                break;
            }
        }
        if huffsize[k] == 0 {
            return;
        }
        loop {
            code <<= 1;
            sz += 1;
            if huffsize[k] == sz {
                break;
            }
        }
    }
}

/// Translation of `tjei_huff_get_extended()`.
fn huff_get_extended(
    out_ehuffsize: &mut [u8; 257],
    out_ehuffcode: &mut [u16; 256],
    huffval: &[u8],
    huffsize: &[u8; 257],
    huffcode: &[u16; 256],
    count: i64,
) {
    let mut k = 0usize;
    loop {
        let val = huffval[k] as usize;
        out_ehuffcode[val] = huffcode[k];
        out_ehuffsize[val] = huffsize[k];
        k += 1;
        if (k as i64) >= count {
            break;
        }
    }
}
// ============================================================

/// Returns:
///  out[1] : number of bits
///  out[0] : bits
/// Translation of `tjei_calculate_variable_length_int()`.
#[inline(always)]
fn calculate_variable_length_int(mut value: i32) -> [u16; 2] {
    let mut out = [0u16; 2];
    let mut abs_val = value;
    if value < 0 {
        abs_val = -abs_val;
        value -= 1;
    }
    out[1] = 1;
    loop {
        abs_val >>= 1;
        if abs_val == 0 {
            break;
        }
        out[1] += 1;
    }
    out[0] = (value & ((1i32 << out[1]) - 1)) as u16;
    out
}

/// Write bits to file. Translation of `tjei_write_bits()`.
#[inline(always)]
fn write_bits(
    state: &mut TjeState<'_>,
    bitbuffer: &mut u32,
    location: &mut u32,
    num_bits: u16,
    bits: u16,
) {
    //   v-- location
    //  [                     ]   <-- bit buffer
    // 32                     0
    //
    // This call pushes to the bitbuffer and saves the location. Data is pushed
    // from most significant to less significant.
    // When we can write a full byte, we write a byte and shift.

    // Push the stack.
    let nloc = *location + num_bits as u32;
    *bitbuffer |= (bits as u32).checked_shl(32 - nloc).unwrap_or(0);
    *location = nloc;
    while *location >= 8 {
        // Grab the most significant byte.
        let c = (*bitbuffer >> 24) as u8;
        // Write it to file.
        state.write(&[c]);
        if c == 0xff {
            // Special case: tell JPEG this is not a marker.
            state.write(&[0]);
        }
        // Pop the stack.
        *bitbuffer <<= 8;
        *location -= 8;
    }
}

/// DCT implementation by Thomas G. Lane.
/// Obtained through NVIDIA
///  <http://developer.download.nvidia.com/SDK/9.5/Samples/vidimaging_samples.html#gpgpu_dct>
///
/// QUOTE:
///  This implementation is based on Arai, Agui, and Nakajima's algorithm for
///  scaled DCT.  Their original paper (Trans. IEICE E-71(11):1095) is in
///  Japanese, but the algorithm is described in the Pennebaker & Mitchell
///  JPEG textbook (see REFERENCES section in file README).  The following code
///  is based directly on figure 4-8 in P&M.
///
/// Translation of `tjei_fdct()`.
fn fdct(data: &mut [f32; 64]) {
    /* Pass 1: process rows. */

    for row in 0..8 {
        let d = &mut data[row * 8..row * 8 + 8];
        let tmp0 = d[0] + d[7];
        let tmp7 = d[0] - d[7];
        let tmp1 = d[1] + d[6];
        let tmp6 = d[1] - d[6];
        let tmp2 = d[2] + d[5];
        let tmp5 = d[2] - d[5];
        let tmp3 = d[3] + d[4];
        let tmp4 = d[3] - d[4];

        /* Even part */

        let tmp10 = tmp0 + tmp3; /* phase 2 */
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;

        d[0] = tmp10 + tmp11; /* phase 3 */
        d[4] = tmp10 - tmp11;

        let z1 = (tmp12 + tmp13) * 0.707106781f32; /* c4 */
        d[2] = tmp13 + z1; /* phase 5 */
        d[6] = tmp13 - z1;

        /* Odd part */

        let tmp10 = tmp4 + tmp5; /* phase 2 */
        let tmp11 = tmp5 + tmp6;
        let tmp12 = tmp6 + tmp7;

        /* The rotator is modified from fig 4-8 to avoid extra negations. */
        let z5 = (tmp10 - tmp12) * 0.382683433f32; /* c6 */
        let z2 = 0.541196100f32 * tmp10 + z5; /* c2-c6 */
        let z4 = 1.306562965f32 * tmp12 + z5; /* c2+c6 */
        let z3 = tmp11 * 0.707106781f32; /* c4 */

        let z11 = tmp7 + z3; /* phase 5 */
        let z13 = tmp7 - z3;

        d[5] = z13 + z2; /* phase 6 */
        d[3] = z13 - z2;
        d[1] = z11 + z4;
        d[7] = z11 - z4;

        /* advance pointer to next row */
    }

    /* Pass 2: process columns. */

    for col in 0..8 {
        let at = |i: usize| col + 8 * i;
        let tmp0 = data[at(0)] + data[at(7)];
        let tmp7 = data[at(0)] - data[at(7)];
        let tmp1 = data[at(1)] + data[at(6)];
        let tmp6 = data[at(1)] - data[at(6)];
        let tmp2 = data[at(2)] + data[at(5)];
        let tmp5 = data[at(2)] - data[at(5)];
        let tmp3 = data[at(3)] + data[at(4)];
        let tmp4 = data[at(3)] - data[at(4)];

        /* Even part */

        let tmp10 = tmp0 + tmp3; /* phase 2 */
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;

        data[at(0)] = tmp10 + tmp11; /* phase 3 */
        data[at(4)] = tmp10 - tmp11;

        let z1 = (tmp12 + tmp13) * 0.707106781f32; /* c4 */
        data[at(2)] = tmp13 + z1; /* phase 5 */
        data[at(6)] = tmp13 - z1;

        /* Odd part */

        let tmp10 = tmp4 + tmp5; /* phase 2 */
        let tmp11 = tmp5 + tmp6;
        let tmp12 = tmp6 + tmp7;

        /* The rotator is modified from fig 4-8 to avoid extra negations. */
        let z5 = (tmp10 - tmp12) * 0.382683433f32; /* c6 */
        let z2 = 0.541196100f32 * tmp10 + z5; /* c2-c6 */
        let z4 = 1.306562965f32 * tmp12 + z5; /* c2+c6 */
        let z3 = tmp11 * 0.707106781f32; /* c4 */

        let z11 = tmp7 + z3; /* phase 5 */
        let z13 = tmp7 - z3;

        data[at(5)] = z13 + z2; /* phase 6 */
        data[at(3)] = z13 - z2;
        data[at(1)] = z11 + z4;
        data[at(7)] = z11 - z4;

        /* advance pointer to next column */
    }
}

/// Translation of `tjei_encode_and_write_MCU()` (with the fast DCT and its
/// pre-processed quantization matrix).
#[allow(clippy::too_many_arguments)]
fn encode_and_write_mcu(
    state: &mut TjeState<'_>,
    mcu: &[f32; 64],
    qt: &[f32; 64], // Pre-processed quantization matrix.
    table_dc: usize,
    table_ac: usize,     // Huffman tables
    pred: &mut i32,      // Previous DC coefficient
    bitbuffer: &mut u32, // Bitstack.
    location: &mut u32,
) {
    let mut du = [0i32; 64]; // Data unit in zig-zag order

    let mut dct_mcu = *mcu;

    fdct(&mut dct_mcu);
    for i in 0..64 {
        let mut fval = dct_mcu[i];
        fval *= qt[i];
        // (the commented-out alternative: round half away from zero)
        fval = floorf(fval + 1024.0 + 0.5);
        fval -= 1024.0;
        let val = fval as i32;
        du[TJEI_ZIG_ZAG[i] as usize] = val;
    }

    // Encode DC coefficient.
    let diff = du[0] - *pred;
    *pred = du[0];
    if diff != 0 {
        let vli = calculate_variable_length_int(diff);
        // Write number of bits with Huffman coding
        let (len, code) = (
            state.ehuffsize[table_dc][vli[1] as usize],
            state.ehuffcode[table_dc][vli[1] as usize],
        );
        write_bits(state, bitbuffer, location, len as u16, code);
        // Write the bits.
        write_bits(state, bitbuffer, location, vli[1], vli[0]);
    } else {
        let (len, code) = (state.ehuffsize[table_dc][0], state.ehuffcode[table_dc][0]);
        write_bits(state, bitbuffer, location, len as u16, code);
    }

    // ==== Encode AC coefficients ====

    let mut last_non_zero_i = 0;
    // Find the last non-zero element.
    for i in (1..64).rev() {
        if du[i] != 0 {
            last_non_zero_i = i;
            break;
        }
    }

    let mut i = 1;
    while i <= last_non_zero_i {
        // If zero, increase count. If >=15, encode (FF,00)
        let mut zero_count = 0u16;
        while du[i] == 0 {
            zero_count += 1;
            i += 1;
            if zero_count == 16 {
                // encode (ff,00) == 0xf0
                let (len, code) = (
                    state.ehuffsize[table_ac][0xf0],
                    state.ehuffcode[table_ac][0xf0],
                );
                write_bits(state, bitbuffer, location, len as u16, code);
                zero_count = 0;
            }
        }
        let vli = calculate_variable_length_int(du[i]);

        debug_assert!(zero_count < 0x10);
        debug_assert!(vli[1] <= 10);

        let sym1 = (zero_count << 4) | vli[1];

        debug_assert!(state.ehuffsize[table_ac][sym1 as usize] != 0);

        // Write symbol 1  --- (RUNLENGTH, SIZE)
        let (len, code) = (
            state.ehuffsize[table_ac][sym1 as usize],
            state.ehuffcode[table_ac][sym1 as usize],
        );
        write_bits(state, bitbuffer, location, len as u16, code);
        // Write symbol 2  --- (AMPLITUDE)
        write_bits(state, bitbuffer, location, vli[1], vli[0]);
        i += 1;
    }

    if last_non_zero_i != 63 {
        // write EOB HUFF(00,00)
        let (len, code) = (state.ehuffsize[table_ac][0], state.ehuffcode[table_ac][0]);
        write_bits(state, bitbuffer, location, len as u16, code);
    }
}

const TJEI_LUMA_DC: usize = 0;
const TJEI_LUMA_AC: usize = 1;
const TJEI_CHROMA_DC: usize = 2;
const TJEI_CHROMA_AC: usize = 3;

/// Translation of `struct TJEProcessedQT`.
struct ProcessedQt {
    chroma: [f32; 64],
    luma: [f32; 64],
}

/// Set up huffman tables in state. Translation of `tjei_huff_expand()`.
fn huff_expand(state: &mut TjeState<'_>) {
    let mut spec_tables_len = [0i32; 4];
    let mut huffsize = [[0u8; 257]; 4];
    let mut huffcode = [[0u16; 256]; 4];

    state.ht_bits[TJEI_LUMA_DC] = &TJEI_DEFAULT_HT_LUMA_DC_LEN;
    state.ht_bits[TJEI_LUMA_AC] = &TJEI_DEFAULT_HT_LUMA_AC_LEN;
    state.ht_bits[TJEI_CHROMA_DC] = &TJEI_DEFAULT_HT_CHROMA_DC_LEN;
    state.ht_bits[TJEI_CHROMA_AC] = &TJEI_DEFAULT_HT_CHROMA_AC_LEN;

    state.ht_vals[TJEI_LUMA_DC] = &TJEI_DEFAULT_HT_LUMA_DC;
    state.ht_vals[TJEI_LUMA_AC] = &TJEI_DEFAULT_HT_LUMA_AC;
    state.ht_vals[TJEI_CHROMA_DC] = &TJEI_DEFAULT_HT_CHROMA_DC;
    state.ht_vals[TJEI_CHROMA_AC] = &TJEI_DEFAULT_HT_CHROMA_AC;

    // How many codes in total for each of LUMA_(DC|AC) and CHROMA_(DC|AC)

    for i in 0..4 {
        for k in 0..16 {
            spec_tables_len[i] += state.ht_bits[i][k] as i32;
        }
    }

    // Fill out the extended tables..
    for i in 0..4 {
        debug_assert!(256 >= spec_tables_len[i]);
        huff_get_code_lengths(&mut huffsize[i], state.ht_bits[i]);
        huff_get_codes(&mut huffcode[i], &huffsize[i], spec_tables_len[i] as i64);
    }
    for i in 0..4 {
        let count = spec_tables_len[i] as i64;
        huff_get_extended(
            &mut state.ehuffsize[i],
            &mut state.ehuffcode[i],
            state.ht_vals[i],
            &huffsize[i],
            &huffcode[i],
            count,
        );
    }
}

/// Translation of `tjei_encode_main()`.
fn encode_main(
    state: &mut TjeState<'_>,
    src_data: &[u8],
    width: i32,
    height: i32,
    src_num_components: i32,
    pitch: i32,
) -> bool {
    // Again, taken from classic japanese implementation.
    //
    /* For float AA&N IDCT method, divisors are equal to quantization
     * coefficients scaled by scalefactor[row]*scalefactor[col], where
     *   scalefactor[0] = 1
     *   scalefactor[k] = cos(k*PI/16) * sqrt(2)    for k=1..7
     * We apply a further scale factor of 8.
     * What's actually stored is 1/divisor so that the inner loop can
     * use a multiplication rather than a division.
     */
    const AAN_SCALES: [f32; 8] = [
        1.0,
        1.387039845,
        1.306562965,
        1.175875602,
        1.0,
        0.785694958,
        0.541196100,
        0.275899379,
    ];
    let mut pqt = ProcessedQt {
        chroma: [0.0; 64],
        luma: [0.0; 64],
    };

    let mut du_y = [0f32; 64];
    let mut du_b = [0f32; 64];
    let mut du_r = [0f32; 64];

    if src_num_components != 3 && src_num_components != 4 {
        return false;
    }

    if width > 0xffff || height > 0xffff {
        return false;
    }

    // build (de)quantization tables
    for y in 0..8 {
        for x in 0..8 {
            let i = y * 8 + x;
            pqt.luma[y * 8 + x] = 1.0
                / (8.0
                    * AAN_SCALES[x]
                    * AAN_SCALES[y]
                    * state.qt_luma[TJEI_ZIG_ZAG[i] as usize] as f32);
            pqt.chroma[y * 8 + x] = 1.0
                / (8.0
                    * AAN_SCALES[x]
                    * AAN_SCALES[y]
                    * state.qt_chroma[TJEI_ZIG_ZAG[i] as usize] as f32);
        }
    }

    {
        // Write header
        let mut header = Vec::with_capacity(TJE_JPEG_HEADER_SIZE);
        // JFIF header.
        header.extend_from_slice(&0xffd8u16.to_be_bytes()); // Sequential DCT
        header.extend_from_slice(&0xffe0u16.to_be_bytes());

        let jfif_len = (TJE_JPEG_HEADER_SIZE - 4/*SOI & APP0 markers*/) as u16;
        header.extend_from_slice(&jfif_len.to_be_bytes());
        header.extend_from_slice(TJEIK_JFIF_ID);
        header.extend_from_slice(&0x0102u16.to_be_bytes()); // version
        header.push(0x01); // Dots-per-inch
        header.extend_from_slice(&0x0060u16.to_be_bytes()); // 96 DPI
        header.extend_from_slice(&0x0060u16.to_be_bytes()); // 96 DPI
        header.push(0); // x_thumb
        header.push(0); // y_thumb
        debug_assert_eq!(header.len(), TJE_JPEG_HEADER_SIZE);
        state.write(&header);
    }
    {
        // Write comment
        let com_len = (2 + TJEIK_COM_STR.len()) as u16;
        let mut com = Vec::new();
        // Comment
        com.extend_from_slice(&0xfffeu16.to_be_bytes());
        com.extend_from_slice(&com_len.to_be_bytes());
        com.extend_from_slice(TJEIK_COM_STR);
        state.write(&com);
    }

    // Write quantization tables.
    let (qt_luma, qt_chroma) = (state.qt_luma, state.qt_chroma);
    state.write_dqt(&qt_luma, 0x00);
    state.write_dqt(&qt_chroma, 0x01);

    {
        // Write the frame marker.
        let tables: [u8; 3] = [
            0, // Luma component gets luma table (see tjei_write_DQT call above.)
            1, // Chroma component gets chroma table
            1, // Chroma component gets chroma table
        ];

        let mut header = Vec::new();
        header.extend_from_slice(&0xffc0u16.to_be_bytes()); // SOF
        header.extend_from_slice(&(8u16 + 3 * 3).to_be_bytes()); // len
        header.push(8); // precision
        debug_assert!(width <= 0xffff);
        debug_assert!(height <= 0xffff);
        header.extend_from_slice(&(height as u16).to_be_bytes());
        header.extend_from_slice(&(width as u16).to_be_bytes());
        header.push(3); // num_components
        for i in 0..3 {
            header.push((i + 1) as u8); // component_id: No particular reason. Just 1, 2, 3.
            header.push(0x11); // sampling_factors
            header.push(tables[i]); // qt
        }
        // Write to file.
        state.write(&header);
    }

    let (bits, vals) = (state.ht_bits, state.ht_vals);
    state.write_dht(bits[TJEI_LUMA_DC], vals[TJEI_LUMA_DC], TJEI_DC, 0);
    state.write_dht(bits[TJEI_LUMA_AC], vals[TJEI_LUMA_AC], TJEI_AC, 0);
    state.write_dht(bits[TJEI_CHROMA_DC], vals[TJEI_CHROMA_DC], TJEI_DC, 1);
    state.write_dht(bits[TJEI_CHROMA_AC], vals[TJEI_CHROMA_AC], TJEI_AC, 1);

    // Write start of scan
    {
        let tables: [u8; 3] = [0x00, 0x11, 0x11];

        let mut header = Vec::new();
        header.extend_from_slice(&0xffdau16.to_be_bytes()); // SOS
        header.extend_from_slice(&(6u16 + 2 * 3).to_be_bytes()); // len
        header.push(3); // num_components

        for i in 0..3 {
            // Must be equal to component_id from frame header above.
            header.push((i + 1) as u8);
            header.push(tables[i]); // dc_ac
        }
        header.push(0); // first
        header.push(63); // last
        header.push(0); // ah_al
        state.write(&header);
    }

    // Write compressed data.

    // Set diff to 0.
    let mut pred_y = 0;
    let mut pred_b = 0;
    let mut pred_r = 0;

    // Bit stack
    let mut bitbuffer: u32 = 0;
    let mut location: u32 = 0;

    for y in (0..height).step_by(8) {
        for x in (0..width).step_by(8) {
            // Block loop: ====
            for off_y in 0..8 {
                for off_x in 0..8 {
                    let block_index = (off_y * 8 + off_x) as usize;

                    let mut src_index = ((y + off_y) * pitch) + ((x + off_x) * src_num_components);

                    let col = x + off_x;
                    let row = y + off_y;

                    if row >= height {
                        src_index -= pitch * (row - height + 1);
                    }
                    if col >= width {
                        src_index -= (col - width + 1) * src_num_components;
                    }
                    debug_assert!(src_index < height * pitch);

                    let src_index = src_index as usize;
                    let r = src_data[src_index] as f32;
                    let g = src_data[src_index + 1] as f32;
                    let b = src_data[src_index + 2] as f32;

                    let luma = 0.299f32 * r + 0.587f32 * g + 0.114f32 * b - 128.0;
                    let cb = -0.1687f32 * r - 0.3313f32 * g + 0.5f32 * b;
                    let cr = 0.5f32 * r - 0.4187f32 * g - 0.0813f32 * b;

                    du_y[block_index] = luma;
                    du_b[block_index] = cb;
                    du_r[block_index] = cr;
                }
            }

            encode_and_write_mcu(
                state,
                &du_y,
                &pqt.luma,
                TJEI_LUMA_DC,
                TJEI_LUMA_AC,
                &mut pred_y,
                &mut bitbuffer,
                &mut location,
            );
            encode_and_write_mcu(
                state,
                &du_b,
                &pqt.chroma,
                TJEI_CHROMA_DC,
                TJEI_CHROMA_AC,
                &mut pred_b,
                &mut bitbuffer,
                &mut location,
            );
            encode_and_write_mcu(
                state,
                &du_r,
                &pqt.chroma,
                TJEI_CHROMA_DC,
                TJEI_CHROMA_AC,
                &mut pred_r,
                &mut bitbuffer,
                &mut location,
            );
        }
    }

    // Finish the image.
    {
        // Flush
        if location > 0 && location < 8 {
            let num_bits = (8 - location) as u16;
            write_bits(state, &mut bitbuffer, &mut location, num_bits, 0);
        }
    }
    state.write(&0xffd9u16.to_be_bytes()); // EOI

    if state.output_buffer_count != 0 {
        (state.write_context)(&state.output_buffer[..state.output_buffer_count]);
        state.output_buffer_count = 0;
    }

    true
}

/// Encode `width` by `height` pixels of `num_components` (3 or 4: RGB or
/// RGBA, the alpha ignored) bytes each, rows `pitch` bytes apart, as a
/// baseline JPEG passed to `func` in pieces. `quality` is 1 (lowest), 2 or
/// 3 (highest). Returns false for invalid parameters.
/// Translation of `tje_encode_with_func()`.
pub(crate) fn encode_with_func(
    func: WriteFunc<'_>,
    quality: i32,
    width: i32,
    height: i32,
    num_components: i32,
    src_data: &[u8],
    pitch: i32,
) -> bool {
    if !(1..=3).contains(&quality) {
        // tje_log("[ERROR] -- Valid 'quality' values are 1 (lowest), 2, or 3 (highest)\n");
        return false;
    }

    let mut qt_factor: u8 = 1;
    let mut state = TjeState {
        ehuffsize: [[0; 257]; 4],
        ehuffcode: [[0; 256]; 4],
        ht_bits: [&[]; 4],
        ht_vals: [&[]; 4],
        qt_luma: [0; 64],
        qt_chroma: [0; 64],
        write_context: func,
        output_buffer_count: 0,
        output_buffer: [0; TJEI_BUFFER_SIZE],
    };

    match quality {
        3 => {
            for i in 0..64 {
                state.qt_luma[i] = 1;
                state.qt_chroma[i] = 1;
            }
        }
        _ => {
            if quality == 2 {
                qt_factor = 10;
            }
            /* fallthrough */
            for i in 0..64 {
                state.qt_luma[i] = TJEI_DEFAULT_QT_LUMA_FROM_SPEC[i] / qt_factor;
                if state.qt_luma[i] == 0 {
                    state.qt_luma[i] = 1;
                }
                state.qt_chroma[i] = TJEI_DEFAULT_QT_CHROMA_FROM_PAPER[i] / qt_factor;
                if state.qt_chroma[i] == 0 {
                    state.qt_chroma[i] = 1;
                }
            }
        }
    }

    huff_expand(&mut state);

    encode_main(&mut state, src_data, width, height, num_components, pitch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huffman_tables_are_the_standard_ones() {
        let mut state = TjeState {
            ehuffsize: [[0; 257]; 4],
            ehuffcode: [[0; 256]; 4],
            ht_bits: [&[]; 4],
            ht_vals: [&[]; 4],
            qt_luma: [0; 64],
            qt_chroma: [0; 64],
            write_context: &mut |_| {},
            output_buffer_count: 0,
            output_buffer: [0; TJEI_BUFFER_SIZE],
        };
        huff_expand(&mut state);
        // Table K.3: luminance DC code lengths and codes
        let luma_dc: [(u8, u16); 12] = [
            (2, 0b00),
            (3, 0b010),
            (3, 0b011),
            (3, 0b100),
            (3, 0b101),
            (3, 0b110),
            (4, 0b1110),
            (5, 0b11110),
            (6, 0b111110),
            (7, 0b1111110),
            (8, 0b11111110),
            (9, 0b111111110),
        ];
        for (i, &(len, code)) in luma_dc.iter().enumerate() {
            assert_eq!(state.ehuffsize[TJEI_LUMA_DC][i], len);
            assert_eq!(state.ehuffcode[TJEI_LUMA_DC][i], code);
        }
        // Table K.5: EOB (0/0) is 1010, ZRL (F/0) is 11111111001
        assert_eq!(state.ehuffsize[TJEI_LUMA_AC][0x00], 4);
        assert_eq!(state.ehuffcode[TJEI_LUMA_AC][0x00], 0b1010);
        assert_eq!(state.ehuffsize[TJEI_LUMA_AC][0xf0], 11);
        assert_eq!(state.ehuffcode[TJEI_LUMA_AC][0xf0], 0b11111111001);
    }

    #[test]
    fn variable_length_ints() {
        assert_eq!(calculate_variable_length_int(1), [1, 1]);
        assert_eq!(calculate_variable_length_int(-1), [0, 1]);
        assert_eq!(calculate_variable_length_int(5), [5, 3]);
        assert_eq!(calculate_variable_length_int(-5), [2, 3]);
        assert_eq!(calculate_variable_length_int(255), [255, 8]);
    }

    #[test]
    fn rejects_invalid_parameters() {
        let mut out = Vec::new();
        let pixels = [0u8; 12];
        assert!(!encode_with_func(
            &mut |d| out.extend_from_slice(d),
            0,
            2,
            2,
            3,
            &pixels,
            6
        ));
        assert!(!encode_with_func(
            &mut |d| out.extend_from_slice(d),
            4,
            2,
            2,
            3,
            &pixels,
            6
        ));
        assert!(!encode_with_func(
            &mut |d| out.extend_from_slice(d),
            3,
            2,
            2,
            2,
            &pixels,
            6
        ));
        assert!(!encode_with_func(
            &mut |d| out.extend_from_slice(d),
            3,
            0x10000,
            1,
            3,
            &pixels,
            6
        ));
        assert!(out.is_empty());
    }
}
