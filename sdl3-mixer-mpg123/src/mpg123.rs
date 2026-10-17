/*
    libmpg123: MPEG Audio Decoder library (version 1.33.7)

    copyright 1995-2023 by the mpg123 project
    free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
*/
// Modified 2026-10-17: translated into Rust from mpg123.h and fmt123.h in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The constants of the public API (`mpg123.h`, `fmt123.h`), as `i32`s
//! with upstream's names.

#![allow(missing_docs)]

/// `MPG123_API_VERSION`
pub const MPG123_API_VERSION: u32 = 49;

// enum mpg123_parms
/** set verbosity value for enabling messages to stderr, >= 0 makes sense (integer) */
pub const MPG123_VERBOSE: i32 = 0;
/** set all flags, p.ex val = MPG123_GAPLESS|MPG123_MONO_MIX (integer) */
pub const MPG123_FLAGS: i32 = 1;
/** add some flags (integer) */
pub const MPG123_ADD_FLAGS: i32 = 2;
/** when value > 0, force output rate to that value (integer) */
pub const MPG123_FORCE_RATE: i32 = 3;
/** 0=native rate, 1=half rate, 2=quarter rate (integer) */
pub const MPG123_DOWN_SAMPLE: i32 = 4;
/** one of the RVA choices above (integer) */
pub const MPG123_RVA: i32 = 5;
/** play a frame N times (integer) */
pub const MPG123_DOWNSPEED: i32 = 6;
/** play every Nth frame (integer) */
pub const MPG123_UPSPEED: i32 = 7;
/** start with this frame (skip frames before that, integer) */
pub const MPG123_START_FRAME: i32 = 8;
/** decode only this number of frames (integer) */
pub const MPG123_DECODE_FRAMES: i32 = 9;
/** Stream contains ICY metadata with this interval (integer). */
pub const MPG123_ICY_INTERVAL: i32 = 10;
/** the scale for output samples (amplitude - integer or float according to mpg123 output format, normally integer) */
pub const MPG123_OUTSCALE: i32 = 11;
/** timeout for reading from a stream (not supported on win32, integer) */
pub const MPG123_TIMEOUT: i32 = 12;
/** remove some flags (inverse of MPG123_ADD_FLAGS, integer) */
pub const MPG123_REMOVE_FLAGS: i32 = 13;
/** Try resync on frame parsing for that many bytes or until end of stream (<0 ... integer). */
pub const MPG123_RESYNC_LIMIT: i32 = 14;
/** Set the frame index size (if supported). */
pub const MPG123_INDEX_SIZE: i32 = 15;
/** Decode/ignore that many frames in advance for layer 3. */
pub const MPG123_PREFRAMES: i32 = 16;
/** For feeder mode, keep that many buffers in a pool to avoid frequent malloc/free. */
pub const MPG123_FEEDPOOL: i32 = 17;
/** Minimal size of one internal feeder buffer. */
pub const MPG123_FEEDBUFFER: i32 = 18;
/** Tell the parser a free-format frame size to avoid read-ahead to get it. */
pub const MPG123_FREEFORMAT_SIZE: i32 = 19;

