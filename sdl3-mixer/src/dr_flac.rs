// Rust translation of src/dr_libs/dr_flac.h (dr_flac v0.13.4) from SDL_mixer.
// dr_flac is by David Reid; it is available under a choice of the public
// domain (Unlicense) or MIT No Attribution. See LICENSE.txt.
// This is an altered (translated) version of the original software.

//! FLAC audio decoder. Translation of dr_flac, in the configuration
//! SDL_mixer builds it with (`DR_FLAC_NO_STDIO`), as an x86-64 build runs
//! it.
//!
//! dr_flac has SSE2, SSE4.1 and NEON versions of some of its loops. The
//! integer decoding is the same in all of them (the scalar code is
//! translated); converting to floats, the SSE2 versions (which upstream's
//! x86 builds use for streams of up to 24 bits per sample) work on samples
//! scaled to 24 bits instead of 32, which makes a difference only for
//! corrupt data, and that's how it's done here too. Where C shifts by the
//! width of a value or more (undefined, and only reachable with corrupt
//! data), this does what x86 does: shift by the count modulo the width.
//!
//! Only what SDL_mixer uses is translated: opening a stream (native FLAC or
//! Ogg FLAC) over an [`IoStream`], with or without a metadata callback,
//! reading frames as `f32` and seeking. Not translated: the memory and
//! stdio initializers, the "relaxed" opening of streams without a
//! STREAMINFO block, the `s16`/`s32` output, the
//! `drflac_open_*_and_read_*` helpers, the custom allocators and the
//! cuesheet track iterator. The metadata callback only gets the
//! VORBIS_COMMENT blocks' data (SDL_mixer's only looks at those), but the
//! other blocks are read (and checked) as upstream reads them when there is
//! a callback.

#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::collapsible_else_if)]

use sdl3::io::{IoStream, IoWhence};

use crate::internal::try_alloc;

const DR_FLAC_BUFFER_SIZE: usize = 4096;

/* Result Codes */
type DrflacResult = i32;
const DRFLAC_SUCCESS: DrflacResult = 0;
const DRFLAC_ERROR: DrflacResult = -1; /* A generic error. */
const DRFLAC_AT_END: DrflacResult = -53;
const DRFLAC_CRC_MISMATCH: DrflacResult = -100;
/* End Result Codes */

const DRFLAC_METADATA_BLOCK_TYPE_STREAMINFO: u8 = 0;
const DRFLAC_METADATA_BLOCK_TYPE_PADDING: u8 = 1;
const DRFLAC_METADATA_BLOCK_TYPE_APPLICATION: u8 = 2;
const DRFLAC_METADATA_BLOCK_TYPE_SEEKTABLE: u8 = 3;
pub(crate) const DRFLAC_METADATA_BLOCK_TYPE_VORBIS_COMMENT: u8 = 4;
const DRFLAC_METADATA_BLOCK_TYPE_CUESHEET: u8 = 5;
const DRFLAC_METADATA_BLOCK_TYPE_PICTURE: u8 = 6;
const DRFLAC_METADATA_BLOCK_TYPE_INVALID: u8 = 127;

const DRFLAC_MAX_SIMD_VECTOR_SIZE: u32 = 64; /* 64 for AVX-512 in the future. */

const DRFLAC_SUBFRAME_CONSTANT: u8 = 0;
const DRFLAC_SUBFRAME_VERBATIM: u8 = 1;
const DRFLAC_SUBFRAME_FIXED: u8 = 8;
const DRFLAC_SUBFRAME_LPC: u8 = 32;
const DRFLAC_SUBFRAME_RESERVED: u8 = 255;

const DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE: u8 = 0;
const DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE2: u8 = 1;

#[allow(dead_code)] // (the default case.)
const DRFLAC_CHANNEL_ASSIGNMENT_INDEPENDENT: u8 = 0;
const DRFLAC_CHANNEL_ASSIGNMENT_LEFT_SIDE: u8 = 8;
const DRFLAC_CHANNEL_ASSIGNMENT_RIGHT_SIDE: u8 = 9;
const DRFLAC_CHANNEL_ASSIGNMENT_MID_SIDE: u8 = 10;

const DRFLAC_SEEKPOINT_SIZE_IN_BYTES: u32 = 18;
const DRFLAC_CUESHEET_TRACK_SIZE_IN_BYTES: i64 = 36;
const DRFLAC_CUESHEET_TRACK_INDEX_SIZE_IN_BYTES: u32 = 12;

/// Translation of `drflac_container`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Container {
    #[default]
    Native,
    Ogg,
}

/// Translation of `drflac_seek_origin`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SeekOrigin {
    Set,
    Cur,
    End,
}

/// Translation of `drflac_seekpoint`.
#[derive(Clone, Copy, Default, Debug)]
struct Seekpoint {
    first_pcm_frame: u64,
    flac_frame_offset: u64, /* The offset from the first byte of the header of the first frame. */
    pcm_frame_count: u16,
}

/// Translation of `drflac_streaminfo`.
#[derive(Clone, Copy, Default, Debug)]
struct Streaminfo {
    min_block_size_in_pcm_frames: u16,
    max_block_size_in_pcm_frames: u16,
    min_frame_size_in_pcm_frames: u32,
    max_frame_size_in_pcm_frames: u32,
    sample_rate: u32,
    channels: u8,
    bits_per_sample: u8,
    total_pcm_frame_count: u64,
    md5: [u8; 16],
}

/// What the metadata callback gets (a reduced `drflac_metadata`): a
/// VORBIS_COMMENT block's vendor string and comments.
pub(crate) struct VorbisComment<'b> {
    pub vendor: &'b [u8],
    pub comment_count: u32,
    pub comments: &'b [u8],
}

/// The metadata callback (`drflac_meta_proc`): the block's type, and the
/// data of a VORBIS_COMMENT block.
pub(crate) type MetaProc<'m> = dyn FnMut(u8, Option<&VorbisComment<'_>>) + 'm;

// drflac -> SDL_IOStream bridge (SDL_mixer's callbacks)...

/// SDL_mixer's `DRFLAC_IoRead()`.
fn io_read(io: &mut IoStream<'_>, buf: &mut [u8]) -> usize {
    io.read(buf)
}

/// SDL_mixer's `DRFLAC_IoSeek()`.
fn io_seek(io: &mut IoStream<'_>, offset: i32, origin: SeekOrigin) -> bool {
    // SDL_IOWhence and drflac_seek_origin happen to match up.
    let whence = match origin {
        SeekOrigin::Set => IoWhence::Set,
        SeekOrigin::Cur => IoWhence::Cur,
        SeekOrigin::End => IoWhence::End,
    };
    io.seek(offset as i64, whence).is_ok()
}

/// SDL_mixer's `DRFLAC_IoTell()`.
fn io_tell(io: &mut IoStream<'_>, pos: &mut i64) -> bool {
    *pos = io.tell().unwrap_or(-1);
    *pos >= 0
}

/// The `onRead`/`onSeek`/`onTell` callbacks and their user data: the
/// stream, or the Ogg layer over it.
trait Callbacks {
    fn on_read(&mut self, buf: &mut [u8]) -> usize;
    fn on_seek(&mut self, offset: i32, origin: SeekOrigin) -> bool;
    fn on_tell(&mut self, cursor: &mut i64) -> bool;
}

impl Callbacks for IoStream<'_> {
    fn on_read(&mut self, buf: &mut [u8]) -> usize {
        io_read(self, buf)
    }
    fn on_seek(&mut self, offset: i32, origin: SeekOrigin) -> bool {
        io_seek(self, offset, origin)
    }
    fn on_tell(&mut self, cursor: &mut i64) -> bool {
        io_tell(self, cursor)
    }
}

/// Where the bit streamer's data comes from.
enum Source<'a> {
    Native(IoStream<'a>),
    Ogg(Box<OggBs<'a>>),
}

impl Callbacks for Source<'_> {
    fn on_read(&mut self, buf: &mut [u8]) -> usize {
        match self {
            Source::Native(io) => io.on_read(buf),
            Source::Ogg(oggbs) => oggbs.on_read(buf),
        }
    }
    fn on_seek(&mut self, offset: i32, origin: SeekOrigin) -> bool {
        match self {
            Source::Native(io) => io.on_seek(offset, origin),
            Source::Ogg(oggbs) => oggbs.on_seek(offset, origin),
        }
    }
    fn on_tell(&mut self, cursor: &mut i64) -> bool {
        match self {
            Source::Native(io) => io.on_tell(cursor),
            Source::Ogg(oggbs) => oggbs.on_tell(cursor),
        }
    }
}

/// Translation of `drflac__unsynchsafe_32()`.
fn unsynchsafe_32(n: u32) -> u32 {
    let mut result = 0u32;
    result |= (n & 0x7F000000) >> 3;
    result |= (n & 0x007F0000) >> 2;
    result |= (n & 0x00007F00) >> 1;
    result |= n & 0x0000007F;

    result
}

/// x86's conversion of a double to an unsigned 64-bit integer (as gcc
/// emits it), for the C casts that are out of range with corrupt data.
fn f64_to_u64(f: f64) -> u64 {
    const TWO63: f64 = 9223372036854775808.0;
    fn cvtt(f: f64) -> i64 {
        if f.is_nan() || !(-TWO63..TWO63).contains(&f) {
            i64::MIN
        } else {
            f as i64
        }
    }
    if !(f >= TWO63) {
        cvtt(f) as u64
    } else {
        (cvtt(f - TWO63) as u64) ^ (1 << 63)
    }
}

/* The CRC code below is based on this document: http://zlib.net/crc_v3.txt */
static DRFLAC_CRC8_TABLE: [u8; 256] = [
    0x00, 0x07, 0x0E, 0x09, 0x1C, 0x1B, 0x12, 0x15, 0x38, 0x3F, 0x36, 0x31, 0x24, 0x23, 0x2A, 0x2D,
    0x70, 0x77, 0x7E, 0x79, 0x6C, 0x6B, 0x62, 0x65, 0x48, 0x4F, 0x46, 0x41, 0x54, 0x53, 0x5A, 0x5D,
    0xE0, 0xE7, 0xEE, 0xE9, 0xFC, 0xFB, 0xF2, 0xF5, 0xD8, 0xDF, 0xD6, 0xD1, 0xC4, 0xC3, 0xCA, 0xCD,
    0x90, 0x97, 0x9E, 0x99, 0x8C, 0x8B, 0x82, 0x85, 0xA8, 0xAF, 0xA6, 0xA1, 0xB4, 0xB3, 0xBA, 0xBD,
    0xC7, 0xC0, 0xC9, 0xCE, 0xDB, 0xDC, 0xD5, 0xD2, 0xFF, 0xF8, 0xF1, 0xF6, 0xE3, 0xE4, 0xED, 0xEA,
    0xB7, 0xB0, 0xB9, 0xBE, 0xAB, 0xAC, 0xA5, 0xA2, 0x8F, 0x88, 0x81, 0x86, 0x93, 0x94, 0x9D, 0x9A,
    0x27, 0x20, 0x29, 0x2E, 0x3B, 0x3C, 0x35, 0x32, 0x1F, 0x18, 0x11, 0x16, 0x03, 0x04, 0x0D, 0x0A,
    0x57, 0x50, 0x59, 0x5E, 0x4B, 0x4C, 0x45, 0x42, 0x6F, 0x68, 0x61, 0x66, 0x73, 0x74, 0x7D, 0x7A,
    0x89, 0x8E, 0x87, 0x80, 0x95, 0x92, 0x9B, 0x9C, 0xB1, 0xB6, 0xBF, 0xB8, 0xAD, 0xAA, 0xA3, 0xA4,
    0xF9, 0xFE, 0xF7, 0xF0, 0xE5, 0xE2, 0xEB, 0xEC, 0xC1, 0xC6, 0xCF, 0xC8, 0xDD, 0xDA, 0xD3, 0xD4,
    0x69, 0x6E, 0x67, 0x60, 0x75, 0x72, 0x7B, 0x7C, 0x51, 0x56, 0x5F, 0x58, 0x4D, 0x4A, 0x43, 0x44,
    0x19, 0x1E, 0x17, 0x10, 0x05, 0x02, 0x0B, 0x0C, 0x21, 0x26, 0x2F, 0x28, 0x3D, 0x3A, 0x33, 0x34,
    0x4E, 0x49, 0x40, 0x47, 0x52, 0x55, 0x5C, 0x5B, 0x76, 0x71, 0x78, 0x7F, 0x6A, 0x6D, 0x64, 0x63,
    0x3E, 0x39, 0x30, 0x37, 0x22, 0x25, 0x2C, 0x2B, 0x06, 0x01, 0x08, 0x0F, 0x1A, 0x1D, 0x14, 0x13,
    0xAE, 0xA9, 0xA0, 0xA7, 0xB2, 0xB5, 0xBC, 0xBB, 0x96, 0x91, 0x98, 0x9F, 0x8A, 0x8D, 0x84, 0x83,
    0xDE, 0xD9, 0xD0, 0xD7, 0xC2, 0xC5, 0xCC, 0xCB, 0xE6, 0xE1, 0xE8, 0xEF, 0xFA, 0xFD, 0xF4, 0xF3,
];

static DRFLAC_CRC16_TABLE: [u16; 256] = [
    0x0000, 0x8005, 0x800F, 0x000A, 0x801B, 0x001E, 0x0014, 0x8011, 0x8033, 0x0036, 0x003C, 0x8039,
    0x0028, 0x802D, 0x8027, 0x0022, 0x8063, 0x0066, 0x006C, 0x8069, 0x0078, 0x807D, 0x8077, 0x0072,
    0x0050, 0x8055, 0x805F, 0x005A, 0x804B, 0x004E, 0x0044, 0x8041, 0x80C3, 0x00C6, 0x00CC, 0x80C9,
    0x00D8, 0x80DD, 0x80D7, 0x00D2, 0x00F0, 0x80F5, 0x80FF, 0x00FA, 0x80EB, 0x00EE, 0x00E4, 0x80E1,
    0x00A0, 0x80A5, 0x80AF, 0x00AA, 0x80BB, 0x00BE, 0x00B4, 0x80B1, 0x8093, 0x0096, 0x009C, 0x8099,
    0x0088, 0x808D, 0x8087, 0x0082, 0x8183, 0x0186, 0x018C, 0x8189, 0x0198, 0x819D, 0x8197, 0x0192,
    0x01B0, 0x81B5, 0x81BF, 0x01BA, 0x81AB, 0x01AE, 0x01A4, 0x81A1, 0x01E0, 0x81E5, 0x81EF, 0x01EA,
    0x81FB, 0x01FE, 0x01F4, 0x81F1, 0x81D3, 0x01D6, 0x01DC, 0x81D9, 0x01C8, 0x81CD, 0x81C7, 0x01C2,
    0x0140, 0x8145, 0x814F, 0x014A, 0x815B, 0x015E, 0x0154, 0x8151, 0x8173, 0x0176, 0x017C, 0x8179,
    0x0168, 0x816D, 0x8167, 0x0162, 0x8123, 0x0126, 0x012C, 0x8129, 0x0138, 0x813D, 0x8137, 0x0132,
    0x0110, 0x8115, 0x811F, 0x011A, 0x810B, 0x010E, 0x0104, 0x8101, 0x8303, 0x0306, 0x030C, 0x8309,
    0x0318, 0x831D, 0x8317, 0x0312, 0x0330, 0x8335, 0x833F, 0x033A, 0x832B, 0x032E, 0x0324, 0x8321,
    0x0360, 0x8365, 0x836F, 0x036A, 0x837B, 0x037E, 0x0374, 0x8371, 0x8353, 0x0356, 0x035C, 0x8359,
    0x0348, 0x834D, 0x8347, 0x0342, 0x03C0, 0x83C5, 0x83CF, 0x03CA, 0x83DB, 0x03DE, 0x03D4, 0x83D1,
    0x83F3, 0x03F6, 0x03FC, 0x83F9, 0x03E8, 0x83ED, 0x83E7, 0x03E2, 0x83A3, 0x03A6, 0x03AC, 0x83A9,
    0x03B8, 0x83BD, 0x83B7, 0x03B2, 0x0390, 0x8395, 0x839F, 0x039A, 0x838B, 0x038E, 0x0384, 0x8381,
    0x0280, 0x8285, 0x828F, 0x028A, 0x829B, 0x029E, 0x0294, 0x8291, 0x82B3, 0x02B6, 0x02BC, 0x82B9,
    0x02A8, 0x82AD, 0x82A7, 0x02A2, 0x82E3, 0x02E6, 0x02EC, 0x82E9, 0x02F8, 0x82FD, 0x82F7, 0x02F2,
    0x02D0, 0x82D5, 0x82DF, 0x02DA, 0x82CB, 0x02CE, 0x02C4, 0x82C1, 0x8243, 0x0246, 0x024C, 0x8249,
    0x0258, 0x825D, 0x8257, 0x0252, 0x0270, 0x8275, 0x827F, 0x027A, 0x826B, 0x026E, 0x0264, 0x8261,
    0x0220, 0x8225, 0x822F, 0x022A, 0x823B, 0x023E, 0x0234, 0x8231, 0x8213, 0x0216, 0x021C, 0x8219,
    0x0208, 0x820D, 0x8207, 0x0202,
];

/// Translation of `drflac_crc8_byte()`.
fn crc8_byte(crc: u8, data: u8) -> u8 {
    DRFLAC_CRC8_TABLE[(crc ^ data) as usize]
}

/// Translation of `drflac_crc8()`.
fn crc8(mut crc: u8, data: u32, count: u32) -> u8 {
    static LEFTOVER_DATA_MASK_TABLE: [u64; 8] = [0x00, 0x01, 0x03, 0x07, 0x0F, 0x1F, 0x3F, 0x7F];

    debug_assert!(count <= 32);

    let whole_bytes = count >> 3;
    let leftover_bits = count - (whole_bytes * 8);
    let leftover_data_mask = LEFTOVER_DATA_MASK_TABLE[leftover_bits as usize];

    // (a switch whose cases fall through.)
    if whole_bytes >= 4 {
        crc = crc8_byte(
            crc,
            ((data & (0xFF000000u32 << leftover_bits)) >> (24 + leftover_bits)) as u8,
        );
    }
    if whole_bytes >= 3 {
        crc = crc8_byte(
            crc,
            ((data & (0x00FF0000u32 << leftover_bits)) >> (16 + leftover_bits)) as u8,
        );
    }
    if whole_bytes >= 2 {
        crc = crc8_byte(
            crc,
            ((data & (0x0000FF00u32 << leftover_bits)) >> (8 + leftover_bits)) as u8,
        );
    }
    if whole_bytes >= 1 {
        crc = crc8_byte(
            crc,
            ((data & (0x000000FFu32 << leftover_bits)) >> leftover_bits) as u8,
        );
    }
    if leftover_bits > 0 {
        crc = (((crc as u32) << leftover_bits)
            ^ DRFLAC_CRC8_TABLE[(((crc as u32) >> (8 - leftover_bits)) as u64
                ^ (data as u64 & leftover_data_mask)) as usize] as u32) as u8;
    }
    crc
}

/// Translation of `drflac_crc16_byte()`.
fn crc16_byte(crc: u16, data: u8) -> u16 {
    (crc << 8) ^ DRFLAC_CRC16_TABLE[((crc >> 8) as u8 ^ data) as usize]
}

/// Translation of `drflac_crc16_cache()`.
fn crc16_cache(mut crc: u16, data: u64) -> u16 {
    crc = crc16_byte(crc, ((data >> 56) & 0xFF) as u8);
    crc = crc16_byte(crc, ((data >> 48) & 0xFF) as u8);
    crc = crc16_byte(crc, ((data >> 40) & 0xFF) as u8);
    crc = crc16_byte(crc, ((data >> 32) & 0xFF) as u8);
    crc = crc16_byte(crc, ((data >> 24) & 0xFF) as u8);
    crc = crc16_byte(crc, ((data >> 16) & 0xFF) as u8);
    crc = crc16_byte(crc, ((data >> 8) & 0xFF) as u8);
    crc = crc16_byte(crc, (data & 0xFF) as u8);

    crc
}

/// Translation of `drflac_crc16_bytes()`.
fn crc16_bytes(mut crc: u16, data: u64, byte_count: u32) -> u16 {
    // (a switch whose cases fall through; other counts change nothing.)
    if !(1..=8).contains(&byte_count) {
        return crc;
    }
    for i in (0..byte_count).rev() {
        crc = crc16_byte(crc, ((data >> (i * 8)) & 0xFF) as u8);
    }

    crc
}

/*
BIT READING ATTEMPT #2

This uses a 32- or 64-bit bit-shifted cache - as bits are read, the cache is shifted such that the first valid bit is sitting
on the most significant bit. It uses the notion of an L1 and L2 cache (borrowed from CPU architecture), where the L1 cache
is a 32- or 64-bit unsigned integer (depending on whether or not a 32- or 64-bit build is being compiled) and the L2 is an
array of "cache lines", with each cache line being the same size as the L1. The L2 is a buffer of about 4KB and is where data
from onRead() is read into.
*/
// (This is the 64-bit build's, `DRFLAC_64BIT`.)
const DRFLAC_CACHE_L1_SIZE_BYTES: u32 = 8;
const DRFLAC_CACHE_L1_SIZE_BITS: u32 = 64;
const DRFLAC_CACHE_L2_SIZE_BYTES: usize = DR_FLAC_BUFFER_SIZE;
const DRFLAC_CACHE_L2_LINE_COUNT: u32 = (DRFLAC_CACHE_L2_SIZE_BYTES / 8) as u32;

/// `DRFLAC_CACHE_L1_SELECTION_MASK()`
fn cache_l1_selection_mask(bit_count: u32) -> u64 {
    !((!0u64).wrapping_shr(bit_count))
}

/// `DRFLAC_CACHE_L1_SELECTION_SHIFT()`
fn cache_l1_selection_shift(bit_count: u32) -> u32 {
    DRFLAC_CACHE_L1_SIZE_BITS.wrapping_sub(bit_count)
}

/// Translation of `drflac_bs`, the bit streamer.
struct Bs<'a> {
    /* The callbacks and their user data. */
    src: Source<'a>,

    /*
    The number of unaligned bytes in the L2 cache. This will always be 0 until the end of the stream is hit. At the end of the
    stream there will be a number of bytes that don't cleanly fit in an L1 cache line, so we use this variable to know whether
    or not the bistreamer needs to run on a slower path to read those last bytes. This will never be more than sizeof(drflac_cache_t).
    */
    unaligned_byte_count: usize,

    /* The content of the unaligned bytes. */
    unaligned_cache: u64,

    /* The index of the next valid cache line in the "L2" cache. */
    next_l2_line: u32,

    /* The number of bits that have been consumed by the cache. This is used to determine how many valid bits are remaining. */
    consumed_bits: u32,

    /*
    The cached data which was most recently read from the client. There are two levels of cache. Data flows as such:
    Client -> L2 -> L1. The L2 -> L1 movement is aligned and runs on a fast path in just a few instructions.
    */
    cache_l2: Box<[u8; DRFLAC_CACHE_L2_SIZE_BYTES]>,
    cache: u64,

    /*
    CRC-16. This is updated whenever bits are read from the bit stream. Manually set this to 0 to reset the CRC. For FLAC, this
    is reset to 0 at the beginning of each frame.
    */
    crc16: u16,
    crc16_cache: u64, /* A cache for optimizing CRC calculations. This is filled when when the L1 cache is reloaded. */
    crc16_cache_ignored_bytes: u32, /* The number of bytes to ignore when updating the CRC-16 from the CRC-16 cache. */
}

