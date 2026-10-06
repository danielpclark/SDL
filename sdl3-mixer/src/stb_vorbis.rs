// Rust translation of src/stb_vorbis/stb_vorbis.h (stb_vorbis v1.22, with
// SDL_mixer's patches) from SDL_mixer.
// Ogg Vorbis audio decoder - public domain. Original version written by Sean
// Barrett in 2007; see the credits below and LICENSE.txt. This is an
// altered (translated) version of the original software.

//! Ogg Vorbis decoder. Translation of stb_vorbis, in the configuration
//! SDL_mixer builds it with: its `STB_VORBIS_SDL` patches (reading through
//! an [`IoStream`], with a 2 KB buffer), `STB_VORBIS_NO_PUSHDATA_API`,
//! `STB_VORBIS_NO_STDIO`, `STB_VORBIS_MAX_CHANNELS` 8, and SDL's math
//! functions (sdl3's translations of them).
//!
//! Only what SDL_mixer uses is translated: opening a stream, its info,
//! comments and length, decoding frames as floats, and seeking to a frame.
//! Not translated: the pushdata API, the memory and stdio initializers, the
//! user-supplied allocation buffer, `stb_vorbis_seek()` (sample-exact
//! seeking within a frame), the `get_samples` functions and the integer
//! conversions.
//!
//! Original version written by Sean Barrett in 2007.
//!
//! Originally sponsored by RAD Game Tools. Seeking implementation
//! sponsored by Phillip Bennefall, Marc Andersen, Aaron Baker,
//! Elias Software, Aras Pranckevicius, and Sean Barrett.
//!
//! Feature contributors: Dougall Johnson (sample-exact seeking),
//! Vitaly Novichkov (sample-accurate tell).
//!
//! Bugfix/warning contributors: Terje Mathisen, Niklas Frykholm, Andy Hill,
//! Casey Muratori, John Bolton, Gargaj, Laurent Gomila, Marc LeBlanc,
//! Ronny Chevalier, Bernhard Wodo, Evan Balster, github:alxprd,
//! Tom Beaumont, Ingo Leitgeb, Nicolas Guillemot, Phillip Bennefall, Rohit,
//! Thiago Goulart, github:manxorist, Saga Musix, github:infatum,
//! Timur Gagiev, Maxwell Koo, Peter Waller, github:audinowho,
//! Dougall Johnson, David Reid, github:Clownacy, Pedro J. Estebanez,
//! Remi Verschelde, AnthoFoxo, github:morlat, Gabriel Ravier, Alice Rowan.
//!
//! Limitations:
//!
//!   - floor 0 not supported (used in old ogg vorbis files pre-2004)
//!   - lossless sample-truncation at beginning ignored
//!   - cannot concatenate multiple vorbis streams
//!   - sample positions are 32-bit, limiting seekable 192Khz
//!     files to around 6 hours (Ogg supports 64-bit)
//!
//! Where upstream asserts (SDL_mixer's builds compile its asserts out),
//! the condition is noted in a comment, since corrupt data can break it.

#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::manual_range_contains)]

use sdl3::io::{IoStream, IoWhence};
use sdl3::stdlib::math;

use crate::internal::try_alloc;

const STB_VORBIS_MAX_CHANNELS: usize = 8; /* For 7.1 surround sound */
const STB_VORBIS_FAST_HUFFMAN_LENGTH: i32 = 10;
const IO_BUFFER_SIZE: usize = 2048;

////////   ERROR CODES

/// Translation of `enum STBVorbisError`.
pub(crate) type StbVorbisError = i32;
pub(crate) const VORBIS__NO_ERROR: StbVorbisError = 0;
pub(crate) const VORBIS_NEED_MORE_DATA: StbVorbisError = 1; // not a real error
pub(crate) const VORBIS_INVALID_API_MIXING: StbVorbisError = 2; // can't mix API modes
pub(crate) const VORBIS_OUTOFMEM: StbVorbisError = 3; // not enough memory
pub(crate) const VORBIS_FEATURE_NOT_SUPPORTED: StbVorbisError = 4; // uses floor 0
pub(crate) const VORBIS_TOO_MANY_CHANNELS: StbVorbisError = 5; // STB_VORBIS_MAX_CHANNELS is too small
pub(crate) const VORBIS_FILE_OPEN_FAILURE: StbVorbisError = 6; // fopen() failed
pub(crate) const VORBIS_SEEK_WITHOUT_LENGTH: StbVorbisError = 7; // can't seek in unknown-length file
pub(crate) const VORBIS_UNEXPECTED_EOF: StbVorbisError = 10; // file is truncated?
pub(crate) const VORBIS_SEEK_INVALID: StbVorbisError = 11; // seek past EOF
                                                           // decoding errors (corrupt/invalid stream) -- you probably
                                                           // don't care about the exact details of these
                                                           // vorbis errors:
pub(crate) const VORBIS_INVALID_SETUP: StbVorbisError = 20;
pub(crate) const VORBIS_INVALID_STREAM: StbVorbisError = 21;
// ogg errors:
pub(crate) const VORBIS_MISSING_CAPTURE_PATTERN: StbVorbisError = 30;
pub(crate) const VORBIS_INVALID_STREAM_STRUCTURE_VERSION: StbVorbisError = 31;
pub(crate) const VORBIS_CONTINUED_PACKET_FLAG_INVALID: StbVorbisError = 32;
pub(crate) const VORBIS_INCORRECT_STREAM_SERIAL_NUMBER: StbVorbisError = 33;
pub(crate) const VORBIS_INVALID_FIRST_PAGE: StbVorbisError = 34;
pub(crate) const VORBIS_BAD_PACKET_TYPE: StbVorbisError = 35;
pub(crate) const VORBIS_CANT_FIND_LAST_PAGE: StbVorbisError = 36;
pub(crate) const VORBIS_SEEK_FAILED: StbVorbisError = 37;
pub(crate) const VORBIS_OGG_SKELETON_NOT_SUPPORTED: StbVorbisError = 38;

const MAX_BLOCKSIZE_LOG: i32 = 13; // from specification

const FAST_HUFFMAN_TABLE_SIZE: usize = 1 << STB_VORBIS_FAST_HUFFMAN_LENGTH;
const FAST_HUFFMAN_TABLE_MASK: u32 = (FAST_HUFFMAN_TABLE_SIZE - 1) as u32;

/// Translation of `Codebook`. (`sorted_values` has the extra slot upstream
/// puts before the array, so its index 0 is upstream's `[-1]`.)
#[derive(Default)]
struct Codebook {
    dimensions: i32,
    entries: i32,
    codeword_lengths: Vec<u8>,
    minimum_value: f32,
    delta_value: f32,
    value_bits: u8,
    lookup_type: u8,
    sequence_p: u8,
    sparse: u8,
    lookup_values: u64, /* entries (24 bits) * dimensions (16 bits) */
    multiplicands: Vec<f32>,
    codewords: Option<Vec<u32>>,
    fast_huffman: Vec<i16>,
    sorted_codewords: Option<Vec<u32>>,
    sorted_values: Vec<i32>,
    sorted_entries: i32,
}

/// Translation of `Floor1` (floor 0 isn't supported: a stream that uses it
/// fails to open).
#[derive(Clone)]
struct Floor1 {
    partitions: u8,
    partition_class_list: [u8; 32], // varies
    class_dimensions: [u8; 16],     // varies
    class_subclasses: [u8; 16],     // varies
    class_masterbooks: [u8; 16],    // varies
    subclass_books: [[i16; 8]; 16], // varies
    xlist: [u16; 31 * 8 + 2],       // varies
    sorted_order: [u8; 31 * 8 + 2],
    neighbors: [[u8; 2]; 31 * 8 + 2],
    floor1_multiplier: u8,
    rangebits: u8,
    values: i32,
}

impl Default for Floor1 {
    fn default() -> Self {
        Floor1 {
            partitions: 0,
            partition_class_list: [0; 32],
            class_dimensions: [0; 16],
            class_subclasses: [0; 16],
            class_masterbooks: [0; 16],
            subclass_books: [[0; 8]; 16],
            xlist: [0; 31 * 8 + 2],
            sorted_order: [0; 31 * 8 + 2],
            neighbors: [[0; 2]; 31 * 8 + 2],
            floor1_multiplier: 0,
            rangebits: 0,
            values: 0,
        }
    }
}

/// Translation of `Residue`.
#[derive(Default)]
struct Residue {
    begin: u32,
    end: u32,
    part_size: u32,
    classifications: u8,
    classbook: u8,
    classdata: Vec<Vec<u8>>,
    residue_books: Vec<[i16; 8]>,
}

/// Translation of `MappingChannel`.
#[derive(Clone, Copy, Default)]
struct MappingChannel {
    magnitude: u8,
    angle: u8,
    mux: u8,
}

/// Translation of `Mapping`.
#[derive(Default)]
struct Mapping {
    chan: Vec<MappingChannel>,
    coupling_steps: u16,
    submaps: u8,
    submap_floor: [u8; 16],   // varies
    submap_residue: [u8; 16], // varies
}

/// Translation of `Mode`.
#[derive(Clone, Copy, Default)]
struct Mode {
    blockflag: u8,
    mapping: u8,
    windowtype: u16,
    transformtype: u16,
}

/// Translation of `ProbedPage`.
#[derive(Clone, Copy, Default)]
struct ProbedPage {
    page_start: u32,
    page_end: u32,
    last_decoded_sample: u32,
}

/// The part of `struct stb_vorbis` that reads the stream: the input (with
/// SDL_mixer's buffering), the page, packet and bit reading state, and the
/// error.
struct Reader<'a> {
    io: IoStream<'a>,
    io_start: u32,
    io_virtual_pos: u32,
    io_buffer_pos: u32,
    io_buffer_fill: u32,
    io_buffer: Box<[u8; IO_BUFFER_SIZE]>,

    stream_len: u32,

    // p_first is the page on which the first audio packet ends
    // (but not necessarily the page on which it starts)
    p_first: ProbedPage,

    // run-time results
    eof: bool,
    error: StbVorbisError,

    // current page/packet/segment streaming info
    last_page: i32,
    segment_count: i32,
    segments: [u8; 255],
    page_flag: u8,
    bytes_in_seg: u8,
    first_decode: bool,
    next_seg: i32,
    last_seg: bool,      // flag that we're on the last segment
    last_seg_which: i32, // what was the segment number of the last seg?
    acc: u32,
    valid_bits: i32,
    packet_bytes: i32,
    end_seg_with_known_loc: i32,
    known_loc_for_packet: u32,
}

/// The setup data of `struct stb_vorbis`: what the headers say.
struct Setup {
    // user-accessible info
    sample_rate: u32,
    channels: i32,

    setup_memory_required: u32,
    temp_memory_required: u32,
    setup_temp_memory_required: u32,

    vendor: Vec<u8>,
    comment_list: Vec<Vec<u8>>,

    // the page to seek to when seeking to start, may be zero
    first_audio_page_offset: u32,

    // header info
    blocksize: [i32; 2],
    blocksize_0: i32,
    blocksize_1: i32,
    codebooks: Vec<Codebook>,
    floor_types: [u16; 64], // varies
    floor_config: Vec<Floor1>,
    residue_types: [u16; 64], // varies
    residue_config: Vec<Residue>,
    mapping: Vec<Mapping>,
    mode_count: i32,
    mode_config: [Mode; 64], // varies

    // per-blocksize precomputed data

    // twiddle factors
    a: [Vec<f32>; 2],
    b: [Vec<f32>; 2],
    c: [Vec<f32>; 2],
    window: [Vec<f32>; 2],
    bit_reverse: [Vec<u16>; 2],
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            sample_rate: 0,
            channels: 0,
            setup_memory_required: 0,
            temp_memory_required: 0,
            setup_temp_memory_required: 0,
            vendor: Vec::new(),
            comment_list: Vec::new(),
            first_audio_page_offset: 0,
            blocksize: [0; 2],
            blocksize_0: 0,
            blocksize_1: 0,
            codebooks: Vec::new(),
            floor_types: [0; 64],
            floor_config: Vec::new(),
            residue_types: [0; 64],
            residue_config: Vec::new(),
            mapping: Vec::new(),
            mode_count: 0,
            mode_config: [Mode::default(); 64],
            a: Default::default(),
            b: Default::default(),
            c: Default::default(),
            window: Default::default(),
            bit_reverse: Default::default(),
        }
    }
}

/// Translation of `struct stb_vorbis` (in three parts, so the setup data
/// can be read while the reader and the decoding state change).
pub(crate) struct StbVorbis<'a> {
    r: Reader<'a>,
    s: Setup,

    total_samples: u32,

    p_last: ProbedPage,

    // decode buffer
    channel_buffers: Vec<Vec<f32>>,

    previous_window: Vec<Vec<f32>>,
    previous_length: i32,

    final_y: Vec<Vec<i16>>,

    pub current_loc: u32, // sample location of next frame to decode
    current_loc_valid: bool,

    current_playback_loc: i32, // sample location of played samples
    current_playback_loc_valid: bool,

    pub discard_samples_deferred: i32,
    samples_output: u32,

    // sample-access
    channel_buffer_start: i32,
    channel_buffer_end: i32,
}

/// Translation of `stb_vorbis_info`.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // (the decoder uses the first two.)
pub(crate) struct StbVorbisInfo {
    pub sample_rate: u32,
    pub channels: i32,

    pub setup_memory_required: u32,
    pub setup_temp_memory_required: u32,
    pub temp_memory_required: u32,

    pub max_frame_size: i32,
}

/// The allocations upstream's `setup_malloc()` and `setup_temp_malloc()`
/// make: none for a size of 0 or more than `INT_MAX - 7` bytes (or if the
/// allocation fails).
fn setup_alloc<T: Clone + Default>(count: u64, size_of: u64) -> Option<Vec<T>> {
    let siz = count.wrapping_mul(size_of);
    let sz = siz as i32;
    if sz <= 0 || (i32::MAX as u64 - 7) < siz {
        return None;
    }
    try_alloc::<T>(count as usize).ok()
}

/// Translation of `crc32_init()` (computed at compile time).
const CRC32_POLY: u32 = 0x04c11db7; // from spec

static CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut s = (i as u32) << 24;
        let mut j = 0;
        while j < 8 {
            s = (s << 1) ^ (if s >= (1u32 << 31) { CRC32_POLY } else { 0 });
            j += 1;
        }
        table[i] = s;
        i += 1;
    }
    table
};

/// Translation of `crc32_update()`.
fn crc32_update(crc: u32, byte: u8) -> u32 {
    (crc << 8) ^ CRC_TABLE[(byte as u32 ^ (crc >> 24)) as usize]
}

// used in setup, and for huffman that doesn't go fast path
/// Translation of `bit_reverse()`.
fn bit_reverse(mut n: u32) -> u32 {
    n = ((n & 0xAAAAAAAA) >> 1) | ((n & 0x55555555) << 1);
    n = ((n & 0xCCCCCCCC) >> 2) | ((n & 0x33333333) << 2);
    n = ((n & 0xF0F0F0F0) >> 4) | ((n & 0x0F0F0F0F) << 4);
    n = ((n & 0xFF00FF00) >> 8) | ((n & 0x00FF00FF) << 8);
    (n >> 16) | (n << 16)
}

/// Translation of `square()`.
fn square(x: f32) -> f32 {
    x * x
}

// this is a weird definition of log2() for which log2(1) = 1, log2(2) = 2, log2(4) = 3
// as required by the specification. fast(?) implementation from stb.h
// @OPTIMIZE: called multiple times per-packet with "constants"; move to setup
/// Translation of `ilog()`.
fn ilog(n: i32) -> i32 {
    static LOG2_4: [i8; 16] = [0, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4];

    if n < 0 {
        return 0; // signed n returns 0
    }

    // 2 compares if n < 16, 3 compares otherwise (4 if signed or n > 1<<29)
    if n < (1 << 14) {
        if n < (1 << 4) {
            LOG2_4[n as usize] as i32
        } else if n < (1 << 9) {
            5 + LOG2_4[(n >> 5) as usize] as i32
        } else {
            10 + LOG2_4[(n >> 10) as usize] as i32
        }
    } else if n < (1 << 24) {
        if n < (1 << 19) {
            15 + LOG2_4[(n >> 15) as usize] as i32
        } else {
            20 + LOG2_4[(n >> 20) as usize] as i32
        }
    } else if n < (1 << 29) {
        25 + LOG2_4[(n >> 25) as usize] as i32
    } else {
        30 + LOG2_4[(n >> 30) as usize] as i32
    }
}

// (`M_PI`, which SDL_mixer's build gets from stb_vorbis's own fallback: a float.)
const M_PI: f32 = 3.14159265358979323846264;

// code length assigned to a value with no huffman encoding
const NO_CODE: u8 = 255;

/////////////////////// LEAF SETUP FUNCTIONS //////////////////////////
//
// these functions are only called at setup, and only a few times
// per file

/// Translation of `float32_unpack()`.
fn float32_unpack(x: u32) -> f32 {
    // from the specification
    let mantissa = x & 0x1fffff;
    let sign = x & 0x80000000;
    let exp = (x & 0x7fe00000) >> 21;
    let res: f64 = if sign != 0 {
        -(mantissa as f64)
    } else {
        mantissa as f64
    };
    math::scalbn(res as f32 as f64, exp as i32 - 788) as f32
}

// zlib & jpeg huffman tables assume that the output symbols
// can either be arbitrarily arranged, or have monotonically
// increasing frequencies--they rely on the lengths being sorted;
// this makes for a very simple generation algorithm.
// vorbis allows a huffman table with non-sorted lengths. This
// requires a more sophisticated construction, since symbols in
// order do not map to huffman codes "in order".
/// Translation of `add_entry()`.
fn add_entry(
    c: &mut Codebook,
    huff_code: u32,
    symbol: i32,
    count: i32,
    len: i32,
    values: &mut [u32],
) {
    let codewords = c.codewords.as_mut().expect("codewords");
    if c.sparse == 0 {
        codewords[symbol as usize] = huff_code;
    } else {
        codewords[count as usize] = huff_code;
        c.codeword_lengths[count as usize] = len as u8;
        values[count as usize] = symbol as u32;
    }
}