// enum mpg123_param_flags
/**     0111 Force some mono mode: This is a test bitmask for seeing if any mono forcing is active. */
pub const MPG123_FORCE_MONO: i64 = 0x7;
/**     0001 Force playback of left channel only.  */
pub const MPG123_MONO_LEFT: i64 = 0x1;
/**     0010 Force playback of right channel only. */
pub const MPG123_MONO_RIGHT: i64 = 0x2;
/**     0100 Force playback of mixed mono.         */
pub const MPG123_MONO_MIX: i64 = 0x4;
/**     1000 Force stereo output.                  */
pub const MPG123_FORCE_STEREO: i64 = 0x8;
/** 00010000 Force 8bit formats.                   */
pub const MPG123_FORCE_8BIT: i64 = 0x10;
/** 00100000 Suppress any printouts (overrules verbose).                    */
pub const MPG123_QUIET: i64 = 0x20;
/** 01000000 Enable gapless decoding (default on if libmpg123 has support). */
pub const MPG123_GAPLESS: i64 = 0x40;
/** 10000000 Disable resync stream after error.                             */
pub const MPG123_NO_RESYNC: i64 = 0x80;
/** 000100000000 Enable small buffer on non-seekable streams to allow some peek-ahead (for better MPEG sync). */
pub const MPG123_SEEKBUFFER: i64 = 0x100;
/** 001000000000 Enable fuzzy seeks (guessing byte offsets or using approximate seek points from Xing TOC) */
pub const MPG123_FUZZY: i64 = 0x200;
/** 010000000000 Force floating point output (32 or 64 bits depends on mpg123 internal precision). */
pub const MPG123_FORCE_FLOAT: i64 = 0x400;
/** 100000000000 Do not convert ID3 text data. */
pub const MPG123_PLAIN_ID3TEXT: i64 = 0x800;
/** 1000000000000 Ignore any stream length information contained in the stream. */
pub const MPG123_IGNORE_STREAMLENGTH: i64 = 0x1000;
/** 10 0000 0000 0000 Do not parse ID3v2 tags, just skip them. */
pub const MPG123_SKIP_ID3V2: i64 = 0x2000;
/** 100 0000 0000 0000 Do not parse the LAME/Xing info frame, treat it as normal MPEG data. */
pub const MPG123_IGNORE_INFOFRAME: i64 = 0x4000;
/** 1000 0000 0000 0000 Allow automatic internal resampling of any kind (default on if supported). */
pub const MPG123_AUTO_RESAMPLE: i64 = 0x8000;
/** 17th bit: Enable storage of pictures from tags (ID3v2 APIC). */
pub const MPG123_PICTURE: i64 = 0x10000;
/** 18th bit: Do not seek to the end of the stream in order to probe the stream length and search for the id3v1 field. */
pub const MPG123_NO_PEEK_END: i64 = 0x20000;
/** 19th bit: Force the stream to be seekable. */
pub const MPG123_FORCE_SEEKABLE: i64 = 0x40000;
/** Store raw ID3 data (even if skipping). */
pub const MPG123_STORE_RAW_ID3: i64 = 0x80000;
/** Enforce endianess of output samples. */
pub const MPG123_FORCE_ENDIAN: i64 = 0x100000;
/** Choose big endian instead of little. */
pub const MPG123_BIG_ENDIAN: i64 = 0x200000;
/** Disable read-ahead in parser. */
pub const MPG123_NO_READAHEAD: i64 = 0x400000;
/** Consider floating point output encoding only after trying other (possibly downsampled) rates. */
pub const MPG123_FLOAT_FALLBACK: i64 = 0x800000;
/** Disable support for Frankenstein streams. */
pub const MPG123_NO_FRANKENSTEIN: i64 = 0x1000000;

// enum mpg123_param_rva
/** RVA disabled (default).   */
pub const MPG123_RVA_OFF: i32 = 0;
/** Use mix/track/radio gain. */
pub const MPG123_RVA_MIX: i32 = 1;
/** Use album/audiophile gain */
pub const MPG123_RVA_ALBUM: i32 = 2;
/** The maximum RVA code, may increase in future. */
pub const MPG123_RVA_MAX: i32 = MPG123_RVA_ALBUM;