impl<'a> Bs<'a> {
    fn new(src: Source<'a>) -> Bs<'a> {
        let mut bs = Bs {
            src,
            unaligned_byte_count: 0,
            unaligned_cache: 0,
            next_l2_line: 0,
            consumed_bits: 0,
            cache_l2: Box::new([0; DRFLAC_CACHE_L2_SIZE_BYTES]),
            cache: 0,
            crc16: 0,
            crc16_cache: 0,
            crc16_cache_ignored_bytes: 0,
        };
        bs.reset_cache();
        bs
    }

    /// `DRFLAC_CACHE_L1_BITS_REMAINING()`
    fn cache_l1_bits_remaining(&self) -> u32 {
        DRFLAC_CACHE_L1_SIZE_BITS.wrapping_sub(self.consumed_bits)
    }

    /// `DRFLAC_CACHE_L1_SELECT_AND_SHIFT()`
    fn cache_l1_select_and_shift(&self, bit_count: u32) -> u64 {
        (self.cache & cache_l1_selection_mask(bit_count))
            .wrapping_shr(cache_l1_selection_shift(bit_count))
    }

    /// `cacheL2[line]`, read as the 64-bit integer it is in memory.
    fn l2_line(&self, line: u32) -> u64 {
        let at = line as usize * 8;
        u64::from_ne_bytes(self.cache_l2[at..at + 8].try_into().unwrap())
    }

    /// `drflac__be2host__cache_line(cacheL2[line])`
    fn l2_line_be(&self, line: u32) -> u64 {
        u64::from_be(self.l2_line(line))
    }

    /// Translation of `drflac__reset_crc16()`.
    fn reset_crc16(&mut self) {
        self.crc16 = 0;
        self.crc16_cache_ignored_bytes = self.consumed_bits >> 3;
    }

    /// Translation of `drflac__update_crc16()`.
    fn update_crc16(&mut self) {
        if self.crc16_cache_ignored_bytes == 0 {
            self.crc16 = crc16_cache(self.crc16, self.crc16_cache);
        } else {
            self.crc16 = crc16_bytes(
                self.crc16,
                self.crc16_cache,
                DRFLAC_CACHE_L1_SIZE_BYTES.wrapping_sub(self.crc16_cache_ignored_bytes),
            );
            self.crc16_cache_ignored_bytes = 0;
        }
    }

    /// Translation of `drflac__flush_crc16()`.
    fn flush_crc16(&mut self) -> u16 {
        /* We should never be flushing in a situation where we are not aligned on a byte boundary. */
        debug_assert!((self.cache_l1_bits_remaining() & 7) == 0);

        /*
        The bits that were read from the L1 cache need to be accumulated. The number of bytes needing to be accumulated is determined
        by the number of bits that have been consumed.
        */
        if self.cache_l1_bits_remaining() == 0 {
            self.update_crc16();
        } else {
            /* We only accumulate the consumed bits. */
            self.crc16 = crc16_bytes(
                self.crc16,
                self.crc16_cache
                    .wrapping_shr(self.cache_l1_bits_remaining()),
                (self.consumed_bits >> 3).wrapping_sub(self.crc16_cache_ignored_bytes),
            );

            /*
            The bits that we just accumulated should never be accumulated again. We need to keep track of how many bytes were accumulated
            so we can handle that later.
            */
            self.crc16_cache_ignored_bytes = self.consumed_bits >> 3;
        }

        self.crc16
    }

    /// Translation of `drflac__reload_l1_cache_from_l2()`.
    fn reload_l1_cache_from_l2(&mut self) -> bool {
        /* Fast path. Try loading straight from L2. */
        if self.next_l2_line < DRFLAC_CACHE_L2_LINE_COUNT {
            self.cache = self.l2_line(self.next_l2_line);
            self.next_l2_line += 1;
            return true;
        }

        /*
        If we get here it means we've run out of data in the L2 cache. We'll need to fetch more from the client, if there's
        any left.
        */
        if self.unaligned_byte_count > 0 {
            return false; /* If we have any unaligned bytes it means there's no more aligned bytes left in the client. */
        }

        let bytes_read = self.src.on_read(&mut self.cache_l2[..]);

        self.next_l2_line = 0;
        if bytes_read == DRFLAC_CACHE_L2_SIZE_BYTES {
            self.cache = self.l2_line(self.next_l2_line);
            self.next_l2_line += 1;
            return true;
        }

        /*
        If we get here it means we were unable to retrieve enough data to fill the entire L2 cache. It probably
        means we've just reached the end of the file. We need to move the valid data down to the end of the buffer
        and adjust the index of the next line accordingly. Also keep in mind that the L2 cache must be aligned to
        the size of the L1 so we'll need to seek backwards by any misaligned bytes.
        */
        let aligned_l1_line_count = bytes_read / DRFLAC_CACHE_L1_SIZE_BYTES as usize;

        /* We need to keep track of any unaligned bytes for later use. */
        self.unaligned_byte_count =
            bytes_read - (aligned_l1_line_count * DRFLAC_CACHE_L1_SIZE_BYTES as usize);
        if self.unaligned_byte_count > 0 {
            self.unaligned_cache = self.l2_line(aligned_l1_line_count as u32);
        }

        if aligned_l1_line_count > 0 {
            let offset = DRFLAC_CACHE_L2_LINE_COUNT as usize - aligned_l1_line_count;
            // (cacheL2[i-1 + offset] = cacheL2[i-1], from the last line down.)
            self.cache_l2
                .copy_within(0..aligned_l1_line_count * 8, offset * 8);

            self.next_l2_line = offset as u32;
            self.cache = self.l2_line(self.next_l2_line);
            self.next_l2_line += 1;
            true
        } else {
            /* If we get into this branch it means we weren't able to load any L1-aligned data. */
            self.next_l2_line = DRFLAC_CACHE_L2_LINE_COUNT;
            false
        }
    }

    /// Translation of `drflac__reload_cache()`.
    fn reload_cache(&mut self) -> bool {
        self.update_crc16();

        /* Fast path. Try just moving the next value in the L2 cache to the L1 cache. */
        if self.reload_l1_cache_from_l2() {
            self.cache = u64::from_be(self.cache);
            self.consumed_bits = 0;
            self.crc16_cache = self.cache;
            return true;
        }

        /* Slow path. */

        /*
        If we get here it means we have failed to load the L1 cache from the L2. Likely we've just reached the end of the stream and the last
        few bytes did not meet the alignment requirements for the L2 cache. In this case we need to fall back to a slower path and read the
        data from the unaligned cache.
        */
        let bytes_read = self.unaligned_byte_count;
        if bytes_read == 0 {
            self.consumed_bits = DRFLAC_CACHE_L1_SIZE_BITS; /* <-- The stream has been exhausted, so marked the bits as consumed. */
            return false;
        }

        debug_assert!(bytes_read < DRFLAC_CACHE_L1_SIZE_BYTES as usize);
        self.consumed_bits = (DRFLAC_CACHE_L1_SIZE_BYTES as usize - bytes_read) as u32 * 8;

        self.cache = u64::from_be(self.unaligned_cache);
        self.cache &= cache_l1_selection_mask(self.cache_l1_bits_remaining()); /* <-- Make sure the consumed bits are always set to zero. Other parts of the library depend on this property. */
        self.unaligned_byte_count = 0; /* <-- At this point the unaligned bytes have been moved into the cache and we thus have no more unaligned bytes. */

        self.crc16_cache = self.cache.wrapping_shr(self.consumed_bits);
        self.crc16_cache_ignored_bytes = self.consumed_bits >> 3;
        true
    }

    /// Translation of `drflac__reset_cache()`.
    fn reset_cache(&mut self) {
        self.next_l2_line = DRFLAC_CACHE_L2_LINE_COUNT; /* <-- This clears the L2 cache. */
        self.consumed_bits = DRFLAC_CACHE_L1_SIZE_BITS; /* <-- This clears the L1 cache. */
        self.cache = 0;
        self.unaligned_byte_count = 0; /* <-- This clears the trailing unaligned bytes. */
        self.unaligned_cache = 0;

        self.crc16_cache = 0;
        self.crc16_cache_ignored_bytes = 0;
    }

    /// Translation of `drflac__read_uint32()`.
    fn read_uint32(&mut self, bit_count: u32, result_out: &mut u32) -> bool {
        debug_assert!(bit_count > 0);
        debug_assert!(bit_count <= 32);

        if self.consumed_bits == DRFLAC_CACHE_L1_SIZE_BITS && !self.reload_cache() {
            return false;
        }

        if bit_count <= self.cache_l1_bits_remaining() {
            /*
            If we want to load all 32-bits from a 32-bit cache we need to do it slightly differently because we can't do
            a 32-bit shift on a 32-bit integer. This will never be the case on 64-bit caches, so we can have a slightly
            more optimal solution for this.
            */
            *result_out = self.cache_l1_select_and_shift(bit_count) as u32;
            self.consumed_bits += bit_count;
            self.cache = self.cache.wrapping_shl(bit_count);

            true
        } else {
            /* It straddles the cached data. It will never cover more than the next chunk. We just read the number in two parts and combine them. */
            let bit_count_hi = self.cache_l1_bits_remaining();
            let bit_count_lo = bit_count - bit_count_hi;

            debug_assert!(bit_count_hi > 0);
            debug_assert!(bit_count_hi < 32);
            let result_hi = self.cache_l1_select_and_shift(bit_count_hi) as u32;

            if !self.reload_cache() {
                return false;
            }
            if bit_count_lo > self.cache_l1_bits_remaining() {
                /* This happens when we get to end of stream */
                return false;
            }

            *result_out = result_hi.wrapping_shl(bit_count_lo)
                | self.cache_l1_select_and_shift(bit_count_lo) as u32;
            self.consumed_bits += bit_count_lo;
            self.cache = self.cache.wrapping_shl(bit_count_lo);
            true
        }
    }

    /// Translation of `drflac__read_int32()`.
    fn read_int32(&mut self, bit_count: u32, result_out: &mut i32) -> bool {
        debug_assert!(bit_count > 0);
        debug_assert!(bit_count <= 32);

        let mut result = 0u32;
        if !self.read_uint32(bit_count, &mut result) {
            return false;
        }

        /* Do not attempt to shift by 32 as it's undefined. */
        if bit_count < 32 {
            let signbit = (result >> (bit_count - 1)) & 0x01;
            result |= (!signbit).wrapping_add(1) << bit_count;
        }

        *result_out = result as i32;
        true
    }

    /// Translation of `drflac__read_uint64()`.
    fn read_uint64(&mut self, bit_count: u32, result_out: &mut u64) -> bool {
        debug_assert!(bit_count <= 64);
        debug_assert!(bit_count > 32);

        let mut result_hi = 0u32;
        let mut result_lo = 0u32;
        if !self.read_uint32(bit_count - 32, &mut result_hi) {
            return false;
        }

        if !self.read_uint32(32, &mut result_lo) {
            return false;
        }

        *result_out = ((result_hi as u64) << 32) | (result_lo as u64);
        true
    }

    /// Translation of `drflac__read_uint16()`.
    fn read_uint16(&mut self, bit_count: u32, result_out: &mut u16) -> bool {
        debug_assert!(bit_count > 0);
        debug_assert!(bit_count <= 16);

        let mut result = 0u32;
        if !self.read_uint32(bit_count, &mut result) {
            return false;
        }

        *result_out = result as u16;
        true
    }

    /// Translation of `drflac__read_uint8()`.
    fn read_uint8(&mut self, bit_count: u32, result_out: &mut u8) -> bool {
        debug_assert!(bit_count > 0);
        debug_assert!(bit_count <= 8);

        let mut result = 0u32;
        if !self.read_uint32(bit_count, &mut result) {
            return false;
        }

        *result_out = result as u8;
        true
    }

    /// Translation of `drflac__read_int8()`.
    fn read_int8(&mut self, bit_count: u32, result_out: &mut i8) -> bool {
        debug_assert!(bit_count > 0);
        debug_assert!(bit_count <= 8);

        let mut result = 0i32;
        if !self.read_int32(bit_count, &mut result) {
            return false;
        }

        *result_out = result as i8;
        true
    }

    /// Translation of `drflac__seek_bits()`.
    fn seek_bits(&mut self, mut bits_to_seek: usize) -> bool {
        if bits_to_seek <= self.cache_l1_bits_remaining() as usize {
            self.consumed_bits = self.consumed_bits.wrapping_add(bits_to_seek as u32);
            self.cache = self.cache.wrapping_shl(bits_to_seek as u32);
            true
        } else {
            /* It straddles the cached data. This function isn't called too frequently so I'm favouring simplicity here. */
            bits_to_seek -= self.cache_l1_bits_remaining() as usize;
            self.consumed_bits = self
                .consumed_bits
                .wrapping_add(self.cache_l1_bits_remaining());
            self.cache = 0;

            /* Simple case. Seek in groups of the same number as bits that fit within a cache line. */
            while bits_to_seek >= DRFLAC_CACHE_L1_SIZE_BITS as usize {
                let mut bin = 0u64;
                if !self.read_uint64(DRFLAC_CACHE_L1_SIZE_BITS, &mut bin) {
                    return false;
                }
                bits_to_seek -= DRFLAC_CACHE_L1_SIZE_BITS as usize;
            }

            /* Whole leftover bytes. */
            while bits_to_seek >= 8 {
                let mut bin = 0u8;
                if !self.read_uint8(8, &mut bin) {
                    return false;
                }
                bits_to_seek -= 8;
            }

            /* Leftover bits. */
            if bits_to_seek > 0 {
                let mut bin = 0u8;
                if !self.read_uint8(bits_to_seek as u32, &mut bin) {
                    return false;
                }
            }

            true
        }
    }

    /* This function moves the bit streamer to the first bit after the sync code (bit 15 of the of the frame header). It will also update the CRC-16. */
    /// Translation of `drflac__find_and_seek_to_next_sync_code()`.
    fn find_and_seek_to_next_sync_code(&mut self) -> bool {
        /*
        The sync code is always aligned to 8 bits. This is convenient for us because it means we can do byte-aligned movements. The first
        thing to do is align to the next byte.
        */
        if !self.seek_bits((self.cache_l1_bits_remaining() & 7) as usize) {
            return false;
        }

        loop {
            self.reset_crc16();

            let mut hi = 0u8;
            if !self.read_uint8(8, &mut hi) {
                return false;
            }

            if hi == 0xFF {
                let mut lo = 0u8;
                if !self.read_uint8(6, &mut lo) {
                    return false;
                }

                if lo == 0x3E {
                    return true;
                } else if !self.seek_bits((self.cache_l1_bits_remaining() & 7) as usize) {
                    return false;
                }
            }
        }
    }

    /// Translation of `drflac__seek_past_next_set_bit()`.
    fn seek_past_next_set_bit(&mut self, offset_out: &mut u32) -> bool {
        let mut zero_counter: u32 = 0;

        while self.cache == 0 {
            zero_counter = zero_counter.wrapping_add(self.cache_l1_bits_remaining());
            if !self.reload_cache() {
                return false;
            }
        }

        if self.cache == 1 {
            /* Not catching this would lead to undefined behaviour: a shift of a 32-bit number by 32 or more is undefined */
            *offset_out = zero_counter
                .wrapping_add(self.cache_l1_bits_remaining())
                .wrapping_sub(1);
            if !self.reload_cache() {
                return false;
            }

            return true;
        }

        let set_bit_offset_plus1 = clz(self.cache) + 1;

        if set_bit_offset_plus1 > self.cache_l1_bits_remaining() {
            /* This happens when we get to end of stream */
            return false;
        }

        self.consumed_bits += set_bit_offset_plus1;
        self.cache = self.cache.wrapping_shl(set_bit_offset_plus1);

        *offset_out = zero_counter.wrapping_add(set_bit_offset_plus1) - 1;
        true
    }

    /// Translation of `drflac__seek_to_byte()`.
    fn seek_to_byte(&mut self, offset_from_start: u64) -> bool {
        debug_assert!(offset_from_start > 0);

        /*
        Seeking from the start is not quite as trivial as it sounds because the onSeek callback takes a signed 32-bit integer (which
        is intentional because it simplifies the implementation of the onSeek callbacks), however offsetFromStart is unsigned 64-bit.
        To resolve we just need to do an initial seek from the start, and then a series of offset seeks to make up the remainder.
        */
        if offset_from_start > 0x7FFFFFFF {
            let mut bytes_remaining = offset_from_start;
            if !self.src.on_seek(0x7FFFFFFF, SeekOrigin::Set) {
                return false;
            }
            bytes_remaining -= 0x7FFFFFFF;

            while bytes_remaining > 0x7FFFFFFF {
                if !self.src.on_seek(0x7FFFFFFF, SeekOrigin::Cur) {
                    return false;
                }
                bytes_remaining -= 0x7FFFFFFF;
            }

            if bytes_remaining > 0 && !self.src.on_seek(bytes_remaining as i32, SeekOrigin::Cur) {
                return false;
            }
        } else if !self.src.on_seek(offset_from_start as i32, SeekOrigin::Set) {
            return false;
        }

        /* The cache should be reset to force a reload of fresh data from the client. */
        self.reset_cache();
        true
    }

    /// Translation of `drflac__read_utf8_coded_number()`.
    fn read_utf8_coded_number(&mut self, number_out: &mut u64, crc_out: &mut u8) -> DrflacResult {
        let mut utf8 = [0u8; 7];

        let mut crc = *crc_out;

        if !self.read_uint8(8, &mut utf8[0]) {
            *number_out = 0;
            return DRFLAC_AT_END;
        }
        crc = crc8(crc, utf8[0] as u32, 8);

        if (utf8[0] & 0x80) == 0 {
            *number_out = utf8[0] as u64;
            *crc_out = crc;
            return DRFLAC_SUCCESS;
        }

        /*byteCount = 1;*/
        let byte_count: u32 = if (utf8[0] & 0xE0) == 0xC0 {
            2
        } else if (utf8[0] & 0xF0) == 0xE0 {
            3
        } else if (utf8[0] & 0xF8) == 0xF0 {
            4
        } else if (utf8[0] & 0xFC) == 0xF8 {
            5
        } else if (utf8[0] & 0xFE) == 0xFC {
            6
        } else if utf8[0] == 0xFE {
            7
        } else {
            *number_out = 0;
            return DRFLAC_CRC_MISMATCH; /* Bad UTF-8 encoding. */
        };

        /* Read extra bytes. */
        debug_assert!(byte_count > 1);

        let mut result = (utf8[0] & (0xFFu32 >> (byte_count + 1)) as u8) as u64;
        for i in 1..byte_count as usize {
            if !self.read_uint8(8, &mut utf8[i]) {
                *number_out = 0;
                return DRFLAC_AT_END;
            }
            crc = crc8(crc, utf8[i] as u32, 8);

            result = (result << 6) | (utf8[i] & 0x3F) as u64;
        }

        *number_out = result;
        *crc_out = crc;
        DRFLAC_SUCCESS
    }
}

/// Translation of `drflac__clz()`.
fn clz(x: u64) -> u32 {
    x.leading_zeros()
}

/// Translation of `drflac__ilog2_u32()`.
fn ilog2_u32(mut x: u32) -> u32 {
    let mut result = 0;
    while x > 0 {
        result += 1;
        x >>= 1;
    }

    result
}

/// Translation of `drflac__use_64_bit_prediction()`.
fn use_64_bit_prediction(bits_per_sample: u32, order: u32, precision: u32) -> bool {
    /* https://web.archive.org/web/20220205005724/https://github.com/ietf-wg-cellar/flac-specification/blob/37a49aa48ba4ba12e8757badfc59c0df35435fec/rfc_backmatter.md */
    bits_per_sample
        .wrapping_add(precision)
        .wrapping_add(ilog2_u32(order))
        > 32
}

/*
The next two functions are responsible for calculating the prediction.

When the bits per sample is >16 we need to use 64-bit integer arithmetic because otherwise we'll run out of precision. It's
safe to assume this will be slower on 32-bit platforms so we use a more optimal solution when the bits per sample is <=16.
*/
/// Translation of `drflac__calculate_prediction_32()`: the prediction for
/// `samples[at]` from the `order` samples before it.
fn calculate_prediction_32(
    order: u32,
    shift: i32,
    coefficients: &[i32],
    samples: &[i32],
    at: usize,
) -> i32 {
    debug_assert!(order <= 32);

    /* 32-bit version. */
    let mut prediction: i32 = 0;
    for k in (1..=order as usize).rev() {
        prediction = prediction.wrapping_add(coefficients[k - 1].wrapping_mul(samples[at - k]));
    }

    prediction.wrapping_shr(shift as u32)
}

/// Translation of `drflac__calculate_prediction_64()`.
fn calculate_prediction_64(
    order: u32,
    shift: i32,
    coefficients: &[i32],
    samples: &[i32],
    at: usize,
) -> i32 {
    debug_assert!(order <= 32);

    /* 64-bit version. */
    let mut prediction: i64 = 0;
    for k in (1..=order as usize).rev() {
        prediction = prediction
            .wrapping_add((coefficients[k - 1] as i64).wrapping_mul(samples[at - k] as i64));
    }

    prediction.wrapping_shr(shift as u32) as i32
}

/// Translation of `drflac__read_rice_parts_x1()`.
fn read_rice_parts_x1(
    bs: &mut Bs<'_>,
    rice_param: u8,
    zero_counter_out: &mut u32,
    rice_param_part_out: &mut u32,
) -> bool {
    let rice_param_plus1 = rice_param as u32 + 1;
    let rice_param_plus1_shift = cache_l1_selection_shift(rice_param_plus1);
    let rice_param_plus1_max_consumed_bits = DRFLAC_CACHE_L1_SIZE_BITS - rice_param_plus1;

    /*
    The idea here is to use local variables for the cache in an attempt to encourage the compiler to store them in registers. I have
    no idea how this will work in practice...
    */
    let mut bs_cache = bs.cache;
    let mut bs_consumed_bits = bs.consumed_bits;

    /* The first thing to do is find the first unset bit. Most likely a bit will be set in the current cache line. */
    let mut lzcount = clz(bs_cache);
    if lzcount < 64 {
        *zero_counter_out = lzcount;
    } else {
        /*
        Getting here means there are no bits set on the cache line. This is a less optimal case because we just wasted a call
        to drflac__clz() and we need to reload the cache.
        */
        let mut zero_counter = DRFLAC_CACHE_L1_SIZE_BITS.wrapping_sub(bs_consumed_bits);
        loop {
            if bs.next_l2_line < DRFLAC_CACHE_L2_LINE_COUNT {
                bs.update_crc16();
                bs_cache = bs.l2_line_be(bs.next_l2_line);
                bs.next_l2_line += 1;
                bs_consumed_bits = 0;
                bs.crc16_cache = bs_cache;
            } else {
                /* Slow path. We need to fetch more data from the client. */
                if !bs.reload_cache() {
                    return false;
                }

                bs_cache = bs.cache;
                bs_consumed_bits = bs.consumed_bits;
            }

            lzcount = clz(bs_cache);
            zero_counter = zero_counter.wrapping_add(lzcount);

            if lzcount < 64 {
                break;
            }
        }

        *zero_counter_out = zero_counter;
    }

    // (extract_rice_param_part:)
    /*
    It is most likely that the riceParam part (which comes after the zero counter) is also on this cache line. When extracting
    this, we include the set bit from the unary coded part because it simplifies cache management. This bit will be handled
    outside of this function at a higher level.
    */
    bs_cache = bs_cache.wrapping_shl(lzcount);
    bs_consumed_bits = bs_consumed_bits.wrapping_add(lzcount);

    if bs_consumed_bits <= rice_param_plus1_max_consumed_bits {
        /* Getting here means the rice parameter part is wholly contained within the current cache line. */
        *rice_param_part_out = bs_cache.wrapping_shr(rice_param_plus1_shift) as u32;
        bs_cache = bs_cache.wrapping_shl(rice_param_plus1);
        bs_consumed_bits += rice_param_plus1;
    } else {
        /*
        Getting here means the rice parameter part straddles the cache line. We need to read from the tail of the current cache
        line, reload the cache, and then combine it with the head of the next cache line.
        */

        /* Grab the high part of the rice parameter part. */
        let rice_param_part_hi = bs_cache.wrapping_shr(rice_param_plus1_shift) as u32;

        /* Before reloading the cache we need to grab the size in bits of the low part. */
        let rice_param_part_lo_bit_count = bs_consumed_bits - rice_param_plus1_max_consumed_bits;
        debug_assert!(rice_param_part_lo_bit_count > 0 && rice_param_part_lo_bit_count < 32);

        /* Now reload the cache. */
        if bs.next_l2_line < DRFLAC_CACHE_L2_LINE_COUNT {
            bs.update_crc16();
            bs_cache = bs.l2_line_be(bs.next_l2_line);
            bs.next_l2_line += 1;
            bs_consumed_bits = rice_param_part_lo_bit_count;
            bs.crc16_cache = bs_cache;
        } else {
            /* Slow path. We need to fetch more data from the client. */
            if !bs.reload_cache() {
                return false;
            }
            if rice_param_part_lo_bit_count > bs.cache_l1_bits_remaining() {
                /* This happens when we get to end of stream */
                return false;
            }

            bs_cache = bs.cache;
            bs_consumed_bits = bs.consumed_bits + rice_param_part_lo_bit_count;
        }

        /* We should now have enough information to construct the rice parameter part. */
        let rice_param_part_lo =
            bs_cache.wrapping_shr(cache_l1_selection_shift(rice_param_part_lo_bit_count)) as u32;
        *rice_param_part_out = rice_param_part_hi | rice_param_part_lo;

        bs_cache = bs_cache.wrapping_shl(rice_param_part_lo_bit_count);
    }

    /* Make sure the cache is restored at the end of it all. */
    bs.cache = bs_cache;
    bs.consumed_bits = bs_consumed_bits;

    true
}

/// Translation of `drflac__seek_rice_parts()`.
fn seek_rice_parts(bs: &mut Bs<'_>, rice_param: u8) -> bool {
    let rice_param_plus1 = rice_param as u32 + 1;
    let rice_param_plus1_max_consumed_bits = DRFLAC_CACHE_L1_SIZE_BITS - rice_param_plus1;

    /*
    The idea here is to use local variables for the cache in an attempt to encourage the compiler to store them in registers. I have
    no idea how this will work in practice...
    */
    let mut bs_cache = bs.cache;
    let mut bs_consumed_bits = bs.consumed_bits;

    /* The first thing to do is find the first unset bit. Most likely a bit will be set in the current cache line. */
    let mut lzcount = clz(bs_cache);
    if lzcount >= 64 {
        /*
        Getting here means there are no bits set on the cache line. This is a less optimal case because we just wasted a call
        to drflac__clz() and we need to reload the cache.
        */
        loop {
            if bs.next_l2_line < DRFLAC_CACHE_L2_LINE_COUNT {
                bs.update_crc16();
                bs_cache = bs.l2_line_be(bs.next_l2_line);
                bs.next_l2_line += 1;
                bs_consumed_bits = 0;
                bs.crc16_cache = bs_cache;
            } else {
                /* Slow path. We need to fetch more data from the client. */
                if !bs.reload_cache() {
                    return false;
                }

                bs_cache = bs.cache;
                bs_consumed_bits = bs.consumed_bits;
            }

            lzcount = clz(bs_cache);
            if lzcount < 64 {
                break;
            }
        }
    }

    // (extract_rice_param_part:)
    /*
    It is most likely that the riceParam part (which comes after the zero counter) is also on this cache line. When extracting
    this, we include the set bit from the unary coded part because it simplifies cache management. This bit will be handled
    outside of this function at a higher level.
    */
    bs_cache = bs_cache.wrapping_shl(lzcount);
    bs_consumed_bits = bs_consumed_bits.wrapping_add(lzcount);

    if bs_consumed_bits <= rice_param_plus1_max_consumed_bits {
        /* Getting here means the rice parameter part is wholly contained within the current cache line. */
        bs_cache = bs_cache.wrapping_shl(rice_param_plus1);
        bs_consumed_bits += rice_param_plus1;
    } else {
        /*
        Getting here means the rice parameter part straddles the cache line. We need to read from the tail of the current cache
        line, reload the cache, and then combine it with the head of the next cache line.
        */

        /* Before reloading the cache we need to grab the size in bits of the low part. */
        let rice_param_part_lo_bit_count = bs_consumed_bits - rice_param_plus1_max_consumed_bits;
        debug_assert!(rice_param_part_lo_bit_count > 0 && rice_param_part_lo_bit_count < 32);

        /* Now reload the cache. */
        if bs.next_l2_line < DRFLAC_CACHE_L2_LINE_COUNT {
            bs.update_crc16();
            bs_cache = bs.l2_line_be(bs.next_l2_line);
            bs.next_l2_line += 1;
            bs_consumed_bits = rice_param_part_lo_bit_count;
            bs.crc16_cache = bs_cache;
        } else {
            /* Slow path. We need to fetch more data from the client. */
            if !bs.reload_cache() {
                return false;
            }

            if rice_param_part_lo_bit_count > bs.cache_l1_bits_remaining() {
                /* This happens when we get to end of stream */
                return false;
            }

            bs_cache = bs.cache;
            bs_consumed_bits = bs.consumed_bits + rice_param_part_lo_bit_count;
        }

        bs_cache = bs_cache.wrapping_shl(rice_param_part_lo_bit_count);
    }

    /* Make sure the cache is restored at the end of it all. */
    bs.cache = bs_cache;
    bs.consumed_bits = bs_consumed_bits;

    true
}

/// Translation of `drflac__decode_samples_with_residual__rice__scalar_zeroorder()`.
fn decode_samples_with_residual_rice_scalar_zeroorder(
    bs: &mut Bs<'_>,
    count: u32,
    rice_param: u8,
    samples: &mut [i32],
    out: usize,
) -> bool {
    let t: [u32; 2] = [0x00000000, 0xFFFFFFFF];
    let mut zero_count_part0 = 0u32;
    let mut rice_param_part0 = 0u32;

    let rice_param_mask = !((!0u64) << rice_param) as u32;

    let mut i = 0;
    while i < count {
        /* Rice extraction. */
        if !read_rice_parts_x1(bs, rice_param, &mut zero_count_part0, &mut rice_param_part0) {
            return false;
        }

        /* Rice reconstruction. */
        rice_param_part0 &= rice_param_mask;
        rice_param_part0 |= zero_count_part0.wrapping_shl(rice_param as u32);
        rice_param_part0 = (rice_param_part0 >> 1) ^ t[(rice_param_part0 & 0x01) as usize];

        samples[out + i as usize] = rice_param_part0 as i32;

        i += 1;
    }

    true
}

/// Translation of `drflac__decode_samples_with_residual__rice__scalar()`
/// (which `drflac__decode_samples_with_residual__rice()` runs on builds
/// without SSE4.1 or NEON): decode `count` samples into `samples[out..]`.
fn decode_samples_with_residual_rice(
    bs: &mut Bs<'_>,
    bits_per_sample: u32,
    count: u32,
    rice_param: u8,
    lpc_order: u32,
    lpc_shift: i32,
    lpc_precision: u32,
    coefficients: &[i32],
    samples: &mut [i32],
    out: usize,
) -> bool {
    let t: [u32; 2] = [0x00000000, 0xFFFFFFFF];

    if lpc_order == 0 {
        return decode_samples_with_residual_rice_scalar_zeroorder(
            bs, count, rice_param, samples, out,
        );
    }

    let rice_param_mask = !((!0u64) << rice_param) as u32;
    let samples_out_end = out + (count & !3) as usize;

    let predict = |samples: &[i32], at: usize| -> i32 {
        if use_64_bit_prediction(bits_per_sample, lpc_order, lpc_precision) {
            calculate_prediction_64(lpc_order, lpc_shift, coefficients, samples, at)
        } else {
            calculate_prediction_32(lpc_order, lpc_shift, coefficients, samples, at)
        }
    };

    // (the 64-bit and 32-bit versions of this loop are the same but for the prediction.)
    let mut p = out;
    while p < samples_out_end {
        /*
        Rice extraction. It's faster to do this one at a time against local variables than it is to use the x4 version
        against an array. Not sure why, but perhaps it's making more efficient use of registers?
        */
        let mut zero_count_part = [0u32; 4];
        let mut rice_param_part = [0u32; 4];
        for k in 0..4 {
            if !read_rice_parts_x1(
                bs,
                rice_param,
                &mut zero_count_part[k],
                &mut rice_param_part[k],
            ) {
                return false;
            }
        }

        for k in 0..4 {
            rice_param_part[k] &= rice_param_mask;
            rice_param_part[k] |= zero_count_part[k].wrapping_shl(rice_param as u32);
            rice_param_part[k] =
                (rice_param_part[k] >> 1) ^ t[(rice_param_part[k] & 0x01) as usize];
        }

        for k in 0..4 {
            let v = (rice_param_part[k] as i32).wrapping_add(predict(samples, p + k));
            samples[p + k] = v;
        }

        p += 4;
    }

    let mut i = count & !3;
    while i < count {
        let mut zero_count_part0 = 0u32;
        let mut rice_param_part0 = 0u32;

        /* Rice extraction. */
        if !read_rice_parts_x1(bs, rice_param, &mut zero_count_part0, &mut rice_param_part0) {
            return false;
        }

        /* Rice reconstruction. */
        rice_param_part0 &= rice_param_mask;
        rice_param_part0 |= zero_count_part0.wrapping_shl(rice_param as u32);
        rice_param_part0 = (rice_param_part0 >> 1) ^ t[(rice_param_part0 & 0x01) as usize];
        /*riceParamPart0  = (riceParamPart0 >> 1) ^ (~(riceParamPart0 & 0x01) + 1);*/

        /* Sample reconstruction. */
        let v = (rice_param_part0 as i32).wrapping_add(predict(samples, p));
        samples[p] = v;

        i += 1;
        p += 1;
    }

    true
}

/* Reads and seeks past a string of residual values as Rice codes. The decoder should be sitting on the first bit of the Rice codes. */
/// Translation of `drflac__read_and_seek_residual__rice()`.
fn read_and_seek_residual_rice(bs: &mut Bs<'_>, count: u32, rice_param: u8) -> bool {
    for _ in 0..count {
        if !seek_rice_parts(bs, rice_param) {
            return false;
        }
    }

    true
}

/// Translation of `drflac__decode_samples_with_residual__unencoded()`.
fn decode_samples_with_residual_unencoded(
    bs: &mut Bs<'_>,
    bits_per_sample: u32,
    count: u32,
    unencoded_bits_per_sample: u8,
    lpc_order: u32,
    lpc_shift: i32,
    lpc_precision: u32,
    coefficients: &[i32],
    samples: &mut [i32],
    out: usize,
) -> bool {
    debug_assert!(unencoded_bits_per_sample <= 31); /* <-- unencodedBitsPerSample is a 5 bit number, so cannot exceed 31. */

    for i in 0..count as usize {
        if unencoded_bits_per_sample > 0 {
            if !bs.read_int32(unencoded_bits_per_sample as u32, &mut samples[out + i]) {
                return false;
            }
        } else {
            samples[out + i] = 0;
        }

        let prediction = if use_64_bit_prediction(bits_per_sample, lpc_order, lpc_precision) {
            calculate_prediction_64(lpc_order, lpc_shift, coefficients, samples, out + i)
        } else {
            calculate_prediction_32(lpc_order, lpc_shift, coefficients, samples, out + i)
        };
        samples[out + i] = samples[out + i].wrapping_add(prediction);
    }

    true
}

/*
Reads and decodes the residual for the sub-frame the decoder is currently sitting on. This function should be called
when the decoder is sitting at the very start of the RESIDUAL block. The first <order> residuals will be ignored. The
<blockSize> and <order> parameters are used to determine how many residual values need to be decoded.
*/
/// Translation of `drflac__decode_samples_with_residual()`: the
/// subframe's samples are `samples[decoded..]`.
fn decode_samples_with_residual(
    bs: &mut Bs<'_>,
    bits_per_sample: u32,
    block_size: u32,
    lpc_order: u32,
    lpc_shift: i32,
    lpc_precision: u32,
    coefficients: &[i32],
    samples: &mut [i32],
    mut decoded: usize,
) -> bool {
    debug_assert!(block_size != 0);

    let mut residual_method = 0u8;
    if !bs.read_uint8(2, &mut residual_method) {
        return false;
    }

    if residual_method != DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE
        && residual_method != DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE2
    {
        return false; /* Unknown or unsupported residual coding method. */
    }

    /* Ignore the first <order> values. */
    decoded += lpc_order as usize;

    let mut partition_order = 0u8;
    if !bs.read_uint8(4, &mut partition_order) {
        return false;
    }

    /*
    From the FLAC spec:
      The Rice partition order in a Rice-coded residual section must be less than or equal to 8.
    */
    if partition_order > 8 {
        return false;
    }

    /* Validation check. */
    if (block_size / (1 << partition_order)) < lpc_order {
        return false;
    }

    let mut samples_in_partition = (block_size / (1 << partition_order)) - lpc_order;
    let mut partitions_remaining: u32 = 1 << partition_order;
    loop {
        let mut rice_param = 0u8;
        if residual_method == DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE {
            if !bs.read_uint8(4, &mut rice_param) {
                return false;
            }
            if rice_param == 15 {
                rice_param = 0xFF;
            }
        } else if residual_method == DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE2 {
            if !bs.read_uint8(5, &mut rice_param) {
                return false;
            }
            if rice_param == 31 {
                rice_param = 0xFF;
            }
        }

        if rice_param != 0xFF {
            if !decode_samples_with_residual_rice(
                bs,
                bits_per_sample,
                samples_in_partition,
                rice_param,
                lpc_order,
                lpc_shift,
                lpc_precision,
                coefficients,
                samples,
                decoded,
            ) {
                return false;
            }
        } else {
            let mut unencoded_bits_per_sample = 0u8;
            if !bs.read_uint8(5, &mut unencoded_bits_per_sample) {
                return false;
            }

            if !decode_samples_with_residual_unencoded(
                bs,
                bits_per_sample,
                samples_in_partition,
                unencoded_bits_per_sample,
                lpc_order,
                lpc_shift,
                lpc_precision,
                coefficients,
                samples,
                decoded,
            ) {
                return false;
            }
        }

        decoded += samples_in_partition as usize;

        if partitions_remaining == 1 {
            break;
        }

        partitions_remaining -= 1;

        if partition_order != 0 {
            samples_in_partition = block_size / (1 << partition_order);
        }
    }

    true
}

/*
Reads and seeks past the residual for the sub-frame the decoder is currently sitting on. This function should be called
when the decoder is sitting at the very start of the RESIDUAL block. The first <order> residuals will be set to 0. The
<blockSize> and <order> parameters are used to determine how many residual values need to be decoded.
*/
/// Translation of `drflac__read_and_seek_residual()`.
fn read_and_seek_residual(bs: &mut Bs<'_>, block_size: u32, order: u32) -> bool {
    debug_assert!(block_size != 0);

    let mut residual_method = 0u8;
    if !bs.read_uint8(2, &mut residual_method) {
        return false;
    }

    if residual_method != DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE
        && residual_method != DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE2
    {
        return false; /* Unknown or unsupported residual coding method. */
    }

    let mut partition_order = 0u8;
    if !bs.read_uint8(4, &mut partition_order) {
        return false;
    }

    /*
    From the FLAC spec:
      The Rice partition order in a Rice-coded residual section must be less than or equal to 8.
    */
    if partition_order > 8 {
        return false;
    }

    /* Validation check. */
    if (block_size / (1 << partition_order)) <= order {
        return false;
    }

    let mut samples_in_partition = (block_size / (1 << partition_order)) - order;
    let mut partitions_remaining: u32 = 1 << partition_order;
    loop {
        let mut rice_param = 0u8;
        if residual_method == DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE {
            if !bs.read_uint8(4, &mut rice_param) {
                return false;
            }
            if rice_param == 15 {
                rice_param = 0xFF;
            }
        } else if residual_method == DRFLAC_RESIDUAL_CODING_METHOD_PARTITIONED_RICE2 {
            if !bs.read_uint8(5, &mut rice_param) {
                return false;
            }
            if rice_param == 31 {
                rice_param = 0xFF;
            }
        }

        if rice_param != 0xFF {
            if !read_and_seek_residual_rice(bs, samples_in_partition, rice_param) {
                return false;
            }
        } else {
            let mut unencoded_bits_per_sample = 0u8;
            if !bs.read_uint8(5, &mut unencoded_bits_per_sample) {
                return false;
            }

            if !bs.seek_bits(
                (unencoded_bits_per_sample as u32).wrapping_mul(samples_in_partition) as usize,
            ) {
                return false;
            }
        }

        if partitions_remaining == 1 {
            break;
        }

        partitions_remaining -= 1;
        samples_in_partition = block_size / (1 << partition_order);
    }

    true
}

/// Translation of `drflac__decode_samples__constant()`.
fn decode_samples_constant(
    bs: &mut Bs<'_>,
    block_size: u32,
    subframe_bits_per_sample: u32,
    samples: &mut [i32],
    decoded: usize,
) -> bool {
    /* Only a single sample needs to be decoded here. */
    let mut sample = 0i32;
    if !bs.read_int32(subframe_bits_per_sample, &mut sample) {
        return false;
    }

    /*
    We don't really need to expand this, but it does simplify the process of reading samples. If this becomes a performance issue (unlikely)
    we'll want to look at a more efficient way.
    */
    samples[decoded..decoded + block_size as usize].fill(sample);

    true
}

/// Translation of `drflac__decode_samples__verbatim()`.
fn decode_samples_verbatim(
    bs: &mut Bs<'_>,
    block_size: u32,
    subframe_bits_per_sample: u32,
    samples: &mut [i32],
    decoded: usize,
) -> bool {
    for i in 0..block_size as usize {
        let mut sample = 0i32;
        if !bs.read_int32(subframe_bits_per_sample, &mut sample) {
            return false;
        }

        samples[decoded + i] = sample;
    }

    true
}

/// Translation of `drflac__decode_samples__fixed()`.
fn decode_samples_fixed(
    bs: &mut Bs<'_>,
    block_size: u32,
    subframe_bits_per_sample: u32,
    lpc_order: u8,
    samples: &mut [i32],
    decoded: usize,
) -> bool {
    static LPC_COEFFICIENTS_TABLE: [[i32; 4]; 5] = [
        [0, 0, 0, 0],
        [1, 0, 0, 0],
        [2, -1, 0, 0],
        [3, -3, 1, 0],
        [4, -6, 4, -1],
    ];

    /* Warm up samples and coefficients. */
    for i in 0..lpc_order as usize {
        let mut sample = 0i32;
        if !bs.read_int32(subframe_bits_per_sample, &mut sample) {
            return false;
        }

        samples[decoded + i] = sample;
    }

    decode_samples_with_residual(
        bs,
        subframe_bits_per_sample,
        block_size,
        lpc_order as u32,
        0,
        4,
        &LPC_COEFFICIENTS_TABLE[lpc_order as usize],
        samples,
        decoded,
    )
}

/// Translation of `drflac__decode_samples__lpc()`.
fn decode_samples_lpc(
    bs: &mut Bs<'_>,
    block_size: u32,
    bits_per_sample: u32,
    lpc_order: u8,
    samples: &mut [i32],
    decoded: usize,
) -> bool {
    let mut coefficients = [0i32; 32];

    /* Warm up samples. */
    for i in 0..lpc_order as usize {
        let mut sample = 0i32;
        if !bs.read_int32(bits_per_sample, &mut sample) {
            return false;
        }

        samples[decoded + i] = sample;
    }

    let mut lpc_precision = 0u8;
    if !bs.read_uint8(4, &mut lpc_precision) {
        return false;
    }
    if lpc_precision == 15 {
        return false; /* Invalid. */
    }
    lpc_precision += 1;

    let mut lpc_shift = 0i8;
    if !bs.read_int8(5, &mut lpc_shift) {
        return false;
    }

    /*
    From the FLAC specification:

        Quantized linear predictor coefficient shift needed in bits (NOTE: this number is signed two's-complement)

    Emphasis on the "signed two's-complement". In practice there does not seem to be any encoders nor decoders supporting negative shifts. For now dr_flac is
    not going to support negative shifts as I don't have any reference files. However, when a reference file comes through I will consider adding support.
    */
    if lpc_shift < 0 {
        return false;
    }

    for i in 0..lpc_order as usize {
        if !bs.read_int32(lpc_precision as u32, &mut coefficients[i]) {
            return false;
        }
    }

    decode_samples_with_residual(
        bs,
        bits_per_sample,
        block_size,
        lpc_order as u32,
        lpc_shift as i32,
        lpc_precision as u32,
        &coefficients,
        samples,
        decoded,
    )
}

/// Translation of `drflac_subframe` (`pSamplesS32` is an offset into the
/// decoder's decoded samples).
#[derive(Clone, Copy, Default, Debug)]
struct Subframe {
    /* The type of the subframe: SUBFRAME_CONSTANT, SUBFRAME_VERBATIM, SUBFRAME_FIXED or SUBFRAME_LPC. */
    subframe_type: u8,