/// Translation of `compute_codewords()`.
fn compute_codewords(c: &mut Codebook, len: &[u8], n: i32, values: &mut [u32]) -> bool {
    let mut available = [0u32; 32];
    let mut m = 0;

    // find the first entry
    let mut k = 0;
    while k < n {
        if len[k as usize] < NO_CODE {
            break;
        }
        k += 1;
    }
    if k == n {
        // (assert(c->sorted_entries == 0))
        return true;
    }
    // assert(len[k] < 32); // no error return required, code reading lens checks this
    // add to the list
    add_entry(c, 0, k, m, len[k as usize] as i32, values);
    m += 1;
    // add all available leaves
    for i in 1..=len[k as usize] as usize {
        available[i] = 1u32 << (32 - i);
    }
    // note that the above code treats the first case specially,
    // but it's really the same as the following code, so they
    // could probably be combined (except the initial code is 0,
    // and I use 0 in available[] to mean 'empty')
    for i in (k + 1)..n {
        let mut z = len[i as usize] as i32;
        if z == NO_CODE as i32 {
            continue;
        }
        // assert(z < 32); // no error return required, code reading lens checks this
        // find lowest available leaf (should always be earliest,
        // which is what the specification calls for)
        // note that this property, and the fact we can never have
        // more than one free leaf at a given level, isn't totally
        // trivial to prove, but it seems true and the assert never
        // fires, so!
        while z > 0 && available[z as usize] == 0 {
            z -= 1;
        }
        if z == 0 {
            return false;
        }
        let res = available[z as usize];
        available[z as usize] = 0;
        add_entry(c, bit_reverse(res), i, m, len[i as usize] as i32, values);
        m += 1;
        // propagate availability up the tree
        if z != len[i as usize] as i32 {
            let mut y = len[i as usize] as i32;
            while y > z {
                // (assert(available[y] == 0))
                available[y as usize] = res.wrapping_add(1u32.wrapping_shl((32 - y) as u32));
                y -= 1;
            }
        }
    }
    true
}

// accelerated huffman table allows fast O(1) match of all symbols
// of length <= STB_VORBIS_FAST_HUFFMAN_LENGTH
/// Translation of `compute_accelerated_huffman()`.
fn compute_accelerated_huffman(c: &mut Codebook) {
    c.fast_huffman = vec![-1; FAST_HUFFMAN_TABLE_SIZE];

    let mut len = if c.sparse != 0 {
        c.sorted_entries
    } else {
        c.entries
    };
    if len > 32767 {
        len = 32767; // largest possible value we can encode!
    }
    for i in 0..len.max(0) as usize {
        if c.codeword_lengths[i] as i32 <= STB_VORBIS_FAST_HUFFMAN_LENGTH {
            let mut z = if c.sparse != 0 {
                bit_reverse(c.sorted_codewords.as_ref().expect("sorted codewords")[i])
            } else {
                c.codewords.as_ref().expect("codewords")[i]
            };
            // set table entries for all bit combinations in the higher bits
            while (z as usize) < FAST_HUFFMAN_TABLE_SIZE {
                c.fast_huffman[z as usize] = i as i16;
                z += 1 << c.codeword_lengths[i];
            }
        }
    }
}

/// Translation of `include_in_sort()`.
fn include_in_sort(c: &Codebook, len: u8) -> bool {
    if c.sparse != 0 {
        // (assert(len != NO_CODE))
        return true;
    }
    if len == NO_CODE {
        return false;
    }
    if len as i32 > STB_VORBIS_FAST_HUFFMAN_LENGTH {
        return true;
    }
    false
}

// if the fast table above doesn't work, we want to binary
// search them... need to reverse the bits
/// Translation of `compute_sorted_huffman()`.
fn compute_sorted_huffman(c: &mut Codebook, lengths: &[u8], values: &[u32]) {
    let sorted_entries = c.sorted_entries as usize;
    // build a list of all the entries
    // OPTIMIZATION: don't include the short ones, since they'll be caught by FAST_HUFFMAN.
    // this is kind of a frivolous optimization--I don't see any performance improvement,
    // but it's like 4 extra lines of code, so.
    {
        let codewords = c.codewords.as_ref().expect("codewords");
        let mut sorted = std::mem::take(c.sorted_codewords.as_mut().expect("sorted codewords"));
        if c.sparse == 0 {
            let mut k = 0;
            for i in 0..c.entries as usize {
                if include_in_sort(c, lengths[i]) {
                    sorted[k] = bit_reverse(codewords[i]);
                    k += 1;
                }
            }
            // (assert(k == c->sorted_entries))
        } else {
            for i in 0..sorted_entries {
                sorted[i] = bit_reverse(codewords[i]);
            }
        }

        // (SDL_qsort: the order of equal codewords doesn't matter.)
        sorted[..sorted_entries].sort_unstable();
        sorted[sorted_entries] = 0xffffffff;
        c.sorted_codewords = Some(sorted);
    }

    let len = if c.sparse != 0 {
        c.sorted_entries
    } else {
        c.entries
    };
    // now we need to indicate how they correspond; we could either
    //   #1: sort a different data structure that says who they correspond to
    //   #2: for each sorted entry, search the original list to find who corresponds
    //   #3: for each original entry, find the sorted entry
    // #1 requires extra storage, #2 is slow, #3 can use binary search!
    for i in 0..len.max(0) as usize {
        let huff_len = if c.sparse != 0 {
            lengths[values[i] as usize]
        } else {
            lengths[i]
        };
        if include_in_sort(c, huff_len) {
            let code = bit_reverse(c.codewords.as_ref().expect("codewords")[i]);
            let sorted = c.sorted_codewords.as_ref().expect("sorted codewords");
            let mut x = 0usize;
            let mut n = c.sorted_entries;
            while n > 1 {
                // invariant: sc[x] <= code < sc[x+n]
                let m = x + (n >> 1) as usize;
                if sorted[m] <= code {
                    x = m;
                    n -= n >> 1;
                } else {
                    n >>= 1;
                }
            }
            // (assert(c->sorted_codewords[x] == code))
            if c.sparse != 0 {
                c.sorted_values[x + 1] = values[i] as i32;
                c.codeword_lengths[x] = huff_len;
            } else {
                c.sorted_values[x + 1] = i as i32;
            }
        }
    }
}

// only run while parsing the header (3 times)
/// Translation of `vorbis_validate()`.
fn vorbis_validate(data: &[u8]) -> bool {
    &data[..6] == b"vorbis"
}

// called from setup only, once per code book
// (formula implied by specification)
/// Translation of `lookup1_values()`.
fn lookup1_values(entries: i32, dim: i32) -> i32 {
    // (the float-to-int casts out of range, which C leaves undefined, are
    // x86's: the "integer indefinite" i32::MIN.)
    fn to_int(x: f64) -> i32 {
        if x.is_nan() || x >= 2147483648.0 || x < -2147483648.0 {
            i32::MIN
        } else {
            x as i32
        }
    }
    let mut r = to_int(math::floor(math::exp(
        ((math::log(entries as f32 as f64) as f32) / dim as f32) as f64,
    )));
    if to_int(math::floor(math::pow((r as f32 + 1.0) as f64, dim as f64))) <= entries {
        // (int) cast for MinGW warning;
        r += 1; // floor() to avoid _ftol() when non-CRT
    }
    if math::pow((r as f32 + 1.0) as f64, dim as f64) <= entries as f64 {
        return -1;
    }
    if to_int(math::floor(math::pow(r as f32 as f64, dim as f64))) > entries {
        return -1;
    }
    r
}

// called twice per file
/// Translation of `compute_twiddle_factors()`.
fn compute_twiddle_factors(n: i32, a: &mut [f32], b: &mut [f32], c: &mut [f32]) {
    let n4 = n >> 2;
    let n8 = n >> 3;

    let mut k2 = 0usize;
    for k in 0..n4 {
        a[k2] = math::cos(((4 * k) as f32 * M_PI / n as f32) as f64) as f32;
        a[k2 + 1] = -(math::sin(((4 * k) as f32 * M_PI / n as f32) as f64) as f32);
        b[k2] = (math::cos(((k2 as i32 + 1) as f32 * M_PI / n as f32 / 2.0) as f64) as f32) * 0.5;
        b[k2 + 1] =
            (math::sin(((k2 as i32 + 1) as f32 * M_PI / n as f32 / 2.0) as f64) as f32) * 0.5;
        k2 += 2;
    }
    let mut k2 = 0usize;
    for _ in 0..n8 {
        c[k2] = math::cos(((2 * (k2 as i32 + 1)) as f32 * M_PI / n as f32) as f64) as f32;
        c[k2 + 1] = -(math::sin(((2 * (k2 as i32 + 1)) as f32 * M_PI / n as f32) as f64) as f32);
        k2 += 2;
    }
}

/// Translation of `compute_window()`.
fn compute_window(n: i32, window: &mut [f32]) {
    let n2 = n >> 1;
    for i in 0..n2 {
        window[i as usize] = math::sin(
            0.5 * M_PI as f64
                * square(math::sin((i as f64 - 0.0 + 0.5) / n2 as f64 * 0.5 * M_PI as f64) as f32)
                    as f64,
        ) as f32;
    }
}

/// Translation of `compute_bitreverse()`.
fn compute_bitreverse(n: i32, rev: &mut [u16]) {
    let ld = ilog(n) - 1; // ilog is off-by-one from normal definitions
    let n8 = n >> 3;
    for i in 0..n8 {
        rev[i as usize] = ((bit_reverse(i as u32) >> (32 - ld + 3)) << 2) as u16;
    }
}

/// Translation of `neighbors()`.
fn neighbors(x: &[u16], n: usize, plow: &mut i32, phigh: &mut i32) {
    let mut low: i32 = -1;
    let mut high: i32 = 65536;
    for i in 0..n {
        if x[i] as i32 > low && x[i] < x[n] {
            *plow = i as i32;
            low = x[i] as i32;
        }
        if (x[i] as i32) < high && x[i] > x[n] {
            *phigh = i as i32;
            high = x[i] as i32;
        }
    }
}

//
/////////////////////// END LEAF SETUP FUNCTIONS //////////////////////////

const PAGEFLAG_CONTINUED_PACKET: u8 = 1;
const PAGEFLAG_FIRST_PAGE: u8 = 2;
const PAGEFLAG_LAST_PAGE: u8 = 4;

const EOP: i32 = -1;
const INVALID_BITS: i32 = -1;

const OGG_PAGE_HEADER: [u8; 4] = [0x4f, 0x67, 0x67, 0x53];

impl Reader<'_> {
    /// Translation of `error()`.
    fn error(&mut self, e: StbVorbisError) -> bool {
        self.error = e;
        if !self.eof && e != VORBIS_NEED_MORE_DATA {
            self.error = e; // breakpoint for debugging
        }
        false
    }

    /// Translation of `get8()`.
    fn get8(&mut self) -> u8 {
        if self.io_buffer_pos >= self.io_buffer_fill {
            self.io_buffer_fill = self.io.read(&mut self.io_buffer[..]) as u32;
            self.io_buffer_pos = 0;
            if self.io_buffer_fill == 0 {
                self.eof = true;
                return 0;
            }
        }
        self.io_virtual_pos = self.io_virtual_pos.wrapping_add(1);
        let b = self.io_buffer[self.io_buffer_pos as usize];
        self.io_buffer_pos += 1;
        b
    }

    /// Translation of `get32()`.
    fn get32(&mut self) -> u32 {
        let mut x = self.get8() as u32;
        x += (self.get8() as u32) << 8;
        x += (self.get8() as u32) << 16;
        x += (self.get8() as u32) << 24;
        x
    }

    /// Translation of `getn()`.
    fn getn(&mut self, data: &mut [u8]) -> bool {
        let mut n = data.len();
        let mut at = 0;
        while n > 0 {
            if self.io_buffer_pos >= self.io_buffer_fill {
                self.io_buffer_fill = self.io.read(&mut self.io_buffer[..]) as u32;
                self.io_buffer_pos = 0;
                if self.io_buffer_fill == 0 {
                    self.eof = true;
                    return false;
                }
            }

            let mut chunk = (self.io_buffer_fill - self.io_buffer_pos) as usize;
            if chunk > n {
                chunk = n;
            }

            let pos = self.io_buffer_pos as usize;
            data[at..at + chunk].copy_from_slice(&self.io_buffer[pos..pos + chunk]);
            self.io_buffer_pos += chunk as u32;
            self.io_virtual_pos = self.io_virtual_pos.wrapping_add(chunk as u32);
            at += chunk;
            n -= chunk;
        }
        true
    }

    /// Translation of `skip()`.
    fn skip(&mut self, n: i32) {
        self.set_file_offset(self.io_virtual_pos.wrapping_add(n as u32));
    }

    /// Translation of `set_file_offset()`.
    fn set_file_offset(&mut self, loc: u32) -> bool {
        self.eof = false;

        let buffer_start = self.io_virtual_pos.wrapping_sub(self.io_buffer_pos);
        let buffer_end = buffer_start.wrapping_add(self.io_buffer_fill);
        self.io_virtual_pos = loc;

        // Move within buffer if possible
        if loc >= buffer_start && loc < buffer_end {
            self.io_buffer_pos = loc - buffer_start;
            return true;
        }

        let mut io_pos = loc.wrapping_add(self.io_start);
        if io_pos < loc || loc >= 0x80000000 {
            io_pos = 0x7fffffff;
            self.eof = true;
        }

        self.io_buffer_pos = 0; // Invalidate buffer
        self.io_buffer_fill = 0;
        if self.io.seek(io_pos as i64, IoWhence::Set).is_ok() {
            return true;
        }
        self.eof = true;
        let _ = self.io.seek(self.io_start as i64, IoWhence::End);
        false
    }

    /// Translation of `stb_vorbis_get_file_offset()`.
    fn get_file_offset(&self) -> u32 {
        self.io_virtual_pos
    }

    /// Translation of `capture_pattern()`.
    fn capture_pattern(&mut self) -> bool {
        if 0x4f != self.get8() {
            return false;
        }
        if 0x67 != self.get8() {
            return false;
        }
        if 0x67 != self.get8() {
            return false;
        }
        if 0x53 != self.get8() {
            return false;
        }
        true
    }

    /// Translation of `start_page_no_capturepattern()`.
    fn start_page_no_capturepattern(&mut self) -> bool {
        if self.first_decode {
            self.p_first.page_start = self.get_file_offset().wrapping_sub(4);
        }
        // stream structure version
        if 0 != self.get8() {
            return self.error(VORBIS_INVALID_STREAM_STRUCTURE_VERSION);
        }
        // header flag
        self.page_flag = self.get8();
        // absolute granule position
        let loc0 = self.get32();
        let loc1 = self.get32();
        // @TODO: validate loc0,loc1 as valid positions?
        // stream serial number -- vorbis doesn't interleave, so discard
        self.get32();
        //if (f->serial != get32(f)) return error(f, VORBIS_incorrect_stream_serial_number);
        // page sequence number
        let n = self.get32();
        self.last_page = n as i32;
        // CRC32
        self.get32();
        // page_segments
        self.segment_count = self.get8() as i32;
        let mut segments = self.segments;
        let ok = self.getn(&mut segments[..self.segment_count as usize]);
        self.segments = segments;
        if !ok {
            return self.error(VORBIS_UNEXPECTED_EOF);
        }
        // assume we _don't_ know any the sample position of any segments
        self.end_seg_with_known_loc = -2;
        if loc0 != !0u32 || loc1 != !0u32 {
            // determine which packet is the last one that will complete
            let mut i = self.segment_count - 1;
            while i >= 0 {
                if self.segments[i as usize] < 255 {
                    break;
                }
                i -= 1;
            }
            // 'i' is now the index of the _last_ segment of a packet that ends
            if i >= 0 {
                self.end_seg_with_known_loc = i;
                self.known_loc_for_packet = loc0;
            }
        }
        if self.first_decode {
            let mut len: i32 = 0;
            for i in 0..self.segment_count as usize {
                len += self.segments[i] as i32;
            }
            len += 27 + self.segment_count;
            self.p_first.page_end = self.p_first.page_start.wrapping_add(len as u32);
            self.p_first.last_decoded_sample = loc0;
        }
        self.next_seg = 0;
        true
    }

    /// Translation of `start_page()`.
    fn start_page(&mut self) -> bool {
        if !self.capture_pattern() {
            return self.error(VORBIS_MISSING_CAPTURE_PATTERN);
        }
        self.start_page_no_capturepattern()
    }

    /// Translation of `start_packet()`.
    fn start_packet(&mut self) -> bool {
        while self.next_seg == -1 {
            if !self.start_page() {
                return false;
            }
            if self.page_flag & PAGEFLAG_CONTINUED_PACKET != 0 {
                return self.error(VORBIS_CONTINUED_PACKET_FLAG_INVALID);
            }
        }
        self.last_seg = false;
        self.valid_bits = 0;
        self.packet_bytes = 0;
        self.bytes_in_seg = 0;
        // f->next_seg is now valid
        true
    }

    /// Translation of `maybe_start_packet()`.
    fn maybe_start_packet(&mut self) -> bool {
        if self.next_seg == -1 {
            let x = self.get8();
            if self.eof {
                return false; // EOF at page boundary is not an error!
            }
            if 0x4f != x {
                return self.error(VORBIS_MISSING_CAPTURE_PATTERN);
            }
            if 0x67 != self.get8() {
                return self.error(VORBIS_MISSING_CAPTURE_PATTERN);
            }
            if 0x67 != self.get8() {
                return self.error(VORBIS_MISSING_CAPTURE_PATTERN);
            }
            if 0x53 != self.get8() {
                return self.error(VORBIS_MISSING_CAPTURE_PATTERN);
            }
            if !self.start_page_no_capturepattern() {
                return false;
            }
            if self.page_flag & PAGEFLAG_CONTINUED_PACKET != 0 {
                // set up enough state that we can read this packet if we want,
                // e.g. during recovery
                self.last_seg = false;
                self.bytes_in_seg = 0;
                return self.error(VORBIS_CONTINUED_PACKET_FLAG_INVALID);
            }
        }
        self.start_packet()
    }

    /// Translation of `next_segment()`.
    fn next_segment(&mut self) -> i32 {
        if self.last_seg {
            return 0;
        }
        if self.next_seg == -1 {
            self.last_seg_which = self.segment_count - 1; // in case start_page fails
            if !self.start_page() {
                self.last_seg = true;
                return 0;
            }
            if self.page_flag & PAGEFLAG_CONTINUED_PACKET == 0 {
                return self.error(VORBIS_CONTINUED_PACKET_FLAG_INVALID) as i32;
            }
        }
        // (a segment past the page's would be outside the segment table;
        // reading one, upstream reads the bytes around it.)
        let len = self
            .segments
            .get(self.next_seg as usize)
            .copied()
            .unwrap_or(0) as i32;
        self.next_seg += 1;
        if len < 255 {
            self.last_seg = true;
            self.last_seg_which = self.next_seg - 1;
        }
        if self.next_seg >= self.segment_count {
            self.next_seg = -1;
        }
        // (assert(f->bytes_in_seg == 0))
        self.bytes_in_seg = len as u8;
        len
    }

    /// Translation of `get8_packet_raw()`.
    fn get8_packet_raw(&mut self) -> i32 {
        if self.bytes_in_seg == 0 {
            // CLANG!
            if self.last_seg || self.next_segment() == 0 {
                return EOP;
            }
        }
        // (assert(f->bytes_in_seg > 0))
        self.bytes_in_seg = self.bytes_in_seg.wrapping_sub(1);
        self.packet_bytes += 1;
        self.get8() as i32
    }

    /// Translation of `get8_packet()`.
    fn get8_packet(&mut self) -> i32 {
        let x = self.get8_packet_raw();
        self.valid_bits = 0;
        x
    }

    /// Translation of `get32_packet()`.
    fn get32_packet(&mut self) -> i32 {
        let mut x = self.get8_packet() as u32;
        x = x.wrapping_add((self.get8_packet() as u32) << 8);
        x = x.wrapping_add((self.get8_packet() as u32) << 16);
        x = x.wrapping_add((self.get8_packet() as u32) << 24);
        x as i32
    }

    /// Translation of `flush_packet()`.
    fn flush_packet(&mut self) {
        while self.get8_packet_raw() != EOP {}
    }

    // @OPTIMIZE: this is the secondary bit decoder, so it's probably not as important
    // as the huffman decoder?
    /// Translation of `get_bits()`.
    fn get_bits(&mut self, n: i32) -> u32 {
        if self.valid_bits < 0 {
            return 0;
        }
        if self.valid_bits < n {
            if n > 24 {
                // the accumulator technique below would not work correctly in this case
                let mut z = self.get_bits(24);
                z = z.wrapping_add(self.get_bits(n - 24) << 24);
                return z;
            }
            if self.valid_bits == 0 {
                self.acc = 0;
            }
            while self.valid_bits < n {
                let z = self.get8_packet_raw();
                if z == EOP {
                    self.valid_bits = INVALID_BITS;
                    return 0;
                }
                self.acc = self.acc.wrapping_add((z as u32) << self.valid_bits);
                self.valid_bits += 8;
            }
        }

        // (assert(f->valid_bits >= n))
        let z = self.acc & ((1u32 << n) - 1);
        self.acc >>= n;
        self.valid_bits -= n;
        z
    }

    // @OPTIMIZE: primary accumulator for huffman
    // expand the buffer to as many bits as possible without reading off end of packet
    // it might be nice to allow f->valid_bits and f->acc to be stored in registers,
    // e.g. cache them locally and decode locally
    /// Translation of `prep_huffman()`.
    fn prep_huffman(&mut self) {
        if self.valid_bits <= 24 {
            if self.valid_bits == 0 {
                self.acc = 0;
            }
            loop {
                if self.last_seg && self.bytes_in_seg == 0 {
                    return;
                }
                let z = self.get8_packet_raw();
                if z == EOP {
                    return;
                }
                self.acc = self
                    .acc
                    .wrapping_add((z as u32).wrapping_shl(self.valid_bits as u32));
                self.valid_bits += 8;
                if self.valid_bits > 24 {
                    break;
                }
            }
        }
    }
}