// enum mpg123_errors
/** Message: Track ended. Stop decoding. */
pub const MPG123_DONE: i32 = -12;
/** Message: Output format will be different on next call. */
pub const MPG123_NEW_FORMAT: i32 = -11;
/** Message: For feed reader: "Feed me more!" */
pub const MPG123_NEED_MORE: i32 = -10;
/** Generic Error */
pub const MPG123_ERR: i32 = -1;
/** Success */
pub const MPG123_OK: i32 = 0;
/** Unable to set up output format! */
pub const MPG123_BAD_OUTFORMAT: i32 = 1;
/** Invalid channel number specified. */
pub const MPG123_BAD_CHANNEL: i32 = 2;
/** Invalid sample rate specified.  */
pub const MPG123_BAD_RATE: i32 = 3;
/** Unable to allocate memory for 16 to 8 converter table! */
pub const MPG123_ERR_16TO8TABLE: i32 = 4;
/** Bad parameter id! */
pub const MPG123_BAD_PARAM: i32 = 5;
/** Bad buffer given -- invalid pointer or too small size. */
pub const MPG123_BAD_BUFFER: i32 = 6;
/** Out of memory -- some malloc() failed. */
pub const MPG123_OUT_OF_MEM: i32 = 7;
/** You didn't initialize the library! */
pub const MPG123_NOT_INITIALIZED: i32 = 8;
/** Invalid decoder choice. */
pub const MPG123_BAD_DECODER: i32 = 9;
/** Invalid mpg123 handle. */
pub const MPG123_BAD_HANDLE: i32 = 10;
/** Unable to initialize frame buffers (out of memory?). */
pub const MPG123_NO_BUFFERS: i32 = 11;
/** Invalid RVA mode. */
pub const MPG123_BAD_RVA: i32 = 12;
/** This build doesn't support gapless decoding. */
pub const MPG123_NO_GAPLESS: i32 = 13;
/** Not enough buffer space. */
pub const MPG123_NO_SPACE: i32 = 14;
/** Incompatible numeric data types. */
pub const MPG123_BAD_TYPES: i32 = 15;
/** Bad equalizer band. */
pub const MPG123_BAD_BAND: i32 = 16;
/** Null pointer given where valid storage address needed. */
pub const MPG123_ERR_NULL: i32 = 17;
/** Error reading the stream. */
pub const MPG123_ERR_READER: i32 = 18;
/** Cannot seek from end (end is not known). */
pub const MPG123_NO_SEEK_FROM_END: i32 = 19;
/** Invalid 'whence' for seek function.*/
pub const MPG123_BAD_WHENCE: i32 = 20;
/** Build does not support stream timeouts. */
pub const MPG123_NO_TIMEOUT: i32 = 21;
/** File access error. */
pub const MPG123_BAD_FILE: i32 = 22;
/** Seek not supported by stream. */
pub const MPG123_NO_SEEK: i32 = 23;
/** No stream opened or no reader callback setup. */
pub const MPG123_NO_READER: i32 = 24;
/** Bad parameter handle. */
pub const MPG123_BAD_PARS: i32 = 25;
/** Bad parameters to mpg123_index() and mpg123_set_index() */
pub const MPG123_BAD_INDEX_PAR: i32 = 26;
/** Lost track in bytestream and did not try to resync. */
pub const MPG123_OUT_OF_SYNC: i32 = 27;
/** Resync failed to find valid MPEG data. */
pub const MPG123_RESYNC_FAIL: i32 = 28;
/** No 8bit encoding possible. */
pub const MPG123_NO_8BIT: i32 = 29;
/** Stack aligmnent error */
pub const MPG123_BAD_ALIGN: i32 = 30;
/** NULL input buffer with non-zero size... */
pub const MPG123_NULL_BUFFER: i32 = 31;
/** Relative seek not possible (screwed up file offset) */
pub const MPG123_NO_RELSEEK: i32 = 32;
/** You gave a null pointer somewhere where you shouldn't have. */
pub const MPG123_NULL_POINTER: i32 = 33;
/** Bad key value given. */
pub const MPG123_BAD_KEY: i32 = 34;
/** No frame index in this build. */
pub const MPG123_NO_INDEX: i32 = 35;
/** Something with frame index went wrong. */
pub const MPG123_INDEX_FAIL: i32 = 36;
/** Something prevents a proper decoder setup */
pub const MPG123_BAD_DECODER_SETUP: i32 = 37;
/** This feature has not been built into libmpg123. */
pub const MPG123_MISSING_FEATURE: i32 = 38;
/** A bad value has been given, somewhere. */
pub const MPG123_BAD_VALUE: i32 = 39;
/** Low-level seek failed. */
pub const MPG123_LSEEK_FAILED: i32 = 40;
/** Custom I/O not prepared. */
pub const MPG123_BAD_CUSTOM_IO: i32 = 41;
/** Offset value overflow during translation of large file API calls. */
pub const MPG123_LFS_OVERFLOW: i32 = 42;
/** Some integer overflow. */
pub const MPG123_INT_OVERFLOW: i32 = 43;
/** Floating-point computations work not as expected. */
pub const MPG123_BAD_FLOAT: i32 = 44;

// enum mpg123_channelcount
/** mono */
pub const MPG123_MONO: i32 = 1;
/** stereo */
pub const MPG123_STEREO: i32 = 2;

// enum mpg123_channels
/** The Left Channel. */
pub const MPG123_LEFT: i32 = 0x1;
/** The Right Channel. */
pub const MPG123_RIGHT: i32 = 0x2;
/** Both left and right channel; same as MPG123_LEFT|MPG123_RIGHT */
pub const MPG123_LR: i32 = 0x3;