    /* The number of wasted bits per sample as specified by the sub-frame header. */
    wasted_bits_per_sample: u8,

    /* The order to use for the prediction stage for SUBFRAME_FIXED and SUBFRAME_LPC. */
    lpc_order: u8,

    /* A pointer to the buffer containing the decoded samples in the subframe. This pointer is an offset from drflac::pExtraData. */
    samples_s32: Option<usize>,
}

/// Translation of `drflac_frame_header`.
#[derive(Clone, Copy, Default, Debug)]
struct FrameHeader {
    /*
    If the stream uses variable block sizes, this will be set to the index of the first PCM frame. If fixed block sizes are used, this will
    always be set to 0. This is 64-bit because the decoded PCM frame number will be 36 bits.
    */
    pcm_frame_number: u64,

    /*
    If the stream uses fixed block sizes, this will be set to the frame number. If variable block sizes are used, this will always be 0. This
    is 32-bit because in fixed block sizes, the maximum frame number will be 31 bits.
    */
    flac_frame_number: u32,

    /* The sample rate of this frame. */
    sample_rate: u32,

    /* The number of PCM frames in each sub-frame within this frame. */
    block_size_in_pcm_frames: u16,

    /*
    The channel assignment of this frame. This is not always set to the channel count. If interchannel decorrelation is being used this
    will be set to DRFLAC_CHANNEL_ASSIGNMENT_LEFT_SIDE, DRFLAC_CHANNEL_ASSIGNMENT_RIGHT_SIDE or DRFLAC_CHANNEL_ASSIGNMENT_MID_SIDE.
    */
    channel_assignment: u8,