const VORBIS_PACKET_ID: i32 = 1;
const VORBIS_PACKET_COMMENT: i32 = 3;
const VORBIS_PACKET_SETUP: i32 = 5;

/// Translation of `codebook_decode_scalar_raw()`.
fn codebook_decode_scalar_raw(f: &mut Reader<'_>, c: &Codebook) -> i32 {
    f.prep_huffman();

    if c.codewords.is_none() && c.sorted_codewords.is_none() {
        return -1;
    }

    // cases to use binary search: sorted_codewords && !c->codewords
    //                             sorted_codewords && c->entries > 8
    if if c.entries > 8 {
        c.sorted_codewords.is_some()
    } else {
        c.codewords.is_none()
    } {
        // binary search
        let code = bit_reverse(f.acc);
        let sorted = c.sorted_codewords.as_ref().expect("sorted codewords");
        let mut x: i32 = 0;
        let mut n = c.sorted_entries;

        while n > 1 {
            // invariant: sc[x] <= code < sc[x+n]
            let m = x + (n >> 1);
            if sorted[m as usize] <= code {
                x = m;
                n -= n >> 1;
            } else {
                n >>= 1;
            }
        }
        // x is now the sorted index
        if c.sparse == 0 {
            x = c.sorted_values[x as usize + 1];
        }
        // x is now sorted index if sparse, or symbol otherwise
        let len = c.codeword_lengths[x as usize] as i32;
        if f.valid_bits >= len {
            f.acc >>= len;
            f.valid_bits -= len;
            return x;
        }

        f.valid_bits = 0;
        return -1;
    }

    // if small, linear search
    // (assert(!c->sparse))
    let codewords = c.codewords.as_ref().expect("codewords");
    for i in 0..c.entries as usize {
        if c.codeword_lengths[i] == NO_CODE {
            continue;
        }
        /* unsigned left shift for 32-bit codewords.
         * https://github.com/nothings/stb/issues/1168 */
        if codewords[i]
            == (f.acc
                & (1u32
                    .wrapping_shl(c.codeword_lengths[i] as u32)
                    .wrapping_sub(1)))
        {
            if f.valid_bits >= c.codeword_lengths[i] as i32 {
                f.acc = f.acc.wrapping_shr(c.codeword_lengths[i] as u32);
                f.valid_bits -= c.codeword_lengths[i] as i32;
                return i as i32;
            }
            f.valid_bits = 0;
            return -1;
        }
    }

    f.error(VORBIS_INVALID_STREAM);
    f.valid_bits = 0;
    -1
}

/// `DECODE_RAW()`
fn decode_raw(f: &mut Reader<'_>, c: &Codebook) -> i32 {
    if f.valid_bits < STB_VORBIS_FAST_HUFFMAN_LENGTH {
        f.prep_huffman();
    }
    let mut var = (f.acc & FAST_HUFFMAN_TABLE_MASK) as i32;
    var = c.fast_huffman[var as usize] as i32;
    if var >= 0 {
        let n = c.codeword_lengths[var as usize] as i32;
        f.acc = f.acc.wrapping_shr(n as u32);
        f.valid_bits -= n;
        if f.valid_bits < 0 {
            f.valid_bits = 0;
            var = -1;
        }
    } else {
        var = codebook_decode_scalar_raw(f, c);
    }
    var
}

/// `DECODE()`
fn decode(f: &mut Reader<'_>, c: &Codebook) -> i32 {
    let mut var = decode_raw(f, c);
    if c.sparse != 0 && var >= 0 {
        var = c.sorted_values[var as usize + 1];
    }
    var
}

// CODEBOOK_ELEMENT_FAST is an optimization for the CODEBOOK_FLOATS case
// where we avoid one addition
// (`CODEBOOK_ELEMENT(c,off)` is `c->multiplicands[off]`, and
// `CODEBOOK_ELEMENT_BASE(c)` is 0.)

/// Translation of `codebook_decode_start()`.
fn codebook_decode_start(f: &mut Reader<'_>, c: &Codebook) -> i32 {
    let mut z = -1;

    // type 0 is only legal in a scalar context
    if c.lookup_type == 0 {
        f.error(VORBIS_INVALID_STREAM);
    } else {
        z = decode_raw(f, c); // DECODE_VQ
                              // (assert(!c->sparse || z < c->sorted_entries))
        if z < 0 {
            // check for EOP
            if f.bytes_in_seg == 0 && f.last_seg {
                return z;
            }
            f.error(VORBIS_INVALID_STREAM);
        }
    }
    z
}

/// Translation of `codebook_decode()`: into `output[at..at + len]`.
fn codebook_decode(
    f: &mut Reader<'_>,
    c: &Codebook,
    output: &mut [f32],
    at: usize,
    mut len: i32,
) -> bool {
    let mut z = codebook_decode_start(f, c);
    if z < 0 {
        return false;
    }
    if len > c.dimensions {
        len = c.dimensions;
    }

    z = z.wrapping_mul(c.dimensions);
    if c.sequence_p != 0 {
        let mut last = 0.0f32;
        for i in 0..len.max(0) as usize {
            let val = c.multiplicands[z as usize + i] + last;
            output[at + i] += val;
            last = val + c.minimum_value;
        }
    } else {
        let last = 0.0f32;
        for i in 0..len.max(0) as usize {
            output[at + i] += c.multiplicands[z as usize + i] + last;
        }
    }

    true
}

/// Translation of `codebook_decode_step()`: into `output[at + i * step]`.
fn codebook_decode_step(
    f: &mut Reader<'_>,
    c: &Codebook,
    output: &mut [f32],
    at: usize,
    mut len: i32,
    step: i32,
) -> bool {
    let mut z = codebook_decode_start(f, c);
    let mut last = 0.0f32;
    if z < 0 {
        return false;
    }
    if len > c.dimensions {
        len = c.dimensions;
    }

    z = z.wrapping_mul(c.dimensions);
    for i in 0..len.max(0) as usize {
        let val = c.multiplicands[z as usize + i] + last;
        output[at + i * step as usize] += val;
        if c.sequence_p != 0 {
            last = val;
        }
    }

    true
}

/// Translation of `codebook_decode_deinterleave_repeat()`.
fn codebook_decode_deinterleave_repeat(
    f: &mut Reader<'_>,
    c: &Codebook,
    outputs: &mut [Option<&mut Vec<f32>>],
    ch: i32,
    c_inter_p: &mut i32,
    p_inter_p: &mut i32,
    len: i32,
    mut total_decode: i32,
) -> bool {
    let mut c_inter = *c_inter_p;
    let mut p_inter = *p_inter_p;
    let mut effective = c.dimensions;

    // type 0 is only legal in a scalar context
    if c.lookup_type == 0 {
        return f.error(VORBIS_INVALID_STREAM);
    }

    while total_decode > 0 {
        let mut last = 0.0f32;
        let mut z = decode_raw(f, c); // DECODE_VQ
                                      // (assert(!c->sparse || z < c->sorted_entries))
        if z < 0 {
            if f.bytes_in_seg == 0 && f.last_seg {
                return false;
            }
            return f.error(VORBIS_INVALID_STREAM);
        }

        // if this will take us off the end of the buffers, stop short!
        // we check by computing the length of the virtual interleaved
        // buffer (len*ch), our current offset within it (p_inter*ch)+(c_inter),
        // and the length we'll be using (effective)
        if c_inter + p_inter * ch + effective > len * ch {
            effective = len * ch - (p_inter * ch + c_inter);
        }

        z = z.wrapping_mul(c.dimensions);
        if c.sequence_p != 0 {
            for i in 0..effective.max(0) as usize {
                let val = c.multiplicands[z as usize + i] + last;
                if let Some(out) = outputs[c_inter as usize].as_deref_mut() {
                    out[p_inter as usize] += val;
                }
                c_inter += 1;
                if c_inter == ch {
                    c_inter = 0;
                    p_inter += 1;
                }
                last = val;
            }
        } else {
            for i in 0..effective.max(0) as usize {
                let val = c.multiplicands[z as usize + i] + last;
                if let Some(out) = outputs[c_inter as usize].as_deref_mut() {
                    out[p_inter as usize] += val;
                }
                c_inter += 1;
                if c_inter == ch {
                    c_inter = 0;
                    p_inter += 1;
                }
            }
        }

        total_decode -= effective;
    }
    *c_inter_p = c_inter;
    *p_inter_p = p_inter;
    true
}

/// Translation of `predict_point()`.
fn predict_point(x: i32, x0: i32, x1: i32, y0: i32, y1: i32) -> i32 {
    let dy = y1 - y0;
    let adx = x1 - x0;
    // @OPTIMIZE: force int division to round in the right direction... is this necessary on x86?
    let err = dy.abs() * (x - x0);
    let off = err / adx;
    if dy < 0 {
        y0 - off
    } else {
        y0 + off
    }
}

// the following table is block-copied from the specification
static INVERSE_DB_TABLE: [f32; 256] = [
    1.0649863e-07,
    1.1341951e-07,
    1.2079015e-07,
    1.2863978e-07,
    1.3699951e-07,
    1.4590251e-07,
    1.5538408e-07,
    1.6548181e-07,
    1.7623575e-07,
    1.8768855e-07,
    1.9988561e-07,
    2.1287530e-07,
    2.2670913e-07,
    2.4144197e-07,
    2.5713223e-07,
    2.7384213e-07,
    2.9163793e-07,
    3.1059021e-07,
    3.3077411e-07,
    3.5226968e-07,
    3.7516214e-07,
    3.9954229e-07,
    4.2550680e-07,
    4.5315863e-07,
    4.8260743e-07,
    5.1396998e-07,
    5.4737065e-07,
    5.8294187e-07,
    6.2082472e-07,
    6.6116941e-07,
    7.0413592e-07,
    7.4989464e-07,
    7.9862701e-07,
    8.5052630e-07,
    9.0579828e-07,
    9.6466216e-07,
    1.0273513e-06,
    1.0941144e-06,
    1.1652161e-06,
    1.2409384e-06,
    1.3215816e-06,
    1.4074654e-06,
    1.4989305e-06,
    1.5963394e-06,
    1.7000785e-06,
    1.8105592e-06,
    1.9282195e-06,
    2.0535261e-06,
    2.1869758e-06,
    2.3290978e-06,
    2.4804557e-06,
    2.6416497e-06,
    2.8133190e-06,
    2.9961443e-06,
    3.1908506e-06,
    3.3982101e-06,
    3.6190449e-06,
    3.8542308e-06,
    4.1047004e-06,
    4.3714470e-06,
    4.6555282e-06,
    4.9580707e-06,
    5.2802740e-06,
    5.6234160e-06,
    5.9888572e-06,
    6.3780469e-06,
    6.7925283e-06,
    7.2339451e-06,
    7.7040476e-06,
    8.2047000e-06,
    8.7378876e-06,
    9.3057248e-06,
    9.9104632e-06,
    1.0554501e-05,
    1.1240392e-05,
    1.1970856e-05,
    1.2748789e-05,
    1.3577278e-05,
    1.4459606e-05,
    1.5399272e-05,
    1.6400004e-05,
    1.7465768e-05,
    1.8600792e-05,
    1.9809576e-05,
    2.1096914e-05,
    2.2467911e-05,
    2.3928002e-05,
    2.5482978e-05,
    2.7139006e-05,
    2.8902651e-05,
    3.0780908e-05,
    3.2781225e-05,
    3.4911534e-05,
    3.7180282e-05,
    3.9596466e-05,
    4.2169667e-05,
    4.4910090e-05,
    4.7828601e-05,
    5.0936773e-05,
    5.4246931e-05,
    5.7772202e-05,
    6.1526565e-05,
    6.5524908e-05,
    6.9783085e-05,
    7.4317983e-05,
    7.9147585e-05,
    8.4291040e-05,
    8.9768747e-05,
    9.5602426e-05,
    0.00010181521,
    0.00010843174,
    0.00011547824,
    0.00012298267,
    0.00013097477,
    0.00013948625,
    0.00014855085,
    0.00015820453,
    0.00016848555,
    0.00017943469,
    0.00019109536,
    0.00020351382,
    0.00021673929,
    0.00023082423,
    0.00024582449,
    0.00026179955,
    0.00027881276,
    0.00029693158,
    0.00031622787,
    0.00033677814,
    0.00035866388,
    0.00038197188,
    0.00040679456,
    0.00043323036,
    0.00046138411,
    0.00049136745,
    0.00052329927,
    0.00055730621,
    0.00059352311,
    0.00063209358,
    0.00067317058,
    0.00071691700,
    0.00076350630,
    0.00081312324,
    0.00086596457,
    0.00092223983,
    0.00098217216,
    0.0010459992,
    0.0011139742,
    0.0011863665,
    0.0012634633,
    0.0013455702,
    0.0014330129,
    0.0015261382,
    0.0016253153,
    0.0017309374,
    0.0018434235,
    0.0019632195,
    0.0020908006,
    0.0022266726,
    0.0023713743,
    0.0025254795,
    0.0026895994,
    0.0028643847,
    0.0030505286,
    0.0032487691,
    0.0034598925,
    0.0036847358,
    0.0039241906,
    0.0041792066,
    0.0044507950,
    0.0047400328,
    0.0050480668,
    0.0053761186,
    0.0057254891,
    0.0060975636,
    0.0064938176,
    0.0069158225,
    0.0073652516,
    0.0078438871,
    0.0083536271,
    0.0088964928,
    0.009474637,
    0.010090352,
    0.010746080,
    0.011444421,
    0.012188144,
    0.012980198,
    0.013823725,
    0.014722068,
    0.015678791,
    0.016697687,
    0.017782797,
    0.018938423,
    0.020169149,
    0.021479854,
    0.022875735,
    0.024362330,
    0.025945531,
    0.027631618,
    0.029427276,
    0.031339626,
    0.033376252,
    0.035545228,
    0.037855157,
    0.040315199,
    0.042935108,
    0.045725273,
    0.048696758,
    0.051861348,
    0.055231591,
    0.058820850,
    0.062643361,
    0.066714279,
    0.071049749,
    0.075666962,
    0.080584227,
    0.085821044,
    0.091398179,
    0.097337747,
    0.10366330,
    0.11039993,
    0.11757434,
    0.12521498,
    0.13335215,
    0.14201813,
    0.15124727,
    0.16107617,
    0.17154380,
    0.18269168,
    0.19456402,
    0.20720788,
    0.22067342,
    0.23501402,
    0.25028656,
    0.26655159,
    0.28387361,
    0.30232132,
    0.32196786,
    0.34289114,
    0.36517414,
    0.38890521,
    0.41417847,
    0.44109412,
    0.46975890,
    0.50028648,
    0.53279791,
    0.56742212,
    0.60429640,
    0.64356699,
    0.68538959,
    0.72993007,
    0.77736504,
    0.82788260,
    0.88168307,
    0.9389798,
    1.0,
];