// enum mpg123_vbr
/** Constant Bitrate Mode (default) */
pub const MPG123_CBR: i32 = 0;
/** Variable Bitrate Mode */
pub const MPG123_VBR: i32 = 1;
/** Average Bitrate Mode */
pub const MPG123_ABR: i32 = 2;

// enum mpg123_version
/** MPEG Version 1.0 */
pub const MPG123_1_0: i32 = 0;
/** MPEG Version 2.0 */
pub const MPG123_2_0: i32 = 1;
/** MPEG Version 2.5 */
pub const MPG123_2_5: i32 = 2;

// enum mpg123_mode
/** Standard Stereo. */
pub const MPG123_M_STEREO: i32 = 0;
/** Joint Stereo. */
pub const MPG123_M_JOINT: i32 = 1;
/** Dual Channel. */
pub const MPG123_M_DUAL: i32 = 2;
/** Single Channel. */
pub const MPG123_M_MONO: i32 = 3;

// enum mpg123_flags
/** The bitstream is error protected using 16-bit CRC. */
pub const MPG123_CRC: i32 = 0x1;
/** The bitstream is copyrighted. */
pub const MPG123_COPYRIGHT: i32 = 0x2;
/** The private bit has been set. */
pub const MPG123_PRIVATE: i32 = 0x4;
/** The bitstream is an original, not a copy. */
pub const MPG123_ORIGINAL: i32 = 0x8;

// enum mpg123_state
/** Query if positons are currently accurate (integer value, 0 if false, 1 if true). */
pub const MPG123_ACCURATE: i32 = 1;
/** Get fill of internal (feed) input buffer as integer byte count returned as long and as double. */
pub const MPG123_BUFFERFILL: i32 = 2;
/** Stream consists of carelessly stitched together files. */
pub const MPG123_FRANKENSTEIN: i32 = 3;
/** Decoder structure has been updated, possibly indicating changed stream. Flag is cleared after retrieval. */
pub const MPG123_FRESH_DECODER: i32 = 4;
/** Encoder delay read from Info tag (layer III, -1 if unknown). */
pub const MPG123_ENC_DELAY: i32 = 5;
/** Encoder padding read from Info tag (layer III, -1 if unknown). */
pub const MPG123_ENC_PADDING: i32 = 6;
/** Decoder delay (for layer III only, -1 otherwise). */
pub const MPG123_DEC_DELAY: i32 = 7;

/** 0011 There is some ID3 info. Also matches 0010 or NEW_ID3. */
pub const MPG123_ID3: i32 = 0x3;
/** 0001 There is ID3 info that changed since last call to mpg123_id3. */
pub const MPG123_NEW_ID3: i32 = 0x1;
/** 1100 There is some ICY info. Also matches 0100 or NEW_ICY.*/
pub const MPG123_ICY: i32 = 0xc;
/** 0100 There is ICY info that changed since last call to mpg123_icy. */
pub const MPG123_NEW_ICY: i32 = 0x4;

// enum mpg123_text_encoding
/** Unkown encoding... mpg123_id3_encoding can return that on invalid codes. */
pub const MPG123_TEXT_UNKNOWN: i32 = 0;
/** UTF-8 */
pub const MPG123_TEXT_UTF8: i32 = 1;
/** ISO-8859-1. */
pub const MPG123_TEXT_LATIN1: i32 = 2;
/** ICY metadata encoding, usually CP-1252 but we take it as UTF-8 if it qualifies as such. */
pub const MPG123_TEXT_ICY: i32 = 3;
/** Really CP-1252 without any guessing. */
pub const MPG123_TEXT_CP1252: i32 = 4;
/** Some UTF-16 encoding. */
pub const MPG123_TEXT_UTF16: i32 = 5;
/** Just an alias for UTF-16, ID3v2 has this as distinct code. */
pub const MPG123_TEXT_UTF16BOM: i32 = 6;
/** Another alias for UTF16 from ID3v2. */
pub const MPG123_TEXT_UTF16BE: i32 = 7;

// enum mpg123_id3_enc
/** Note: This sometimes can mean anything in practice... */
pub const MPG123_ID3_LATIN1: u8 = 0;
/** UTF16, UCS-2 ... it's all the same for practical purposes. */
pub const MPG123_ID3_UTF16BOM: u8 = 1;
/** Big-endian UTF-16, BOM see note for mpg123_text_utf16be. */
pub const MPG123_ID3_UTF16BE: u8 = 2;
/** Our lovely overly ASCII-compatible 8 byte encoding for the world. */
pub const MPG123_ID3_UTF8: u8 = 3;
/** Placeholder to check valid range of encoding byte. */
pub const MPG123_ID3_ENC_MAX: u8 = 3;