    /* The number of bits per sample within this frame. */
    bits_per_sample: u8,

    /* The frame's CRC. */
    crc8: u8,
}

/// Translation of `drflac_frame`.
#[derive(Clone, Copy, Default, Debug)]
struct Frame {
    /* The header. */
    header: FrameHeader,

    /*
    The number of PCM frames left to be read in this FLAC frame. This is initially set to the block size. As PCM frames are read,
    this will be decremented. When it reaches 0, the decoder will see this frame as fully consumed and load the next frame.
    */
    pcm_frames_remaining: u32,

    /* The list of sub-frames within the frame. There is one sub-frame for each channel, and there's a maximum of 8 channels. */
    subframes: [Subframe; 8],
}

/// Translation of `drflac__read_next_flac_frame_header()`.
fn read_next_flac_frame_header(
    bs: &mut Bs<'_>,
    streaminfo_bits_per_sample: u8,
    header: &mut FrameHeader,
) -> bool {
    const SAMPLE_RATE_TABLE: [u32; 12] = [
        0, 88200, 176400, 192000, 8000, 16000, 22050, 24000, 32000, 44100, 48000, 96000,
    ];
    const BITS_PER_SAMPLE_TABLE: [u8; 8] = [0, 8, 12, 0xFF, 16, 20, 24, 0xFF]; /* -1 = reserved. */

    /* Keep looping until we find a valid sync code. */
    loop {
        let mut crc8_ = 0xCEu8; /* 0xCE = drflac_crc8(0, 0x3FFE, 14); */
        let mut reserved = 0u8;
        let mut blocking_strategy = 0u8;
        let mut block_size = 0u8;
        let mut sample_rate = 0u8;
        let mut channel_assignment = 0u8;
        let mut bits_per_sample = 0u8;

        if !bs.find_and_seek_to_next_sync_code() {
            return false;
        }

        if !bs.read_uint8(1, &mut reserved) {
            return false;
        }
        if reserved == 1 {
            continue;
        }
        crc8_ = crc8(crc8_, reserved as u32, 1);

        if !bs.read_uint8(1, &mut blocking_strategy) {
            return false;
        }
        crc8_ = crc8(crc8_, blocking_strategy as u32, 1);

        if !bs.read_uint8(4, &mut block_size) {
            return false;
        }
        if block_size == 0 {
            continue;
        }
        crc8_ = crc8(crc8_, block_size as u32, 4);

        if !bs.read_uint8(4, &mut sample_rate) {
            return false;
        }
        crc8_ = crc8(crc8_, sample_rate as u32, 4);

        if !bs.read_uint8(4, &mut channel_assignment) {
            return false;
        }
        if channel_assignment > 10 {
            continue;
        }
        crc8_ = crc8(crc8_, channel_assignment as u32, 4);

        if !bs.read_uint8(3, &mut bits_per_sample) {
            return false;
        }
        if bits_per_sample == 3 || bits_per_sample == 7 {
            continue;
        }
        crc8_ = crc8(crc8_, bits_per_sample as u32, 3);

        if !bs.read_uint8(1, &mut reserved) {
            return false;
        }
        if reserved == 1 {
            continue;
        }
        crc8_ = crc8(crc8_, reserved as u32, 1);

        let is_variable_block_size = blocking_strategy == 1;
        if is_variable_block_size {
            let mut pcm_frame_number = 0u64;
            let result = bs.read_utf8_coded_number(&mut pcm_frame_number, &mut crc8_);
            if result != DRFLAC_SUCCESS {
                if result == DRFLAC_AT_END {
                    return false;
                } else {
                    continue;
                }
            }
            header.flac_frame_number = 0;
            header.pcm_frame_number = pcm_frame_number;
        } else {
            let mut flac_frame_number = 0u64;
            let result = bs.read_utf8_coded_number(&mut flac_frame_number, &mut crc8_);
            if result != DRFLAC_SUCCESS {
                if result == DRFLAC_AT_END {
                    return false;
                } else {
                    continue;
                }
            }
            header.flac_frame_number = flac_frame_number as u32; /* <-- Safe cast. */
            header.pcm_frame_number = 0;
        }

        debug_assert!(block_size > 0);
        if block_size == 1 {
            header.block_size_in_pcm_frames = 192;
        } else if block_size <= 5 {
            debug_assert!(block_size >= 2);
            header.block_size_in_pcm_frames = 576 * (1 << (block_size - 2));
        } else if block_size == 6 {
            if !bs.read_uint16(8, &mut header.block_size_in_pcm_frames) {
                return false;
            }
            crc8_ = crc8(crc8_, header.block_size_in_pcm_frames as u32, 8);
            header.block_size_in_pcm_frames += 1;
        } else if block_size == 7 {
            if !bs.read_uint16(16, &mut header.block_size_in_pcm_frames) {
                return false;
            }
            crc8_ = crc8(crc8_, header.block_size_in_pcm_frames as u32, 16);
            if header.block_size_in_pcm_frames == 0xFFFF {
                return false; /* Frame is too big. This is the size of the frame minus 1. The STREAMINFO block defines the max block size which is 16-bits. Adding one will make it 17 bits and therefore too big. */
            }
            header.block_size_in_pcm_frames += 1;
        } else {
            debug_assert!(block_size >= 8);
            header.block_size_in_pcm_frames = (256u32 * (1 << (block_size - 8))) as u16;
        }

        if sample_rate <= 11 {
            header.sample_rate = SAMPLE_RATE_TABLE[sample_rate as usize];
        } else if sample_rate == 12 {
            if !bs.read_uint32(8, &mut header.sample_rate) {
                return false;
            }
            crc8_ = crc8(crc8_, header.sample_rate, 8);
            header.sample_rate *= 1000;
        } else if sample_rate == 13 {
            if !bs.read_uint32(16, &mut header.sample_rate) {
                return false;
            }
            crc8_ = crc8(crc8_, header.sample_rate, 16);
        } else if sample_rate == 14 {
            if !bs.read_uint32(16, &mut header.sample_rate) {
                return false;
            }
            crc8_ = crc8(crc8_, header.sample_rate, 16);
            header.sample_rate *= 10;
        } else {
            continue; /* Invalid. Assume an invalid block. */
        }

        header.channel_assignment = channel_assignment;

        header.bits_per_sample = BITS_PER_SAMPLE_TABLE[bits_per_sample as usize];
        if header.bits_per_sample == 0 {
            header.bits_per_sample = streaminfo_bits_per_sample;
        }

        if header.bits_per_sample != streaminfo_bits_per_sample {
            /* If this subframe has a different bitsPerSample then streaminfo or the first frame, reject it */
            return false;
        }

        if !bs.read_uint8(8, &mut header.crc8) {
            return false;
        }

        if header.crc8 != crc8_ {
            continue; /* CRC mismatch. Loop back to the top and find the next sync code. */
        }
        return true;
    }
}

/// Translation of `drflac__read_subframe_header()`.
fn read_subframe_header(bs: &mut Bs<'_>, subframe: &mut Subframe) -> bool {
    let mut header = 0u8;
    if !bs.read_uint8(8, &mut header) {
        return false;
    }

    /* First bit should always be 0. */
    if (header & 0x80) != 0 {
        return false;
    }

    /*
    Default to 0 for the LPC order. It's important that we always set this to 0 for non LPC
    and FIXED subframes because we'll be using it in a generic validation check later.
    */
    subframe.lpc_order = 0;

    let type_ = (header & 0x7E) >> 1;
    if type_ == 0 {
        subframe.subframe_type = DRFLAC_SUBFRAME_CONSTANT;
    } else if type_ == 1 {
        subframe.subframe_type = DRFLAC_SUBFRAME_VERBATIM;
    } else {
        if (type_ & 0x20) != 0 {
            subframe.subframe_type = DRFLAC_SUBFRAME_LPC;
            subframe.lpc_order = (type_ & 0x1F) + 1;
        } else if (type_ & 0x08) != 0 {
            subframe.subframe_type = DRFLAC_SUBFRAME_FIXED;
            subframe.lpc_order = type_ & 0x07;
            if subframe.lpc_order > 4 {
                subframe.subframe_type = DRFLAC_SUBFRAME_RESERVED;
                subframe.lpc_order = 0;
            }
        } else {
            subframe.subframe_type = DRFLAC_SUBFRAME_RESERVED;
        }
    }

    if subframe.subframe_type == DRFLAC_SUBFRAME_RESERVED {
        return false;
    }

    /* Wasted bits per sample. */
    subframe.wasted_bits_per_sample = 0;
    if (header & 0x01) == 1 {
        let mut wasted_bits_per_sample = 0u32;
        if !bs.seek_past_next_set_bit(&mut wasted_bits_per_sample) {
            return false;
        }
        subframe.wasted_bits_per_sample = (wasted_bits_per_sample as u8).wrapping_add(1);
    }

    true
}

/// Translation of `drflac__decode_subframe()`: decode into
/// `samples[decoded..]`.
fn decode_subframe(
    bs: &mut Bs<'_>,
    frame: &mut Frame,
    subframe_index: usize,
    samples: &mut [i32],
    decoded: usize,
) -> bool {
    let header = frame.header;
    let subframe = &mut frame.subframes[subframe_index];
    if !read_subframe_header(bs, subframe) {
        return false;
    }

    /* Side channels require an extra bit per sample. Took a while to figure that one out... */
    let mut subframe_bits_per_sample = header.bits_per_sample as u32;
    if (header.channel_assignment == DRFLAC_CHANNEL_ASSIGNMENT_LEFT_SIDE
        || header.channel_assignment == DRFLAC_CHANNEL_ASSIGNMENT_MID_SIDE)
        && subframe_index == 1
    {
        subframe_bits_per_sample += 1;
    } else if header.channel_assignment == DRFLAC_CHANNEL_ASSIGNMENT_RIGHT_SIDE
        && subframe_index == 0
    {
        subframe_bits_per_sample += 1;
    }

    if subframe_bits_per_sample > 32 {
        /* libFLAC and ffmpeg reject 33-bit subframes as well */
        return false;
    }

    /* Need to handle wasted bits per sample. */
    if subframe.wasted_bits_per_sample as u32 >= subframe_bits_per_sample {
        return false;
    }
    subframe_bits_per_sample -= subframe.wasted_bits_per_sample as u32;

    subframe.samples_s32 = Some(decoded);

    /*
    pDecodedSamplesOut will be pointing to a buffer that was allocated with enough memory to store
    maxBlockSizeInPCMFrames samples (as specified in the FLAC header). We need to guard against an
    overflow here. At a higher level we are checking maxBlockSizeInPCMFrames from the header, but
    here we need to do an additional check to ensure this frame's block size fully encompasses any
    warmup samples which is determined by the LPC order. For non LPC and FIXED subframes, the LPC
    order will be have been set to 0 in drflac__read_subframe_header().
    */
    if (header.block_size_in_pcm_frames as u32) < subframe.lpc_order as u32 {
        return false;
    }

    let block_size = header.block_size_in_pcm_frames as u32;
    match subframe.subframe_type {
        DRFLAC_SUBFRAME_CONSTANT => {
            decode_samples_constant(bs, block_size, subframe_bits_per_sample, samples, decoded)
        }
        DRFLAC_SUBFRAME_VERBATIM => {
            decode_samples_verbatim(bs, block_size, subframe_bits_per_sample, samples, decoded)
        }
        DRFLAC_SUBFRAME_FIXED => decode_samples_fixed(
            bs,
            block_size,
            subframe_bits_per_sample,
            subframe.lpc_order,
            samples,
            decoded,
        ),
        DRFLAC_SUBFRAME_LPC => decode_samples_lpc(
            bs,
            block_size,
            subframe_bits_per_sample,
            subframe.lpc_order,
            samples,
            decoded,
        ),
        _ => false,
    }
}

/// Translation of `drflac__seek_subframe()`.
fn seek_subframe(bs: &mut Bs<'_>, frame: &mut Frame, subframe_index: usize) -> bool {
    let header = frame.header;
    let subframe = &mut frame.subframes[subframe_index];
    if !read_subframe_header(bs, subframe) {
        return false;
    }

    /* Side channels require an extra bit per sample. Took a while to figure that one out... */
    let mut subframe_bits_per_sample = header.bits_per_sample as u32;
    if (header.channel_assignment == DRFLAC_CHANNEL_ASSIGNMENT_LEFT_SIDE
        || header.channel_assignment == DRFLAC_CHANNEL_ASSIGNMENT_MID_SIDE)
        && subframe_index == 1
    {
        subframe_bits_per_sample += 1;
    } else if header.channel_assignment == DRFLAC_CHANNEL_ASSIGNMENT_RIGHT_SIDE
        && subframe_index == 0
    {
        subframe_bits_per_sample += 1;
    }

    /* Need to handle wasted bits per sample. */
    if subframe.wasted_bits_per_sample as u32 >= subframe_bits_per_sample {
        return false;
    }
    subframe_bits_per_sample -= subframe.wasted_bits_per_sample as u32;

    subframe.samples_s32 = None;

    match subframe.subframe_type {
        DRFLAC_SUBFRAME_CONSTANT => {
            if !bs.seek_bits(subframe_bits_per_sample as usize) {
                return false;
            }
        }

        DRFLAC_SUBFRAME_VERBATIM => {
            let bits_to_seek =
                (header.block_size_in_pcm_frames as u32).wrapping_mul(subframe_bits_per_sample);
            if !bs.seek_bits(bits_to_seek as usize) {
                return false;
            }
        }

        DRFLAC_SUBFRAME_FIXED => {
            let bits_to_seek = subframe.lpc_order as u32 * subframe_bits_per_sample;
            if !bs.seek_bits(bits_to_seek as usize) {
                return false;
            }

            if !read_and_seek_residual(
                bs,
                header.block_size_in_pcm_frames as u32,
                subframe.lpc_order as u32,
            ) {
                return false;
            }
        }

        DRFLAC_SUBFRAME_LPC => {
            let bits_to_seek = subframe.lpc_order as u32 * subframe_bits_per_sample;
            if !bs.seek_bits(bits_to_seek as usize) {
                return false;
            }

            let mut lpc_precision = 0u8;
            if !bs.read_uint8(4, &mut lpc_precision) {
                return false;
            }
            if lpc_precision == 15 {
                return false; /* Invalid. */
            }
            lpc_precision += 1;

            let bits_to_seek = (subframe.lpc_order as u32 * lpc_precision as u32) + 5; /* +5 for shift. */
            if !bs.seek_bits(bits_to_seek as usize) {
                return false;
            }

            if !read_and_seek_residual(
                bs,
                header.block_size_in_pcm_frames as u32,
                subframe.lpc_order as u32,
            ) {
                return false;
            }
        }

        _ => return false,
    }

    true
}

/// Translation of `drflac__get_channel_count_from_channel_assignment()`.
fn get_channel_count_from_channel_assignment(channel_assignment: u8) -> u8 {
    const LOOKUP: [u8; 11] = [1, 2, 3, 4, 5, 6, 7, 8, 2, 2, 2];

    debug_assert!(channel_assignment <= 10);
    LOOKUP[channel_assignment as usize]
}

/// Translation of `drflac`.
pub(crate) struct Drflac<'a> {
    /* The sample rate. Will be set to something like 44100. */
    pub sample_rate: u32,

    /*
    The number of channels. This will be set to 1 for monaural streams, 2 for stereo, etc. Maximum 8. This is set based on the
    value specified in the STREAMINFO block.
    */
    pub channels: u8,

    /* The bits per sample. Will be set to something like 16, 24, etc. */
    pub bits_per_sample: u8,

    /* The maximum block size, in samples. This number represents the number of samples in each channel (not combined). */
    max_block_size_in_pcm_frames: u16,

    /*
    The total number of PCM Frames making up the stream. Can be 0 in which case it's still a valid stream, but just means
    the total PCM frame count is unknown. Likely the case with streams like internet radio.
    */
    pub total_pcm_frame_count: u64,

    /* The container type. This is set based on whether or not the decoder was opened from a native or Ogg stream. */
    container: Container,

    /* Information about the frame the decoder is currently sitting on. */
    current_flac_frame: Frame,

    /* The index of the PCM frame the decoder is currently sitting on. This is only used for seeking. */
    current_pcm_frame: u64,

    /* The position of the first FLAC frame in the stream. This is only ever used for seeking. */
    first_flac_frame_pos_in_bytes: u64,

    /* The decoded sample data (`pDecodedSamples`). */
    decoded_samples: Vec<i32>,

    /* The seek table, or None if there is no seek table (`pSeekpoints` and `seekpointCount`). */
    seekpoints: Option<Vec<Seekpoint>>,

    /* The bit streamer. The raw FLAC data is fed through this object. */
    bs: Bs<'a>,
}

impl<'a> Drflac<'a> {
    /// Translation of `drflac__decode_flac_frame()`.
    fn decode_flac_frame(&mut self) -> DrflacResult {
        /* This function should be called while the stream is sitting on the first byte after the frame header. */
        self.current_flac_frame.pcm_frames_remaining = 0;
        self.current_flac_frame.subframes = [Subframe::default(); 8];

        /* The frame block size must never be larger than the maximum block size defined by the FLAC stream. */
        if self.current_flac_frame.header.block_size_in_pcm_frames
            > self.max_block_size_in_pcm_frames
        {
            return DRFLAC_ERROR;
        }

        /* The number of channels in the frame must match the channel count from the STREAMINFO block. */
        let channel_count = get_channel_count_from_channel_assignment(
            self.current_flac_frame.header.channel_assignment,
        );
        if channel_count != self.channels {
            return DRFLAC_ERROR;
        }

        for i in 0..channel_count as usize {
            let decoded = self.current_flac_frame.header.block_size_in_pcm_frames as usize * i;
            if !decode_subframe(
                &mut self.bs,
                &mut self.current_flac_frame,
                i,
                &mut self.decoded_samples,
                decoded,
            ) {
                return DRFLAC_ERROR;
            }
        }

        let padding_size_in_bits = (self.bs.cache_l1_bits_remaining() & 7) as u8;
        if padding_size_in_bits > 0 {
            let mut padding = 0u8;
            if !self
                .bs
                .read_uint8(padding_size_in_bits as u32, &mut padding)
            {
                return DRFLAC_AT_END;
            }
        }

        let actual_crc16 = self.bs.flush_crc16();
        let mut desired_crc16 = 0u16;
        if !self.bs.read_uint16(16, &mut desired_crc16) {
            return DRFLAC_AT_END;
        }

        if actual_crc16 != desired_crc16 {
            return DRFLAC_CRC_MISMATCH; /* CRC mismatch. */
        }

        self.current_flac_frame.pcm_frames_remaining =
            self.current_flac_frame.header.block_size_in_pcm_frames as u32;

        DRFLAC_SUCCESS
    }

    /// Translation of `drflac__seek_flac_frame()`.
    fn seek_flac_frame(&mut self) -> DrflacResult {
        self.current_flac_frame.pcm_frames_remaining = 0;

        let result = 'error: {
            let channel_count = get_channel_count_from_channel_assignment(
                self.current_flac_frame.header.channel_assignment,
            );
            for i in 0..channel_count as usize {
                if !seek_subframe(&mut self.bs, &mut self.current_flac_frame, i) {
                    break 'error DRFLAC_ERROR;
                }
            }

            /* Padding. */
            if !self
                .bs
                .seek_bits((self.bs.cache_l1_bits_remaining() & 7) as usize)
            {
                break 'error DRFLAC_ERROR;
            }

            /* CRC. */
            let actual_crc16 = self.bs.flush_crc16();
            let mut desired_crc16 = 0u16;
            if !self.bs.read_uint16(16, &mut desired_crc16) {
                break 'error DRFLAC_AT_END;
            }

            if actual_crc16 != desired_crc16 {
                break 'error DRFLAC_CRC_MISMATCH; /* CRC mismatch. */
            }

            return DRFLAC_SUCCESS;
        };