// @OPTIMIZE: if you want to replace this bresenham line-drawing routine,
// note that you must produce bit-identical output to decode correctly;
// this specific sequence of operations is specified in the spec (it's
// drawing integer-quantized frequency-space lines that the encoder
// expects to be exactly the same)
//     ... also, isn't the whole point of Bresenham's algorithm to NOT
// have to divide in the setup? sigh.
// (`LINE_OP(a,b)` is `a *= b`.)

/// Translation of `draw_line()`.
fn draw_line(output: &mut [f32], x0: i32, y0: i32, mut x1: i32, y1: i32, n: i32) {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let mut ady = dy.abs();
    let mut x = x0;
    let mut y = y0;
    let mut err = 0;

    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    ady -= base.abs() * adx;
    if x1 > n {
        x1 = n;
    }
    if x < x1 {
        output[x as usize] *= INVERSE_DB_TABLE[(y as u32 & 255) as usize];
        x += 1;
        while x < x1 {
            err += ady;
            if err >= adx {
                err -= adx;
                y += sy;
            } else {
                y += base;
            }
            output[x as usize] *= INVERSE_DB_TABLE[(y as u32 & 255) as usize];
            x += 1;
        }
    }
}

/// Translation of `residue_decode()`.
fn residue_decode(
    f: &mut Reader<'_>,
    book: &Codebook,
    target: &mut [f32],
    mut offset: i32,
    n: i32,
    rtype: i32,
) -> bool {
    if rtype == 0 {
        let step = n / book.dimensions;
        for k in 0..step {
            if !codebook_decode_step(f, book, target, (offset + k) as usize, n - offset - k, step) {
                return false;
            }
        }
    } else {
        let mut k = 0;
        while k < n {
            if !codebook_decode(f, book, target, offset as usize, n - k) {
                return false;
            }
            k += book.dimensions;
            offset += book.dimensions;
        }
    }
    true
}

// n is 1/2 of the blocksize --
// specification: "Correct per-vector decode length is [n]/2"
/// Translation of `decode_residue()`.
fn decode_residue(
    f: &mut Reader<'_>,
    s: &Setup,
    residue_buffers: &mut [Option<&mut Vec<f32>>],
    ch: i32,
    n: i32,
    rn: usize,
    do_not_decode: &[bool],
) {
    let r = &s.residue_config[rn];
    let rtype = s.residue_types[rn] as i32;
    let c = r.classbook as usize;
    let classwords = s.codebooks[c].dimensions;
    let actual_size: u32 = if rtype == 2 { (n * 2) as u32 } else { n as u32 };
    let limit_r_begin = if r.begin < actual_size {
        r.begin
    } else {
        actual_size
    };
    let limit_r_end = if r.end < actual_size {
        r.end
    } else {
        actual_size
    };
    let n_read = limit_r_end.wrapping_sub(limit_r_begin) as i32;
    let part_read = n_read / r.part_size as i32;
    // (`temp_block_array()`: the classdata rows, per channel.)
    let mut part_classdata: Vec<Vec<usize>> =
        vec![vec![0; part_read.max(0) as usize]; s.channels as usize];

    for i in 0..ch as usize {
        if !do_not_decode[i] {
            if let Some(buffer) = residue_buffers[i].as_deref_mut() {
                buffer[..n as usize].fill(0.0);
            }
        }
    }

    'done: {
        if rtype == 2 && ch != 1 {
            let mut j = 0;
            while j < ch {
                if !do_not_decode[j as usize] {
                    break;
                }
                j += 1;
            }
            if j == ch {
                break 'done;
            }

            for pass in 0..8 {
                let mut pcount = 0;
                let mut class_set = 0usize;
                while pcount < part_read {
                    let z = (r.begin as i32).wrapping_add(pcount.wrapping_mul(r.part_size as i32));
                    let (mut c_inter, mut p_inter) = if ch == 2 {
                        (z & 1, z >> 1)
                    } else {
                        (z % ch, z / ch)
                    };
                    if pass == 0 {
                        let cb = &s.codebooks[r.classbook as usize];
                        let q = decode(f, cb);
                        if q == EOP {
                            break 'done;
                        }
                        part_classdata[0][class_set] = q as usize;
                    }
                    let mut i = 0;
                    while i < classwords && pcount < part_read {
                        let mut z =
                            (r.begin as i32).wrapping_add(pcount.wrapping_mul(r.part_size as i32));
                        let c = r.classdata[part_classdata[0][class_set]][i as usize] as usize;
                        let b = r.residue_books[c][pass];
                        if b >= 0 {
                            let book = &s.codebooks[b as usize];
                            if !codebook_decode_deinterleave_repeat(
                                f,
                                book,
                                residue_buffers,
                                ch,
                                &mut c_inter,
                                &mut p_inter,
                                n,
                                r.part_size as i32,
                            ) {
                                break 'done;
                            }
                        } else {
                            z = z.wrapping_add(r.part_size as i32);
                            if ch == 2 {
                                c_inter = z & 1;
                                p_inter = z >> 1;
                            } else {
                                c_inter = z % ch;
                                p_inter = z / ch;
                            }
                        }
                        i += 1;
                        pcount += 1;
                    }
                    class_set += 1;
                }
            }
            break 'done;
        }

        for pass in 0..8 {
            let mut pcount = 0;
            let mut class_set = 0usize;
            while pcount < part_read {
                if pass == 0 {
                    for j in 0..ch as usize {
                        if !do_not_decode[j] {
                            let cb = &s.codebooks[r.classbook as usize];
                            let temp = decode(f, cb);
                            if temp == EOP {
                                break 'done;
                            }
                            part_classdata[j][class_set] = temp as usize;
                        }
                    }
                }
                let mut i = 0;
                while i < classwords && pcount < part_read {
                    for j in 0..ch as usize {
                        if !do_not_decode[j] {
                            let c = r.classdata[part_classdata[j][class_set]][i as usize] as usize;
                            let b = r.residue_books[c][pass];
                            if b >= 0 {
                                let target =
                                    residue_buffers[j].as_deref_mut().expect("residue buffer");
                                let offset = (r.begin as i32)
                                    .wrapping_add(pcount.wrapping_mul(r.part_size as i32));
                                let n = r.part_size as i32;
                                let book = &s.codebooks[b as usize];
                                if !residue_decode(f, book, target, offset, n, rtype) {
                                    break 'done;
                                }
                            }
                        }
                    }
                    i += 1;
                    pcount += 1;
                }
                class_set += 1;
            }
        }
    }
    // (done:)
}

// the following were split out into separate functions while optimizing;
// they could be pushed back up but eh. __forceinline showed no change;
// they're probably already being inlined.
// (Pointers into `e` and `a` are indices here.)

/// Translation of `imdct_step3_iter0_loop()`.
fn imdct_step3_iter0_loop(n: i32, e: &mut [f32], i_off: i32, k_off: i32, a: &[f32]) {
    let mut ee0 = i_off as isize;
    let mut ee2 = ee0 + k_off as isize;
    let mut aa = 0usize;

    // (assert((n & 3) == 0))
    let mut i = n >> 2;
    while i > 0 {
        for q in [0isize, 2, 4, 6] {
            let i0 = (ee0 - q) as usize;
            let i1 = (ee0 - q - 1) as usize;
            let j0 = (ee2 - q) as usize;
            let j1 = (ee2 - q - 1) as usize;
            let k00_20 = e[i0] - e[j0];
            let k01_21 = e[i1] - e[j1];
            e[i0] += e[j0]; //ee0[ 0] = ee0[ 0] + ee2[ 0];
            e[i1] += e[j1]; //ee0[-1] = ee0[-1] + ee2[-1];
            e[j0] = k00_20 * a[aa] - k01_21 * a[aa + 1];
            e[j1] = k01_21 * a[aa] + k00_20 * a[aa + 1];
            aa += 8;
        }
        ee0 -= 8;
        ee2 -= 8;
        i -= 1;
    }
}

/// Translation of `imdct_step3_inner_r_loop()`.
fn imdct_step3_inner_r_loop(lim: i32, e: &mut [f32], d0: i32, k_off: i32, a: &[f32], k1: i32) {
    let mut e0 = d0 as isize;
    let mut e2 = e0 + k_off as isize;
    let mut aa = 0usize;

    let mut i = lim >> 2;
    while i > 0 {
        for q in [0isize, 2, 4, 6] {
            let i0 = (e0 - q) as usize;
            let i1 = (e0 - q - 1) as usize;
            let j0 = (e2 - q) as usize;
            let j1 = (e2 - q - 1) as usize;
            let k00_20 = e[i0] - e[j0];
            let k01_21 = e[i1] - e[j1];
            e[i0] += e[j0]; //e0[-0] = e0[-0] + e2[-0];
            e[i1] += e[j1]; //e0[-1] = e0[-1] + e2[-1];
            e[j0] = (k00_20) * a[aa] - (k01_21) * a[aa + 1];
            e[j1] = (k01_21) * a[aa] + (k00_20) * a[aa + 1];

            aa += k1 as usize;
        }

        e0 -= 8;
        e2 -= 8;
        i -= 1;
    }
}

/// Translation of `imdct_step3_inner_s_loop()`.
fn imdct_step3_inner_s_loop(
    n: i32,
    e: &mut [f32],
    i_off: i32,
    k_off: i32,
    a: &[f32],
    a_off: i32,
    k0: i32,
) {
    let a_off = a_off as usize;
    let coeffs = [
        (a[0], a[1]),
        (a[a_off], a[a_off + 1]),
        (a[a_off * 2], a[a_off * 2 + 1]),
        (a[a_off * 3], a[a_off * 3 + 1]),
    ];

    let mut ee0 = i_off as isize;
    let mut ee2 = ee0 + k_off as isize;

    let mut i = n;
    while i > 0 {
        for (q, &(a0, a1)) in [0isize, 2, 4, 6].iter().zip(coeffs.iter()) {
            let i0 = (ee0 - q) as usize;
            let i1 = (ee0 - q - 1) as usize;
            let j0 = (ee2 - q) as usize;
            let j1 = (ee2 - q - 1) as usize;
            let k00 = e[i0] - e[j0];
            let k11 = e[i1] - e[j1];
            e[i0] += e[j0];
            e[i1] += e[j1];
            e[j0] = (k00) * a0 - (k11) * a1;
            e[j1] = (k11) * a0 + (k00) * a1;
        }

        ee0 -= k0 as isize;
        ee2 -= k0 as isize;
        i -= 1;
    }
}

/// Translation of `iter_54()`: on `e[z - 7..=z]`.
fn iter_54(e: &mut [f32], z: usize) {
    let k00 = e[z] - e[z - 4];
    let y0 = e[z] + e[z - 4];
    let y2 = e[z - 2] + e[z - 6];
    let k22 = e[z - 2] - e[z - 6];

    e[z] = y0 + y2; // z0 + z4 + z2 + z6
    e[z - 2] = y0 - y2; // z0 + z4 - z2 - z6

    // done with y0,y2

    let k33 = e[z - 3] - e[z - 7];

    e[z - 4] = k00 + k33; // z0 - z4 + z3 - z7
    e[z - 6] = k00 - k33; // z0 - z4 - z3 + z7

    // done with k33

    let k11 = e[z - 1] - e[z - 5];
    let y1 = e[z - 1] + e[z - 5];
    let y3 = e[z - 3] + e[z - 7];

    e[z - 1] = y1 + y3; // z1 + z5 + z3 + z7
    e[z - 3] = y1 - y3; // z1 + z5 - z3 - z7
    e[z - 5] = k11 - k22; // z1 - z5 + z2 - z6
    e[z - 7] = k11 + k22; // z1 - z5 - z2 + z6
}

/// Translation of `imdct_step3_inner_s_loop_ld654()`.
fn imdct_step3_inner_s_loop_ld654(n: i32, e: &mut [f32], i_off: i32, a: &[f32], base_n: i32) {
    let a_off = (base_n >> 3) as usize;
    let a2 = a[a_off];
    let mut z = i_off as isize;
    let base = z - 16 * n as isize;

    while z > base {
        let zz = z as usize;

        let k00 = e[zz] - e[zz - 8];
        let k11 = e[zz - 1] - e[zz - 9];
        let l00 = e[zz - 2] - e[zz - 10];
        let l11 = e[zz - 3] - e[zz - 11];
        e[zz] += e[zz - 8];
        e[zz - 1] += e[zz - 9];
        e[zz - 2] += e[zz - 10];
        e[zz - 3] += e[zz - 11];
        e[zz - 8] = k00;
        e[zz - 9] = k11;
        e[zz - 10] = (l00 + l11) * a2;
        e[zz - 11] = (l11 - l00) * a2;

        let k00 = e[zz - 4] - e[zz - 12];
        let k11 = e[zz - 5] - e[zz - 13];
        let l00 = e[zz - 6] - e[zz - 14];
        let l11 = e[zz - 7] - e[zz - 15];
        e[zz - 4] += e[zz - 12];
        e[zz - 5] += e[zz - 13];
        e[zz - 6] += e[zz - 14];
        e[zz - 7] += e[zz - 15];
        e[zz - 12] = k11;
        e[zz - 13] = -k00;
        e[zz - 14] = (l11 - l00) * a2;
        e[zz - 15] = (l00 + l11) * -a2;

        iter_54(e, zz);
        iter_54(e, zz - 8);
        z -= 16;
    }
}

