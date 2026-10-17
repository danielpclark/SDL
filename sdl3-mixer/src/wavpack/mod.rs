// Rust translation of include/wavpack.h and src/wavpack_local.h from
// WavPack 5.9.0 (https://www.wavpack.com, at the revision SDL_mixer's
// external/wavpack pins).
// Copyright (c) 1998 - 2024 David Bryant.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! A Rust translation of the decoding half of libwavpack 5.9.0, as
//! SDL_mixer's CMake builds the copy it vendors: `WAVPACK_ENABLE_THREADS`
//! off, `WAVPACK_ENABLE_LEGACY` off (its default, so pre-4.0 files are
//! refused) and `WAVPACK_ENABLE_DSD` on (its default: DSD blocks are
//! parsed and decoded, although SDL_mixer opens files without
//! `OPEN_DSD_NATIVE` or `OPEN_DSD_AS_PCM`, so a file that starts with one is
//! refused). The C code paths are translated, not the assembly versions of
//! the decorrelation passes that the build also enables on x86, x86-64 and
//! ARM (they compute the same thing), and only what decoding through
//! `WavpackOpenFileInputEx64()` reaches: opening with the stream reader
//! callbacks (open_utils.c), block and metadata parsing, unpacking
//! (unpack_utils.c, unpack.c, unpack_floats.c, unpack_dsd.c without the
//! decimation that only `OPEN_DSD_AS_PCM` uses), the entropy decoder
//! (read_words.c, entropy_utils.c), the decorrelation metadata
//! (decorr_utils.c), seeking (unpack_seek.c), loading the ID3v1/APEv2 tag
//! (tags.c) and the API functions of common_utils.c that SDL_mixer calls.
//! The encoder, tag editing and reading (tag_utils.c), opening files by
//! name (open_filename.c) and raw blocks (open_raw.c) aren't.
//!
//! The configuration is the one of a little-endian build with GCC or Clang
//! on Linux and macOS, whatever the target: bitstreams are read as 16-bit
//! words (`BITSTREAM_SHORTS`), `count_bits()` is the `__builtin_clz()`
//! version, the entropy decoder counts ones with its 8-bit table
//! (`USE_NEXT8_OPTIMIZATION`: CMake checks `__builtin_clz()` but not
//! `__builtin_ctz()`), `unsigned long` and `long` are 64 bits wide (for
//! `read_code()` and `labs()`), and shifts by 32 or more of 32-bit values
//! act as x86's do (`wrapping_shl()`/`wrapping_shr()`). These only differ
//! from a Windows build on damaged data.

// The translation keeps upstream's loops, declarations and arithmetic,
// which these lints would have rewritten.
#![allow(
    clippy::assign_op_pattern,
    clippy::bad_bit_mask,
    clippy::bool_to_int_with_if,
    clippy::collapsible_if,
    clippy::field_reassign_with_default,
    clippy::int_plus_one,
    clippy::manual_range_contains,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::nonminimal_bool,
    clippy::too_many_arguments,
    clippy::vec_box
)]

pub(crate) mod common_utils;
pub(crate) mod decorr_utils;
pub(crate) mod entropy_utils;
pub(crate) mod open_utils;
pub(crate) mod read_words;
pub(crate) mod tags;
pub(crate) mod unpack;
pub(crate) mod unpack_dsd;
pub(crate) mod unpack_floats;
pub(crate) mod unpack_seek;
pub(crate) mod unpack_utils;

// ---------------------------------------------------------------------------
// wavpack.h
// ---------------------------------------------------------------------------

// This header file contains all the definitions required to use the
// functions in "wputils.c" to read and write WavPack files and streams.

// This is the ONLY structure that occurs in WavPack files (as of version
// 4.0), and is the preamble to every block in both the .wv and .wvc
// files (in little-endian format). Normally, this structure has no use
// to an application using the library to read or write WavPack files,
// but if an application needs to manually parse WavPack files then this
// would be used (with appropriate endian correction).

/// Translation of `WavpackHeader`. Its 32 bytes in memory (what upstream
/// `memcpy()`s in and out of block buffers) are [`to_bytes()`](Self::to_bytes)
/// and [`from_bytes()`](Self::from_bytes) (`WavpackHeaderFormat`, in the
/// native order of a little-endian machine).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct WavpackHeader {
    pub ck_id: [u8; 4],
    pub ck_size: u32,
    pub version: i16,
    pub block_index_u8: u8,
    pub total_samples_u8: u8,
    pub total_samples: u32,
    pub block_index: u32,
    pub block_samples: u32,
    pub flags: u32,
    pub crc: u32,
}

impl WavpackHeader {
    /// The header from its 32 bytes (`memcpy()` and
    /// `WavpackLittleEndianToNative(…, WavpackHeaderFormat)`).
    pub(crate) fn from_bytes(b: &[u8]) -> WavpackHeader {
        let mut h = [0u8; 32];
        let n = b.len().min(32);
        h[..n].copy_from_slice(&b[..n]);
        let le32 = |i: usize| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
        WavpackHeader {
            ck_id: [h[0], h[1], h[2], h[3]],
            ck_size: le32(4),
            version: i16::from_le_bytes([h[8], h[9]]),
            block_index_u8: h[10],
            total_samples_u8: h[11],
            total_samples: le32(12),
            block_index: le32(16),
            block_samples: le32(20),
            flags: le32(24),
            crc: le32(28),
        }
    }