        // (error:)
        self.current_flac_frame.subframes = [Subframe::default(); 8];
        result
    }

    /// Translation of `drflac__read_and_decode_next_flac_frame()`.
    fn read_and_decode_next_flac_frame(&mut self) -> bool {
        loop {
            if !read_next_flac_frame_header(
                &mut self.bs,
                self.bits_per_sample,
                &mut self.current_flac_frame.header,
            ) {
                return false;
            }

            let result = self.decode_flac_frame();
            if result != DRFLAC_SUCCESS {
                if result == DRFLAC_CRC_MISMATCH {
                    continue; /* CRC mismatch. Skip to the next frame. */
                } else {
                    return false;
                }
            }

            return true;
        }
    }

    /// Translation of `drflac__get_pcm_frame_range_of_current_flac_frame()`.
    fn get_pcm_frame_range_of_current_flac_frame(&self) -> (u64, u64) {
        let mut first_pcm_frame = self.current_flac_frame.header.pcm_frame_number;
        if first_pcm_frame == 0 {
            first_pcm_frame = (self.current_flac_frame.header.flac_frame_number as u64)
                .wrapping_mul(self.max_block_size_in_pcm_frames as u64);
        }

        let mut last_pcm_frame = first_pcm_frame
            .wrapping_add(self.current_flac_frame.header.block_size_in_pcm_frames as u64);
        if last_pcm_frame > 0 {
            last_pcm_frame -= 1; /* Needs to be zero based. */
        }

        (first_pcm_frame, last_pcm_frame)
    }

    /// Translation of `drflac__seek_to_first_frame()`.
    fn seek_to_first_frame(&mut self) -> bool {
        let result = self.bs.seek_to_byte(self.first_flac_frame_pos_in_bytes);

        self.current_flac_frame = Frame::default();
        self.current_pcm_frame = 0;

        result
    }

    /// Translation of `drflac__seek_to_next_flac_frame()`.
    fn seek_to_next_flac_frame(&mut self) -> DrflacResult {
        /* This function should only ever be called while the decoder is sitting on the first byte past the FRAME_HEADER section. */
        self.seek_flac_frame()
    }

    /// Translation of `drflac__seek_forward_by_pcm_frames()`.
    fn seek_forward_by_pcm_frames(&mut self, mut pcm_frames_to_seek: u64) -> u64 {
        let mut pcm_frames_read: u64 = 0;
        while pcm_frames_to_seek > 0 {
            if self.current_flac_frame.pcm_frames_remaining == 0 {
                if !self.read_and_decode_next_flac_frame() {
                    break; /* Couldn't read the next frame, so just break from the loop and return. */
                }
            } else if self.current_flac_frame.pcm_frames_remaining as u64 > pcm_frames_to_seek {
                pcm_frames_read += pcm_frames_to_seek;
                self.current_flac_frame.pcm_frames_remaining -= pcm_frames_to_seek as u32; /* <-- Safe cast. Will always be < currentFrame.pcmFramesRemaining < 65536. */
                pcm_frames_to_seek = 0;
            } else {
                pcm_frames_read += self.current_flac_frame.pcm_frames_remaining as u64;
                pcm_frames_to_seek -= self.current_flac_frame.pcm_frames_remaining as u64;
                self.current_flac_frame.pcm_frames_remaining = 0;
            }
        }

        self.current_pcm_frame = self.current_pcm_frame.wrapping_add(pcm_frames_read);
        pcm_frames_read
    }

    /// The frame-by-frame search that ends `drflac__seek_to_pcm_frame__brute_force()`
    /// and `drflac__seek_to_pcm_frame__seek_table()`.
    fn seek_frame_by_frame(
        &mut self,
        pcm_frame_index: u64,
        mut running_pcm_frame_count: u64,
        mut is_mid_frame: bool,
    ) -> bool {
        /*
        We need to as quickly as possible find the frame that contains the target sample. To do this, we iterate over each frame and inspect its
        header. If based on the header we can determine that the frame contains the sample, we do a full decode of that frame.
        */
        loop {
            'next_iteration: {
                let (first_pcm_frame_in_flac_frame, last_pcm_frame_in_flac_frame) =
                    self.get_pcm_frame_range_of_current_flac_frame();

                let pcm_frame_count_in_this_flac_frame = last_pcm_frame_in_flac_frame
                    .wrapping_sub(first_pcm_frame_in_flac_frame)
                    .wrapping_add(1);
                if pcm_frame_index
                    < running_pcm_frame_count.wrapping_add(pcm_frame_count_in_this_flac_frame)
                {
                    /*
                    The sample should be in this frame. We need to fully decode it, however if it's an invalid frame (a CRC mismatch), we need to pretend
                    it never existed and keep iterating.
                    */
                    let pcm_frames_to_decode =
                        pcm_frame_index.wrapping_sub(running_pcm_frame_count);

                    if !is_mid_frame {
                        let result = self.decode_flac_frame();
                        if result == DRFLAC_SUCCESS {
                            /* The frame is valid. We just need to skip over some samples to ensure it's sample-exact. */
                            return self.seek_forward_by_pcm_frames(pcm_frames_to_decode)
                                == pcm_frames_to_decode; /* <-- If this fails, something bad has happened (it should never fail). */
                        } else if result == DRFLAC_CRC_MISMATCH {
                            break 'next_iteration; /* CRC mismatch. Pretend this frame never existed. */
                        } else {
                            return false;
                        }
                    } else {
                        /* We started seeking mid-frame which means we need to skip the frame decoding part. */
                        return self.seek_forward_by_pcm_frames(pcm_frames_to_decode)
                            == pcm_frames_to_decode;
                    }
                } else {
                    /*
                    It's not in this frame. We need to seek past the frame, but check if there was a CRC mismatch. If so, we pretend this
                    frame never existed and leave the running sample count untouched.
                    */
                    if !is_mid_frame {
                        let result = self.seek_to_next_flac_frame();
                        if result == DRFLAC_SUCCESS {
                            running_pcm_frame_count = running_pcm_frame_count
                                .wrapping_add(pcm_frame_count_in_this_flac_frame);
                        } else if result == DRFLAC_CRC_MISMATCH {
                            break 'next_iteration; /* CRC mismatch. Pretend this frame never existed. */
                        } else {
                            return false;
                        }
                    } else {
                        /*
                        We started seeking mid-frame which means we need to seek by reading to the end of the frame instead of with
                        drflac__seek_to_next_flac_frame() which only works if the decoder is sitting on the byte just after the frame header.
                        */
                        running_pcm_frame_count = running_pcm_frame_count
                            .wrapping_add(self.current_flac_frame.pcm_frames_remaining as u64);
                        self.current_flac_frame.pcm_frames_remaining = 0;
                        is_mid_frame = false;
                    }

                    /* If we are seeking to the end of the file and we've just hit it, we're done. */
                    if pcm_frame_index == self.total_pcm_frame_count
                        && running_pcm_frame_count == self.total_pcm_frame_count
                    {
                        return true;
                    }
                }
            }

            // (next_iteration:)
            /* Grab the next frame in preparation for the next iteration. */
            if !read_next_flac_frame_header(
                &mut self.bs,
                self.bits_per_sample,
                &mut self.current_flac_frame.header,
            ) {
                return false;
            }
        }
    }

    /// Translation of `drflac__seek_to_pcm_frame__brute_force()`.
    fn seek_to_pcm_frame_brute_force(&mut self, pcm_frame_index: u64) -> bool {
        let mut is_mid_frame = false;
        let running_pcm_frame_count;

        /* If we are seeking forward we start from the current position. Otherwise we need to start all the way from the start of the file. */
        if pcm_frame_index >= self.current_pcm_frame {
            /* Seeking forward. Need to seek from the current position. */
            running_pcm_frame_count = self.current_pcm_frame;

            /* The frame header for the first frame may not yet have been read. We need to do that if necessary. */
            if self.current_pcm_frame == 0 && self.current_flac_frame.pcm_frames_remaining == 0 {
                if !read_next_flac_frame_header(
                    &mut self.bs,
                    self.bits_per_sample,
                    &mut self.current_flac_frame.header,
                ) {
                    return false;
                }
            } else {
                is_mid_frame = true;
            }
        } else {
            /* Seeking backwards. Need to seek from the start of the file. */
            running_pcm_frame_count = 0;

            /* Move back to the start. */
            if !self.seek_to_first_frame() {
                return false;
            }

            /* Decode the first frame in preparation for sample-exact seeking below. */
            if !read_next_flac_frame_header(
                &mut self.bs,
                self.bits_per_sample,
                &mut self.current_flac_frame.header,
            ) {
                return false;
            }
        }

        self.seek_frame_by_frame(pcm_frame_index, running_pcm_frame_count, is_mid_frame)
    }
}

/*
We use an average compression ratio to determine our approximate start location. FLAC files are generally about 50%-70% the size of their
uncompressed counterparts so we'll use this as a basis. I'm going to split the middle and use a factor of 0.6 to determine the starting
location.
*/
const DRFLAC_BINARY_SEARCH_APPROX_COMPRESSION_RATIO: f32 = 0.6;

impl<'a> Drflac<'a> {
    /// Translation of `drflac__seek_to_approximate_flac_frame_to_byte()`.
    fn seek_to_approximate_flac_frame_to_byte(
        &mut self,
        mut target_byte: u64,
        range_lo: u64,
        mut range_hi: u64,
        last_successful_seek_offset: &mut u64,
    ) -> bool {
        *last_successful_seek_offset = self.first_flac_frame_pos_in_bytes;

        loop {
            /* After rangeLo == rangeHi == targetByte fails, we need to break out. */
            let last_target_byte = target_byte;

            /* When seeking to a byte, failure probably means we've attempted to seek beyond the end of the stream. To counter this we just halve it each attempt. */
            if !self.bs.seek_to_byte(target_byte) {
                /* If we couldn't even seek to the first byte in the stream we have a problem. Just abandon the whole thing. */
                if target_byte == 0 {
                    self.seek_to_first_frame(); /* Try to recover. */
                    return false;
                }

                /* Halve the byte location and continue. */
                target_byte = range_lo.wrapping_add(range_hi.wrapping_sub(range_lo) / 2);
                range_hi = target_byte;
            } else {
                /* Getting here should mean that we have seeked to an appropriate byte. */

                /* Clear the details of the FLAC frame so we don't misreport data. */
                self.current_flac_frame = Frame::default();

                /*
                Now seek to the next FLAC frame. We need to decode the entire frame (not just the header) because it's possible for the header to incorrectly pass the
                CRC check and return bad data. We need to decode the entire frame to be more certain. Although this seems unlikely, this has happened to me in testing
                so it needs to stay this way for now.
                */
                if !self.read_and_decode_next_flac_frame() {
                    /* Halve the byte location and continue. */
                    target_byte = range_lo.wrapping_add(range_hi.wrapping_sub(range_lo) / 2);
                    range_hi = target_byte;
                } else {
                    break;
                }
            }

            /* We already tried this byte and there are no more to try, break out. */
            if target_byte == last_target_byte {
                return false;
            }
        }

        /* The current PCM frame needs to be updated based on the frame we just seeked to. */
        self.current_pcm_frame = self.get_pcm_frame_range_of_current_flac_frame().0;

        debug_assert!(target_byte <= range_hi);

        *last_successful_seek_offset = target_byte;
        true
    }

    /// Translation of `drflac__decode_flac_frame_and_seek_forward_by_pcm_frames()`.
    fn decode_flac_frame_and_seek_forward_by_pcm_frames(&mut self, offset: u64) -> bool {
        self.seek_forward_by_pcm_frames(offset) == offset
    }

    /// Translation of `drflac__seek_to_pcm_frame__binary_search_internal()`.
    fn seek_to_pcm_frame_binary_search_internal(
        &mut self,
        pcm_frame_index: u64,
        mut byte_range_lo: u64,
        mut byte_range_hi: u64,
    ) -> bool {
        /* This assumes pFlac->currentPCMFrame is sitting on byteRangeLo upon entry. */

        let mut pcm_range_lo = self.total_pcm_frame_count;
        let mut last_successful_seek_offset: u64 = u64::MAX;
        let mut closest_seek_offset_before_target_pcm_frame = byte_range_lo;
        let seek_forward_threshold: u32 = if self.max_block_size_in_pcm_frames != 0 {
            self.max_block_size_in_pcm_frames as u32 * 2
        } else {
            4096
        };

        let bits = |frames: u64, flac: &Drflac<'_>| -> i64 {
            frames
                .wrapping_mul(flac.channels as u64)
                .wrapping_mul(flac.bits_per_sample as u64) as i64
        };

        let mut target_byte = byte_range_lo.wrapping_add(f64_to_u64(
            ((bits(pcm_frame_index.wrapping_sub(self.current_pcm_frame), self) as f32 / 8.0f32)
                * DRFLAC_BINARY_SEARCH_APPROX_COMPRESSION_RATIO) as f64,
        ));
        if target_byte > byte_range_hi {
            target_byte = byte_range_hi;
        }

        loop {
            /*
            If only two adjacent byte offsets remain, binary search cannot narrow the range any further. Seek to the closest frame before the target and decode
            forward from there.
            */
            if byte_range_hi.wrapping_sub(byte_range_lo) == 1 {
                if !self.seek_to_approximate_flac_frame_to_byte(
                    closest_seek_offset_before_target_pcm_frame,
                    closest_seek_offset_before_target_pcm_frame,
                    byte_range_hi,
                    &mut last_successful_seek_offset,
                ) {
                    break;
                }

                if self.current_pcm_frame <= pcm_frame_index
                    && self.decode_flac_frame_and_seek_forward_by_pcm_frames(
                        pcm_frame_index - self.current_pcm_frame,
                    )
                {
                    return true;
                }

                break;
            }

            if self.seek_to_approximate_flac_frame_to_byte(
                target_byte,
                byte_range_lo,
                byte_range_hi,
                &mut last_successful_seek_offset,
            ) {
                /* We found a FLAC frame. We need to check if it contains the sample we're looking for. */
                let (new_pcm_range_lo, new_pcm_range_hi) =
                    self.get_pcm_frame_range_of_current_flac_frame();

                /* If we selected the same frame, it means we should be pretty close. Just decode the rest. */
                if pcm_range_lo == new_pcm_range_lo {
                    if !self.seek_to_approximate_flac_frame_to_byte(
                        closest_seek_offset_before_target_pcm_frame,
                        closest_seek_offset_before_target_pcm_frame,
                        byte_range_hi,
                        &mut last_successful_seek_offset,
                    ) {
                        break; /* Failed to seek to closest frame. */
                    }

                    if self.decode_flac_frame_and_seek_forward_by_pcm_frames(
                        pcm_frame_index.wrapping_sub(self.current_pcm_frame),
                    ) {
                        return true;
                    } else {
                        break; /* Failed to seek forward. */
                    }
                }

                pcm_range_lo = new_pcm_range_lo;
                let pcm_range_hi = new_pcm_range_hi;

                if pcm_range_lo <= pcm_frame_index && pcm_range_hi >= pcm_frame_index {
                    /* The target PCM frame is in this FLAC frame. */
                    if self.decode_flac_frame_and_seek_forward_by_pcm_frames(
                        pcm_frame_index.wrapping_sub(self.current_pcm_frame),
                    ) {
                        return true;
                    } else {
                        break; /* Failed to seek to FLAC frame. */
                    }
                } else if pcm_range_lo > pcm_frame_index {
                    /* We seeked too far forward. We need to move our target byte backward and try again. */
                    byte_range_hi = last_successful_seek_offset;
                    if byte_range_lo > byte_range_hi {
                        byte_range_lo = byte_range_hi;
                    }

                    target_byte =
                        byte_range_lo.wrapping_add(byte_range_hi.wrapping_sub(byte_range_lo) / 2);
                    if target_byte < byte_range_lo {
                        target_byte = byte_range_lo;
                    }
                } else
                /*if (pcmRangeHi < pcmFrameIndex)*/
                {
                    /* We didn't seek far enough. We need to move our target byte forward and try again. */

                    /* If we're close enough we can just seek forward. */
                    if pcm_frame_index.wrapping_sub(pcm_range_lo) < seek_forward_threshold as u64 {
                        if self.decode_flac_frame_and_seek_forward_by_pcm_frames(
                            pcm_frame_index.wrapping_sub(self.current_pcm_frame),
                        ) {
                            return true;
                        } else {
                            break; /* Failed to seek to FLAC frame. */
                        }
                    } else {
                        let approx_compression_ratio: f64 = last_successful_seek_offset
                            .wrapping_sub(self.first_flac_frame_pos_in_bytes)
                            as i64
                            as f64
                            / (bits(pcm_range_lo, self) as f64 / 8.0);

                        byte_range_lo = last_successful_seek_offset;
                        if byte_range_hi < byte_range_lo {
                            byte_range_hi = byte_range_lo;
                        }

                        target_byte = last_successful_seek_offset.wrapping_add(f64_to_u64(
                            (bits(pcm_frame_index.wrapping_sub(pcm_range_lo), self) as f64 / 8.0)
                                * approx_compression_ratio,
                        ));
                        if target_byte > byte_range_hi {
                            target_byte = byte_range_hi;
                        }

                        if closest_seek_offset_before_target_pcm_frame < last_successful_seek_offset
                        {
                            closest_seek_offset_before_target_pcm_frame =
                                last_successful_seek_offset;
                        }
                    }
                }
            } else {
                /* Getting here is really bad. We just recover as best we can, but moving to the first frame in the stream, and then abort. */
                break;
            }
        }

        self.seek_to_first_frame(); /* <-- Try to recover. */
        false
    }

    /// Translation of `drflac__seek_to_pcm_frame__binary_search()`.
    fn seek_to_pcm_frame_binary_search(&mut self, pcm_frame_index: u64) -> bool {
        let seek_forward_threshold: u32 = if self.max_block_size_in_pcm_frames != 0 {
            self.max_block_size_in_pcm_frames as u32 * 2
        } else {
            4096
        };

        /* Our algorithm currently assumes the FLAC stream is currently sitting at the start. */
        if !self.seek_to_first_frame() {
            return false;
        }

        /* If we're close enough to the start, just move to the start and seek forward. */
        if pcm_frame_index < seek_forward_threshold as u64 {
            return self.seek_forward_by_pcm_frames(pcm_frame_index) == pcm_frame_index;
        }

        /*
        Our starting byte range is the byte position of the first FLAC frame and the approximate end of the file as if it were completely uncompressed. This ensures
        the entire file is included, even though most of the time it'll exceed the end of the actual stream. This is OK as the frame searching logic will handle it.
        */
        let byte_range_lo = self.first_flac_frame_pos_in_bytes;
        let byte_range_hi = self.first_flac_frame_pos_in_bytes.wrapping_add(f64_to_u64(
            (self
                .total_pcm_frame_count
                .wrapping_mul(self.channels as u64)
                .wrapping_mul(self.bits_per_sample as u64) as i64 as f32
                / 8.0f32) as f64,
        ));

        self.seek_to_pcm_frame_binary_search_internal(pcm_frame_index, byte_range_lo, byte_range_hi)
    }

    /// Translation of `drflac__seek_to_pcm_frame__seek_table()`.
    fn seek_to_pcm_frame_seek_table(&mut self, pcm_frame_index: u64) -> bool {
        let mut i_closest_seekpoint: usize = 0;
        let mut is_mid_frame = false;
        let running_pcm_frame_count;

        let Some(seekpoints) = self.seekpoints.as_ref().filter(|s| !s.is_empty()) else {
            return false;
        };
        let seekpoint_count = seekpoints.len();

        /* Do not use the seektable if pcmFramIndex is not coverd by it. */
        if seekpoints[0].first_pcm_frame > pcm_frame_index {
            return false;
        }

        for i_seekpoint in 0..seekpoint_count {
            if seekpoints[i_seekpoint].first_pcm_frame >= pcm_frame_index {
                break;
            }

            i_closest_seekpoint = i_seekpoint;
        }

        let closest = seekpoints[i_closest_seekpoint];
        let next = seekpoints.get(i_closest_seekpoint + 1).copied();

        /* There's been cases where the seek table contains only zeros. We need to do some basic validation on the closest seekpoint. */
        if closest.pcm_frame_count == 0
            || closest.pcm_frame_count > self.max_block_size_in_pcm_frames
        {
            return false;
        }
        if closest.first_pcm_frame > self.total_pcm_frame_count && self.total_pcm_frame_count > 0 {
            return false;
        }

        /* At this point we should know the closest seek point. We can use a binary search for this. We need to know the total sample count for this. */
        if self.total_pcm_frame_count > 0 {
            let mut byte_range_hi = self.first_flac_frame_pos_in_bytes.wrapping_add(f64_to_u64(
                (self
                    .total_pcm_frame_count
                    .wrapping_mul(self.channels as u64)
                    .wrapping_mul(self.bits_per_sample as u64) as i64 as f32
                    / 8.0f32) as f64,
            ));
            let byte_range_lo = self
                .first_flac_frame_pos_in_bytes
                .wrapping_add(closest.flac_frame_offset);

            /*
            If our closest seek point is not the last one, we only need to search between it and the next one. The section below calculates an appropriate starting
            value for byteRangeHi which will clamp it appropriately.

            Note that the next seekpoint must have an offset greater than the closest seekpoint because otherwise our binary search algorithm will break down. There
            have been cases where a seektable consists of seek points where every byte offset is set to 0 which causes problems. If this happens we need to abort.
            */
            if let Some(next) = next {
                /* Basic validation on the seekpoints to ensure they're usable. */
                if closest.flac_frame_offset >= next.flac_frame_offset || next.pcm_frame_count == 0
                {
                    return false; /* The next seekpoint doesn't look right. The seek table cannot be trusted from here. Abort. */
                }

                if next.first_pcm_frame != u64::MAX {
                    /* Make sure it's not a placeholder seekpoint. */
                    byte_range_hi = self
                        .first_flac_frame_pos_in_bytes
                        .wrapping_add(next.flac_frame_offset)
                        .wrapping_sub(1); /* byteRangeHi must be zero based. */
                }
            }

            if self.bs.seek_to_byte(
                self.first_flac_frame_pos_in_bytes
                    .wrapping_add(closest.flac_frame_offset),
            ) && read_next_flac_frame_header(
                &mut self.bs,
                self.bits_per_sample,
                &mut self.current_flac_frame.header,
            ) {
                self.current_pcm_frame = self.get_pcm_frame_range_of_current_flac_frame().0;

                if self.seek_to_pcm_frame_binary_search_internal(
                    pcm_frame_index,
                    byte_range_lo,
                    byte_range_hi,
                ) {
                    return true;
                }
            }
        }

        /* Getting here means we need to use a slower algorithm because the binary search method failed or cannot be used. */

        /*
        If we are seeking forward and the closest seekpoint is _before_ the current sample, we just seek forward from where we are. Otherwise we start seeking
        from the seekpoint's first sample.
        */
        if pcm_frame_index >= self.current_pcm_frame
            && closest.first_pcm_frame <= self.current_pcm_frame
        {
            /* Optimized case. Just seek forward from where we are. */
            running_pcm_frame_count = self.current_pcm_frame;

            /* The frame header for the first frame may not yet have been read. We need to do that if necessary. */
            if self.current_pcm_frame == 0 && self.current_flac_frame.pcm_frames_remaining == 0 {
                if !read_next_flac_frame_header(
                    &mut self.bs,
                    self.bits_per_sample,
                    &mut self.current_flac_frame.header,
                ) {
                    return false;
                }
            } else {
                is_mid_frame = true;
            }
        } else {
            /* Slower case. Seek to the start of the seekpoint and then seek forward from there. */
            running_pcm_frame_count = closest.first_pcm_frame;

            if !self.bs.seek_to_byte(
                self.first_flac_frame_pos_in_bytes
                    .wrapping_add(closest.flac_frame_offset),
            ) {
                return false;
            }

            /* Grab the frame the seekpoint is sitting on in preparation for the sample-exact seeking below. */
            if !read_next_flac_frame_header(
                &mut self.bs,
                self.bits_per_sample,
                &mut self.current_flac_frame.header,
            ) {
                return false;
            }
        }

        self.seek_frame_by_frame(pcm_frame_index, running_pcm_frame_count, is_mid_frame)
    }
}

/// Translation of `drflac_ogg_page_header`.
#[derive(Clone, Copy)]
struct OggPageHeader {
    capture_pattern: [u8; 4], /* Should be "OggS" */
    structure_version: u8,    /* Always 0. */
    header_type: u8,
    granule_position: u64,
    serial_number: u32,
    sequence_number: u32,
    checksum: u32,
    segment_count: u8,
    segment_table: [u8; 255],
}

impl Default for OggPageHeader {
    fn default() -> Self {
        OggPageHeader {
            capture_pattern: [0; 4],
            structure_version: 0,
            header_type: 0,
            granule_position: 0,
            serial_number: 0,
            sequence_number: 0,
            checksum: 0,
            segment_count: 0,
            segment_table: [0; 255],
        }
    }
}

/// Translation of `drflac_init_info` (without what only the "relaxed"
/// opening uses).
#[derive(Default)]
struct InitInfo {
    container: Container,
    sample_rate: u32,
    channels: u8,
    bits_per_sample: u8,
    total_pcm_frame_count: u64,
    max_block_size_in_pcm_frames: u16,
    running_file_pos: u64,
    has_stream_info_block: bool,
    has_metadata_blocks: bool,

    ogg_serial: u32,
    ogg_first_byte_pos: u64,
    ogg_bos_header: OggPageHeader,
}

/// Translation of `drflac__decode_block_header()`.
fn decode_block_header(
    block_header: u32,
    is_last_block: &mut u8,
    block_type: &mut u8,
    block_size: &mut u32,
) {
    // (`blockHeader` is the bytes as read, big endian.)
    *is_last_block = ((block_header & 0x80000000) >> 31) as u8;
    *block_type = ((block_header & 0x7F000000) >> 24) as u8;
    *block_size = block_header & 0x00FFFFFF;
}

/// Translation of `drflac__read_and_decode_block_header()`.
fn read_and_decode_block_header(
    cb: &mut dyn Callbacks,
    is_last_block: &mut u8,
    block_type: &mut u8,
    block_size: &mut u32,
) -> bool {
    let mut block_header = [0u8; 4];

    *block_size = 0;
    if cb.on_read(&mut block_header) != 4 {
        return false;
    }

    decode_block_header(
        u32::from_be_bytes(block_header),
        is_last_block,
        block_type,
        block_size,
    );
    true
}