/// Translation of `inverse_mdct()`.
fn inverse_mdct(buffer: &mut [f32], n: i32, s: &Setup, blocktype: usize) {
    let n2 = n >> 1;
    let n4 = n >> 2;
    let n8 = n >> 3;
    // @OPTIMIZE: reduce register pressure by using fewer variables?
    // (temp_alloc(): the work buffer, which every value is written to before it's read.)
    let mut buf2 = vec![0.0f32; n2 as usize];
    // twiddle factors
    let a = &s.a[blocktype][..];

    // IMDCT algorithm from "The use of multirate filter banks for coding of high quality digital audio"
    // See notes about bugs in that paper in less-optimal implementation 'inverse_mdct_old' after this function.

    // kernel from paper

    // merged:
    //   copy and reflect spectral data
    //   step 0

    // note that it turns out that the items added together during
    // this step are, in fact, being added to themselves (as reflected
    // by step 0). inexplicable inefficiency! this became obvious
    // once I combined the passes.

    // so there's a missing 'times 2' here (for adding X to itself).
    // this propagates through linearly to the end, where the numbers
    // are 1/2 too small, and need to be compensated for.

    {
        let mut d = n2 as isize - 2;
        let mut aa = 0usize;
        let mut e = 0usize;
        let e_stop = n2 as usize;
        while e != e_stop {
            let du = d as usize;
            buf2[du + 1] = buffer[e] * a[aa] - buffer[e + 2] * a[aa + 1];
            buf2[du] = buffer[e] * a[aa + 1] + buffer[e + 2] * a[aa];
            d -= 2;
            aa += 2;
            e += 4;
        }

        let mut e = n2 as isize - 3;
        while d >= 0 {
            let du = d as usize;
            let eu = e as usize;
            buf2[du + 1] = -buffer[eu + 2] * a[aa] - -buffer[eu] * a[aa + 1];
            buf2[du] = -buffer[eu + 2] * a[aa + 1] + -buffer[eu] * a[aa];
            d -= 2;
            aa += 2;
            e -= 4;
        }
    }

    // now we use symbolic names for these, so that we can
    // possibly swap their meaning as we change which operations
    // are in place

    let u = buffer;
    let v = &mut buf2;

    // step 2    (paper output is w, now u)
    // this could be in place, but the data ends up in the wrong
    // place... _somebody_'s got to swap it, so this is nominated
    {
        let mut aa = n2 as isize - 8;
        let mut e0 = n4 as usize;
        let mut e1 = 0usize;

        let mut d0 = n4 as usize;
        let mut d1 = 0usize;

        while aa >= 0 {
            let au = aa as usize;

            let v41_21 = v[e0 + 1] - v[e1 + 1];
            let v40_20 = v[e0] - v[e1];
            u[d0 + 1] = v[e0 + 1] + v[e1 + 1];
            u[d0] = v[e0] + v[e1];
            u[d1 + 1] = v41_21 * a[au + 4] - v40_20 * a[au + 5];
            u[d1] = v40_20 * a[au + 4] + v41_21 * a[au + 5];

            let v41_21 = v[e0 + 3] - v[e1 + 3];
            let v40_20 = v[e0 + 2] - v[e1 + 2];
            u[d0 + 3] = v[e0 + 3] + v[e1 + 3];
            u[d0 + 2] = v[e0 + 2] + v[e1 + 2];
            u[d1 + 3] = v41_21 * a[au] - v40_20 * a[au + 1];
            u[d1 + 2] = v40_20 * a[au] + v41_21 * a[au + 1];

            aa -= 8;

            d0 += 4;
            d1 += 4;
            e0 += 4;
            e1 += 4;
        }
    }

    // step 3
    let ld = ilog(n) - 1; // ilog is off-by-one from normal definitions

    // optimized step 3:

    // the original step3 loop can be nested r inside s or s inside r;
    // it's written originally as s inside r, but this is dumb when r
    // iterates many times, and s few. So I have two copies of it and
    // switch between them halfway.

    // this is iteration 0 of step 3
    imdct_step3_iter0_loop(n >> 4, u, n2 - 1 - n4 * 0, -(n >> 3), a);
    imdct_step3_iter0_loop(n >> 4, u, n2 - 1 - n4 * 1, -(n >> 3), a);

    // this is iteration 1 of step 3
    imdct_step3_inner_r_loop(n >> 5, u, n2 - 1 - n8 * 0, -(n >> 4), a, 16);
    imdct_step3_inner_r_loop(n >> 5, u, n2 - 1 - n8 * 1, -(n >> 4), a, 16);
    imdct_step3_inner_r_loop(n >> 5, u, n2 - 1 - n8 * 2, -(n >> 4), a, 16);
    imdct_step3_inner_r_loop(n >> 5, u, n2 - 1 - n8 * 3, -(n >> 4), a, 16);

    let mut l = 2;
    while l < (ld - 3) >> 1 {
        let k0 = n >> (l + 2);
        let k0_2 = k0 >> 1;
        let lim = 1 << (l + 1);
        for i in 0..lim {
            imdct_step3_inner_r_loop(n >> (l + 4), u, n2 - 1 - k0 * i, -k0_2, a, 1 << (l + 3));
        }
        l += 1;
    }

    while l < ld - 6 {
        let k0 = n >> (l + 2);
        let k1 = 1 << (l + 3);
        let k0_2 = k0 >> 1;
        let rlim = n >> (l + 6);
        let lim = 1 << (l + 1);
        let mut a0 = 0usize;
        let mut i_off = n2 - 1;
        let mut r = rlim;
        while r > 0 {
            imdct_step3_inner_s_loop(lim, u, i_off, -k0_2, &a[a0..], k1, k0);
            a0 += (k1 * 4) as usize;
            i_off -= 8;
            r -= 1;
        }
        l += 1;
    }

    // iterations with count:
    //   ld-6,-5,-4 all interleaved together
    //       the big win comes from getting rid of needless flops
    //         due to the constants on pass 5 & 4 being all 1 and 0;
    //       combining them to be simultaneous to improve cache made little difference
    imdct_step3_inner_s_loop_ld654(n >> 5, u, n2 - 1, a, n);

    // output is u

    // step 4, 5, and 6
    // cannot be in-place because of step 5
    {
        let bitrev = &s.bit_reverse[blocktype];
        let mut br = 0usize;
        // weirdly, I'd have thought reading sequentially and writing
        // erratically would have been better than vice-versa, but in
        // fact that's not what my testing showed. (That is, with
        // j = bitreverse(i), do you read i and write j, or read j and write i.)

        let mut d0 = n4 as isize - 4;
        let mut d1 = n2 as isize - 4;
        while d0 >= 0 {
            let (d0u, d1u) = (d0 as usize, d1 as usize);

            let k4 = bitrev[br] as usize;
            v[d1u + 3] = u[k4];
            v[d1u + 2] = u[k4 + 1];
            v[d0u + 3] = u[k4 + 2];
            v[d0u + 2] = u[k4 + 3];

            let k4 = bitrev[br + 1] as usize;
            v[d1u + 1] = u[k4];
            v[d1u] = u[k4 + 1];
            v[d0u + 1] = u[k4 + 2];
            v[d0u] = u[k4 + 3];

            d0 -= 4;
            d1 -= 4;
            br += 2;
        }
    }
    // (paper output is u, now v)

    // data must be in buf2

    // step 7   (paper output is v, now v)
    // this is now in place
    {
        let c = &s.c[blocktype];
        let mut cc = 0usize;

        let mut d = 0usize;
        let mut e = n2 as usize - 4;

        while d < e {
            let a02 = v[d] - v[e + 2];
            let a11 = v[d + 1] + v[e + 3];

            let b0 = c[cc + 1] * a02 + c[cc] * a11;
            let b1 = c[cc + 1] * a11 - c[cc] * a02;

            let b2 = v[d] + v[e + 2];
            let b3 = v[d + 1] - v[e + 3];

            v[d] = b2 + b0;
            v[d + 1] = b3 + b1;
            v[e + 2] = b2 - b0;
            v[e + 3] = b1 - b3;

            let a02 = v[d + 2] - v[e];
            let a11 = v[d + 3] + v[e + 1];

            let b0 = c[cc + 3] * a02 + c[cc + 2] * a11;
            let b1 = c[cc + 3] * a11 - c[cc + 2] * a02;

            let b2 = v[d + 2] + v[e];
            let b3 = v[d + 3] - v[e + 1];

            v[d + 2] = b2 + b0;
            v[d + 3] = b3 + b1;
            v[e] = b2 - b0;
            v[e + 1] = b1 - b3;

            cc += 4;
            d += 4;
            if e < 4 {
                break;
            }
            e -= 4;
        }
    }

    // data must be in buf2

    // step 8+decode   (paper output is X, now buffer)
    // this generates pairs of data a la 8 and pushes them directly through
    // the decode kernel (pushing rather than pulling) to avoid having
    // to make another pass later

    // this cannot POSSIBLY be in place, so we refer to the buffers directly

    {
        let b = &s.b[blocktype];
        let mut bb = n2 as isize - 8;
        let mut e = n2 as isize - 8;
        let mut d0 = 0usize;
        let mut d1 = n2 as usize - 4;
        let mut d2 = n2 as usize;
        let mut d3 = n as usize - 4;
        while e >= 0 {
            let (eu, bu) = (e as usize, bb as usize);

            let p3 = v[eu + 6] * b[bu + 7] - v[eu + 7] * b[bu + 6];
            let p2 = -v[eu + 6] * b[bu + 6] - v[eu + 7] * b[bu + 7];

            u[d0] = p3;
            u[d1 + 3] = -p3;
            u[d2] = p2;
            u[d3 + 3] = p2;

            let p1 = v[eu + 4] * b[bu + 5] - v[eu + 5] * b[bu + 4];
            let p0 = -v[eu + 4] * b[bu + 4] - v[eu + 5] * b[bu + 5];

            u[d0 + 1] = p1;
            u[d1 + 2] = -p1;
            u[d2 + 1] = p0;
            u[d3 + 2] = p0;

            let p3 = v[eu + 2] * b[bu + 3] - v[eu + 3] * b[bu + 2];
            let p2 = -v[eu + 2] * b[bu + 2] - v[eu + 3] * b[bu + 3];

            u[d0 + 2] = p3;
            u[d1 + 1] = -p3;
            u[d2 + 2] = p2;
            u[d3 + 1] = p2;

            let p1 = v[eu] * b[bu + 1] - v[eu + 1] * b[bu];
            let p0 = -v[eu] * b[bu] - v[eu + 1] * b[bu + 1];

            u[d0 + 3] = p1;
            u[d1] = -p1;
            u[d2 + 3] = p0;
            u[d3] = p0;

            bb -= 8;
            e -= 8;
            d0 += 4;
            d2 += 4;
            d1 = d1.wrapping_sub(4);
            d3 = d3.wrapping_sub(4);
        }
    }
}

impl StbVorbis<'_> {
    /// Translation of `get_window()`: which of the windows.
    fn get_window(&self, mut len: i32) -> Option<usize> {
        len <<= 1;
        if len == self.s.blocksize_0 {
            return Some(0);
        }
        if len == self.s.blocksize_1 {
            return Some(1);
        }
        None
    }
}

/// Translation of `do_floor()`.
fn do_floor(
    s: &Setup,
    map: &Mapping,
    i: usize,
    n: i32,
    target: &mut [f32],
    final_y: &[i16],
) -> bool {
    let n2 = n >> 1;
    let sm = map.chan[i].mux as usize;
    let floor = map.submap_floor[sm] as usize;
    if s.floor_types[floor] == 0 {
        // (error(f, VORBIS_invalid_stream): the caller ignores it, and a
        // floor 0 stream can't be opened anyway.)
        return false;
    }
    let g = &s.floor_config[floor];
    let mut lx = 0;
    let mut ly = final_y[0] as i32 * g.floor1_multiplier as i32;
    for q in 1..g.values as usize {
        let j = g.sorted_order[q] as usize;
        if final_y[j] >= 0 {
            let hy = final_y[j] as i32 * g.floor1_multiplier as i32;
            let hx = g.xlist[j] as i32;
            if lx != hx {
                draw_line(target, lx, ly, hx, hy, n2);
            }
            lx = hx;
            ly = hy;
        }
    }
    if lx < n2 {
        // optimization of: draw_line(target, lx,ly, n,ly, n2);
        for j in lx..n2 {
            target[j as usize] *= INVERSE_DB_TABLE[(ly & 255) as usize];
        }
    }
    true
}

// The meaning of "left" and "right"
//
// For a given frame:
//     we compute samples from 0..n
//     window_center is n/2
//     we'll window and mix the samples from left_start to left_end with data from the previous frame
//     all of the samples from left_end to right_start can be output without mixing; however,
//        this interval is 0-length except when transitioning between short and long frames
//     all of the samples from right_start to right_end need to be mixed with the next frame,
//        which we don't have, so those get saved in a buffer
//     frame N's right_end-right_start, the number of samples to mix with the next frame,
//        has to be the same as frame N+1's left_end-left_start (which they are by
//        construction)

impl StbVorbis<'_> {
    /// Translation of `vorbis_decode_initial()`.
    fn vorbis_decode_initial(
        &mut self,
        p_left_start: &mut i32,
        p_left_end: &mut i32,
        p_right_start: &mut i32,
        p_right_end: &mut i32,
        mode: &mut i32,
    ) -> bool {
        let f = &mut self.r;
        let s = &self.s;
        self.channel_buffer_start = 0;
        self.channel_buffer_end = 0;

        loop {
            // (retry:)
            if f.eof {
                return false;
            }
            if !f.maybe_start_packet() {
                return false;
            }
            // check packet type
            if f.get_bits(1) != 0 {
                while EOP != f.get8_packet() {}
                continue;
            }
            break;
        }

        let i = f.get_bits(ilog(s.mode_count - 1)) as i32;
        if i == EOP {
            return false;
        }
        if i >= s.mode_count {
            return false;
        }
        *mode = i;
        let m = &s.mode_config[i as usize];
        let n;
        let prev;
        let next;
        if m.blockflag != 0 {
            n = s.blocksize_1;
            prev = f.get_bits(1);
            next = f.get_bits(1);
        } else {
            prev = 0;
            next = 0;
            n = s.blocksize_0;
        }

        // WINDOWING

        let window_center = n >> 1;
        if m.blockflag != 0 && prev == 0 {
            *p_left_start = (n - s.blocksize_0) >> 2;
            *p_left_end = (n + s.blocksize_0) >> 2;
        } else {
            *p_left_start = 0;
            *p_left_end = window_center;
        }
        if m.blockflag != 0 && next == 0 {
            *p_right_start = (n * 3 - s.blocksize_0) >> 2;
            *p_right_end = (n * 3 + s.blocksize_0) >> 2;
        } else {
            *p_right_start = window_center;
            *p_right_end = n;
        }

        true
    }

    /// Translation of `vorbis_decode_packet_rest()`.
    fn vorbis_decode_packet_rest(
        &mut self,
        len: &mut i32,
        mode: usize,
        mut left_start: i32,
        _left_end: i32,
        right_start: i32,
        right_end: i32,
        p_left: &mut i32,
    ) -> bool {
        let mut zero_channel = [false; 256];
        let mut really_zero_channel = [false; 256];

        let StbVorbis {
            r: f,
            s,
            channel_buffers,
            final_y,
            ..
        } = self;
        let s: &Setup = s;
        let m = s.mode_config[mode];

        // WINDOWING

        let n = s.blocksize[m.blockflag as usize];
        let map = &s.mapping[m.mapping as usize];

        // FLOORS
        let n2 = n >> 1;

        for i in 0..s.channels as usize {
            let sm = map.chan[i].mux as usize;
            zero_channel[i] = false;
            let floor = map.submap_floor[sm] as usize;
            if s.floor_types[floor] == 0 {
                return f.error(VORBIS_INVALID_STREAM);
            } else {
                let g = &s.floor_config[floor];
                let mut decoded = false;
                if f.get_bits(1) != 0 {
                    'error: {
                        let mut step2_flag = [0u8; 256];
                        static RANGE_LIST: [i32; 4] = [256, 128, 86, 64];
                        let range = RANGE_LIST[(g.floor1_multiplier - 1) as usize];
                        let mut offset = 2usize;
                        let final_yi = &mut final_y[i];
                        final_yi[0] = f.get_bits(ilog(range) - 1) as i16;
                        final_yi[1] = f.get_bits(ilog(range) - 1) as i16;
                        for j in 0..g.partitions as usize {
                            let pclass = g.partition_class_list[j] as usize;
                            let cdim = g.class_dimensions[pclass] as i32;
                            let cbits = g.class_subclasses[pclass] as i32;
                            let csub = (1 << cbits) - 1;
                            let mut cval = 0;
                            if cbits != 0 {
                                let c = &s.codebooks[g.class_masterbooks[pclass] as usize];
                                cval = decode(f, c);
                            }
                            for _ in 0..cdim {
                                let book = g.subclass_books[pclass][(cval & csub) as usize];
                                cval >>= cbits;
                                if book >= 0 {
                                    let c = &s.codebooks[book as usize];
                                    let temp = decode(f, c);
                                    final_yi[offset] = temp as i16;
                                } else {
                                    final_yi[offset] = 0;
                                }
                                offset += 1;
                            }
                        }
                        if f.valid_bits == INVALID_BITS {
                            break 'error; // behavior according to spec
                        }
                        step2_flag[0] = 1;
                        step2_flag[1] = 1;
                        for j in 2..g.values as usize {
                            let low = g.neighbors[j][0] as usize;
                            let high = g.neighbors[j][1] as usize;
                            //neighbors(g->Xlist, j, &low, &high);
                            let pred = predict_point(
                                g.xlist[j] as i32,
                                g.xlist[low] as i32,
                                g.xlist[high] as i32,
                                final_yi[low] as i32,
                                final_yi[high] as i32,
                            );
                            let val = final_yi[j] as i32;
                            let highroom = range - pred;
                            let lowroom = pred;
                            let room = if highroom < lowroom {
                                highroom * 2
                            } else {
                                lowroom * 2
                            };
                            if val != 0 {
                                step2_flag[low] = 1;
                                step2_flag[high] = 1;
                                step2_flag[j] = 1;
                                if val >= room {
                                    if highroom > lowroom {
                                        final_yi[j] = (val - lowroom + pred) as i16;
                                    } else {
                                        final_yi[j] = (pred - val + highroom - 1) as i16;
                                    }
                                } else if val & 1 != 0 {
                                    final_yi[j] = (pred - ((val + 1) >> 1)) as i16;
                                } else {
                                    final_yi[j] = (pred + (val >> 1)) as i16;
                                }
                            } else {
                                step2_flag[j] = 0;
                                final_yi[j] = pred as i16;
                            }
                        }

                        // defer final floor computation until _after_ residue
                        for j in 0..g.values as usize {
                            if step2_flag[j] == 0 {
                                final_yi[j] = -1;
                            }
                        }
                        decoded = true;
                    }
                }
                if !decoded {
                    // (error:)
                    zero_channel[i] = true;
                }
                // So we just defer everything else to later

                // at this point we've decoded the floor into buffer
            }
        }
        // at this point we've decoded all floors

        // re-enable coupled channels if necessary
        really_zero_channel[..s.channels as usize]
            .copy_from_slice(&zero_channel[..s.channels as usize]);
        for i in 0..map.coupling_steps as usize {
            if !zero_channel[map.chan[i].magnitude as usize]
                || !zero_channel[map.chan[i].angle as usize]
            {
                zero_channel[map.chan[i].magnitude as usize] = false;
                zero_channel[map.chan[i].angle as usize] = false;
            }
        }

        // RESIDUE DECODE
        for i in 0..map.submaps as usize {
            let mut residue_buffers: Vec<Option<&mut Vec<f32>>> =
                Vec::with_capacity(STB_VORBIS_MAX_CHANNELS);
            let mut do_not_decode = [false; 256];
            let mut ch = 0usize;
            for (j, buffer) in channel_buffers.iter_mut().enumerate() {
                if map.chan[j].mux as usize == i {
                    if zero_channel[j] {
                        do_not_decode[ch] = true;
                        residue_buffers.push(None);
                    } else {
                        do_not_decode[ch] = false;
                        residue_buffers.push(Some(buffer));
                    }
                    ch += 1;
                }
            }
            let r = map.submap_residue[i] as usize;
            decode_residue(f, s, &mut residue_buffers, ch as i32, n2, r, &do_not_decode);
        }

        // INVERSE COUPLING
        let mut i = map.coupling_steps as i32 - 1;
        while i >= 0 {
            let n2 = n >> 1;
            let mi = map.chan[i as usize].magnitude as usize;
            let ai = map.chan[i as usize].angle as usize;
            // (the setup checks the two are different channels.)
            let (m, a) = if mi < ai {
                let (lo, hi) = channel_buffers.split_at_mut(ai);
                (&mut lo[mi], &mut hi[0])
            } else {
                let (lo, hi) = channel_buffers.split_at_mut(mi);
                (&mut hi[0], &mut lo[ai])
            };
            for j in 0..n2 as usize {
                let a2;
                let m2;
                if m[j] > 0.0 {
                    if a[j] > 0.0 {
                        m2 = m[j];
                        a2 = m[j] - a[j];
                    } else {
                        a2 = m[j];
                        m2 = m[j] + a[j];
                    }
                } else if a[j] > 0.0 {
                    m2 = m[j];
                    a2 = m[j] + a[j];
                } else {
                    a2 = m[j];
                    m2 = m[j] - a[j];
                }
                m[j] = m2;
                a[j] = a2;
            }
            i -= 1;
        }

        // finish decoding the floors
        for i in 0..s.channels as usize {
            if really_zero_channel[i] {
                channel_buffers[i][..n2 as usize].fill(0.0);
            } else {
                do_floor(s, map, i, n, &mut channel_buffers[i], &final_y[i]);
            }
        }

        // INVERSE MDCT
        for i in 0..s.channels as usize {
            inverse_mdct(&mut channel_buffers[i], n, s, m.blockflag as usize);
        }

        // this shouldn't be necessary, unless we exited on an error
        // and want to flush to get to the next packet
        f.flush_packet();

        if f.first_decode {
            // assume we start so first non-discarded sample is sample 0
            // this isn't to spec, but spec would require us to read ahead
            // and decode the size of all current frames--could be done,
            // but presumably it's not a commonly used feature
            self.current_loc = 0u32.wrapping_sub(n2 as u32); // start of first frame is positioned for discard (NB this is an intentional unsigned overflow/wrap-around)
                                                             // we might have to discard samples "from" the next frame too,
                                                             // if we're lapping a large block then a small at the start?
            self.discard_samples_deferred = n - right_end;
            self.current_loc_valid = true;
            self.r.first_decode = false;
        } else if self.discard_samples_deferred != 0 {
            if self.discard_samples_deferred >= right_start - left_start {
                self.discard_samples_deferred -= right_start - left_start;
                left_start = right_start;
                *p_left = left_start;
            } else {
                left_start += self.discard_samples_deferred;
                *p_left = left_start;
                self.discard_samples_deferred = 0;
            }
        } else if self.previous_length == 0 && self.current_loc_valid {
            // we're recovering from a seek... that means we're going to discard
            // the samples from this packet even though we know our position from
            // the last page header, so we need to update the position based on
            // the discarded samples here
            // but wait, the code below is going to add this in itself even
            // on a discard, so we don't need to do it here...
        }

        let f = &mut self.r;
        // check if we have ogg information about the sample # for this packet
        if f.last_seg_which == f.end_seg_with_known_loc {
            // if we have a valid current loc, and this is final:
            if self.current_loc_valid && (f.page_flag & PAGEFLAG_LAST_PAGE) != 0 {
                let current_end = f.known_loc_for_packet;
                // then let's infer the size of the (probably) short final frame
                if current_end
                    < self
                        .current_loc
                        .wrapping_add((right_end - left_start) as u32)
                {
                    if current_end < self.current_loc {
                        // negative truncation, that's impossible!
                        *len = 0;
                    } else {
                        *len = current_end.wrapping_sub(self.current_loc) as i32;
                    }
                    *len = len.wrapping_add(left_start); // this doesn't seem right, but has no ill effect on my test files
                    if *len > right_end {
                        *len = right_end; // this should never happen
                    }
                    self.current_loc = self.current_loc.wrapping_add(*len as u32);
                    return true;
                }
            }
            // otherwise, just set our sample loc
            // guess that the ogg granule pos refers to the _middle_ of the
            // last frame?
            // set f->current_loc to the position of left_start
            self.current_loc = f
                .known_loc_for_packet
                .wrapping_sub((n2 - left_start) as u32);
            self.current_loc_valid = true;
        }
        if self.current_loc_valid {
            self.current_loc = self
                .current_loc
                .wrapping_add((right_start - left_start) as u32);
        }

        *len = right_end; // ignore samples after the window goes to 0

        true
    }

    /// Translation of `vorbis_decode_packet()`.
    fn vorbis_decode_packet(&mut self, len: &mut i32, p_left: &mut i32, p_right: &mut i32) -> bool {
        let mut mode = 0;
        let mut left_end = 0;
        let mut right_end = 0;
        if !self.vorbis_decode_initial(p_left, &mut left_end, p_right, &mut right_end, &mut mode) {
            return false;
        }
        let (left, right) = (*p_left, *p_right);
        self.vorbis_decode_packet_rest(len, mode as usize, left, left_end, right, right_end, p_left)
    }

    /// Translation of `vorbis_finish_frame()`.
    fn vorbis_finish_frame(&mut self, len: i32, left: i32, mut right: i32) -> i32 {
        // we use right&left (the start of the right- and left-window sin()-regions)
        // to determine how much to return, rather than inferring from the rules
        // (same result, clearer code); 'left' indicates where our sin() window
        // starts, therefore where the previous window's right edge starts, and
        // therefore where to start mixing from the previous buffer. 'right'
        // indicates where our sin() ending-window starts, therefore that's where
        // we start saving, and where our returned-data ends.

        // mixin from previous window
        if self.previous_length != 0 {
            let n = self.previous_length;
            let Some(w) = self.get_window(n) else {
                return 0;
            };
            let w = &self.s.window[w];
            for i in 0..self.s.channels as usize {
                let buffer = &mut self.channel_buffers[i];
                let previous = &self.previous_window[i];
                for j in 0..n as usize {
                    let at = (left as isize + j as isize) as usize;
                    buffer[at] = buffer[at] * w[j] + previous[j] * w[n as usize - 1 - j];
                }
            }
        }

        let prev = self.previous_length;

        // last half of this data becomes previous window
        self.previous_length = len - right;

        // @OPTIMIZE: could avoid this copy by double-buffering the
        // output (flipping previous_window with channel_buffers), but
        // then previous_window would have to be 2x as large, and
        // channel_buffers couldn't be temp mem (although they're NOT
        // currently temp mem, they could be (unless we want to level
        // performance by spreading out the computation))
        for i in 0..self.s.channels as usize {
            let mut j = 0;
            while right + j < len {
                self.previous_window[i][j as usize] = self.channel_buffers[i][(right + j) as usize];
                j += 1;
            }
        }

        if prev == 0 {
            // there was no previous packet, so this data isn't valid...
            // this isn't entirely true, only the would-have-overlapped data
            // isn't valid, but this seems to be what the spec requires
            return 0;
        }

        // truncate a short frame
        if len < right {
            right = len;
        }

        self.samples_output = self.samples_output.wrapping_add((right - left) as u32);

        right - left
    }

    /// Translation of `vorbis_pump_first_frame()`.
    fn vorbis_pump_first_frame(&mut self) -> bool {
        let mut len = 0;
        let mut right = 0;
        let mut left = 0;
        let res = self.vorbis_decode_packet(&mut len, &mut left, &mut right);
        if res {
            self.vorbis_finish_frame(len, left, right);
        }
        self.current_playback_loc = 0;
        self.current_playback_loc_valid = true;
        res
    }
}