    /// The header's 32 bytes in memory.
    pub(crate) fn to_bytes(self) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[..4].copy_from_slice(&self.ck_id);
        h[4..8].copy_from_slice(&self.ck_size.to_le_bytes());
        h[8..10].copy_from_slice(&self.version.to_le_bytes());
        h[10] = self.block_index_u8;
        h[11] = self.total_samples_u8;
        h[12..16].copy_from_slice(&self.total_samples.to_le_bytes());
        h[16..20].copy_from_slice(&self.block_index.to_le_bytes());
        h[20..24].copy_from_slice(&self.block_samples.to_le_bytes());
        h[24..28].copy_from_slice(&self.flags.to_le_bytes());
        h[28..32].copy_from_slice(&self.crc.to_le_bytes());
        h
    }
}

/// `memcpy (buffer, &wphdr, 32)`.
pub(crate) fn put_header(buffer: &mut [u8], wphdr: &WavpackHeader) {
    let n = buffer.len().min(32);
    buffer[..n].copy_from_slice(&wphdr.to_bytes()[..n]);
}

/// `((WavpackHeader *) buffer)->ckSize`
pub(crate) fn buffer_ck_size(buffer: &[u8]) -> u32 {
    WavpackHeader::from_bytes(buffer).ck_size
}

// Macros to access the 40-bit block_index field

/// Translation of `GET_BLOCK_INDEX()`.
#[inline]
pub(crate) fn get_block_index(hdr: &WavpackHeader) -> i64 {
    hdr.block_index as i64 + ((hdr.block_index_u8 as i64) << 32)
}

/// Translation of `SET_BLOCK_INDEX()`.
#[inline]
pub(crate) fn set_block_index(hdr: &mut WavpackHeader, value: i64) {
    let tmp = value;
    hdr.block_index = tmp as u32;
    hdr.block_index_u8 = (tmp >> 32) as u8;
}

// Macros to access the 40-bit total_samples field, which is complicated by the fact that
// all 1's in the lower 32 bits indicates "unknown" (regardless of upper 8 bits)

/// Translation of `GET_TOTAL_SAMPLES()`.
#[inline]
pub(crate) fn get_total_samples(hdr: &WavpackHeader) -> i64 {
    if hdr.total_samples == u32::MAX {
        -1
    } else {
        hdr.total_samples as i64 + ((hdr.total_samples_u8 as i64) << 32)
            - hdr.total_samples_u8 as i64
    }
}

// or-values for WavpackHeader.flags
pub(crate) const BYTES_STORED: u32 = 3; // 1-4 bytes/sample
pub(crate) const MONO_FLAG: u32 = 4; // not stereo
pub(crate) const HYBRID_FLAG: u32 = 8; // hybrid mode
pub(crate) const JOINT_STEREO: u32 = 0x10; // joint stereo
pub(crate) const CROSS_DECORR: u32 = 0x20; // no-delay cross decorrelation
pub(crate) const HYBRID_SHAPE: u32 = 0x40; // noise shape (hybrid mode only)
pub(crate) const FLOAT_DATA: u32 = 0x80; // ieee 32-bit floating point data

pub(crate) const INT32_DATA: u32 = 0x100; // special extended int handling
pub(crate) const HYBRID_BITRATE: u32 = 0x200; // bitrate noise (hybrid mode only)
pub(crate) const HYBRID_BALANCE: u32 = 0x400; // balance noise (hybrid stereo mode only)

pub(crate) const INITIAL_BLOCK: u32 = 0x800; // initial block of multichannel segment
pub(crate) const FINAL_BLOCK: u32 = 0x1000; // final block of multichannel segment

pub(crate) const SHIFT_LSB: u32 = 13;
pub(crate) const SHIFT_MASK: u32 = 0x1f << SHIFT_LSB;

pub(crate) const MAG_LSB: u32 = 18;
pub(crate) const MAG_MASK: u32 = 0x1f << MAG_LSB;

pub(crate) const SRATE_LSB: u32 = 23;
pub(crate) const SRATE_MASK: u32 = 0xf << SRATE_LSB;

pub(crate) const FALSE_STEREO: u32 = 0x40000000; // block is stereo, but data is mono
pub(crate) const NEW_SHAPING: u32 = 0x20000000; // use IIR filter for negative shaping

pub(crate) const MONO_DATA: u32 = MONO_FLAG | FALSE_STEREO;

// Introduced in WavPack 5.0:
pub(crate) const HAS_CHECKSUM: u32 = 0x10000000; // block contains a trailing checksum
pub(crate) const DSD_FLAG: u32 = 0x80000000; // block is encoded DSD (1-bit PCM)