/// Translation of `drflac__read_streaminfo()`.
fn read_streaminfo(cb: &mut dyn Callbacks, stream_info: &mut Streaminfo) -> bool {
    let mut block_sizes = [0u8; 4];
    let mut frame_sizes = [0u8; 8];
    let mut important_props = [0u8; 8];
    let mut md5 = [0u8; 16];

    /* min/max block size. */
    if cb.on_read(&mut block_sizes) != 4 {
        return false;
    }

    /* min/max frame size. */
    if cb.on_read(&mut frame_sizes[..6]) != 6 {
        return false;
    }

    /* Sample rate, channels, bits per sample and total sample count. */
    if cb.on_read(&mut important_props) != 8 {
        return false;
    }

    /* MD5 */
    if cb.on_read(&mut md5) != md5.len() {
        return false;
    }

    let block_sizes = u32::from_be_bytes(block_sizes);
    let frame_sizes = u64::from_be_bytes(frame_sizes);
    let important_props = u64::from_be_bytes(important_props);

    stream_info.min_block_size_in_pcm_frames = ((block_sizes & 0xFFFF0000) >> 16) as u16;
    stream_info.max_block_size_in_pcm_frames = (block_sizes & 0x0000FFFF) as u16;
    stream_info.min_frame_size_in_pcm_frames =
        ((frame_sizes & ((0x00FFFFFFu64 << 16) << 24)) >> 40) as u32;
    stream_info.max_frame_size_in_pcm_frames = ((frame_sizes & (0x00FFFFFFu64 << 16)) >> 16) as u32;
    stream_info.sample_rate = ((important_props & ((0x000FFFFFu64 << 16) << 28)) >> 44) as u32;
    stream_info.channels = (((important_props & ((0x0000000Eu64 << 16) << 24)) >> 41) + 1) as u8;
    stream_info.bits_per_sample =
        (((important_props & ((0x0000001Fu64 << 16) << 20)) >> 36) + 1) as u8;
    stream_info.total_pcm_frame_count =
        important_props & (((0x0000000Fu64 << 16) << 16) | 0xFFFFFFFF);
    stream_info.md5 = md5;

    true
}

/// Read exactly `len` bytes into a new buffer (an allocation and an
/// `onRead()` upstream); `None` if the allocation or the read fails.
fn read_block(cb: &mut dyn Callbacks, len: usize) -> Option<Vec<u8>> {
    let mut data = try_alloc::<u8>(len).ok()?;
    if cb.on_read(&mut data) != len {
        return None;
    }
    Some(data)
}

/// `drflac__le2host_32_ptr_unaligned()`
fn le32(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}

/// Translation of `drflac__read_and_decode_metadata()`.
fn read_and_decode_metadata(
    cb: &mut dyn Callbacks,
    mut on_meta: Option<&mut MetaProc<'_>>,
    first_frame_pos: &mut u64,
    seektable_pos_out: &mut u64,
    seekpoint_count_out: &mut u32,
) -> bool {
    /*
    We want to keep track of the byte position in the stream of the seektable. At the time of calling this function we know that
    we'll be sitting on byte 42.
    */
    let mut running_file_pos: u64 = 42;
    let mut seektable_pos: u64 = 0;
    let mut seektable_size: u32 = 0;
    let mut file_size: i64 = 0;
    let mut has_known_file_size = false;

    /* We'll be doing some memory allocations here against untrusted data. We'll do a basic validation check that they don't exceed the size of the file. */
    if cb.on_seek(0, SeekOrigin::End) {
        if cb.on_tell(&mut file_size) {
            has_known_file_size = true;
        }

        cb.on_seek(running_file_pos as i32, SeekOrigin::Set); /* Safe cast because runningFilePos should always be 42 at this point. */
    }

    loop {
        let mut is_last_block = 0u8;
        let mut block_type = 0u8;
        let mut block_size = 0u32;
        if !read_and_decode_block_header(cb, &mut is_last_block, &mut block_type, &mut block_size) {
            return false;
        }

        if has_known_file_size
            && (block_size as u64 > (file_size as u64).wrapping_sub(running_file_pos))
        {
            return false; /* Block size exceeds the size of the file. */
        }

        running_file_pos += 4;

        match block_type {
            DRFLAC_METADATA_BLOCK_TYPE_APPLICATION => {
                if block_size < 4 {
                    return false;
                }

                if let Some(on_meta) = on_meta.as_deref_mut() {
                    if read_block(cb, block_size as usize).is_none() {
                        return false;
                    }
                    on_meta(block_type, None);
                }
            }

            DRFLAC_METADATA_BLOCK_TYPE_SEEKTABLE => {
                seektable_pos = running_file_pos;
                seektable_size = block_size;

                if let Some(on_meta) = on_meta.as_deref_mut() {
                    let seekpoint_count = block_size / DRFLAC_SEEKPOINT_SIZE_IN_BYTES;

                    if try_alloc::<Seekpoint>(seekpoint_count as usize).is_err() {
                        return false;
                    }

                    /* We need to read seekpoint by seekpoint and do some processing. */
                    for _ in 0..seekpoint_count {
                        let mut seekpoint = [0u8; DRFLAC_SEEKPOINT_SIZE_IN_BYTES as usize];
                        if cb.on_read(&mut seekpoint) != DRFLAC_SEEKPOINT_SIZE_IN_BYTES as usize {
                            return false;
                        }
                    }

                    on_meta(block_type, None);
                }
            }

            DRFLAC_METADATA_BLOCK_TYPE_VORBIS_COMMENT => {
                if block_size < 8 {
                    return false;
                }

                if let Some(on_meta) = on_meta.as_deref_mut() {
                    let Some(raw_data) = read_block(cb, block_size as usize) else {
                        return false;
                    };

                    let running_data_end = block_size as i64;
                    let mut running_data: i64 = 0;

                    let vendor_length = le32(&raw_data, 0);
                    running_data += 4;

                    /* Need space for the rest of the block */
                    if (running_data_end - running_data) - 4 < vendor_length as i64 {
                        /* <-- Note the order of operations to avoid overflow to a valid value */
                        return false;
                    }
                    let vendor = &raw_data
                        [running_data as usize..running_data as usize + vendor_length as usize];
                    running_data += vendor_length as i64;
                    let comment_count = le32(&raw_data, running_data as usize);
                    running_data += 4;

                    /* Need space for 'commentCount' comments after the block, which at minimum is a drflac_uint32 per comment */
                    if ((running_data_end - running_data) as u64 / 4) < comment_count as u64 {
                        /* <-- Note the order of operations to avoid overflow to a valid value */
                        return false;
                    }
                    let comments_start = running_data as usize;

                    /* Check that the comments section is valid before passing it to the callback */
                    for _ in 0..comment_count {
                        if running_data_end - running_data < 4 {
                            return false;
                        }

                        let comment_length = le32(&raw_data, running_data as usize);
                        running_data += 4;
                        if running_data_end - running_data < comment_length as i64 {
                            /* <-- Note the order of operations to avoid overflow to a valid value */
                            return false;
                        }
                        running_data += comment_length as i64;
                    }

                    on_meta(
                        block_type,
                        Some(&VorbisComment {
                            vendor,
                            comment_count,
                            comments: &raw_data[comments_start..],
                        }),
                    );
                }
            }

            DRFLAC_METADATA_BLOCK_TYPE_CUESHEET => {
                if block_size < 396 {
                    return false;
                }

                if let Some(on_meta) = on_meta.as_deref_mut() {
                    /*
                    This needs to be loaded in two passes. The first pass is used to calculate the size of the memory allocation
                    we need for storing the necessary data. The second pass will fill that buffer with usable data.
                    */
                    let Some(raw_data) = read_block(cb, block_size as usize) else {
                        return false;
                    };

                    let running_data_end = block_size as i64;
                    // (catalog (128), lead-in sample count (8), the CD flag and reserved bytes (259))
                    let mut running_data: i64 = 128 + 8 + 259;
                    let track_count = raw_data[running_data as usize];
                    running_data += 1;

                    /* Pass 1: Calculate the size of the buffer for the track data. */
                    for _ in 0..track_count {
                        if running_data_end - running_data < DRFLAC_CUESHEET_TRACK_SIZE_IN_BYTES {
                            return false;
                        }

                        /* Skip to the index point count */
                        running_data += 35;

                        let index_count = raw_data[running_data as usize];
                        running_data += 1;

                        /* Quick validation check. */
                        let index_point_size =
                            index_count as u32 * DRFLAC_CUESHEET_TRACK_INDEX_SIZE_IN_BYTES;
                        if running_data_end - running_data < index_point_size as i64 {
                            return false;
                        }

                        running_data += index_point_size as i64;
                    }

                    /* Pass 2 (filling the track data) only serves the callback, which isn't given it here. */
                    on_meta(block_type, None);
                }
            }

            DRFLAC_METADATA_BLOCK_TYPE_PICTURE => {
                if block_size < 32 {
                    return false;
                }

                if let Some(on_meta) = on_meta.as_deref_mut() {
                    let result = 'done_flac: {
                        let mut block_size_remaining = block_size;
                        let mut word = [0u8; 4];

                        /* type */
                        if block_size_remaining < 4 || cb.on_read(&mut word) != 4 {
                            break 'done_flac false;
                        }
                        block_size_remaining -= 4;

                        /* mime */
                        if block_size_remaining < 4 || cb.on_read(&mut word) != 4 {
                            break 'done_flac false;
                        }
                        block_size_remaining -= 4;
                        let mime_length = u32::from_be_bytes(word);

                        if block_size_remaining < mime_length {
                            break 'done_flac false;
                        }

                        if read_block(cb, mime_length as usize).is_none() {
                            break 'done_flac false;
                        }
                        block_size_remaining -= mime_length;

                        /* description */
                        if block_size_remaining < 4 || cb.on_read(&mut word) != 4 {
                            break 'done_flac false;
                        }
                        block_size_remaining -= 4;
                        let description_length = u32::from_be_bytes(word);

                        if block_size_remaining < description_length {
                            break 'done_flac false;
                        }

                        if read_block(cb, description_length as usize).is_none() {
                            break 'done_flac false;
                        }
                        block_size_remaining -= description_length;

                        /* width, height, color depth, index color count */
                        for _ in 0..4 {
                            if block_size_remaining < 4 || cb.on_read(&mut word) != 4 {
                                break 'done_flac false;
                            }
                            block_size_remaining -= 4;
                        }

                        /* Picture data. */
                        if block_size_remaining < 4 || cb.on_read(&mut word) != 4 {
                            break 'done_flac false;
                        }
                        block_size_remaining -= 4;
                        let picture_data_size = u32::from_be_bytes(word);

                        if block_size_remaining < picture_data_size {
                            break 'done_flac false;
                        }

                        /* For the actual image data we want to store the offset to the start of the stream. */
                        let picture_data_offset =
                            running_file_pos + (block_size - block_size_remaining) as u64;

                        /*
                        For the allocation of image data, we can allow memory allocation to fail, in which case we just leave
                        the pointer as null. If it fails, we need to fall back to seeking past the image data.
                        */
                        let picture_data = match try_alloc::<u8>(picture_data_size as usize) {
                            Ok(mut data) => {
                                if cb.on_read(&mut data) != picture_data_size as usize {
                                    break 'done_flac false;
                                }
                                Some(data)
                            }
                            Err(_) => {
                                /* Allocation failed. We need to seek past the picture data. */
                                if !cb.on_seek(picture_data_size as i32, SeekOrigin::Cur) {
                                    break 'done_flac false;
                                }
                                None
                            }
                        };

                        /* Only fire the callback if we actually have a way to read the image data. We must have either a valid offset, or a valid data pointer. */
                        if picture_data_offset != 0 || picture_data.is_some() {
                            on_meta(block_type, None);
                        }

                        true
                    };

                    if !result {
                        return false;
                    }
                }
            }

            DRFLAC_METADATA_BLOCK_TYPE_PADDING => {
                if let Some(on_meta) = on_meta.as_deref_mut() {
                    /* Padding doesn't have anything meaningful in it, so just skip over it, but make sure the caller is aware of it by firing the callback. */
                    if !cb.on_seek(block_size as i32, SeekOrigin::Cur) {
                        is_last_block = 1; /* An error occurred while seeking. Attempt to recover by treating this as the last block which will in turn terminate the loop. */
                    } else {
                        on_meta(block_type, None);
                    }
                }
            }

            DRFLAC_METADATA_BLOCK_TYPE_INVALID => {
                /* Invalid chunk. Just skip over this one. */
                if on_meta.is_some() && !cb.on_seek(block_size as i32, SeekOrigin::Cur) {
                    is_last_block = 1; /* An error occurred while seeking. Attempt to recover by treating this as the last block which will in turn terminate the loop. */
                }
            }

            _ => {
                /*
                It's an unknown chunk, but not necessarily invalid. There's a chance more metadata blocks might be defined later on, so we
                can at the very least report the chunk to the application and let it look at the raw data.
                */
                if let Some(on_meta) = on_meta.as_deref_mut() {
                    match try_alloc::<u8>(block_size as usize) {
                        Ok(mut raw_data) => {
                            if cb.on_read(&mut raw_data) != block_size as usize {
                                return false;
                            }
                        }
                        Err(_) => {
                            /* Allocation failed. We need to seek past the block. */
                            if !cb.on_seek(block_size as i32, SeekOrigin::Cur) {
                                return false;
                            }
                        }
                    }

                    on_meta(block_type, None);
                }
            }
        }

        /* If we're not handling metadata, just skip over the block. If we are, it will have been handled earlier in the switch statement above. */
        if on_meta.is_none() && block_size > 0 && !cb.on_seek(block_size as i32, SeekOrigin::Cur) {
            is_last_block = 1;
        }

        running_file_pos += block_size as u64;
        if is_last_block != 0 {
            break;
        }
    }

    *seektable_pos_out = seektable_pos;
    *seekpoint_count_out = seektable_size / DRFLAC_SEEKPOINT_SIZE_IN_BYTES;
    *first_frame_pos = running_file_pos;

    true
}

/// Translation of `drflac__init_private__native()` (the strict opening
/// only: SDL_mixer never asks for the relaxed one).
fn init_private_native(
    init: &mut InitInfo,
    cb: &mut dyn Callbacks,
    on_meta: Option<&mut MetaProc<'_>>,
) -> bool {
    /* Pre Condition: The bit stream should be sitting just past the 4-byte id header. */

    let mut is_last_block = 0u8;
    let mut block_type = 0u8;
    let mut block_size = 0u32;

    init.container = Container::Native;

    /* The first metadata block should be the STREAMINFO block. */
    if !read_and_decode_block_header(cb, &mut is_last_block, &mut block_type, &mut block_size) {
        return false;
    }

    if block_type != DRFLAC_METADATA_BLOCK_TYPE_STREAMINFO || block_size != 34 {
        /* We're opening in strict mode and the first block is not the STREAMINFO block. Error. */
        false
    } else {
        let mut streaminfo = Streaminfo::default();
        if !read_streaminfo(cb, &mut streaminfo) {
            return false;
        }

        init.has_stream_info_block = true;
        init.sample_rate = streaminfo.sample_rate;
        init.channels = streaminfo.channels;
        init.bits_per_sample = streaminfo.bits_per_sample;
        init.total_pcm_frame_count = streaminfo.total_pcm_frame_count;
        init.max_block_size_in_pcm_frames = streaminfo.max_block_size_in_pcm_frames; /* Don't care about the min block size - only the max (used for determining the size of the memory allocation). */
        init.has_metadata_blocks = is_last_block == 0;

        if let Some(on_meta) = on_meta {
            on_meta(DRFLAC_METADATA_BLOCK_TYPE_STREAMINFO, None);
        }

        true
    }
}

const DRFLAC_OGG_MAX_PAGE_SIZE: usize = 65307;
const DRFLAC_OGG_CAPTURE_PATTERN_CRC32: u32 = 1605413199; /* CRC-32 of "OggS". */

/// Translation of `drflac_ogg_crc_mismatch_recovery`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OggCrcMismatchRecovery {
    RecoverOnCrcMismatch,
    FailOnCrcMismatch,
}

static DRFLAC_CRC32_TABLE: [u32; 256] = [
    0x00000000, 0x04C11DB7, 0x09823B6E, 0x0D4326D9, 0x130476DC, 0x17C56B6B, 0x1A864DB2, 0x1E475005,
    0x2608EDB8, 0x22C9F00F, 0x2F8AD6D6, 0x2B4BCB61, 0x350C9B64, 0x31CD86D3, 0x3C8EA00A, 0x384FBDBD,
    0x4C11DB70, 0x48D0C6C7, 0x4593E01E, 0x4152FDA9, 0x5F15ADAC, 0x5BD4B01B, 0x569796C2, 0x52568B75,
    0x6A1936C8, 0x6ED82B7F, 0x639B0DA6, 0x675A1011, 0x791D4014, 0x7DDC5DA3, 0x709F7B7A, 0x745E66CD,
    0x9823B6E0, 0x9CE2AB57, 0x91A18D8E, 0x95609039, 0x8B27C03C, 0x8FE6DD8B, 0x82A5FB52, 0x8664E6E5,
    0xBE2B5B58, 0xBAEA46EF, 0xB7A96036, 0xB3687D81, 0xAD2F2D84, 0xA9EE3033, 0xA4AD16EA, 0xA06C0B5D,
    0xD4326D90, 0xD0F37027, 0xDDB056FE, 0xD9714B49, 0xC7361B4C, 0xC3F706FB, 0xCEB42022, 0xCA753D95,
    0xF23A8028, 0xF6FB9D9F, 0xFBB8BB46, 0xFF79A6F1, 0xE13EF6F4, 0xE5FFEB43, 0xE8BCCD9A, 0xEC7DD02D,
    0x34867077, 0x30476DC0, 0x3D044B19, 0x39C556AE, 0x278206AB, 0x23431B1C, 0x2E003DC5, 0x2AC12072,
    0x128E9DCF, 0x164F8078, 0x1B0CA6A1, 0x1FCDBB16, 0x018AEB13, 0x054BF6A4, 0x0808D07D, 0x0CC9CDCA,
    0x7897AB07, 0x7C56B6B0, 0x71159069, 0x75D48DDE, 0x6B93DDDB, 0x6F52C06C, 0x6211E6B5, 0x66D0FB02,
    0x5E9F46BF, 0x5A5E5B08, 0x571D7DD1, 0x53DC6066, 0x4D9B3063, 0x495A2DD4, 0x44190B0D, 0x40D816BA,
    0xACA5C697, 0xA864DB20, 0xA527FDF9, 0xA1E6E04E, 0xBFA1B04B, 0xBB60ADFC, 0xB6238B25, 0xB2E29692,
    0x8AAD2B2F, 0x8E6C3698, 0x832F1041, 0x87EE0DF6, 0x99A95DF3, 0x9D684044, 0x902B669D, 0x94EA7B2A,
    0xE0B41DE7, 0xE4750050, 0xE9362689, 0xEDF73B3E, 0xF3B06B3B, 0xF771768C, 0xFA325055, 0xFEF34DE2,
    0xC6BCF05F, 0xC27DEDE8, 0xCF3ECB31, 0xCBFFD686, 0xD5B88683, 0xD1799B34, 0xDC3ABDED, 0xD8FBA05A,
    0x690CE0EE, 0x6DCDFD59, 0x608EDB80, 0x644FC637, 0x7A089632, 0x7EC98B85, 0x738AAD5C, 0x774BB0EB,
    0x4F040D56, 0x4BC510E1, 0x46863638, 0x42472B8F, 0x5C007B8A, 0x58C1663D, 0x558240E4, 0x51435D53,
    0x251D3B9E, 0x21DC2629, 0x2C9F00F0, 0x285E1D47, 0x36194D42, 0x32D850F5, 0x3F9B762C, 0x3B5A6B9B,
    0x0315D626, 0x07D4CB91, 0x0A97ED48, 0x0E56F0FF, 0x1011A0FA, 0x14D0BD4D, 0x19939B94, 0x1D528623,
    0xF12F560E, 0xF5EE4BB9, 0xF8AD6D60, 0xFC6C70D7, 0xE22B20D2, 0xE6EA3D65, 0xEBA91BBC, 0xEF68060B,
    0xD727BBB6, 0xD3E6A601, 0xDEA580D8, 0xDA649D6F, 0xC423CD6A, 0xC0E2D0DD, 0xCDA1F604, 0xC960EBB3,
    0xBD3E8D7E, 0xB9FF90C9, 0xB4BCB610, 0xB07DABA7, 0xAE3AFBA2, 0xAAFBE615, 0xA7B8C0CC, 0xA379DD7B,
    0x9B3660C6, 0x9FF77D71, 0x92B45BA8, 0x9675461F, 0x8832161A, 0x8CF30BAD, 0x81B02D74, 0x857130C3,
    0x5D8A9099, 0x594B8D2E, 0x5408ABF7, 0x50C9B640, 0x4E8EE645, 0x4A4FFBF2, 0x470CDD2B, 0x43CDC09C,
    0x7B827D21, 0x7F436096, 0x7200464F, 0x76C15BF8, 0x68860BFD, 0x6C47164A, 0x61043093, 0x65C52D24,
    0x119B4BE9, 0x155A565E, 0x18197087, 0x1CD86D30, 0x029F3D35, 0x065E2082, 0x0B1D065B, 0x0FDC1BEC,
    0x3793A651, 0x3352BBE6, 0x3E119D3F, 0x3AD08088, 0x2497D08D, 0x2056CD3A, 0x2D15EBE3, 0x29D4F654,
    0xC5A92679, 0xC1683BCE, 0xCC2B1D17, 0xC8EA00A0, 0xD6AD50A5, 0xD26C4D12, 0xDF2F6BCB, 0xDBEE767C,
    0xE3A1CBC1, 0xE760D676, 0xEA23F0AF, 0xEEE2ED18, 0xF0A5BD1D, 0xF464A0AA, 0xF9278673, 0xFDE69BC4,
    0x89B8FD09, 0x8D79E0BE, 0x803AC667, 0x84FBDBD0, 0x9ABC8BD5, 0x9E7D9662, 0x933EB0BB, 0x97FFAD0C,
    0xAFB010B1, 0xAB710D06, 0xA6322BDF, 0xA2F33668, 0xBCB4666D, 0xB8757BDA, 0xB5365D03, 0xB1F740B4,
];

/// Translation of `drflac_crc32_byte()`.
fn crc32_byte(crc32: u32, data: u8) -> u32 {
    (crc32 << 8) ^ DRFLAC_CRC32_TABLE[(((crc32 >> 24) & 0xFF) as u8 ^ data) as usize]
}

/// Translation of `drflac_crc32_buffer()`.
fn crc32_buffer(mut crc32: u32, data: &[u8]) -> u32 {
    /* This can be optimized. */
    for &b in data {
        crc32 = crc32_byte(crc32, b);
    }
    crc32
}

/// Translation of `drflac_ogg__is_capture_pattern()`.
fn ogg_is_capture_pattern(pattern: &[u8; 4]) -> bool {
    pattern == b"OggS"
}

/// Translation of `drflac_ogg__get_page_header_size()`.
fn ogg_get_page_header_size(header: &OggPageHeader) -> u32 {
    27 + header.segment_count as u32
}