/// What `start_decoder()` allocates: (count, element size), checked like
/// `setup_malloc()`.
macro_rules! setup_malloc {
    ($f:expr, $ty:ty, $count:expr) => {
        match setup_alloc::<$ty>($count as u64, std::mem::size_of::<$ty>() as u64) {
            Some(v) => v,
            None => return $f.error(VORBIS_OUTOFMEM),
        }
    };
}

impl<'a> StbVorbis<'a> {
    /// Translation of `start_decoder()`.
    fn start_decoder(&mut self) -> bool {
        let mut header = [0u8; 6];
        let mut max_submaps = 0;
        let mut longest_floorlist = 0;
        let f = &mut self.r;
        let s = &mut self.s;

        // first page, first packet
        f.first_decode = true;

        if !f.start_page() {
            return false;
        }
        // validate page flag
        if f.page_flag & PAGEFLAG_FIRST_PAGE == 0 {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }
        if f.page_flag & PAGEFLAG_LAST_PAGE != 0 {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }
        if f.page_flag & PAGEFLAG_CONTINUED_PACKET != 0 {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }
        // check for expected packet length
        if f.segment_count != 1 {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }
        if f.segments[0] != 30 {
            // check for the Ogg skeleton fishead identifying header to refine our error
            if f.segments[0] == 64
                && f.getn(&mut header)
                && &header == b"fishea"
                && f.get8() == b'd'
                && f.get8() == b'\0'
            {
                return f.error(VORBIS_OGG_SKELETON_NOT_SUPPORTED);
            } else {
                return f.error(VORBIS_INVALID_FIRST_PAGE);
            }
        }

        // read packet
        // check packet header
        if f.get8() as i32 != VORBIS_PACKET_ID {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }
        if !f.getn(&mut header) {
            return f.error(VORBIS_UNEXPECTED_EOF);
        }
        if !vorbis_validate(&header) {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }
        // vorbis_version
        if f.get32() != 0 {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }
        s.channels = f.get8() as i32;
        if s.channels == 0 {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }
        if s.channels > STB_VORBIS_MAX_CHANNELS as i32 {
            return f.error(VORBIS_TOO_MANY_CHANNELS);
        }
        s.sample_rate = f.get32();
        if s.sample_rate == 0 {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }
        f.get32(); // bitrate_maximum
        f.get32(); // bitrate_nominal
        f.get32(); // bitrate_minimum
        let x = f.get8();
        {
            let log0 = (x & 15) as i32;
            let log1 = (x >> 4) as i32;
            s.blocksize_0 = 1 << log0;
            s.blocksize_1 = 1 << log1;
            if log0 < 6 || log0 > MAX_BLOCKSIZE_LOG {
                return f.error(VORBIS_INVALID_SETUP);
            }
            if log1 < 6 || log1 > MAX_BLOCKSIZE_LOG {
                return f.error(VORBIS_INVALID_SETUP);
            }
            if log0 > log1 {
                return f.error(VORBIS_INVALID_SETUP);
            }
        }

        // framing_flag
        let x = f.get8();
        if x & 1 == 0 {
            return f.error(VORBIS_INVALID_FIRST_PAGE);
        }

        // second packet!
        if !f.start_page() {
            return false;
        }

        if !f.start_packet() {
            return false;
        }

        if f.next_segment() == 0 {
            return false;
        }

        if f.get8_packet() != VORBIS_PACKET_COMMENT {
            return f.error(VORBIS_INVALID_SETUP);
        }
        for i in 0..6 {
            header[i] = f.get8_packet() as u8;
        }
        if !vorbis_validate(&header) {
            return f.error(VORBIS_INVALID_SETUP);
        }
        //file vendor
        let len = f.get32_packet();
        let mut vendor = setup_malloc!(f, u8, len.wrapping_add(1) as i64 as u64);
        for i in 0..len.max(0) as usize {
            vendor[i] = f.get8_packet() as u8;
        }
        vendor[len as usize] = b'\0';
        s.vendor = vendor;
        //user comments
        let comment_list_length = f.get32_packet();
        s.comment_list = Vec::new();
        if comment_list_length > 0 {
            if i32::MAX / 8 < comment_list_length {
                // (no_comment:)
                return f.error(VORBIS_OUTOFMEM);
            }
            let len = 8 * comment_list_length;
            if setup_alloc::<u8>(len as u64, 1).is_none() {
                return f.error(VORBIS_OUTOFMEM);
            }
        }

        for _ in 0..comment_list_length.max(0) {
            let len = f.get32_packet();
            let mut comment = setup_malloc!(f, u8, len.wrapping_add(1) as i64 as u64);

            for j in 0..len.max(0) as usize {
                comment[j] = f.get8_packet() as u8;
            }
            comment[len as usize] = b'\0';
            s.comment_list.push(comment);
        }

        // framing_flag
        let x = f.get8_packet();
        if x & 1 == 0 {
            return f.error(VORBIS_INVALID_SETUP);
        }

        f.skip(f.bytes_in_seg as i32);
        f.bytes_in_seg = 0;

        loop {
            let len = f.next_segment();
            f.skip(len);
            f.bytes_in_seg = 0;
            if len == 0 {
                break;
            }
        }

        // third packet!
        if !f.start_packet() {
            return false;
        }

        if f.get8_packet() != VORBIS_PACKET_SETUP {
            return f.error(VORBIS_INVALID_SETUP);
        }
        for i in 0..6 {
            header[i] = f.get8_packet() as u8;
        }
        if !vorbis_validate(&header) {
            return f.error(VORBIS_INVALID_SETUP);
        }

        // codebooks

        let codebook_count = f.get_bits(8) as i32 + 1;
        if f.valid_bits < 0 {
            return f.error(VORBIS_UNEXPECTED_EOF);
        }
        if setup_alloc::<u8>(codebook_count as u64, 1).is_none() {
            return f.error(VORBIS_OUTOFMEM);
        }
        s.codebooks = (0..codebook_count).map(|_| Codebook::default()).collect();
        for i in 0..codebook_count as usize {
            let mut total = 0;
            let c = &mut s.codebooks[i];
            let x = f.get_bits(8) as u8;
            if x != 0x42 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            let x = f.get_bits(8) as u8;
            if x != 0x43 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            let x = f.get_bits(8) as u8;
            if x != 0x56 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            let x = f.get_bits(8) as u8;
            c.dimensions = ((f.get_bits(8) << 8) + x as u32) as i32;
            let x = f.get_bits(8) as u8;
            let y = f.get_bits(8) as u8;
            c.entries = ((f.get_bits(8) << 16) + ((y as u32) << 8) + x as u32) as i32;
            let ordered = f.get_bits(1);
            c.sparse = if ordered != 0 { 0 } else { f.get_bits(1) as u8 };

            if c.dimensions == 0 && c.entries != 0 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            if f.valid_bits < 0 {
                return f.error(VORBIS_UNEXPECTED_EOF);
            }

            // (for a sparse codebook, the lengths are temporary.)
            let mut lengths: Vec<u8> = setup_malloc!(f, u8, c.entries);

            if ordered != 0 {
                let mut current_entry = 0;
                let mut current_length = f.get_bits(5) as i32 + 1;
                while current_entry < c.entries {
                    let limit = c.entries - current_entry;
                    let n = f.get_bits(ilog(limit)) as i32;
                    if f.valid_bits < 0 {
                        return f.error(VORBIS_UNEXPECTED_EOF);
                    }
                    if current_length >= 32 {
                        return f.error(VORBIS_INVALID_SETUP);
                    }
                    if current_entry + n > c.entries {
                        return f.error(VORBIS_INVALID_SETUP);
                    }
                    lengths[current_entry as usize..(current_entry + n) as usize]
                        .fill(current_length as u8);
                    current_entry += n;
                    current_length += 1;
                }
            } else {
                for j in 0..c.entries as usize {
                    let present = if c.sparse != 0 { f.get_bits(1) } else { 1 };
                    if f.valid_bits < 0 {
                        return f.error(VORBIS_UNEXPECTED_EOF);
                    }
                    if present != 0 {
                        lengths[j] = f.get_bits(5) as u8 + 1;
                        total += 1;
                        if lengths[j] == 32 {
                            return f.error(VORBIS_INVALID_SETUP);
                        }
                    } else {
                        lengths[j] = NO_CODE;
                    }
                }
            }

            if c.sparse != 0 && total >= c.entries >> 2 {
                // convert sparse items to non-sparse!
                if c.entries > s.setup_temp_memory_required as i32 {
                    s.setup_temp_memory_required = c.entries as u32;
                }

                if setup_alloc::<u8>(c.entries as u64, 1).is_none() {
                    return f.error(VORBIS_OUTOFMEM);
                }
                c.sparse = 0;
            }
            if c.sparse == 0 {
                c.codeword_lengths = lengths.clone();
            }

            // compute the size of the sorted tables
            let sorted_count = if c.sparse != 0 {
                total
            } else {
                let mut sorted_count = 0;
                for j in 0..c.entries as usize {
                    if lengths[j] as i32 > STB_VORBIS_FAST_HUFFMAN_LENGTH && lengths[j] != NO_CODE {
                        sorted_count += 1;
                    }
                }
                sorted_count
            };

            c.sorted_entries = sorted_count;
            let mut values: Vec<u32> = Vec::new();

            if c.sparse == 0 {
                c.codewords = Some(setup_malloc!(f, u32, c.entries));
            } else {
                if c.sorted_entries != 0 {
                    c.codeword_lengths = setup_malloc!(f, u8, c.sorted_entries);
                    c.codewords = Some(setup_malloc!(f, u32, c.sorted_entries));
                    values = setup_malloc!(f, u32, c.sorted_entries);
                }
                let size = (c.entries as u32).wrapping_add((4 + 4) * c.sorted_entries as u32);
                if size > s.setup_temp_memory_required {
                    s.setup_temp_memory_required = size;
                }
            }

            if !compute_codewords(c, &lengths, c.entries, &mut values) {
                return f.error(VORBIS_INVALID_SETUP);
            }

            if c.sorted_entries != 0 {
                // allocate an extra slot for sentinels
                c.sorted_codewords = Some(setup_malloc!(f, u32, c.sorted_entries + 1));
                // allocate an extra slot at the front so that c->sorted_values[-1] is defined
                // so that we can catch that case without an extra if
                c.sorted_values = setup_malloc!(f, i32, c.sorted_entries + 1);
                c.sorted_values[0] = -1;
                compute_sorted_huffman(c, &lengths, &values);
            }

            if c.sparse != 0 {
                c.codewords = None;
            }

            compute_accelerated_huffman(c);

            c.lookup_type = f.get_bits(4) as u8;
            if c.lookup_type > 2 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            if c.lookup_type > 0 {
                c.minimum_value = float32_unpack(f.get_bits(32));
                c.delta_value = float32_unpack(f.get_bits(32));
                c.value_bits = f.get_bits(4) as u8 + 1;
                c.sequence_p = f.get_bits(1) as u8;
                if c.lookup_type == 1 {
                    let values = lookup1_values(c.entries, c.dimensions);
                    if values < 0 {
                        return f.error(VORBIS_INVALID_SETUP);
                    }
                    c.lookup_values = values as u32 as u64;
                } else {
                    /* changed to unsigned multiply for:
                     * https://github.com/nothings/stb/issues/1168
                     * https://github.com/nothings/stb/issues/1947
                     * https://github.com/nothings/stb/issues/1933
                     * https://github.com/libxmp/libxmp/issues/996 */
                    c.lookup_values = (c.entries as u64).wrapping_mul(c.dimensions as u64);
                }
                if c.lookup_values == 0 {
                    return f.error(VORBIS_INVALID_SETUP);
                }
                let mut mults: Vec<u16> = setup_malloc!(f, u16, c.lookup_values);
                for j in 0..(c.lookup_values as i32).max(0) as usize {
                    let q = f.get_bits(c.value_bits as i32) as i32;
                    if f.valid_bits < 0 {
                        return f.error(VORBIS_INVALID_SETUP);
                    }
                    mults[j] = q as u16;
                }

                if c.lookup_type == 1 {
                    let sparse = c.sparse != 0;
                    let mut last = 0.0f32;
                    // pre-expand the lookup1-style multiplicands, to avoid a divide in the inner loop
                    if !(sparse && c.sorted_entries == 0) {
                        let count = if sparse { c.sorted_entries } else { c.entries } as u64;
                        c.multiplicands =
                            setup_malloc!(f, f32, count.wrapping_mul(c.dimensions as u64));
                        let len = if sparse { c.sorted_entries } else { c.entries };
                        for j in 0..len.max(0) as usize {
                            let z: u32 = if sparse {
                                c.sorted_values[j + 1] as u32
                            } else {
                                j as u32
                            };
                            let mut div: u32 = 1;
                            for k in 0..c.dimensions {
                                let off = ((z / div) as u64 % c.lookup_values) as usize;
                                let val = mults[off] as i32 as f32 * c.delta_value
                                    + c.minimum_value
                                    + last;
                                c.multiplicands[j * c.dimensions as usize + k as usize] = val;
                                if c.sequence_p != 0 {
                                    last = val;
                                }
                                if k + 1 < c.dimensions {
                                    if div > u32::MAX / c.lookup_values as u32 {
                                        return f.error(VORBIS_INVALID_SETUP);
                                    }
                                    div = div.wrapping_mul(c.lookup_values as u32);
                                }
                            }
                        }
                        c.lookup_type = 2;
                    }
                    // (skip:)
                } else {
                    let mut last = 0.0f32;
                    c.multiplicands = setup_malloc!(f, f32, c.lookup_values);
                    for j in 0..c.lookup_values as usize {
                        let val = mults[j] as i32 as f32 * c.delta_value + c.minimum_value + last;
                        c.multiplicands[j] = val;
                        if c.sequence_p != 0 {
                            last = val;
                        }
                    }
                }
            }
        }

        // time domain transfers (notused)

        let x = f.get_bits(6) + 1;
        for _ in 0..x {
            let z = f.get_bits(16);
            if z != 0 {
                return f.error(VORBIS_INVALID_SETUP);
            }
        }

        // Floors
        let floor_count = f.get_bits(6) as i32 + 1;
        if f.valid_bits < 0 {
            return f.error(VORBIS_UNEXPECTED_EOF);
        }
        s.floor_config = vec![Floor1::default(); floor_count as usize];
        for i in 0..floor_count as usize {
            s.floor_types[i] = f.get_bits(16) as u16;
            if s.floor_types[i] > 1 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            if s.floor_types[i] == 0 {
                // (Floor0: order, rate, bark_map_size, amplitude_bits, amplitude_offset)
                f.get_bits(8);
                f.get_bits(16);
                f.get_bits(16);
                f.get_bits(6);
                f.get_bits(8);
                let number_of_books = f.get_bits(4) + 1;
                for _ in 0..number_of_books {
                    f.get_bits(8); // book_list
                }
                return f.error(VORBIS_FEATURE_NOT_SUPPORTED);
            } else {
                let g = &mut s.floor_config[i];
                let mut max_class: i32 = -1;
                g.partitions = f.get_bits(5) as u8;
                for j in 0..g.partitions as usize {
                    g.partition_class_list[j] = f.get_bits(4) as u8;
                    if g.partition_class_list[j] as i32 > max_class {
                        max_class = g.partition_class_list[j] as i32;
                    }
                }
                for j in 0..(max_class + 1) as usize {
                    g.class_dimensions[j] = f.get_bits(3) as u8 + 1;
                    g.class_subclasses[j] = f.get_bits(2) as u8;
                    if f.valid_bits < 0 {
                        return f.error(VORBIS_UNEXPECTED_EOF);
                    }
                    if g.class_subclasses[j] != 0 {
                        g.class_masterbooks[j] = f.get_bits(8) as u8;
                        if g.class_masterbooks[j] as i32 >= codebook_count {
                            return f.error(VORBIS_INVALID_SETUP);
                        }
                    }
                    for k in 0..(1usize << g.class_subclasses[j]) {
                        g.subclass_books[j][k] = (f.get_bits(8) as i16) - 1;
                        if g.subclass_books[j][k] as i32 >= codebook_count {
                            return f.error(VORBIS_INVALID_SETUP);
                        }
                    }
                }
                g.floor1_multiplier = f.get_bits(2) as u8 + 1;
                g.rangebits = f.get_bits(4) as u8;
                g.xlist[0] = 0;
                g.xlist[1] = 1 << g.rangebits;
                g.values = 2;
                for j in 0..g.partitions as usize {
                    let c = g.partition_class_list[j] as usize;
                    for _ in 0..g.class_dimensions[c] {
                        g.xlist[g.values as usize] = f.get_bits(g.rangebits as i32) as u16;
                        g.values += 1;
                    }
                }
                // precompute the sorting
                // (SDL_qsort by x; with equal xs, the setup fails below whatever the order.)
                let mut p: Vec<(u16, u16)> = (0..g.values as usize)
                    .map(|j| (g.xlist[j], j as u16))
                    .collect();
                p.sort_by_key(|&(x, _)| x);
                for j in 0..(g.values - 1).max(0) as usize {
                    if p[j].0 == p[j + 1].0 {
                        return f.error(VORBIS_INVALID_SETUP);
                    }
                }
                for j in 0..g.values as usize {
                    g.sorted_order[j] = p[j].1 as u8;
                }
                // precompute the neighbors
                for j in 2..g.values as usize {
                    let mut low = 0;
                    let mut hi = 0;
                    neighbors(&g.xlist, j, &mut low, &mut hi);
                    g.neighbors[j][0] = low as u8;
                    g.neighbors[j][1] = hi as u8;
                }

                if g.values > longest_floorlist {
                    longest_floorlist = g.values;
                }
            }
        }

        // Residue
        let residue_count = f.get_bits(6) as i32 + 1;
        if f.valid_bits < 0 {
            return f.error(VORBIS_UNEXPECTED_EOF);
        }
        s.residue_config = (0..residue_count).map(|_| Residue::default()).collect();
        for i in 0..residue_count as usize {
            let mut residue_cascade = [0u8; 64];
            s.residue_types[i] = f.get_bits(16) as u16;
            if s.residue_types[i] > 2 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            let r = &mut s.residue_config[i];
            r.begin = f.get_bits(24);
            r.end = f.get_bits(24);
            if r.end < r.begin {
                return f.error(VORBIS_INVALID_SETUP);
            }
            r.part_size = f.get_bits(24) + 1;
            r.classifications = f.get_bits(6) as u8 + 1;
            r.classbook = f.get_bits(8) as u8;
            if f.valid_bits < 0 {
                return f.error(VORBIS_UNEXPECTED_EOF);
            }
            if r.classbook as i32 >= codebook_count {
                return f.error(VORBIS_INVALID_SETUP);
            }
            for j in 0..r.classifications as usize {
                let mut high_bits = 0u8;
                let low_bits = f.get_bits(3) as u8;
                if f.get_bits(1) != 0 {
                    high_bits = f.get_bits(5) as u8;
                }
                residue_cascade[j] = high_bits.wrapping_mul(8).wrapping_add(low_bits);
            }
            if f.valid_bits < 0 {
                return f.error(VORBIS_UNEXPECTED_EOF);
            }
            r.residue_books = setup_malloc!(f, [i16; 8], r.classifications);
            for j in 0..r.classifications as usize {
                for k in 0..8 {
                    if residue_cascade[j] & (1 << k) != 0 {
                        r.residue_books[j][k] = f.get_bits(8) as i16;
                        if f.valid_bits < 0 {
                            return f.error(VORBIS_UNEXPECTED_EOF);
                        }
                        if r.residue_books[j][k] as i32 >= codebook_count {
                            return f.error(VORBIS_INVALID_SETUP);
                        }
                    } else {
                        r.residue_books[j][k] = -1;
                    }
                }
            }
            // precompute the classifications[] array to avoid inner-loop mod/divide
            // call it 'classdata' since we already have r->classifications
            let classbook = &s.codebooks[r.classbook as usize];
            if setup_alloc::<usize>(classbook.entries as u64, 8).is_none() {
                return f.error(VORBIS_OUTOFMEM);
            }
            r.classdata = Vec::with_capacity(classbook.entries as usize);
            for j in 0..classbook.entries {
                let classwords = classbook.dimensions;
                let mut temp = j;
                let mut row: Vec<u8> = setup_malloc!(f, u8, classwords);
                let mut k = classwords - 1;
                while k >= 0 {
                    row[k as usize] = (temp % r.classifications as i32) as u8;
                    temp /= r.classifications as i32;
                    k -= 1;
                }
                r.classdata.push(row);
            }
        }

        let mapping_count = f.get_bits(6) as i32 + 1;
        if f.valid_bits < 0 {
            return f.error(VORBIS_UNEXPECTED_EOF);
        }
        s.mapping = (0..mapping_count).map(|_| Mapping::default()).collect();
        for i in 0..mapping_count as usize {
            let m = &mut s.mapping[i];
            let mapping_type = f.get_bits(16);
            if mapping_type != 0 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            m.chan = setup_malloc!(f, MappingChannel, s.channels);
            if f.get_bits(1) != 0 {
                m.submaps = f.get_bits(4) as u8 + 1;
            } else {
                m.submaps = 1;
            }
            if m.submaps as i32 > max_submaps {
                max_submaps = m.submaps as i32;
            }
            if f.get_bits(1) != 0 {
                m.coupling_steps = f.get_bits(8) as u16 + 1;
                if m.coupling_steps as i32 > s.channels {
                    return f.error(VORBIS_INVALID_SETUP);
                }
                for k in 0..m.coupling_steps as usize {
                    m.chan[k].magnitude = f.get_bits(ilog(s.channels - 1)) as u8;
                    m.chan[k].angle = f.get_bits(ilog(s.channels - 1)) as u8;
                    if f.valid_bits < 0 {
                        return f.error(VORBIS_UNEXPECTED_EOF);
                    }
                    if m.chan[k].magnitude as i32 >= s.channels {
                        return f.error(VORBIS_INVALID_SETUP);
                    }
                    if m.chan[k].angle as i32 >= s.channels {
                        return f.error(VORBIS_INVALID_SETUP);
                    }
                    if m.chan[k].magnitude == m.chan[k].angle {
                        return f.error(VORBIS_INVALID_SETUP);
                    }
                }
            } else {
                m.coupling_steps = 0;
            }

            // reserved field
            if f.get_bits(2) != 0 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            if m.submaps > 1 {
                for j in 0..s.channels as usize {
                    m.chan[j].mux = f.get_bits(4) as u8;
                    if m.chan[j].mux >= m.submaps {
                        return f.error(VORBIS_INVALID_SETUP);
                    }
                }
            } else {
                // @SPECIFICATION: this case is missing from the spec
                for j in 0..s.channels as usize {
                    m.chan[j].mux = 0;
                }
            }

            for j in 0..m.submaps as usize {
                f.get_bits(8); // discard
                m.submap_floor[j] = f.get_bits(8) as u8;
                m.submap_residue[j] = f.get_bits(8) as u8;
                if m.submap_floor[j] as i32 >= floor_count {
                    return f.error(VORBIS_INVALID_SETUP);
                }
                if m.submap_residue[j] as i32 >= residue_count {
                    return f.error(VORBIS_INVALID_SETUP);
                }
            }
        }

        // Modes
        s.mode_count = f.get_bits(6) as i32 + 1;
        for i in 0..s.mode_count as usize {
            let m = &mut s.mode_config[i];
            m.blockflag = f.get_bits(1) as u8;
            m.windowtype = f.get_bits(16) as u16;
            m.transformtype = f.get_bits(16) as u16;
            m.mapping = f.get_bits(8) as u8;
            if f.valid_bits < 0 {
                return f.error(VORBIS_UNEXPECTED_EOF);
            }
            if m.windowtype != 0 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            if m.transformtype != 0 {
                return f.error(VORBIS_INVALID_SETUP);
            }
            if m.mapping as i32 >= mapping_count {
                return f.error(VORBIS_INVALID_SETUP);
            }
        }

        f.flush_packet();

        self.previous_length = 0;

        self.channel_buffers.clear();
        self.previous_window.clear();
        self.final_y.clear();
        for _ in 0..s.channels {
            let channel_buffer = setup_alloc::<f32>(s.blocksize_1 as u64, 4);
            let previous_window = setup_alloc::<f32>((s.blocksize_1 / 2) as u64, 4);
            let final_y = setup_alloc::<i16>(longest_floorlist as u64, 2);
            match (channel_buffer, previous_window, final_y) {
                (Some(c), Some(p), Some(y)) => {
                    self.channel_buffers.push(c);
                    self.previous_window.push(p);
                    self.final_y.push(y);
                }
                _ => return f.error(VORBIS_OUTOFMEM),
            }
        }

        if !init_blocksize(f, s, 0, s.blocksize_0) {
            return false;
        }
        if !init_blocksize(f, s, 1, s.blocksize_1) {
            return false;
        }
        s.blocksize[0] = s.blocksize_0;
        s.blocksize[1] = s.blocksize_1;

        // compute how much temporary memory is needed

        // 1.
        {
            let imdct_mem = (s.blocksize_1 as u32 * 4) >> 1;
            let mut max_part_read = 0;
            for i in 0..residue_count as usize {
                let r = &s.residue_config[i];
                let rtype = s.residue_types[i] as u32;
                let actual_size: u32 = if rtype == 2 {
                    s.blocksize_1 as u32
                } else {
                    s.blocksize_1 as u32 / 2
                };
                let limit_r_begin = if r.begin < actual_size {
                    r.begin
                } else {
                    actual_size
                };
                let limit_r_end = if r.end < actual_size {
                    r.end
                } else {
                    actual_size
                };
                let n_read = limit_r_end.wrapping_sub(limit_r_begin) as i32;
                let part_read = n_read / r.part_size as i32;
                if part_read > max_part_read {
                    max_part_read = part_read;
                }
            }
            let classify_mem = (s.channels as u32).wrapping_mul(8 + max_part_read as u32 * 8);

            // maximum reasonable partition size is f->blocksize_1

            s.temp_memory_required = classify_mem;
            if imdct_mem > s.temp_memory_required {
                s.temp_memory_required = imdct_mem;
            }
        }

        // (the work buffer: buffers of this size are made as they're needed.)
        if setup_alloc::<u8>(s.temp_memory_required as u64, 1).is_none() {
            return f.error(VORBIS_OUTOFMEM);
        }

        // @TODO: stb_vorbis_seek_start expects first_audio_page_offset to point to a page
        // without PAGEFLAG_continued_packet, so this either points to the first page, or
        // the page after the end of the headers. It might be cleaner to point to a page
        // in the middle of the headers, when that's the page where the first audio packet
        // starts, but we'd have to also correctly skip the end of any continued packet in
        // stb_vorbis_seek_start.
        if f.next_seg == -1 {
            s.first_audio_page_offset = f.get_file_offset();
        } else {
            s.first_audio_page_offset = 0;
        }

        true
    }
}