#[allow(dead_code)]
pub(crate) const IGNORED_FLAGS: u32 = 0x08000000; // reserved, but ignore if encountered
pub(crate) const UNKNOWN_FLAGS: u32 = 0x00000000; // we no longer have any of these spares

pub(crate) const MIN_STREAM_VERS: u32 = 0x402; // lowest stream version we'll decode
pub(crate) const MAX_STREAM_VERS: u32 = 0x410; // highest stream version we'll decode or encode

// These are the mask bit definitions for the metadata chunk id byte (see format.txt)

pub(crate) const ID_UNIQUE: u8 = 0x3f;
pub(crate) const ID_OPTIONAL_DATA: u8 = 0x20;
pub(crate) const ID_ODD_SIZE: u8 = 0x40;
pub(crate) const ID_LARGE: u8 = 0x80;

pub(crate) const ID_DUMMY: u8 = 0x0;
#[allow(dead_code)]
pub(crate) const ID_ENCODER_INFO: u8 = 0x1;
pub(crate) const ID_DECORR_TERMS: u8 = 0x2;
pub(crate) const ID_DECORR_WEIGHTS: u8 = 0x3;
pub(crate) const ID_DECORR_SAMPLES: u8 = 0x4;
pub(crate) const ID_ENTROPY_VARS: u8 = 0x5;
pub(crate) const ID_HYBRID_PROFILE: u8 = 0x6;
pub(crate) const ID_SHAPING_WEIGHTS: u8 = 0x7;
pub(crate) const ID_FLOAT_INFO: u8 = 0x8;
pub(crate) const ID_INT32_INFO: u8 = 0x9;
pub(crate) const ID_WV_BITSTREAM: u8 = 0xa;
pub(crate) const ID_WVC_BITSTREAM: u8 = 0xb;
pub(crate) const ID_WVX_BITSTREAM: u8 = 0xc;
pub(crate) const ID_CHANNEL_INFO: u8 = 0xd;
pub(crate) const ID_DSD_BLOCK: u8 = 0xe;

pub(crate) const ID_RIFF_HEADER: u8 = ID_OPTIONAL_DATA | 0x1;
pub(crate) const ID_RIFF_TRAILER: u8 = ID_OPTIONAL_DATA | 0x2;
pub(crate) const ID_ALT_HEADER: u8 = ID_OPTIONAL_DATA | 0x3;
pub(crate) const ID_ALT_TRAILER: u8 = ID_OPTIONAL_DATA | 0x4;
pub(crate) const ID_CONFIG_BLOCK: u8 = ID_OPTIONAL_DATA | 0x5;
pub(crate) const ID_MD5_CHECKSUM: u8 = ID_OPTIONAL_DATA | 0x6;
pub(crate) const ID_SAMPLE_RATE: u8 = ID_OPTIONAL_DATA | 0x7;
pub(crate) const ID_ALT_EXTENSION: u8 = ID_OPTIONAL_DATA | 0x8;
pub(crate) const ID_ALT_MD5_CHECKSUM: u8 = ID_OPTIONAL_DATA | 0x9;
pub(crate) const ID_NEW_CONFIG_BLOCK: u8 = ID_OPTIONAL_DATA | 0xa;
pub(crate) const ID_CHANNEL_IDENTITIES: u8 = ID_OPTIONAL_DATA | 0xb;
pub(crate) const ID_WVX_NEW_BITSTREAM: u8 = ID_OPTIONAL_DATA | ID_WVX_BITSTREAM;
pub(crate) const ID_BLOCK_CHECKSUM: u8 = ID_OPTIONAL_DATA | 0xf;

///////////////////////// WavPack Configuration ///////////////////////////////

// This external structure is used during encode to provide configuration to
// the encoding engine and during decoding to provide fle information back to
// the higher level functions. Not all fields are used in both modes.

/// Translation of `WavpackConfig` (the fields decoding uses; `tag_strings`
/// "is not used").
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct WavpackConfig {
    pub bits_per_sample: i32,
    pub bytes_per_sample: i32,
    pub qmode: i32,
    pub flags: i32,
    pub xmode: i32,
    pub num_channels: i32,
    pub float_norm_exp: i32,
    pub sample_rate: i32,
    pub channel_mask: i32,
    pub md5_checksum: [u8; 16],
    pub md5_read: u8,
}

pub(crate) const CONFIG_HYBRID_FLAG: i32 = 8; // hybrid mode
pub(crate) const CONFIG_FAST_FLAG: i32 = 0x200; // fast mode
pub(crate) const CONFIG_HIGH_FLAG: i32 = 0x800; // high quality mode
pub(crate) const CONFIG_VERY_HIGH_FLAG: i32 = 0x1000; // very high
pub(crate) const CONFIG_DYNAMIC_SHAPING: i32 = 0x20000; // dynamic noise shaping
pub(crate) const CONFIG_CREATE_EXE: i32 = 0x40000; // create executable
pub(crate) const CONFIG_EXTRA_MODE: i32 = 0x2000000; // extra processing mode
pub(crate) const CONFIG_MD5_CHECKSUM: i32 = 0x8000000; // store MD5 signature