/// Translation of `drflac_ogg__get_page_body_size()`.
fn ogg_get_page_body_size(header: &OggPageHeader) -> u32 {
    let mut page_body_size = 0u32;
    for i in 0..header.segment_count as usize {
        page_body_size += header.segment_table[i] as u32;
    }

    page_body_size
}

/// Translation of `drflac_ogg__read_page_header_after_capture_pattern()`.
fn ogg_read_page_header_after_capture_pattern(
    cb: &mut dyn Callbacks,
    header: &mut OggPageHeader,
    bytes_read: &mut u32,
    crc32: &mut u32,
) -> DrflacResult {
    let mut data = [0u8; 23];

    debug_assert!(*crc32 == DRFLAC_OGG_CAPTURE_PATTERN_CRC32);

    if cb.on_read(&mut data) != 23 {
        return DRFLAC_AT_END;
    }
    *bytes_read += 23;

    /*
    It's not actually used, but set the capture pattern to 'OggS' for completeness. Not doing this will cause static analysers to complain about
    us trying to access uninitialized data. We could alternatively just comment out this member of the drflac_ogg_page_header structure, but I
    like to have it map to the structure of the underlying data.
    */
    header.capture_pattern = *b"OggS";

    header.structure_version = data[0];
    header.header_type = data[1];
    header.granule_position = u64::from_le_bytes(data[2..10].try_into().unwrap());
    header.serial_number = u32::from_le_bytes(data[10..14].try_into().unwrap());
    header.sequence_number = u32::from_le_bytes(data[14..18].try_into().unwrap());
    header.checksum = u32::from_le_bytes(data[18..22].try_into().unwrap());
    header.segment_count = data[22];

    /* Calculate the CRC. Note that for the calculation the checksum part of the page needs to be set to 0. */
    data[18] = 0;
    data[19] = 0;
    data[20] = 0;
    data[21] = 0;

    for i in 0..23 {
        *crc32 = crc32_byte(*crc32, data[i]);
    }

    let segment_count = header.segment_count as usize;
    if cb.on_read(&mut header.segment_table[..segment_count]) != segment_count {
        return DRFLAC_AT_END;
    }
    *bytes_read += segment_count as u32;

    for i in 0..segment_count {
        *crc32 = crc32_byte(*crc32, header.segment_table[i]);
    }

    DRFLAC_SUCCESS
}

/// Translation of `drflac_ogg__read_page_header()`.
fn ogg_read_page_header(
    cb: &mut dyn Callbacks,
    header: &mut OggPageHeader,
    bytes_read: &mut u32,
    crc32: &mut u32,
) -> DrflacResult {
    let mut id = [0u8; 4];

    *bytes_read = 0;

    if cb.on_read(&mut id) != 4 {
        return DRFLAC_AT_END;
    }
    *bytes_read += 4;

    /* We need to read byte-by-byte until we find the OggS capture pattern. */
    loop {
        if ogg_is_capture_pattern(&id) {
            *crc32 = DRFLAC_OGG_CAPTURE_PATTERN_CRC32;

            // (this never reports a CRC mismatch, which upstream would retry on.)
            return ogg_read_page_header_after_capture_pattern(cb, header, bytes_read, crc32);
        } else {
            /* The first 4 bytes did not equal the capture pattern. Read the next byte and try again. */
            id[0] = id[1];
            id[1] = id[2];
            id[2] = id[3];
            if cb.on_read(&mut id[3..4]) != 1 {
                return DRFLAC_AT_END;
            }
            *bytes_read += 1;
        }
    }
}

/*
The main part of the Ogg encapsulation is the conversion from the physical Ogg bitstream to the native FLAC bitstream. It works
in three general stages: Ogg Physical Bitstream -> Ogg/FLAC Logical Bitstream -> FLAC Native Bitstream. dr_flac is designed
in such a way that the core sections assume everything is delivered in native format. Therefore, for each encapsulation type
dr_flac is supporting there needs to be a layer sitting on top of the onRead and onSeek callbacks that ensures the bits read from
the physical Ogg bitstream are converted and delivered in native FLAC format.
*/
/// Translation of `drflac_oggbs` (oggbs = Ogg Bitstream).
struct OggBs<'a> {
    io: IoStream<'a>, /* The original stream (and callbacks) from drflac_open() and family. */
    current_byte_pos: u64, /* The position of the byte we are sitting on in the physical byte stream. Used for efficient seeking. */
    first_byte_pos: u64, /* The position of the first byte in the physical bitstream. Points to the start of the "OggS" identifier of the FLAC bos page. */
    serial_number: u32, /* The serial number of the FLAC audio pages. This is determined by the initial header page that was read during initialization. */
    _bos_page_header: OggPageHeader, /* Used for seeking. */
    current_page_header: OggPageHeader,
    bytes_remaining_in_page: u32,
    page_data_size: u32,
    page_data: Box<[u8; DRFLAC_OGG_MAX_PAGE_SIZE]>,
}

impl OggBs<'_> {
    /// Translation of `drflac_oggbs__read_physical()`.
    fn read_physical(&mut self, len: usize) -> usize {
        let bytes_actually_read = io_read(&mut self.io, &mut self.page_data[..len]);
        self.current_byte_pos += bytes_actually_read as u64;

        bytes_actually_read
    }

    /// Translation of `drflac_oggbs__seek_physical()`.
    fn seek_physical(&mut self, mut offset: u64, origin: SeekOrigin) -> bool {
        if origin == SeekOrigin::Set {
            if offset <= 0x7FFFFFFF {
                if !io_seek(&mut self.io, offset as i32, SeekOrigin::Set) {
                    return false;
                }
                self.current_byte_pos = offset;

                true
            } else {
                if !io_seek(&mut self.io, 0x7FFFFFFF, SeekOrigin::Set) {
                    return false;
                }
                self.current_byte_pos = offset;

                self.seek_physical(offset - 0x7FFFFFFF, SeekOrigin::Cur)
            }
        } else {
            while offset > 0x7FFFFFFF {
                if !io_seek(&mut self.io, 0x7FFFFFFF, SeekOrigin::Cur) {
                    return false;
                }
                self.current_byte_pos = self.current_byte_pos.wrapping_add(0x7FFFFFFF);
                offset -= 0x7FFFFFFF;
            }

            if !io_seek(&mut self.io, offset as i32, SeekOrigin::Cur) {
                /* <-- Safe cast thanks to the loop above. */
                return false;
            }
            self.current_byte_pos = self.current_byte_pos.wrapping_add(offset);

            true
        }
    }

    /// Translation of `drflac_oggbs__goto_next_page()`.
    fn goto_next_page(&mut self, recovery_method: OggCrcMismatchRecovery) -> bool {
        let mut header = OggPageHeader::default();
        loop {
            let mut crc32 = 0u32;
            let mut bytes_read = 0u32;

            if ogg_read_page_header(&mut self.io, &mut header, &mut bytes_read, &mut crc32)
                != DRFLAC_SUCCESS
            {
                return false;
            }
            self.current_byte_pos += bytes_read as u64;

            let page_body_size = ogg_get_page_body_size(&header);
            if page_body_size as usize > DRFLAC_OGG_MAX_PAGE_SIZE {
                continue; /* Invalid page size. Assume it's corrupted and just move to the next page. */
            }

            if header.serial_number != self.serial_number {
                /* It's not a FLAC page. Skip it. */
                if page_body_size > 0 && !self.seek_physical(page_body_size as u64, SeekOrigin::Cur)
                {
                    return false;
                }
                continue;
            }

            /* We need to read the entire page and then do a CRC check on it. If there's a CRC mismatch we need to skip this page. */
            if self.read_physical(page_body_size as usize) != page_body_size as usize {
                return false;
            }
            self.page_data_size = page_body_size;

            let actual_crc32 = crc32_buffer(crc32, &self.page_data[..self.page_data_size as usize]);
            if actual_crc32 != header.checksum {
                if recovery_method == OggCrcMismatchRecovery::RecoverOnCrcMismatch {
                    continue; /* CRC mismatch. Skip this page. */
                } else {
                    /*
                    Even though we are failing on a CRC mismatch, we still want our stream to be in a good state. Therefore we
                    go to the next valid page to ensure we're in a good state, but return false to let the caller know that the
                    seek did not fully complete.
                    */
                    self.goto_next_page(OggCrcMismatchRecovery::RecoverOnCrcMismatch);
                    return false;
                }
            }

            self.current_page_header = header;
            self.bytes_remaining_in_page = page_body_size;
            return true;
        }
    }
}

impl Callbacks for OggBs<'_> {
    /// Translation of `drflac__on_read_ogg()`.
    fn on_read(&mut self, buffer_out: &mut [u8]) -> usize {
        let bytes_to_read = buffer_out.len();
        let mut bytes_read: usize = 0;

        /* Reading is done page-by-page. If we've run out of bytes in the page we need to move to the next one. */
        while bytes_read < bytes_to_read {
            let bytes_remaining_to_read = bytes_to_read - bytes_read;
            let page_pos = (self.page_data_size - self.bytes_remaining_in_page) as usize;

            if self.bytes_remaining_in_page as usize >= bytes_remaining_to_read {
                buffer_out[bytes_read..bytes_to_read]
                    .copy_from_slice(&self.page_data[page_pos..page_pos + bytes_remaining_to_read]);
                bytes_read += bytes_remaining_to_read;
                self.bytes_remaining_in_page -= bytes_remaining_to_read as u32;
                break;
            }

            /* If we get here it means some of the requested data is contained in the next pages. */
            if self.bytes_remaining_in_page > 0 {
                let n = self.bytes_remaining_in_page as usize;
                buffer_out[bytes_read..bytes_read + n]
                    .copy_from_slice(&self.page_data[page_pos..page_pos + n]);
                bytes_read += n;
                self.bytes_remaining_in_page = 0;
            }

            debug_assert!(bytes_remaining_to_read > 0);
            if !self.goto_next_page(OggCrcMismatchRecovery::RecoverOnCrcMismatch) {
                break; /* Failed to go to the next page. Might have simply hit the end of the stream. */
            }
        }

        bytes_read
    }

    /// Translation of `drflac__on_seek_ogg()`.
    fn on_seek(&mut self, offset: i32, origin: SeekOrigin) -> bool {
        let mut bytes_seeked: i32 = 0;

        debug_assert!(offset >= 0); /* <-- Never seek backwards. */

        /* Seeking is always forward which makes things a lot simpler. */
        if origin == SeekOrigin::Set {
            if !self.seek_physical(self.first_byte_pos as i32 as i64 as u64, SeekOrigin::Set) {
                return false;
            }

            if !self.goto_next_page(OggCrcMismatchRecovery::FailOnCrcMismatch) {
                return false;
            }

            return self.on_seek(offset, SeekOrigin::Cur);
        } else if origin == SeekOrigin::Cur {
            while bytes_seeked < offset {
                let bytes_remaining_to_seek = offset - bytes_seeked;
                debug_assert!(bytes_remaining_to_seek >= 0);

                if self.bytes_remaining_in_page as usize >= bytes_remaining_to_seek as usize {
                    self.bytes_remaining_in_page -= bytes_remaining_to_seek as u32;
                    break;
                }

                /* If we get here it means some of the requested data is contained in the next pages. */
                if self.bytes_remaining_in_page > 0 {
                    bytes_seeked += self.bytes_remaining_in_page as i32;
                    self.bytes_remaining_in_page = 0;
                }

                debug_assert!(bytes_remaining_to_seek > 0);
                if !self.goto_next_page(OggCrcMismatchRecovery::FailOnCrcMismatch) {
                    /* Failed to go to the next page. We either hit the end of the stream or had a CRC mismatch. */
                    return false;
                }
            }
        } else if origin == SeekOrigin::End {
            /* Seeking to the end is not supported. */
            return false;
        }

        true
    }

    /// Translation of `drflac__on_tell_ogg()`.
    fn on_tell(&mut self, _cursor: &mut i64) -> bool {
        /*
        Not implemented for Ogg containers because we don't currently track the byte position of the logical bitstream. To support this, we'll need
        to track the position in drflac__on_read_ogg and drflac__on_seek_ogg.
        */
        false
    }
}

impl<'a> Drflac<'a> {
    /// The Ogg layer under the bit streamer.
    fn oggbs(&mut self) -> &mut OggBs<'a> {
        match &mut self.bs.src {
            Source::Ogg(oggbs) => oggbs,
            Source::Native(_) => unreachable!("not an Ogg stream"),
        }
    }

    /// Translation of `drflac_ogg__seek_to_pcm_frame()`.
    fn ogg_seek_to_pcm_frame(&mut self, pcm_frame_index: u64) -> bool {
        let original_byte_pos = self.oggbs().current_byte_pos; /* For recovery. Points to the OggS identifier. */

        /* First seek to the first frame. */
        if !self.bs.seek_to_byte(self.first_flac_frame_pos_in_bytes) {
            return false;
        }
        self.oggbs().bytes_remaining_in_page = 0;

        let mut running_granule_position: u64 = 0;
        let running_frame_byte_pos;
        loop {
            let oggbs = self.oggbs();
            if !oggbs.goto_next_page(OggCrcMismatchRecovery::RecoverOnCrcMismatch) {
                oggbs.seek_physical(original_byte_pos, SeekOrigin::Set);
                return false; /* Never did find that sample... */
            }

            let frame_byte_pos = oggbs
                .current_byte_pos
                .wrapping_sub(ogg_get_page_header_size(&oggbs.current_page_header) as u64)
                .wrapping_sub(oggbs.page_data_size as u64);
            if oggbs.current_page_header.granule_position >= pcm_frame_index {
                running_frame_byte_pos = frame_byte_pos;
                break; /* The sample is somewhere in the previous page. */
            }

            /*
            At this point we know the sample is not in the previous page. It could possibly be in this page. For simplicity we
            disregard any pages that do not begin a fresh packet.
            */
            if (oggbs.current_page_header.header_type & 0x01) == 0
                && oggbs.current_page_header.segment_table[0] >= 2
            {
                /* <-- Is it a fresh page? */
                let first_bytes_in_page = [oggbs.page_data[0], oggbs.page_data[1]];

                if (first_bytes_in_page[0] == 0xFF) && (first_bytes_in_page[1] & 0xFC) == 0xF8 {
                    /* <-- Does the page begin with a frame's sync code? */
                    running_granule_position = oggbs.current_page_header.granule_position;
                }

                continue;
            }
        }

        /*
        We found the page that that is closest to the sample, so now we need to find it. The first thing to do is seek to the
        start of that page. In the loop above we checked that it was a fresh page which means this page is also the start of
        a new frame. This property means that after we've seeked to the page we can immediately start looping over frames until
        we find the one containing the target sample.
        */
        if !self
            .oggbs()
            .seek_physical(running_frame_byte_pos, SeekOrigin::Set)
        {
            return false;
        }
        if !self
            .oggbs()
            .goto_next_page(OggCrcMismatchRecovery::RecoverOnCrcMismatch)
        {
            return false;
        }

        /*
        At this point we'll be sitting on the first byte of the frame header of the first frame in the page. We just keep
        looping over these frames until we find the one containing the sample we're after.
        */
        let mut running_pcm_frame_count = running_granule_position;
        loop {
            /*
            There are two ways to find the sample and seek past irrelevant frames:
              1) Use the native FLAC decoder.
              2) Use Ogg's framing system.

            Both of these options have their own pros and cons. Using the native FLAC decoder is slower because it needs to
            do a full decode of the frame. Using Ogg's framing system is faster, but more complicated and involves some code
            duplication for the decoding of frame headers.

            Another thing to consider is that using the Ogg framing system will perform direct seeking of the physical Ogg
            bitstream. This is important to consider because it means we cannot read data from the drflac_bs object using the
            standard drflac__*() APIs because that will read in extra data for its own internal caching which in turn breaks
            the positioning of the read pointer of the physical Ogg bitstream. Therefore, anything that would normally be read
            using the native FLAC decoding APIs, such as drflac__read_next_flac_frame_header(), need to be re-implemented so as to
            avoid the use of the drflac_bs object.

            Considering these issues, I have decided to use the slower native FLAC decoding method for the following reasons:
              1) Seeking is already partially accelerated using Ogg's paging system in the code block above.
              2) Seeking in an Ogg encapsulated FLAC stream is probably quite uncommon.
              3) Simplicity.
            */
            if !read_next_flac_frame_header(
                &mut self.bs,
                self.bits_per_sample,
                &mut self.current_flac_frame.header,
            ) {
                return false;
            }

            let (first_pcm_frame_in_flac_frame, last_pcm_frame_in_flac_frame) =
                self.get_pcm_frame_range_of_current_flac_frame();

            let pcm_frame_count_in_this_frame = last_pcm_frame_in_flac_frame
                .wrapping_sub(first_pcm_frame_in_flac_frame)
                .wrapping_add(1);

            /* If we are seeking to the end of the file and we've just hit it, we're done. */
            if pcm_frame_index == self.total_pcm_frame_count
                && running_pcm_frame_count.wrapping_add(pcm_frame_count_in_this_frame)
                    == self.total_pcm_frame_count
            {
                let result = self.decode_flac_frame();
                if result == DRFLAC_SUCCESS {
                    self.current_pcm_frame = pcm_frame_index;
                    self.current_flac_frame.pcm_frames_remaining = 0;
                    return true;
                } else {
                    return false;
                }
            }

            if pcm_frame_index < running_pcm_frame_count.wrapping_add(pcm_frame_count_in_this_frame)
            {
                /*
                The sample should be in this FLAC frame. We need to fully decode it, however if it's an invalid frame (a CRC mismatch), we need to pretend
                it never existed and keep iterating.
                */
                let result = self.decode_flac_frame();
                if result == DRFLAC_SUCCESS {
                    /* The frame is valid. We just need to skip over some samples to ensure it's sample-exact. */
                    let pcm_frames_to_decode =
                        pcm_frame_index.wrapping_sub(running_pcm_frame_count); /* <-- Safe cast because the maximum number of samples in a frame is 65535. */
                    if pcm_frames_to_decode == 0 {
                        return true;
                    }

                    self.current_pcm_frame = running_pcm_frame_count;

                    return self.seek_forward_by_pcm_frames(pcm_frames_to_decode)
                        == pcm_frames_to_decode; /* <-- If this fails, something bad has happened (it should never fail). */
                } else if result == DRFLAC_CRC_MISMATCH {
                    continue; /* CRC mismatch. Pretend this frame never existed. */
                } else {
                    return false;
                }
            } else {
                /*
                It's not in this frame. We need to seek past the frame, but check if there was a CRC mismatch. If so, we pretend this
                frame never existed and leave the running sample count untouched.
                */
                let result = self.seek_to_next_flac_frame();
                if result == DRFLAC_SUCCESS {
                    running_pcm_frame_count =
                        running_pcm_frame_count.wrapping_add(pcm_frame_count_in_this_frame);
                } else if result == DRFLAC_CRC_MISMATCH {
                    continue; /* CRC mismatch. Pretend this frame never existed. */
                } else {
                    return false;
                }
            }
        }
    }
}

/// Translation of `drflac__init_private__ogg()`.
fn init_private_ogg(
    init: &mut InitInfo,
    cb: &mut dyn Callbacks,
    on_meta: Option<&mut MetaProc<'_>>,
) -> bool {
    let mut header = OggPageHeader::default();
    let mut crc32 = DRFLAC_OGG_CAPTURE_PATTERN_CRC32;
    let mut bytes_read = 0u32;

    /* Pre Condition: The bit stream should be sitting just past the 4-byte OggS capture pattern. */

    init.container = Container::Ogg;
    init.ogg_first_byte_pos = 0;

    /*
    We'll get here if the first 4 bytes of the stream were the OggS capture pattern, however it doesn't necessarily mean the
    stream includes FLAC encoded audio. To check for this we need to scan the beginning-of-stream page markers and check if
    any match the FLAC specification. Important to keep in mind that the stream may be multiplexed.
    */
    if ogg_read_page_header_after_capture_pattern(cb, &mut header, &mut bytes_read, &mut crc32)
        != DRFLAC_SUCCESS
    {
        return false;
    }
    init.running_file_pos += bytes_read as u64;

    let mut on_meta = on_meta;
    loop {
        /* Break if we're past the beginning of stream page. */
        if (header.header_type & 0x02) == 0 {
            return false;
        }

        /* Check if it's a FLAC header. */
        let page_body_size = ogg_get_page_body_size(&header) as i32;
        if page_body_size == 51 {
            /* 51 = the lacing value of the FLAC header packet. */
            /* It could be a FLAC page... */
            let mut bytes_remaining_in_page = page_body_size as u32;
            let mut packet_type = [0u8; 1];

            if cb.on_read(&mut packet_type) != 1 {
                return false;
            }

            bytes_remaining_in_page -= 1;
            if packet_type[0] == 0x7F {
                /* Increasingly more likely to be a FLAC page... */
                let mut sig = [0u8; 4];
                if cb.on_read(&mut sig) != 4 {
                    return false;
                }

                bytes_remaining_in_page -= 4;
                if &sig == b"FLAC" {
                    /* Almost certainly a FLAC page... */
                    let mut mapping_version = [0u8; 2];
                    if cb.on_read(&mut mapping_version) != 2 {
                        return false;
                    }

                    if mapping_version[0] != 1 {
                        return false; /* Only supporting version 1.x of the Ogg mapping. */
                    }

                    /*
                    The next 2 bytes are the non-audio packets, not including this one. We don't care about this because we're going to
                    be handling it in a generic way based on the serial number and packet types.
                    */
                    if !cb.on_seek(2, SeekOrigin::Cur) {
                        return false;
                    }

                    /* Expecting the native FLAC signature "fLaC". */
                    if cb.on_read(&mut sig) != 4 {
                        return false;
                    }

                    if &sig == b"fLaC" {
                        /* The remaining data in the page should be the STREAMINFO block. */
                        let mut is_last_block = 0u8;
                        let mut block_type = 0u8;
                        let mut block_size = 0u32;
                        if !read_and_decode_block_header(
                            cb,
                            &mut is_last_block,
                            &mut block_type,
                            &mut block_size,
                        ) {
                            return false;
                        }

                        if block_type != DRFLAC_METADATA_BLOCK_TYPE_STREAMINFO || block_size != 34 {
                            return false; /* Invalid block type. First block must be the STREAMINFO block. */
                        }

                        let mut streaminfo = Streaminfo::default();
                        if read_streaminfo(cb, &mut streaminfo) {
                            /* Success! */
                            init.has_stream_info_block = true;
                            init.sample_rate = streaminfo.sample_rate;
                            init.channels = streaminfo.channels;
                            init.bits_per_sample = streaminfo.bits_per_sample;
                            init.total_pcm_frame_count = streaminfo.total_pcm_frame_count;
                            init.max_block_size_in_pcm_frames =
                                streaminfo.max_block_size_in_pcm_frames;
                            init.has_metadata_blocks = is_last_block == 0;

                            if let Some(on_meta) = on_meta.as_deref_mut() {
                                on_meta(DRFLAC_METADATA_BLOCK_TYPE_STREAMINFO, None);
                            }

                            init.running_file_pos += page_body_size as u64;
                            init.ogg_first_byte_pos = init.running_file_pos.wrapping_sub(79); /* Subtracting 79 will place us right on top of the "OggS" identifier of the FLAC bos page. */
                            init.ogg_serial = header.serial_number;
                            init.ogg_bos_header = header;
                            break;
                        } else {
                            /* Failed to read STREAMINFO block. Aww, so close... */
                            return false;
                        }
                    } else {
                        /* Invalid file. */
                        return false;
                    }
                } else {
                    /* Not a FLAC header. Skip it. */
                    if !cb.on_seek(bytes_remaining_in_page as i32, SeekOrigin::Cur) {
                        return false;
                    }
                }
            } else {
                /* Not a FLAC header. Seek past the entire page and move on to the next. */
                if !cb.on_seek(bytes_remaining_in_page as i32, SeekOrigin::Cur) {
                    return false;
                }
            }
        } else if !cb.on_seek(page_body_size, SeekOrigin::Cur) {
            return false;
        }

        init.running_file_pos += page_body_size as u64;

        /* Read the header of the next page. */
        if ogg_read_page_header(cb, &mut header, &mut bytes_read, &mut crc32) != DRFLAC_SUCCESS {
            return false;
        }
        init.running_file_pos += bytes_read as u64;
    }

    /*
    If we get here it means we found a FLAC audio stream. We should be sitting on the first byte of the header of the next page. The next
    packets in the FLAC logical stream contain the metadata. The only thing left to do in the initialization phase for Ogg is to create the
    Ogg bistream object.
    */
    init.has_metadata_blocks = true; /* <-- Always have at least VORBIS_COMMENT metadata block. */
    true
}