/// Translation of `init_blocksize()`.
fn init_blocksize(f: &mut Reader<'_>, s: &mut Setup, b: usize, n: i32) -> bool {
    let n2 = n >> 1;
    let n4 = n >> 2;
    let n8 = n >> 3;
    let a = setup_alloc::<f32>(n2 as u64, 4);
    let bb = setup_alloc::<f32>(n2 as u64, 4);
    let c = setup_alloc::<f32>(n4 as u64, 4);
    let (Some(mut a), Some(mut bb), Some(mut c)) = (a, bb, c) else {
        return f.error(VORBIS_OUTOFMEM);
    };
    compute_twiddle_factors(n, &mut a, &mut bb, &mut c);
    s.a[b] = a;
    s.b[b] = bb;
    s.c[b] = c;
    let Some(mut window) = setup_alloc::<f32>(n2 as u64, 4) else {
        return f.error(VORBIS_OUTOFMEM);
    };
    compute_window(n, &mut window);
    s.window[b] = window;
    let Some(mut bit_reverse) = setup_alloc::<u16>(n8 as u64, 2) else {
        return f.error(VORBIS_OUTOFMEM);
    };
    compute_bitreverse(n, &mut bit_reverse);
    s.bit_reverse[b] = bit_reverse;
    true
}