////////////// Callbacks used for reading & writing WavPack streams //////////

// Extended version of structure for handling large files and added
// functionality for truncating and closing files

/// Translation of `WavpackStreamReader64` with a stream (`void *id`): the
/// callbacks SDL_mixer gives (`write_bytes`, `truncate_here` and `close`
/// are `NULL` there, and decoding doesn't call them).
pub(crate) trait WavpackStreamReader64 {
    fn read_bytes(&mut self, data: &mut [u8]) -> i32;
    fn get_pos(&mut self) -> i64; // new signature for large files
    fn set_pos_abs(&mut self, pos: i64) -> i32; // new signature for large files
    fn set_pos_rel(&mut self, delta: i64, mode: i32) -> i32; // new signature for large files
    fn push_back_byte(&mut self, c: i32) -> i32;
    fn get_length(&mut self) -> i64; // new signature for large files
    fn can_seek(&mut self) -> i32;
}

pub(crate) const SEEK_SET: i32 = 0;
pub(crate) const SEEK_CUR: i32 = 1;
pub(crate) const SEEK_END: i32 = 2;

#[allow(dead_code)]
pub(crate) const OPEN_WVC: i32 = 0x1; // open/read "correction" file
pub(crate) const OPEN_TAGS: i32 = 0x2; // read ID3v1 / APEv2 tags (seekable file)
pub(crate) const OPEN_WRAPPER: i32 = 0x4; // make audio wrapper available (i.e. RIFF)
pub(crate) const OPEN_2CH_MAX: i32 = 0x8; // open multichannel as stereo (no downmix)
pub(crate) const OPEN_NORMALIZE: i32 = 0x10; // normalize floating point data to +/- 1.0
pub(crate) const OPEN_STREAMING: i32 = 0x20; // "streaming" mode blindly unpacks blocks
                                             // w/o regard to header file position info
pub(crate) const OPEN_EDIT_TAGS: i32 = 0x40; // allow editing of tags

// new for version 5

pub(crate) const OPEN_DSD_NATIVE: i32 = 0x100; // open DSD files as bitstreams
                                               // (returned as 8-bit "samples" stored in 32-bit words)
pub(crate) const OPEN_DSD_AS_PCM: i32 = 0x200; // open DSD files as 24-bit PCM (decimated 8x)
pub(crate) const OPEN_ALT_TYPES: i32 = 0x400; // application is aware of alternate file types & qmode
                                              // (just affects retrieving wrappers & MD5 checksums)
pub(crate) const OPEN_NO_CHECKSUM: i32 = 0x800; // don't verify block checksums before decoding

pub(crate) const MODE_WVC: i32 = 0x1;
pub(crate) const MODE_LOSSLESS: i32 = 0x2;
pub(crate) const MODE_HYBRID: i32 = 0x4;
pub(crate) const MODE_FLOAT: i32 = 0x8;
pub(crate) const MODE_VALID_TAG: i32 = 0x10;
pub(crate) const MODE_HIGH: i32 = 0x20;
pub(crate) const MODE_FAST: i32 = 0x40;
pub(crate) const MODE_EXTRA: i32 = 0x80; // extra mode used, see MODE_XMODE for possible level
pub(crate) const MODE_APETAG: i32 = 0x100;
pub(crate) const MODE_SFX: i32 = 0x200;
pub(crate) const MODE_VERY_HIGH: i32 = 0x400;
pub(crate) const MODE_MD5: i32 = 0x800;
pub(crate) const MODE_DNS: i32 = 0x8000;

// ---------------------------------------------------------------------------
// wavpack_local.h
// ---------------------------------------------------------------------------

// Because the C99 specification states that "The order of allocation of
// bit-fields within a unit (high-order to low-order or low-order to
// high-order) is implementation-defined" (6.7.2.1), I decided to change
// the representation of floating-point values from a structure of
// bit-fields to a 32-bit integer with access macros. Note that the WavPack
// library doesn't use any floating-point math to implement compression of
// floating-point data (although a little floating-point math is used in
// high-level functions unrelated to the codec).

pub(crate) type F32 = i32;

#[inline]
pub(crate) fn get_mantissa(f: F32) -> i32 {
    f & 0x7fffff
}
#[inline]
pub(crate) fn get_exponent(f: F32) -> i32 {
    (f >> 23) & 0xff
}
#[inline]
pub(crate) fn get_sign(f: F32) -> i32 {
    (f >> 31) & 0x1
}

#[inline]
pub(crate) fn set_mantissa(f: &mut F32, v: i32) {
    *f ^= (*f ^ v) & 0x7fffff;
}
#[inline]
pub(crate) fn set_exponent(f: &mut F32, v: i32) {
    *f ^= ((*f as u32 ^ ((v as u32) << 23)) & 0x7f800000) as i32;
}
#[inline]
pub(crate) fn set_sign(f: &mut F32, v: i32) {
    *f ^= ((*f as u32 ^ ((v as u32) << 31)) & 0x80000000) as i32;
}