/// Translation of `drflac__init_private()` (with an unknown container, as
/// `drflac_open()` and `drflac_open_with_metadata()` have).
fn init_private(
    init: &mut InitInfo,
    io: &mut IoStream<'_>,
    on_meta: Option<&mut MetaProc<'_>>,
) -> bool {
    let mut id = [0u8; 4];

    *init = InitInfo::default();

    /* Skip over any ID3 tags. */
    loop {
        if io.on_read(&mut id) != 4 {
            return false; /* Ran out of data. */
        }
        init.running_file_pos += 4;

        if &id[..3] == b"ID3" {
            let mut header = [0u8; 6];

            if io.on_read(&mut header) != 6 {
                return false; /* Ran out of data. */
            }
            init.running_file_pos += 6;

            let flags = header[1];

            let mut header_size =
                unsynchsafe_32(u32::from_be_bytes(header[2..6].try_into().unwrap()));
            if flags & 0x10 != 0 {
                header_size += 10;
            }

            if !io.on_seek(header_size as i32, SeekOrigin::Cur) {
                return false; /* Failed to seek past the tag. */
            }
            init.running_file_pos += header_size as u64;
        } else {
            break;
        }
    }

    if &id == b"fLaC" {
        return init_private_native(init, io, on_meta);
    }
    if &id == b"OggS" {
        return init_private_ogg(init, io, on_meta);
    }

    /* Unsupported container. */
    false
}

impl<'a> Drflac<'a> {
    /// Translation of `drflac_open_with_metadata_private()`.
    fn open_with_metadata_private(
        mut io: IoStream<'a>,
        mut on_meta: Option<&mut MetaProc<'_>>,
    ) -> Option<Drflac<'a>> {
        let mut init = InitInfo::default();

        if !init_private(&mut init, &mut io, on_meta.as_deref_mut()) {
            return None;
        }

        /*
        The size of the allocation for the drflac object needs to be large enough to fit the following:
          1) The main members of the drflac structure
          2) A block of memory large enough to store the decoded samples of the largest frame in the stream
          3) If the container is Ogg, a drflac_oggbs object

        The complicated part of the allocation is making sure there's enough room the decoded samples, taking into consideration
        the different SIMD instruction sets.
        */

        /*
        The allocation size for decoded frames depends on the number of 32-bit integers that fit inside the largest SIMD vector
        we are supporting.
        */
        let samples_per_vector = DRFLAC_MAX_SIMD_VECTOR_SIZE / 4;
        let whole_simd_vector_count_per_channel =
            if (init.max_block_size_in_pcm_frames as u32 % samples_per_vector) == 0 {
                init.max_block_size_in_pcm_frames as u32 / samples_per_vector
            } else {
                (init.max_block_size_in_pcm_frames as u32 / samples_per_vector) + 1
            };

        let decoded_samples_count =
            whole_simd_vector_count_per_channel * samples_per_vector * init.channels as u32;

        /* There's additional data required for Ogg streams. */
        let mut src = if init.container == Container::Ogg {
            Source::Ogg(Box::new(OggBs {
                io,
                current_byte_pos: init.ogg_first_byte_pos,
                first_byte_pos: init.ogg_first_byte_pos,
                serial_number: init.ogg_serial,
                _bos_page_header: init.ogg_bos_header,
                current_page_header: OggPageHeader::default(),
                bytes_remaining_in_page: 0,
                page_data_size: 0,
                page_data: Box::new([0; DRFLAC_OGG_MAX_PAGE_SIZE]),
            }))
        } else {
            Source::Native(io)
        };

        /*
        This part is a bit awkward. We need to load the seektable so that it can be referenced in-memory, but I want the drflac object to
        consist of only a single heap allocation. To this, the size of the seek table needs to be known, which we determine when reading
        and decoding the metadata.
        */
        let mut first_frame_pos: u64 = 42; /* <-- We know we are at byte 42 at this point. */
        let mut seektable_pos: u64 = 0;
        let mut seekpoint_count: u32 = 0;
        if init.has_metadata_blocks {
            // (for Ogg, the metadata is read through the Ogg layer.)
            if !read_and_decode_metadata(
                &mut src,
                on_meta.as_deref_mut(),
                &mut first_frame_pos,
                &mut seektable_pos,
                &mut seekpoint_count,
            ) {
                return None;
            }

            // (upstream checks the seek table fits a 32-bit allocation size
            // with the rest, which it always does: its block is at most
            // 2^24 bytes.)
        }

        let decoded_samples = try_alloc::<i32>(decoded_samples_count as usize).ok()?;

        let mut flac = Drflac {
            sample_rate: init.sample_rate,
            channels: init.channels,
            bits_per_sample: init.bits_per_sample,
            max_block_size_in_pcm_frames: init.max_block_size_in_pcm_frames,
            total_pcm_frame_count: init.total_pcm_frame_count,
            container: init.container,
            current_flac_frame: Frame::default(),
            current_pcm_frame: 0,
            first_flac_frame_pos_in_bytes: first_frame_pos,
            decoded_samples,
            seekpoints: None,
            bs: Bs::new(src),
        };

        /* NOTE: Seektables are not currently compatible with Ogg encapsulation (Ogg has its own accelerated seeking system). I may change this later, so I'm leaving this here for now. */
        if flac.container != Container::Ogg {
            /* If we have a seektable we need to load it now, making sure we move back to where we were previously. */
            if seektable_pos != 0 {
                let mut seekpoints = try_alloc::<Seekpoint>(seekpoint_count as usize).ok()?;

                /* Seek to the seektable, then just read directly into our seektable buffer. */
                if flac.bs.src.on_seek(seektable_pos as i32, SeekOrigin::Set) {
                    let mut ok = true;
                    for seekpoint in seekpoints.iter_mut() {
                        let mut raw = [0u8; DRFLAC_SEEKPOINT_SIZE_IN_BYTES as usize];
                        if flac.bs.src.on_read(&mut raw) == DRFLAC_SEEKPOINT_SIZE_IN_BYTES as usize
                        {
                            /* Endian swap. */
                            seekpoint.first_pcm_frame =
                                u64::from_be_bytes(raw[0..8].try_into().unwrap());
                            seekpoint.flac_frame_offset =
                                u64::from_be_bytes(raw[8..16].try_into().unwrap());
                            seekpoint.pcm_frame_count =
                                u16::from_be_bytes(raw[16..18].try_into().unwrap());
                        } else {
                            /* Failed to read the seektable. Pretend we don't have one. */
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        flac.seekpoints = Some(seekpoints);
                    }

                    /* We need to seek back to where we were. If this fails it's a critical error. */
                    if !flac
                        .bs
                        .src
                        .on_seek(flac.first_flac_frame_pos_in_bytes as i32, SeekOrigin::Set)
                    {
                        return None;
                    }
                } else {
                    /* Failed to seek to the seektable. Ominous sign, but for now we can just pretend we don't have one. */
                }
            }
        }

        // (Without a STREAMINFO block (the relaxed opening), upstream
        // decodes the first frame here.)
        debug_assert!(init.has_stream_info_block);

        Some(flac)
    }

    /// Translation of `drflac_open()`: `None` if it's not a FLAC stream.
    pub fn open(io: IoStream<'a>) -> Option<Drflac<'a>> {
        Self::open_with_metadata_private(io, None)
    }

    /// Translation of `drflac_open_with_metadata()`.
    pub fn open_with_metadata(io: IoStream<'a>, on_meta: &mut MetaProc<'_>) -> Option<Drflac<'a>> {
        Self::open_with_metadata_private(io, Some(on_meta))
    }

    /// Translation of `drflac__is_current_flac_frame_valid()`.
    fn is_current_flac_frame_valid(&self) -> bool {
        if self.current_flac_frame.header.block_size_in_pcm_frames
            > self.max_block_size_in_pcm_frames
            || self.current_flac_frame.pcm_frames_remaining
                > self.current_flac_frame.header.block_size_in_pcm_frames as u32
        {
            return false;
        }

        for i_channel in 0..self.channels as usize {
            if self.current_flac_frame.subframes[i_channel]
                .samples_s32
                .is_none()
            {
                return false;
            }
        }

        true
    }

    /// Translation of `drflac_read_pcm_frames_f32()`: read up to
    /// `frames_to_read` frames into `buffer_out` (interleaved), or skip them
    /// if it's `None`.
    pub fn read_pcm_frames_f32(
        &mut self,
        mut frames_to_read: u64,
        buffer_out: Option<&mut [f32]>,
    ) -> u64 {
        if frames_to_read == 0 {
            return 0;
        }

        let Some(buffer_out) = buffer_out else {
            return self.seek_forward_by_pcm_frames(frames_to_read);
        };

        debug_assert!(self.bits_per_sample <= 32);
        let unused_bits_per_sample = 32u32.wrapping_sub(self.bits_per_sample as u32);

        let mut out_pos = 0usize;
        let mut frames_read: u64 = 0;
        while frames_to_read > 0 {
            /* If we've run out of samples in this frame, go to the next. */
            if self.current_flac_frame.pcm_frames_remaining == 0 {
                if !self.read_and_decode_next_flac_frame() {
                    break; /* Couldn't read the next frame, so just break from the loop and return. */
                }
            } else {
                let frame = &self.current_flac_frame;
                let channel_count =
                    get_channel_count_from_channel_assignment(frame.header.channel_assignment)
                        as usize;
                let i_first_pcm_frame = (frame.header.block_size_in_pcm_frames as u32)
                    .wrapping_sub(frame.pcm_frames_remaining)
                    as usize;
                let mut frame_count_this_iteration = frames_to_read;

                if frame_count_this_iteration > frame.pcm_frames_remaining as u64 {
                    frame_count_this_iteration = frame.pcm_frames_remaining as u64;
                }
                let n = frame_count_this_iteration as usize;

                // (each channel's samples; the decoder's invariants keep
                // these in the buffer, but don't trust them blindly.)
                let input = |j: usize| -> &[i32] {
                    let start = frame.subframes[j].samples_s32.unwrap_or(0) + i_first_pcm_frame;
                    self.decoded_samples.get(start..start + n).unwrap_or(&[])
                };
                let out = buffer_out
                    .get_mut(out_pos..out_pos + n * channel_count)
                    .unwrap_or(&mut []);

                if channel_count == 2 {
                    let (in0, in1) = (input(0), input(1));
                    if in0.len() == n && in1.len() == n && out.len() == n * 2 {
                        let wasted0 = frame.subframes[0].wasted_bits_per_sample as u32;
                        let wasted1 = frame.subframes[1].wasted_bits_per_sample as u32;
                        let sse2 = self.bits_per_sample <= 24;
                        match frame.header.channel_assignment {
                            DRFLAC_CHANNEL_ASSIGNMENT_LEFT_SIDE => decode_left_side_f32(
                                sse2,
                                unused_bits_per_sample,
                                wasted0,
                                wasted1,
                                in0,
                                in1,
                                out,
                            ),
                            DRFLAC_CHANNEL_ASSIGNMENT_RIGHT_SIDE => decode_right_side_f32(
                                sse2,
                                unused_bits_per_sample,
                                wasted0,
                                wasted1,
                                in0,
                                in1,
                                out,
                            ),
                            DRFLAC_CHANNEL_ASSIGNMENT_MID_SIDE => decode_mid_side_f32(
                                sse2,
                                unused_bits_per_sample,
                                wasted0,
                                wasted1,
                                in0,
                                in1,
                                out,
                            ),
                            _ => decode_independent_stereo_f32(
                                sse2,
                                unused_bits_per_sample,
                                wasted0,
                                wasted1,
                                in0,
                                in1,
                                out,
                            ),
                        }
                    }
                } else {
                    /* Generic interleaving. */
                    for j in 0..channel_count {
                        let samples = input(j);
                        let shift = unused_bits_per_sample
                            + frame.subframes[j].wasted_bits_per_sample as u32;
                        for i in 0..samples.len().min(n) {
                            let sample_s32 = (samples[i] as u32).wrapping_shl(shift) as i32;
                            if let Some(o) = out.get_mut((i * channel_count) + j) {
                                *o = (sample_s32 as f64 / 2147483648.0) as f32;
                            }
                        }
                    }
                }

                frames_read += frame_count_this_iteration;
                out_pos += n * channel_count;
                frames_to_read -= frame_count_this_iteration;
                self.current_pcm_frame = self
                    .current_pcm_frame
                    .wrapping_add(frame_count_this_iteration);
                self.current_flac_frame.pcm_frames_remaining -= frame_count_this_iteration as u32;
            }
        }

        frames_read
    }

    /// Translation of `drflac_seek_to_pcm_frame()`.
    pub fn seek_to_pcm_frame(&mut self, mut pcm_frame_index: u64) -> bool {
        /* Don't do anything if we're already on the seek point. */
        if self.current_pcm_frame == pcm_frame_index {
            return true;
        }

        /*
        If we don't know where the first frame begins then we can't seek. This will happen when the STREAMINFO block was not present
        when the decoder was opened.
        */
        if self.first_flac_frame_pos_in_bytes == 0 {
            return false;
        }

        if pcm_frame_index == 0 {
            self.current_pcm_frame = 0;
            self.seek_to_first_frame()
        } else {
            let original_pcm_frame = self.current_pcm_frame;

            /* Clamp the sample to the end. */
            if pcm_frame_index > self.total_pcm_frame_count {
                pcm_frame_index = self.total_pcm_frame_count;
            }

            /* If the target sample and the current sample are in the same frame we just move the position forward. */
            if self.is_current_flac_frame_valid() {
                if pcm_frame_index > self.current_pcm_frame {
                    /* Forward. */
                    let offset = (pcm_frame_index - self.current_pcm_frame) as u32;
                    if self.current_flac_frame.pcm_frames_remaining > offset {
                        self.current_flac_frame.pcm_frames_remaining -= offset;
                        self.current_pcm_frame = pcm_frame_index;
                        return true;
                    }
                } else {
                    /* Backward. */
                    let offset_abs = (self.current_pcm_frame - pcm_frame_index) as u32;
                    let current_flac_frame_pcm_frame_count =
                        self.current_flac_frame.header.block_size_in_pcm_frames as u32;
                    let current_flac_frame_pcm_frames_consumed = current_flac_frame_pcm_frame_count
                        .wrapping_sub(self.current_flac_frame.pcm_frames_remaining);
                    if current_flac_frame_pcm_frames_consumed > offset_abs {
                        self.current_flac_frame.pcm_frames_remaining += offset_abs;
                        self.current_pcm_frame = pcm_frame_index;
                        return true;
                    }
                }
            }

            /*
            Different techniques depending on encapsulation. Using the native FLAC seektable with Ogg encapsulation is a bit awkward so
            we'll instead use Ogg's natural seeking facility.
            */
            let mut was_successful;
            if self.container == Container::Ogg {
                was_successful = self.ogg_seek_to_pcm_frame(pcm_frame_index);
            } else {
                /* First try seeking via the seek table. If this fails, fall back to a brute force seek which is much slower. */
                was_successful = self.seek_to_pcm_frame_seek_table(pcm_frame_index);

                /* Fall back to binary search if seek table seeking fails. This requires the length of the stream to be known. */
                if !was_successful && self.total_pcm_frame_count > 0 {
                    was_successful = self.seek_to_pcm_frame_binary_search(pcm_frame_index);
                }

                /* Fall back to brute force if all else fails. */
                if !was_successful {
                    was_successful = self.seek_to_pcm_frame_brute_force(pcm_frame_index);
                }
            }

            if was_successful {
                self.current_pcm_frame = pcm_frame_index;
            } else {
                /* Seek failed. Try putting the decoder back to it's original state. */
                if !self.seek_to_pcm_frame(original_pcm_frame) {
                    /* Failed to seek back to the original PCM frame. Fall back to 0. */
                    self.seek_to_pcm_frame(0);
                }
            }

            was_successful
        }
    }
}

/// `(drflac_int32)x * factor` of the scalar loops, or the SSE2 loops'
/// conversion of samples scaled to 24 bits.
fn to_f32(x: u32, sse2: bool) -> f32 {
    if sse2 {
        (x as i32 as f32) * (1.0f32 / 8388608.0f32)
    } else {
        (x as i32 as f32) * (1.0 / 2147483648.0f64) as f32
    }
}

/// The left shift of the samples: the SSE2 loops scale them to 24 bits
/// (and shift by 32 or more to 0), the scalar ones to 32 (where x86 shifts
/// by the count modulo 32).
fn shl(x: u32, shift: u32, sse2: bool) -> u32 {
    if sse2 {
        if shift > 31 {
            0
        } else {
            x << shift
        }
    } else {
        x.wrapping_shl(shift)
    }
}

/// Translation of `drflac_read_pcm_frames_f32__decode_left_side()`
/// (its `__sse2` and `__scalar` versions).
fn decode_left_side_f32(
    sse2: bool,
    unused: u32,
    wasted0: u32,
    wasted1: u32,
    in0: &[i32],
    in1: &[i32],
    out: &mut [f32],
) {
    let shift0 = (unused + wasted0).wrapping_sub(if sse2 { 8 } else { 0 });
    let shift1 = (unused + wasted1).wrapping_sub(if sse2 { 8 } else { 0 });
    for i in 0..in0.len() {
        let left = shl(in0[i] as u32, shift0, sse2);
        let side = shl(in1[i] as u32, shift1, sse2);
        let right = left.wrapping_sub(side);

        out[i * 2] = to_f32(left, sse2);
        out[i * 2 + 1] = to_f32(right, sse2);
    }
}

/// Translation of `drflac_read_pcm_frames_f32__decode_right_side()`.
fn decode_right_side_f32(
    sse2: bool,
    unused: u32,
    wasted0: u32,
    wasted1: u32,
    in0: &[i32],
    in1: &[i32],
    out: &mut [f32],
) {
    let shift0 = (unused + wasted0).wrapping_sub(if sse2 { 8 } else { 0 });
    let shift1 = (unused + wasted1).wrapping_sub(if sse2 { 8 } else { 0 });
    for i in 0..in0.len() {
        let side = shl(in0[i] as u32, shift0, sse2);
        let right = shl(in1[i] as u32, shift1, sse2);
        let left = right.wrapping_add(side);

        out[i * 2] = to_f32(left, sse2);
        out[i * 2 + 1] = to_f32(right, sse2);
    }
}

/// Translation of `drflac_read_pcm_frames_f32__decode_mid_side()`.
fn decode_mid_side_f32(
    sse2: bool,
    unused: u32,
    wasted0: u32,
    wasted1: u32,
    in0: &[i32],
    in1: &[i32],
    out: &mut [f32],
) {
    let frame_count = in0.len();
    let frame_count4 = frame_count >> 2;
    for i in 0..frame_count {
        let side = shl(in1[i] as u32, wasted1, sse2);
        let mut mid = shl(in0[i] as u32, wasted0, sse2);

        mid = (mid << 1) | (side & 0x01);

        let (left, right) = if sse2 {
            let shift = unused.wrapping_sub(8);
            if shift == 0 {
                (
                    (mid.wrapping_add(side) as i32 >> 1) as u32,
                    (mid.wrapping_sub(side) as i32 >> 1) as u32,
                )
            } else {
                (
                    shl(mid.wrapping_add(side), shift - 1, true),
                    shl(mid.wrapping_sub(side), shift - 1, true),
                )
            }
        } else if i < frame_count4 << 2 {
            // (the scalar version's main loop, four frames at a time.)
            if unused > 0 {
                (
                    mid.wrapping_add(side).wrapping_shl(unused - 1),
                    mid.wrapping_sub(side).wrapping_shl(unused - 1),
                )
            } else {
                (
                    (mid.wrapping_add(side) as i32 >> 1) as u32,
                    (mid.wrapping_sub(side) as i32 >> 1) as u32,
                )
            }
        } else {
            // (and its loop over the rest.)
            (
                ((mid.wrapping_add(side) as i32 >> 1) as u32).wrapping_shl(unused),
                ((mid.wrapping_sub(side) as i32 >> 1) as u32).wrapping_shl(unused),
            )
        };

        out[i * 2] = to_f32(left, sse2);
        out[i * 2 + 1] = to_f32(right, sse2);
    }
}

/// Translation of `drflac_read_pcm_frames_f32__decode_independent_stereo()`.
fn decode_independent_stereo_f32(
    sse2: bool,
    unused: u32,
    wasted0: u32,
    wasted1: u32,
    in0: &[i32],
    in1: &[i32],
    out: &mut [f32],
) {
    let shift0 = (unused + wasted0).wrapping_sub(if sse2 { 8 } else { 0 });
    let shift1 = (unused + wasted1).wrapping_sub(if sse2 { 8 } else { 0 });
    for i in 0..in0.len() {
        out[i * 2] = to_f32(shl(in0[i] as u32, shift0, sse2), sse2);
        out[i * 2 + 1] = to_f32(shl(in1[i] as u32, shift1, sse2), sse2);
    }
}

/// Translation of `drflac_vorbis_comment_iterator`.
pub(crate) struct VorbisCommentIterator<'b> {
    count_remaining: u32,
    running_data: &'b [u8],
}

impl<'b> VorbisCommentIterator<'b> {
    /// Translation of `drflac_init_vorbis_comment_iterator()`.
    pub fn new(comment_count: u32, comments: &'b [u8]) -> Self {
        VorbisCommentIterator {
            count_remaining: comment_count,
            running_data: comments,
        }
    }

    /// Translation of `drflac_next_vorbis_comment()`. (The comments were
    /// checked to fit in the block when it was read.)
    pub fn next_comment(&mut self) -> Option<&'b [u8]> {
        if self.count_remaining == 0 || self.running_data.len() < 4 {
            return None;
        }

        let length = le32(self.running_data, 0) as usize;
        let rest = &self.running_data[4..];
        let comment = rest.get(..length)?;
        self.running_data = &rest[length..];
        self.count_remaining -= 1;

        Some(comment)
    }
}