impl<'a> StbVorbis<'a> {
    /// Translation of `vorbis_init()`.
    fn new(io: IoStream<'a>) -> StbVorbis<'a> {
        StbVorbis {
            r: Reader {
                io,
                io_start: 0,
                io_virtual_pos: 0,
                io_buffer_pos: 0,
                io_buffer_fill: 0,
                io_buffer: Box::new([0; IO_BUFFER_SIZE]),
                stream_len: 0,
                p_first: ProbedPage::default(),
                eof: false,
                error: VORBIS__NO_ERROR,
                last_page: 0,
                segment_count: 0,
                segments: [0; 255],
                page_flag: 0,
                bytes_in_seg: 0,
                first_decode: false,
                next_seg: 0,
                last_seg: false,
                last_seg_which: 0,
                acc: 0,
                valid_bits: 0,
                packet_bytes: 0,
                end_seg_with_known_loc: 0,
                known_loc_for_packet: 0,
            },
            s: Setup::default(),
            total_samples: 0,
            p_last: ProbedPage::default(),
            channel_buffers: Vec::new(),
            previous_window: Vec::new(),
            previous_length: 0,
            final_y: Vec::new(),
            current_loc: 0,
            current_loc_valid: false,
            current_playback_loc: 0,
            current_playback_loc_valid: false,
            discard_samples_deferred: 0,
            samples_output: 0,
            channel_buffer_start: 0,
            channel_buffer_end: 0,
        }
    }

    /// Translation of `stb_vorbis_get_info()`.
    pub fn get_info(&self) -> StbVorbisInfo {
        StbVorbisInfo {
            channels: self.s.channels,
            sample_rate: self.s.sample_rate,
            setup_memory_required: self.s.setup_memory_required,
            setup_temp_memory_required: self.s.setup_temp_memory_required,
            temp_memory_required: self.s.temp_memory_required,
            max_frame_size: self.s.blocksize_1 >> 1,
        }
    }

    /// Translation of `stb_vorbis_get_comment()`: the vendor and comments,
    /// as C strings.
    pub fn get_comment(&self) -> (&[u8], &[Vec<u8>]) {
        (&self.s.vendor, &self.s.comment_list)
    }

    /// Translation of `stb_vorbis_get_error()`.
    pub fn get_error(&mut self) -> StbVorbisError {
        let e = self.r.error;
        self.r.error = VORBIS__NO_ERROR;
        e
    }
}

//
// DATA-PULLING API
//

impl Reader<'_> {
    /// Translation of `vorbis_find_page()`.
    fn vorbis_find_page(&mut self, end: Option<&mut u32>, last: Option<&mut u32>) -> bool {
        loop {
            if self.eof {
                return false;
            }
            let n = self.get8();
            if n == 0x4f {
                // page header candidate
                let retry_loc = self.get_file_offset();
                // check if we're off the end of a file_section stream
                if retry_loc.wrapping_sub(25) > self.stream_len {
                    return false;
                }
                // check the rest of the header
                let mut i = 1;
                while i < 4 {
                    if self.get8() != OGG_PAGE_HEADER[i] {
                        break;
                    }
                    i += 1;
                }
                if self.eof {
                    return false;
                }
                if i == 4 {
                    'invalid: {
                        let mut header = [0u8; 27];
                        header[..4].copy_from_slice(&OGG_PAGE_HEADER);
                        for b in header.iter_mut().skip(4) {
                            *b = self.get8();
                        }
                        if self.eof {
                            return false;
                        }
                        if header[4] != 0 {
                            break 'invalid;
                        }
                        let goal = header[22] as u32
                            + ((header[23] as u32) << 8)
                            + ((header[24] as u32) << 16)
                            + ((header[25] as u32) << 24);
                        header[22..26].fill(0);
                        let mut crc = 0u32;
                        for &b in header.iter() {
                            crc = crc32_update(crc, b);
                        }
                        let mut len = 0u32;
                        for _ in 0..header[26] {
                            let s = self.get8();
                            crc = crc32_update(crc, s);
                            len += s as u32;
                        }
                        if len != 0 && self.eof {
                            return false;
                        }
                        for _ in 0..len {
                            crc = crc32_update(crc, self.get8());
                        }
                        // finished parsing probable page
                        if crc == goal {
                            // we could now check that it's either got the last
                            // page flag set, OR it's followed by the capture
                            // pattern, but I guess TECHNICALLY you could have
                            // a file with garbage between each ogg page and recover
                            // from it automatically? So even though that paranoia
                            // might decrease the chance of an invalid decode by
                            // another 2^32, not worth it since it would hose those
                            // invalid-but-useful files?
                            if let Some(end) = end {
                                *end = self.get_file_offset();
                            }
                            if let Some(last) = last {
                                *last = if header[5] & 0x04 != 0 { 1 } else { 0 };
                            }
                            self.set_file_offset(retry_loc.wrapping_sub(1));
                            return true;
                        }
                    }
                }
                // (invalid:)
                // not a valid page, so rewind and look for next one
                self.set_file_offset(retry_loc);
            }
        }
    }

    // seeking is implemented with a binary search, which narrows down the range to
    // 64K, before using a linear search (because finding the synchronization
    // pattern can be expensive, and the chance we'd find the end page again is
    // relatively high for small ranges)
    //
    // two initial interpolation-style probes are used at the start of the search
    // to try to bound either side of the binary search sensibly, while still
    // working in O(log n) time if they fail.

    /// Translation of `get_seek_page_info()`.
    fn get_seek_page_info(&mut self, z: &mut ProbedPage) -> bool {
        let mut header = [0u8; 27];
        let mut lacing = [0u8; 255];

        // record where the page starts
        z.page_start = self.get_file_offset();

        // parse the header
        if !self.getn(&mut header) {
            return false;
        }
        if &header[..4] != b"OggS" {
            return false;
        }
        if !self.getn(&mut lacing[..header[26] as usize]) {
            return false;
        }

        // determine the length of the payload
        let mut len: i32 = 0;
        for i in 0..header[26] as usize {
            len += lacing[i] as i32;
        }

        // this implies where the page ends
        z.page_end = z
            .page_start
            .wrapping_add(27)
            .wrapping_add(header[26] as u32)
            .wrapping_add(len as u32);

        // read the last-decoded sample out of the data
        z.last_decoded_sample = (header[6] as u32)
            .wrapping_add((header[7] as u32) << 8)
            .wrapping_add((header[8] as u32) << 16)
            .wrapping_add((header[9] as u32) << 24);

        // restore file state to where we were
        self.set_file_offset(z.page_start);
        true
    }
}

const SAMPLE_UNKNOWN: u32 = 0xffffffff;

impl<'a> StbVorbis<'a> {
    // rarely used function to seek back to the preceding page while finding the
    // start of a packet
    /// Translation of `go_to_page_before()`.
    fn go_to_page_before(&mut self, limit_offset: u32) -> bool {
        let mut end = 0u32;

        // now we want to seek back 64K from the limit
        let previous_safe =
            if limit_offset >= 65536 && limit_offset - 65536 >= self.s.first_audio_page_offset {
                limit_offset - 65536
            } else {
                self.s.first_audio_page_offset
            };

        self.r.set_file_offset(previous_safe);

        while self.r.vorbis_find_page(Some(&mut end), None) {
            if end >= limit_offset && self.r.get_file_offset() < limit_offset {
                return true;
            }
            self.r.set_file_offset(end);
        }

        false
    }

    // implements the search logic for finding a page and starting decoding. if
    // the function succeeds, current_loc_valid will be true and current_loc will
    // be less than or equal to the provided sample number (the closer the
    // better).
    /// Translation of `seek_to_sample_coarse()`.
    fn seek_to_sample_coarse(&mut self, sample_number: u32) -> bool {
        let mut mid = ProbedPage::default();
        let mut offset: f64 = 0.0;
        let mut bytes_per_sample: f64 = 0.0;
        let mut probe = 0;

        // find the last page and validate the target sample
        let stream_length = self.stream_length_in_samples();
        if stream_length == 0 {
            return self.r.error(VORBIS_SEEK_WITHOUT_LENGTH);
        }
        if sample_number > stream_length {
            return self.r.error(VORBIS_SEEK_INVALID);
        }

        'error: {
            // this is the maximum difference between the window-center (which is the
            // actual granule position value), and the right-start (which the spec
            // indicates should be the granule position (give or take one)).
            let padding = ((self.s.blocksize_1 - self.s.blocksize_0) >> 2) as u32;
            let last_sample_limit = if sample_number < padding {
                0
            } else {
                sample_number - padding
            };

            let mut left = self.r.p_first;
            while left.last_decoded_sample == !0u32 {
                // (untested) the first page does not have a 'last_decoded_sample'
                self.r.set_file_offset(left.page_end);
                if !self.r.get_seek_page_info(&mut left) {
                    break 'error;
                }
            }

            let mut right = self.p_last;
            // (assert(right.last_decoded_sample != ~0U))

            // starting from the start is handled differently
            if last_sample_limit <= left.last_decoded_sample {
                if self.seek_start() {
                    if self.current_loc > sample_number {
                        return self.r.error(VORBIS_SEEK_FAILED);
                    }
                    return true;
                }
                return false;
            }

            while left.page_end != right.page_start {
                // (assert(left.page_end < right.page_start))
                // search range in bytes
                let delta = right.page_start.wrapping_sub(left.page_end);
                if delta <= 65536 {
                    // there's only 64K left to search - handle it linearly
                    self.r.set_file_offset(left.page_end);
                } else {
                    if probe < 2 {
                        if probe == 0 {
                            // first probe (interpolate)
                            let data_bytes = right.page_end.wrapping_sub(left.page_start) as f64;
                            bytes_per_sample = data_bytes / right.last_decoded_sample as f64;
                            offset = left.page_start as f64
                                + bytes_per_sample
                                    * last_sample_limit.wrapping_sub(left.last_decoded_sample)
                                        as f64;
                        } else {
                            // second probe (try to bound the other side)
                            let mut error = (last_sample_limit as f64
                                - mid.last_decoded_sample as f64)
                                * bytes_per_sample;
                            if error >= 0.0 && error < 8000.0 {
                                error = 8000.0;
                            }
                            if error < 0.0 && error > -8000.0 {
                                error = -8000.0;
                            }
                            offset += error * 2.0;
                        }

                        // ensure the offset is valid
                        if offset < left.page_end as f64 {
                            offset = left.page_end as f64;
                        }
                        if offset > right.page_start.wrapping_sub(65536) as f64 {
                            offset = right.page_start.wrapping_sub(65536) as f64;
                        }

                        self.r.set_file_offset(f64_to_u32(offset));
                    } else {
                        // binary search for large ranges (offset by 32K to ensure
                        // we don't hit the right page)
                        self.r.set_file_offset(
                            left.page_end.wrapping_add(delta / 2).wrapping_sub(32768),
                        );
                    }

                    if !self.r.vorbis_find_page(None, None) {
                        break 'error;
                    }
                }

                loop {
                    if !self.r.get_seek_page_info(&mut mid) {
                        break 'error;
                    }
                    if mid.last_decoded_sample != !0u32 {
                        break;
                    }
                    // (untested) no frames end on this page
                    self.r.set_file_offset(mid.page_end);
                    // (assert(mid.page_start < right.page_start))
                }

                // if we've just found the last page again then we're in a tricky file,
                // and we're close enough (if it wasn't an interpolation probe).
                if mid.page_start == right.page_start {
                    if probe >= 2 || delta <= 65536 {
                        break;
                    }
                } else if last_sample_limit < mid.last_decoded_sample {
                    right = mid;
                } else {
                    left = mid;
                }

                probe += 1;
            }

            // seek back to start of the last packet
            let mut page_start = left.page_start;
            self.r.set_file_offset(page_start);
            if !self.r.start_page() {
                return self.r.error(VORBIS_SEEK_FAILED);
            }
            let mut end_pos = self.r.end_seg_with_known_loc;
            // (assert(end_pos >= 0))

            let start_seg_with_known_loc;
            loop {
                let mut i = end_pos;
                while i > 0 {
                    if self.r.segments[(i - 1) as usize] != 255 {
                        break;
                    }
                    i -= 1;
                }

                let start_seg = i;

                if start_seg > 0 || (self.r.page_flag & PAGEFLAG_CONTINUED_PACKET) == 0 {
                    start_seg_with_known_loc = start_seg;
                    break;
                }

                // (untested) the final packet begins on an earlier page
                if !self.go_to_page_before(page_start) {
                    break 'error;
                }

                page_start = self.r.get_file_offset();
                if !self.r.start_page() {
                    break 'error;
                }
                end_pos = self.r.segment_count - 1;
            }

            // prepare to start decoding
            self.current_loc_valid = false;
            self.r.last_seg = false;
            self.r.valid_bits = 0;
            self.r.packet_bytes = 0;
            self.r.bytes_in_seg = 0;
            self.previous_length = 0;
            self.r.next_seg = start_seg_with_known_loc;

            for i in 0..start_seg_with_known_loc.max(0) as usize {
                self.r.skip(self.r.segments[i] as i32);
            }

            // start decoding (optimizable - this frame is generally discarded)
            if !self.vorbis_pump_first_frame() {
                return false;
            }
            if self.current_loc > sample_number {
                return self.r.error(VORBIS_SEEK_FAILED);
            }
            return true;
        }

        // (error:)
        // try to restore the file to a valid state
        self.seek_start();
        self.r.error(VORBIS_SEEK_FAILED)
    }

    // the same as vorbis_decode_initial, but without advancing
    /// Translation of `peek_decode_initial()`.
    fn peek_decode_initial(
        &mut self,
        p_left_start: &mut i32,
        p_left_end: &mut i32,
        p_right_start: &mut i32,
        p_right_end: &mut i32,
        mode: &mut i32,
    ) -> bool {
        if !self.vorbis_decode_initial(p_left_start, p_left_end, p_right_start, p_right_end, mode) {
            return false;
        }

        // either 1 or 2 bytes were read, figure out which so we can rewind
        let mut bits_read = 1 + ilog(self.s.mode_count - 1);
        if self.s.mode_config[*mode as usize].blockflag != 0 {
            bits_read += 2;
        }
        let bytes_read = (bits_read + 7) / 8;

        let f = &mut self.r;
        f.bytes_in_seg = f.bytes_in_seg.wrapping_add(bytes_read as u8);
        f.packet_bytes -= bytes_read;
        f.skip(-bytes_read);
        if f.next_seg == -1 {
            f.next_seg = f.segment_count - 1;
        } else {
            f.next_seg -= 1;
        }
        f.valid_bits = 0;

        true
    }

    /// Translation of `stb_vorbis_seek_frame()`.
    pub fn seek_frame(&mut self, sample_number: u32) -> bool {
        // fast page-level search
        if !self.seek_to_sample_coarse(sample_number) {
            return false;
        }

        // (assert(f->current_loc_valid))
        // (assert(f->current_loc <= sample_number))

        // linear search for the relevant packet
        let max_frame_samples = ((self.s.blocksize_1 * 3 - self.s.blocksize_0) >> 2) as u32;
        while self.current_loc < sample_number {
            let mut left_start = 0;
            let mut left_end = 0;
            let mut right_start = 0;
            let mut right_end = 0;
            let mut mode = 0;
            if !self.peek_decode_initial(
                &mut left_start,
                &mut left_end,
                &mut right_start,
                &mut right_end,
                &mut mode,
            ) {
                return self.r.error(VORBIS_SEEK_FAILED);
            }
            // calculate the number of samples returned by the next frame
            let frame_samples = right_start - left_start;
            if self.current_loc.wrapping_add(frame_samples as u32) > sample_number {
                return true; // the next frame will contain the sample
            } else if self
                .current_loc
                .wrapping_add(frame_samples as u32)
                .wrapping_add(max_frame_samples)
                > sample_number
            {
                // there's a chance the frame after this could contain the sample
                self.vorbis_pump_first_frame();
            } else {
                // this frame is too early to be relevant
                self.current_loc = self.current_loc.wrapping_add(frame_samples as u32);
                self.previous_length = 0;
                self.r.maybe_start_packet();
                self.r.flush_packet();
            }
        }
        // the next frame should start with the sample
        if self.current_loc != sample_number {
            return self.r.error(VORBIS_SEEK_FAILED);
        }
        self.current_playback_loc = sample_number as i32;
        true
    }

    /// Translation of `stb_vorbis_seek_start()`.
    pub fn seek_start(&mut self) -> bool {
        self.r.set_file_offset(self.s.first_audio_page_offset);
        self.previous_length = 0;
        self.r.first_decode = true;
        self.r.next_seg = -1;
        self.vorbis_pump_first_frame()
    }

    /// Translation of `stb_vorbis_stream_length_in_samples()`.
    pub fn stream_length_in_samples(&mut self) -> u32 {
        if self.total_samples == 0 {
            let mut end = 0u32;
            let mut last = 0u32;
            let mut header = [0u8; 6];

            // first, store the current decode position so we can restore it
            let restore_offset = self.r.get_file_offset();

            // now we want to seek back 64K from the end (the last page must
            // be at most a little less than 64K, but let's allow a little slop)
            let previous_safe = if self.r.stream_len >= 65536
                && self.r.stream_len - 65536 >= self.s.first_audio_page_offset
            {
                self.r.stream_len - 65536
            } else {
                self.s.first_audio_page_offset
            };

            self.r.set_file_offset(previous_safe);
            // previous_safe is now our candidate 'earliest known place that seeking
            // to will lead to the final page'

            'done: {
                if !self.r.vorbis_find_page(Some(&mut end), Some(&mut last)) {
                    // if we can't find a page, we're hosed!
                    self.r.error = VORBIS_CANT_FIND_LAST_PAGE;
                    self.total_samples = 0xffffffff;
                    break 'done;
                }

                // check if there are more pages
                let mut last_page_loc = self.r.get_file_offset();

                // stop when the last_page flag is set, not when we reach eof;
                // this allows us to stop short of a 'file_section' end without
                // explicitly checking the length of the section
                while last == 0 {
                    self.r.set_file_offset(end);
                    if !self.r.vorbis_find_page(Some(&mut end), Some(&mut last)) {
                        // the last page we found didn't have the 'last page' flag
                        // set. whoops!
                        break;
                    }
                    //previous_safe = last_page_loc+1; // NOTE: not used after this point, but note for debugging
                    last_page_loc = self.r.get_file_offset();
                }

                self.r.set_file_offset(last_page_loc);

                // parse the header
                self.r.getn(&mut header);
                // extract the absolute granule position
                let mut lo = self.r.get32();
                let hi = self.r.get32();
                if lo == 0xffffffff && hi == 0xffffffff {
                    self.r.error = VORBIS_CANT_FIND_LAST_PAGE;
                    self.total_samples = SAMPLE_UNKNOWN;
                    break 'done;
                }
                if hi != 0 {
                    lo = 0xfffffffe; // saturate
                }
                self.total_samples = lo;

                self.p_last.page_start = last_page_loc;
                self.p_last.page_end = end;
                self.p_last.last_decoded_sample = lo;
            }

            // (done:)
            self.r.set_file_offset(restore_offset);
        }
        if self.total_samples == SAMPLE_UNKNOWN {
            0
        } else {
            self.total_samples
        }
    }

    /// Translation of `stb_vorbis_get_frame_float()`: the number of samples
    /// per channel, each channel's being `channel(i)[..len]`.
    pub fn get_frame_float(&mut self) -> i32 {
        let mut len = 0;
        let mut right = 0;
        let mut left = 0;

        if !self.vorbis_decode_packet(&mut len, &mut left, &mut right) {
            self.channel_buffer_start = 0;
            self.channel_buffer_end = 0;
            return 0;
        }

        let len = self.vorbis_finish_frame(len, left, right);

        self.channel_buffer_start = left;
        self.channel_buffer_end = left + len;

        len
    }

    /// Where `f->outputs[i]` points in channel `i`'s buffer (see
    /// `channel_buffer()`).
    pub fn output_start(&self) -> usize {
        self.channel_buffer_start.max(0) as usize
    }

    /// `f->channel_buffers[i]`: the buffer channel `i`'s output points into.
    pub fn channel_buffer(&self, i: usize) -> &[f32] {
        &self.channel_buffers[i]
    }

    /// Translation of `stb_vorbis_open_io_section()`: on failure, the error.
    pub fn open_io_section(io: IoStream<'a>, length: u32) -> Result<StbVorbis<'a>, StbVorbisError> {
        let mut p = StbVorbis::new(io);
        p.r.io_start = p.r.io.tell().unwrap_or(-1) as u32;
        p.r.stream_len = length;
        if p.start_decoder() {
            p.vorbis_pump_first_frame();
            return Ok(p);
        }
        Err(p.r.error)
    }

    /// Translation of `stb_vorbis_open_io()`.
    pub fn open_io(mut io: IoStream<'a>) -> Result<StbVorbis<'a>, StbVorbisError> {
        let start = io.tell().unwrap_or(-1) as u32;
        let len = (io.size().unwrap_or(-1) as u32).wrapping_sub(start);
        Self::open_io_section(io, len)
    }
}

/// x86's conversion of a double to an unsigned 32-bit integer (as gcc
/// emits it: through a signed 64-bit conversion), for the C cast that's
/// out of range with corrupt data.
fn f64_to_u32(f: f64) -> u32 {
    if f.is_nan() || f >= 9223372036854775808.0 || f < -9223372036854775808.0 {
        0
    } else {
        f as i64 as u32
    }
}