pub(crate) const FALSE: i32 = 0;
pub(crate) const TRUE: i32 = 1;

// ID3v1 and APEv2 TAG formats (may occur at the end of WavPack files)

/// `sizeof (ID3_Tag)`: `char tag_id [3], title [30], artist [30], album [30];
/// char year [4], comment [30], genre;`
pub(crate) const ID3_TAG_SIZE: usize = 128;

/// `sizeof (APE_Tag_Hdr)`: `char ID [8]; int32_t version, length,
/// item_count, flags; char res [8];` (`APE_Tag_Hdr_Format` is "8LLLL").
pub(crate) const APE_TAG_HDR_SIZE: usize = 32;

#[allow(dead_code)]
pub(crate) const APE_TAG_TYPE_TEXT: i32 = 0x0;
#[allow(dead_code)]
pub(crate) const APE_TAG_TYPE_BINARY: i32 = 0x1;
pub(crate) const APE_TAG_THIS_IS_HEADER: i32 = 0x20000000;
pub(crate) const APE_TAG_CONTAINS_HEADER: i32 = 0x80000000u32 as i32;
pub(crate) const APE_TAG_MAX_LENGTH: i32 = 1024 * 1024 * 16;

/// Translation of `M_Tag`. `id3_tag` and `ape_tag_hdr` are the structures'
/// bytes, as they're read into them.
#[derive(Clone, Debug)]
pub(crate) struct MTag {
    pub tag_file_pos: i64,
    pub tag_begins_file: i32,
    pub id3_tag: [u8; ID3_TAG_SIZE],
    pub ape_tag_hdr: [u8; APE_TAG_HDR_SIZE],
    pub ape_tag_data: Option<Vec<u8>>,
}

impl Default for MTag {
    fn default() -> Self {
        MTag {
            tag_file_pos: 0,
            tag_begins_file: 0,
            id3_tag: [0; ID3_TAG_SIZE],
            ape_tag_hdr: [0; APE_TAG_HDR_SIZE],
            ape_tag_data: None,
        }
    }
}

impl MTag {
    fn ape_field(&self, i: usize) -> i32 {
        let h = &self.ape_tag_hdr;
        i32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]])
    }
    /// `ape_tag_hdr.version`
    pub(crate) fn ape_version(&self) -> i32 {
        self.ape_field(8)
    }
    /// `ape_tag_hdr.length`
    pub(crate) fn ape_length(&self) -> i32 {
        self.ape_field(12)
    }
    /// `ape_tag_hdr.item_count`
    pub(crate) fn ape_item_count(&self) -> i32 {
        self.ape_field(16)
    }
    /// `ape_tag_hdr.flags`
    pub(crate) fn ape_flags(&self) -> i32 {
        self.ape_field(20)
    }
}

//////////////////////////// WavPack Metadata /////////////////////////////////

// This is an internal representation of metadata.

/// Translation of `WavpackMetadata` (`data` is `NULL` when it's `None`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct WavpackMetadata<'b> {
    pub byte_length: i32,
    pub data: Option<&'b [u8]>,
    pub id: u8,
}

impl<'b> WavpackMetadata<'b> {
    /// `wpmd->data` as bytes (none for `NULL`).
    pub(crate) fn bytes(&self) -> &'b [u8] {
        self.data.unwrap_or(&[])
    }
}

/// `((unsigned char *) wpmd->data) [i]`, 0 past the end (never read: the
/// lengths are checked first).
#[inline]
pub(crate) fn byte_at(data: &[u8], i: usize) -> u8 {
    data.get(i).copied().unwrap_or(0)
}

/*
 * These config flags are not actually used for external configuration, which is
 * why they're not in the external wavpack.h file, but they are used internally
 * in the flags field of the WavpackConfig struct.
 */

#[allow(dead_code)]
pub(crate) const CONFIG_MONO_FLAG: i32 = 4; // not stereo
pub(crate) const CONFIG_FLOAT_DATA: i32 = 0x80; // ieee 32-bit floating point data

pub(crate) const CONFIG_LOSSY_MODE: i32 = 0x1000000; // obsolete (for information)

//////////////////////////////// WavPack Stream ///////////////////////////////

// This internal structure contains everything required to handle a WavPack
// "stream", which is defined as a stereo or mono stream of audio samples. For
// multichannel audio several of these would be required. Each stream contains
// pointers to hold a complete allocated block of WavPack data, although it's
// possible to decode WavPack blocks without buffering an entire block.

/// Translation of `Bitstream` (`struct bs`), with `BITSTREAM_SHORTS`: the
/// stream's own copy of its 16-bit words, `ptr` as an index into them
/// (upstream's `buf - 1` is -1), `ptr != NULL` as `open`, and `wrap` always
/// `bs_read()` (decoding never writes).
#[derive(Clone, Debug, Default)]
pub(crate) struct Bitstream {
    pub buf: Vec<u16>,
    pub ptr: isize,
    pub open: bool,
    pub error: i32,
    pub bc: i32,
    pub sr: u32,
}