// enum mpg123_enc_enum (fmt123.h)
/* 0000 0000 0000 1111 Some 8 bit  integer encoding. */
pub const MPG123_ENC_8: i32 = 0x00f;
/* 0000 0000 0100 0000 Some 16 bit integer encoding. */
pub const MPG123_ENC_16: i32 = 0x040;
/* 0100 0000 0000 0000 Some 24 bit integer encoding. */
pub const MPG123_ENC_24: i32 = 0x4000;
/* 0000 0001 0000 0000 Some 32 bit integer encoding. */
pub const MPG123_ENC_32: i32 = 0x100;
/* 0000 0000 1000 0000 Some signed integer encoding. */
pub const MPG123_ENC_SIGNED: i32 = 0x080;
/* 0000 1110 0000 0000 Some float encoding. */
pub const MPG123_ENC_FLOAT: i32 = 0xe00;
/* 0000 0000 1101 0000 signed 16 bit */
pub const MPG123_ENC_SIGNED_16: i32 = MPG123_ENC_16 | MPG123_ENC_SIGNED | 0x10;
/* 0000 0000 0110 0000 unsigned 16 bit */
pub const MPG123_ENC_UNSIGNED_16: i32 = MPG123_ENC_16 | 0x20;
/* 0000 0000 0000 0001 unsigned 8 bit */
pub const MPG123_ENC_UNSIGNED_8: i32 = 0x01;
/* 0000 0000 1000 0010 signed 8 bit */
pub const MPG123_ENC_SIGNED_8: i32 = MPG123_ENC_SIGNED | 0x02;
/* 0000 0000 0000 0100 ulaw 8 bit */
pub const MPG123_ENC_ULAW_8: i32 = 0x04;
/* 0000 0000 0000 1000 alaw 8 bit */
pub const MPG123_ENC_ALAW_8: i32 = 0x08;
/* 0001 0001 1000 0000 signed 32 bit */
pub const MPG123_ENC_SIGNED_32: i32 = MPG123_ENC_32 | MPG123_ENC_SIGNED | 0x1000;
/* 0010 0001 0000 0000 unsigned 32 bit */
pub const MPG123_ENC_UNSIGNED_32: i32 = MPG123_ENC_32 | 0x2000;
/* 0101 0000 1000 0000 signed 24 bit */
pub const MPG123_ENC_SIGNED_24: i32 = MPG123_ENC_24 | MPG123_ENC_SIGNED | 0x1000;
/* 0110 0000 0000 0000 unsigned 24 bit */
pub const MPG123_ENC_UNSIGNED_24: i32 = MPG123_ENC_24 | 0x2000;
/* 0000 0010 0000 0000 32bit float */
pub const MPG123_ENC_FLOAT_32: i32 = 0x200;
/* 0000 0100 0000 0000 64bit float */
pub const MPG123_ENC_FLOAT_64: i32 = 0x400;
/* Any possibly known encoding from the list above. */
pub const MPG123_ENC_ANY: i32 = MPG123_ENC_SIGNED_16
    | MPG123_ENC_UNSIGNED_16
    | MPG123_ENC_UNSIGNED_8
    | MPG123_ENC_SIGNED_8
    | MPG123_ENC_ULAW_8
    | MPG123_ENC_ALAW_8
    | MPG123_ENC_SIGNED_32
    | MPG123_ENC_UNSIGNED_32
    | MPG123_ENC_SIGNED_24
    | MPG123_ENC_UNSIGNED_24
    | MPG123_ENC_FLOAT_32
    | MPG123_ENC_FLOAT_64;

/// Translation of the `MPG123_SAMPLESIZE()` macro: the size of one PCM
/// sample with given encoding.
pub const fn mpg123_samplesize(enc: i32) -> i32 {
    if enc < 1 {
        0
    } else if enc & MPG123_ENC_8 != 0 {
        1
    } else if enc & MPG123_ENC_16 != 0 {
        2
    } else if enc & MPG123_ENC_24 != 0 {
        3
    } else if (enc & MPG123_ENC_32 != 0) || enc == MPG123_ENC_FLOAT_32 {
        4
    } else if enc == MPG123_ENC_FLOAT_64 {
        8
    } else {
        0
    }
}