impl Bitstream {
    /// `*(bs)->ptr` (a word read after the stream wrapped is the first one).
    #[inline]
    pub(crate) fn word(&self) -> u32 {
        self.buf.get(self.ptr as usize).copied().unwrap_or(0) as u32
    }

    /// `if (++((bs)->ptr) == (bs)->end) (bs)->wrap (bs);`
    #[inline]
    pub(crate) fn advance(&mut self) {
        self.ptr += 1;
        if self.ptr == self.buf.len() as isize {
            bs_read(self);
        }
    }
}

/// Translation of `bs_read()` (open_utils.c), the `wrap` of every stream:
/// This function is only called from the getbit() and getbits() macros when
/// the BitStream has been exhausted and more data is required. Since these
/// bistreams no longer access files, this function simple sets an error and
/// resets the buffer.
#[inline]
pub(crate) fn bs_read(bs: &mut Bitstream) {
    bs.ptr = 0;
    bs.error = 1;
}

#[allow(dead_code)]
pub(crate) const MAX_WRAPPER_BYTES: u32 = 16777216;
#[allow(dead_code)]
pub(crate) const NEW_MAX_STREAMS: i32 = 4096;
pub(crate) const OLD_MAX_STREAMS: i32 = 8;
pub(crate) const MAX_NTERMS: usize = 16;
pub(crate) const MAX_TERM: i32 = 8;

// DSD-specific definitions

pub(crate) const MAX_HISTORY_BITS: u8 = 5; // maximum number of history bits in DSD "fast" mode
                                           // note that 5 history bits requires 32 history bins
pub(crate) const MAX_BYTES_PER_BIN: i32 = 1280; // maximum bytes for the value lookup array (per bin)
                                                //  such that the total storage per bin = 2K (also
                                                //  counting probabilities and summed_probabilities)

// Note that this structure is directly accessed in assembly files, so modify with care

/// Translation of `struct decorr_pass` (`aweight_X` and `sum_X` are the
/// encoder's).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DecorrPass {
    pub term: i32,
    pub delta: i32,
    pub weight_a: i32,
    pub weight_b: i32,
    pub samples_a: [i32; MAX_TERM as usize],
    pub samples_b: [i32; MAX_TERM as usize],
}

/// Translation of `struct entropy_data`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EntropyData {
    pub median: [u32; 3],
    pub slow_level: u32,
    pub error_limit: u32,
}

/// Translation of `struct words_data` (`pend_data`, `pend_count` are the
/// encoder's).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WordsData {
    pub bitrate_delta: [u32; 2],
    pub bitrate_acc: [u32; 2],
    pub holding_one: u32,
    pub zeros_acc: u32,
    pub holding_zero: i32,
    pub c: [EntropyData; 2],
}

/// Translation of `DSDfilters`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DsdFilters {
    pub value: i32,
    pub filter0: i32,
    pub filter1: i32,
    pub filter2: i32,
    pub filter3: i32,
    pub filter4: i32,
    pub filter5: i32,
    pub filter6: i32,
    pub factor: i32,
    pub byte: u32,
}

/// The `dc` member of `WavpackStream` (the noise figures and shaping
/// arrays are the encoder's).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DcData {
    pub shaping_acc: [i32; 2],
    pub shaping_delta: [i32; 2],
    pub error: [i32; 2],
}

/// The `dsd` member of `WavpackStream`. `byteptr` and `endptr` are indexes
/// into `data`, the stream's copy of the `ID_DSD_BLOCK` metadata they point
/// into; `value_lookup` holds offsets into `lookup_buffer`.
#[derive(Clone, Debug, Default)]
pub(crate) struct DsdData {
    pub data: Vec<u8>,
    pub byteptr: usize,
    pub endptr: usize,
    pub probabilities: Option<Vec<[u8; 256]>>,
    pub lookup_buffer: Option<Vec<u8>>,
    pub value_lookup: Option<Vec<Option<usize>>>,
    pub mode: u8,
    pub ready: u8,
    pub history_bins: i32,
    pub p0: i32,
    pub p1: i32,
    pub summed_probabilities: Option<Vec<[u16; 256]>>,
    pub low: u32,
    pub high: u32,
    pub value: u32,
    pub filters: [DsdFilters; 2],
    pub ptable: Option<Vec<i32>>,
}

/// Translation of `WavpackStream`, the fields decoding uses. `wpc` (the
/// context, which unpacking reads) is passed along instead.
#[derive(Clone, Debug, Default)]
pub(crate) struct WavpackStream {
    pub wphdr: WavpackHeader,
    pub w: WordsData,
    pub stream_index: i32,

    pub blockbuff: Option<Vec<u8>>,
    pub block2buff: Option<Vec<u8>>,

    pub sample_index: i64,
    pub num_terms: i32,
    pub mute_error: i32,
    pub crc: u32,
    pub crc_x: u32,
    pub crc_wvx: u32,
    pub wvbits: Bitstream,
    pub wvcbits: Bitstream,
    pub wvxbits: Bitstream,
    pub init_done: i32,
    pub wvc_skip: i32,

    pub int32_sent_bits: u8,
    pub int32_zeros: u8,
    pub int32_ones: u8,
    pub int32_dups: u8,
    pub float_flags: u8,
    pub float_shift: u8,
    pub float_max_exp: u8,
    pub float_norm_exp: u8,
    pub int32_max_width: u8,
    pub float_max_shifted_ones: u8,
    pub float_min_shifted_zeros: u8,

    pub dc: DcData,

    pub decorr_passes: [DecorrPass; MAX_NTERMS],

    pub dsd: DsdData,
}

// flags for float_flags:

pub(crate) const FLOAT_SHIFT_ONES: u8 = 1; // bits left-shifted into float = '1'
pub(crate) const FLOAT_SHIFT_SAME: u8 = 2; // bits left-shifted into float are the same
pub(crate) const FLOAT_SHIFT_SENT: u8 = 4; // bits shifted into float are sent literally
pub(crate) const FLOAT_ZEROS_SENT: u8 = 8; // "zeros" are not all real zeros
pub(crate) const FLOAT_NEG_ZEROS: u8 = 0x10; // contains negative zeros
pub(crate) const FLOAT_EXCEPTIONS: u8 = 0x20; // contains exceptions (inf, nan, etc.)

/////////////////////////////// WavPack Context ///////////////////////////////

// This internal structure holds everything required to encode or decode WavPack
// files. This is an opaque pointer to clients of libwavpack.

/// Translation of `struct WavpackContext`, the fields decoding uses. The
/// reader and its streams (`reader`, `wv_in`, `wvc_in`) are `wv_in` and
/// `wvc_in`; `num_streams` is `streams.len()`. `WavpackCloseFile()` is its
/// `Drop`.
pub(crate) struct WavpackContext<'a> {
    pub config: WavpackConfig,

    pub wrapper_data: Option<Vec<u8>>,
    pub wrapper_bytes: u32,

    pub wv_in: Box<dyn WavpackStreamReader64 + Send + 'a>,
    pub wvc_in: Option<Box<dyn WavpackStreamReader64 + Send + 'a>>,

    pub filelen: i64,
    pub file2len: i64,
    pub filepos: i64,
    pub file2pos: i64,
    pub total_samples: i64,
    pub initial_index: i64,
    pub crc_errors: u32,
    pub wvc_flag: i32,
    pub open_flags: i32,
    pub norm_offset: i32,
    pub reduced_channels: i32,
    pub lossy_blocks: i32,
    pub version_five: i32,
    pub m_tag: MTag,

    pub max_streams: i32,
    pub streams: Vec<Box<WavpackStream>>,

    // these items were added in 5.0 to support alternate file types (especially CAF & DSD)
    pub file_format: u8,
    pub channel_reordering: Option<Vec<u8>>,
    pub channel_identities: Option<Vec<u8>>,
    pub channel_layout: u32,
    pub dsd_multiplier: u32,
    pub file_extension: [u8; 8],

    pub error_message: String,
}

impl std::fmt::Debug for WavpackContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WavpackContext")
            .field("config", &self.config)
            .field("total_samples", &self.total_samples)
            .finish_non_exhaustive()
    }
}

//////////////////////////////// decorrelation //////////////////////////////
// modules: pack.c, unpack.c, unpack_floats.c, extra1.c, extra2.c

// These macros implement the weight application and update operations
// that are at the heart of the decorrelation loops. Note that there are
// sometimes two and even three versions of each macro. These should be
// equivalent and produce identical results, but some may perform better
// or worse on a given architecture.

// PERFCOND - apply decorrelation weight when no 32-bit overflow possible
/// Translation of `apply_weight_i()`.
#[inline]
pub(crate) fn apply_weight_i(weight: i32, sample: i32) -> i32 {
    weight.wrapping_mul(sample).wrapping_add(512) >> 10
}

// PERFCOND - apply decorrelation weight when 32-bit overflow is possible
/// Translation of `apply_weight_f()`.
#[inline]
pub(crate) fn apply_weight_f(weight: i32, sample: i32) -> i32 {
    ((((sample & 0xffff).wrapping_mul(weight)) >> 9)
        .wrapping_add(((sample & !0xffff) >> 9).wrapping_mul(weight))
        .wrapping_add(1))
        >> 1
}

// PERFCOND - universal version that checks input magnitude or always uses long version
/// Translation of `apply_weight()`.
#[inline]
pub(crate) fn apply_weight(weight: i32, sample: i32) -> i32 {
    if sample != sample as i16 as i32 {
        apply_weight_f(weight, sample)
    } else {
        apply_weight_i(weight, sample)
    }
}

// PERFCOND
/// Translation of `update_weight()`.
#[inline]
pub(crate) fn update_weight(weight: &mut i32, delta: i32, source: i32, result: i32) {
    if source != 0 && result != 0 {
        let s = (source ^ result) >> 31;
        *weight = (delta ^ s).wrapping_add(weight.wrapping_sub(s));
    }
}

/// Translation of `update_weight_clip()`.
#[inline]
pub(crate) fn update_weight_clip(weight: &mut i32, delta: i32, source: i32, result: i32) {
    if source != 0 && result != 0 {
        let s = (source ^ result) >> 31;
        *weight = (*weight ^ s).wrapping_add(delta.wrapping_sub(s));
        if *weight > 1024 {
            *weight = 1024;
        }
        *weight = (*weight ^ s).wrapping_sub(s);
    }
}

////////////////////////////// bitstream macros & functions /////////////////////////////

/// Translation of `bs_is_open()`.
#[inline]
pub(crate) fn bs_is_open(bs: &Bitstream) -> bool {
    bs.open
}

/// Translation of the `getbit()` macro.
#[inline]
pub(crate) fn getbit(bs: &mut Bitstream) -> u32 {
    let bit = if bs.bc != 0 {
        bs.bc -= 1;
        bs.sr & 1
    } else {
        bs.advance();
        bs.bc = 16 - 1;
        bs.sr = bs.word();
        bs.sr & 1
    };
    bs.sr >>= 1;
    bit
}

/// Translation of the `getbits()` macro (the bits are the return value).
#[inline]
pub(crate) fn getbits(nbits: i32, bs: &mut Bitstream) -> u32 {
    while nbits > bs.bc {
        bs.advance();
        bs.sr |= bs.word().wrapping_shl(bs.bc as u32);
        bs.bc += 16;
    }
    let value = bs.sr;
    if bs.bc > 32 {
        bs.bc -= nbits;
        bs.sr = bs.word().wrapping_shr((16 - bs.bc) as u32);
    } else {
        bs.bc -= nbits;
        bs.sr = bs.sr.wrapping_shr(nbits as u32);
    }
    value
}

///////////////////////////// entropy encoder / decoder ////////////////////////////
// modules: entropy_utils.c, read_words.c, write_words.c

// these control the time constant "slow_level" which is used for hybrid mode
// that controls bitrate as a function of residual level (HYBRID_BITRATE).
pub(crate) const SLS: u32 = 8;
pub(crate) const SLO: u32 = 1 << (SLS - 1);

pub(crate) const LIMIT_ONES: u32 = 16; // maximum consecutive 1s sent for "div" data

// these control the time constant of the 3 median level breakpoints
pub(crate) const DIV0: u32 = 128; // 5/7 of samples
pub(crate) const DIV1: u32 = 64; // 10/49 of samples
pub(crate) const DIV2: u32 = 32; // 20/343 of samples

// this macro retrieves the specified median breakpoint (without frac; min = 1)
/// Translation of `GET_MED()`.
#[inline]
pub(crate) fn get_med(c: &EntropyData, med: usize) -> u32 {
    (c.median[med] >> 4).wrapping_add(1)
}

// These macros update the specified median breakpoints. Note that the median
// is incremented when the sample is higher than the median, else decremented.
// They are designed so that the median will never drop below 1 and the value
// is essentially stationary if there are 2 increments for every 5 decrements.

/// Translation of `INC_MED0()`.
#[inline]
pub(crate) fn inc_med0(c: &mut EntropyData) {
    c.median[0] = c.median[0].wrapping_add((c.median[0].wrapping_add(DIV0) / DIV0).wrapping_mul(5));
}
/// Translation of `DEC_MED0()`.
#[inline]
pub(crate) fn dec_med0(c: &mut EntropyData) {
    c.median[0] =
        c.median[0].wrapping_sub((c.median[0].wrapping_add(DIV0 - 2) / DIV0).wrapping_mul(2));
}
/// Translation of `INC_MED1()`.
#[inline]
pub(crate) fn inc_med1(c: &mut EntropyData) {
    c.median[1] = c.median[1].wrapping_add((c.median[1].wrapping_add(DIV1) / DIV1).wrapping_mul(5));
}
/// Translation of `DEC_MED1()`.
#[inline]
pub(crate) fn dec_med1(c: &mut EntropyData) {
    c.median[1] =
        c.median[1].wrapping_sub((c.median[1].wrapping_add(DIV1 - 2) / DIV1).wrapping_mul(2));
}
/// Translation of `INC_MED2()`.
#[inline]
pub(crate) fn inc_med2(c: &mut EntropyData) {
    c.median[2] = c.median[2].wrapping_add((c.median[2].wrapping_add(DIV2) / DIV2).wrapping_mul(5));
}
/// Translation of `DEC_MED2()`.
#[inline]
pub(crate) fn dec_med2(c: &mut EntropyData) {
    c.median[2] =
        c.median[2].wrapping_sub((c.median[2].wrapping_add(DIV2 - 2) / DIV2).wrapping_mul(2));
}

/// Translation of `count_bits()` (the `HAVE___BUILTIN_CLZ` version).
#[inline]
pub(crate) fn count_bits(av: u32) -> i32 {
    if av != 0 {
        32 - av.leading_zeros() as i32
    } else {
        0
    }
}

pub(crate) const WORD_EOF: i32 = (1u32 << 31) as i32;
