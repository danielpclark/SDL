// Rust translation of zlib 1.3.1 (https://zlib.net, as SDL_image's
// external/zlib pins it): zlib.h, zconf.h, zutil.h and zutil.c, and the
// inflater and deflater files that libpng calls.
// Copyright (C) 1995-2024 Jean-loup Gailly and Mark Adler.
// This is an altered (translated) version of the original software; zlib is
// used under the zlib license (see zlib.h and LICENSE.txt).

//! The parts of zlib that libpng's APNG paths in SDL_image use:
//! `inflate()` (with `inflateInit2()`, `inflateReset2()` and
//! `inflateEnd()`), `deflate()` (with `deflateInit2()`, `deflateReset()`
//! and `deflateEnd()`), `crc32()` and `adler32()`.
//!
//! Translation notes:
//! - `z_stream` holds positions in the caller's buffers (`next_in`,
//!   `next_out`) rather than pointers; `inflate()` and `deflate()` take the
//!   input and output buffers those positions index.
//! - Memory comes from Rust allocations (libpng routes zlib's `zalloc` to
//!   `png_malloc_warn()`); allocation failure maps to `Z_MEM_ERROR`. The
//!   allocations are zeroed, where zlib's `malloc()` leaves them as they
//!   are (zlib never lets those bytes reach its output).
//! - The fixed inflate tables are built on first use, as zlib does with
//!   `BUILDFIXED` (they are identical to `inffixed.h`), and the static
//!   deflate trees by const evaluation of `tr_static_init()` (identical to
//!   `trees.h`, which it generates).
//! - `crc32()` uses zlib's byte-wise table loop rather than its braided
//!   word-at-a-time variant; both compute the same CRC.
//! - The gzip wrapper, dictionaries, `deflateParams()`, `deflateBound()`,
//!   `deflateCopy()`, `inflateSync()`, `inflateGetHeader()` and the other
//!   entry points libpng doesn't call are not translated (libpng only asks
//!   for zlib streams with 8 to 15 window bits).
//!
//! The inflater is adapted from this workspace's translation of the zlib
//! 1.3 copy FreeType bundles (in `sdl3-ttf`), which has a one-shot API over
//! borrowed buffers; libpng needs a stream that lasts across calls with
//! changing buffers, 1.3.1's tables and the deflater, so the crates don't
//! share it.

// (the translation follows the C code's loops, branches, and arithmetic)
#![allow(
    clippy::manual_div_ceil,
    clippy::manual_is_multiple_of,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::too_many_lines
)]
// (zlib.h's whole constant set, and the state fields only the entry points
// not translated read)
#![allow(dead_code)]

pub(crate) mod adler32;
pub(crate) mod crc32;
pub(crate) mod deflate;
pub(crate) mod inffast;
pub(crate) mod inflate;
pub(crate) mod inftrees;
pub(crate) mod trees;

pub(crate) use adler32::adler32;
pub(crate) use crc32::crc32;
pub(crate) use deflate::{deflate, deflate_end, deflate_init2, deflate_reset};
// (inflateEnd() and inflateReset(): for zlib's API; libpng's inflate
// state is dropped, and reset with inflateReset2())
#[allow(unused_imports)]
pub(crate) use inflate::{inflate, inflate_end, inflate_init2, inflate_reset, inflate_reset2};

/* zlib.h */

pub(crate) const ZLIB_VERNUM: u32 = 0x1310;

/* Allowed flush values; see deflate() and inflate() below for details */
pub(crate) const Z_NO_FLUSH: i32 = 0;
pub(crate) const Z_PARTIAL_FLUSH: i32 = 1;
pub(crate) const Z_SYNC_FLUSH: i32 = 2;
pub(crate) const Z_FULL_FLUSH: i32 = 3;
pub(crate) const Z_FINISH: i32 = 4;
pub(crate) const Z_BLOCK: i32 = 5;
pub(crate) const Z_TREES: i32 = 6;

/* Return codes for the compression/decompression functions. Negative values
 * are errors, positive values are used for special but normal events.
 */
pub(crate) const Z_OK: i32 = 0;
pub(crate) const Z_STREAM_END: i32 = 1;
pub(crate) const Z_NEED_DICT: i32 = 2;
pub(crate) const Z_ERRNO: i32 = -1;
pub(crate) const Z_STREAM_ERROR: i32 = -2;
pub(crate) const Z_DATA_ERROR: i32 = -3;
pub(crate) const Z_MEM_ERROR: i32 = -4;
pub(crate) const Z_BUF_ERROR: i32 = -5;
pub(crate) const Z_VERSION_ERROR: i32 = -6;

/* compression levels */
pub(crate) const Z_DEFAULT_COMPRESSION: i32 = -1;

/* compression strategy; see deflateInit2() below for details */
pub(crate) const Z_FILTERED: i32 = 1;
pub(crate) const Z_HUFFMAN_ONLY: i32 = 2;
pub(crate) const Z_RLE: i32 = 3;
pub(crate) const Z_FIXED: i32 = 4;
pub(crate) const Z_DEFAULT_STRATEGY: i32 = 0;

/* Possible values of the data_type field for deflate() */
pub(crate) const Z_BINARY: i32 = 0;
pub(crate) const Z_TEXT: i32 = 1;
pub(crate) const Z_UNKNOWN: i32 = 2;

/* The deflate compression method (the only one supported in this version) */
pub(crate) const Z_DEFLATED: i32 = 8;

/* zconf.h */

/* Maximum value for memLevel in deflateInit2 */
pub(crate) const MAX_MEM_LEVEL: i32 = 9;

/* Maximum value for windowBits in deflateInit2 and inflateInit2.
 * WARNING: reducing MAX_WBITS makes minigzip unable to extract .gz files
 * created by gzip. (Files created by minigzip can still be extracted by
 * gzip.)
 */
pub(crate) const MAX_WBITS: i32 = 15; /* 32K LZ77 window */

/// `z_stream`: the state of a compression or decompression stream. The
/// buffers aren't part of it: `next_in` and `next_out` are positions in
/// the buffers passed to each `inflate()` or `deflate()` call.
#[derive(Debug, Default)]
pub(crate) struct ZStream {
    pub(crate) next_in: usize, /* next input byte */
    pub(crate) avail_in: u32,  /* number of bytes available at next_in */
    pub(crate) total_in: u64,  /* total number of input bytes read so far */

    pub(crate) next_out: usize, /* next output byte will go here */
    pub(crate) avail_out: u32,  /* remaining free space at next_out */
    pub(crate) total_out: u64,  /* total number of bytes output so far */

    pub(crate) msg: Option<&'static str>, /* last error message, NULL if no error */
    /* not visible by applications (the inflate or the deflate state) */
    pub(crate) istate: Option<Box<inflate::InflateState>>,
    pub(crate) dstate: Option<Box<deflate::DeflateState>>,

    pub(crate) data_type: i32, /* best guess about the data type: binary or text
                               for deflate, or the decoding state for inflate */
    pub(crate) adler: u64, /* Adler-32 or CRC-32 value of the uncompressed data */
}

/* zutil.h */

/* default windowBits for decompression. MAX_WBITS is for compression only */
pub(crate) const DEF_MEM_LEVEL: i32 = 8;
/* default memLevel */

pub(crate) const STORED_BLOCK: i32 = 0;
pub(crate) const STATIC_TREES: i32 = 1;
pub(crate) const DYN_TREES: i32 = 2;
/* The three kinds of block type */

pub(crate) const MIN_MATCH: u32 = 3;
pub(crate) const MAX_MATCH: u32 = 258;
/* The minimum and maximum match lengths */

pub(crate) const PRESET_DICT: u32 = 0x20; /* preset dictionary flag in zlib header */

/* zutil.c */

/// `z_errmsg`
static Z_ERRMSG: [&str; 10] = [
    "need dictionary",      /* Z_NEED_DICT       2  */
    "stream end",           /* Z_STREAM_END      1  */
    "",                     /* Z_OK              0  */
    "file error",           /* Z_ERRNO         (-1) */
    "stream error",         /* Z_STREAM_ERROR  (-2) */
    "data error",           /* Z_DATA_ERROR    (-3) */
    "insufficient memory",  /* Z_MEM_ERROR     (-4) */
    "buffer error",         /* Z_BUF_ERROR     (-5) */
    "incompatible version", /* Z_VERSION_ERROR (-6) */
    "",
];

/// `ERR_MSG(err)`
pub(crate) fn err_msg(err: i32) -> &'static str {
    Z_ERRMSG[if !(-6..=2).contains(&err) {
        9
    } else {
        (2 - err) as usize
    }]
}

/// `zcalloc()` (through libpng's `png_zalloc()`): `n` zeroed elements, or
/// `None` when they can't be allocated.
pub(crate) fn zalloc<T: Clone + Default>(n: usize) -> Option<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    v.resize(n, T::default());
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        assert_eq!(adler32(1, Some(b"Wikipedia")), 0x11E60398);
        assert_eq!(adler32(0, None), 1);
        assert_eq!(adler32(5, Some(&[])), 5);
        assert_eq!(crc32(0, Some(b"123456789")), 0xCBF43926);
        assert_eq!(crc32(7, None), 0);
    }

    /// Deflate `data` at `level` in pieces of `step` bytes into a buffer
    /// of `out_size` bytes at a time, then inflate it back.
    fn round_trip(data: &[u8], level: i32, window_bits: i32, step: usize, out_size: usize) {
        let mut strm = ZStream::default();
        assert_eq!(
            deflate_init2(
                &mut strm,
                level,
                Z_DEFLATED,
                window_bits,
                8,
                Z_DEFAULT_STRATEGY
            ),
            Z_OK
        );
        let mut compressed = Vec::new();
        let mut out = vec![0u8; out_size];
        let mut pos = 0;
        loop {
            let n = step.min(data.len() - pos);
            let input = &data[pos..pos + n];
            pos += n;
            let flush = if pos == data.len() {
                Z_FINISH
            } else {
                Z_NO_FLUSH
            };
            strm.next_in = 0;
            strm.avail_in = n as u32;
            loop {
                strm.next_out = 0;
                strm.avail_out = out_size as u32;
                let ret = deflate(&mut strm, input, &mut out, flush);
                assert!(ret == Z_OK || ret == Z_STREAM_END, "deflate: {ret}");
                compressed.extend_from_slice(&out[..out_size - strm.avail_out as usize]);
                if ret == Z_STREAM_END || (strm.avail_out != 0 && flush == Z_NO_FLUSH) {
                    break;
                }
            }
            if flush == Z_FINISH {
                break;
            }
        }
        assert_eq!(deflate_end(&mut strm), Z_OK);

        let mut back = vec![0u8; data.len() + 1];
        let mut strm = ZStream::default();
        assert_eq!(inflate_init2(&mut strm, 0), Z_OK);
        strm.avail_in = compressed.len() as u32;
        strm.avail_out = back.len() as u32;
        assert_eq!(
            inflate(&mut strm, &compressed, &mut back, Z_FINISH),
            Z_STREAM_END
        );
        assert_eq!(strm.total_out as usize, data.len());
        assert_eq!(inflate_end(&mut strm), Z_OK);
        assert_eq!(&back[..data.len()], data);
    }

    /// The round-trip test's data: runs and noise.
    fn test_data() -> Vec<u8> {
        let mut data = Vec::new();
        let mut x = 1u32;
        for i in 0..70000u32 {
            x = x.wrapping_mul(1103515245).wrapping_add(12345);
            data.push(if i % 1000 < 600 {
                (i / 7) as u8
            } else {
                (x >> 24) as u8
            });
        }
        data
    }

    #[test]
    fn deflate_round_trips() {
        let data = test_data();
        for level in [0, 1, 2, 3, 4, 6, 9] {
            round_trip(&data, level, 15, 1000, 8192);
            round_trip(&data[..3000], level, 9, 77, 100);
        }
        round_trip(&[], 6, 15, 1, 16);
    }

    /// The size and FNV-1a hash of what zlib 1.3.1's deflate() makes of
    /// the test data, for each level and strategy, in steps of input and
    /// output sizes (as a C program linked with SDL_image's external/zlib
    /// prints them).
    const DEFLATE_REFERENCE: &str = "\
0 15 0 70000 1000 8192 70021 152f56b782ab12e7\n\
0 9 0 3000 77 100 3036 3c3a212d600db5e9\n\
0 12 0 20000 20000 8192 20021 aedfe657314a8796\n\
0 15 1 70000 1000 8192 70021 152f56b782ab12e7\n\
0 9 1 3000 77 100 3036 3c3a212d600db5e9\n\
0 12 1 20000 20000 8192 20021 aedfe657314a8796\n\
0 15 2 70000 1000 8192 70021 152f56b782ab12e7\n\
0 9 2 3000 77 100 3036 3c3a212d600db5e9\n\
0 12 2 20000 20000 8192 20021 aedfe657314a8796\n\
0 15 3 70000 1000 8192 70021 152f56b782ab12e7\n\
0 9 3 3000 77 100 3036 3c3a212d600db5e9\n\
0 12 3 20000 20000 8192 20021 aedfe657314a8796\n\
0 15 4 70000 1000 8192 70021 152f56b782ab12e7\n\
0 9 4 3000 77 100 3036 3c3a212d600db5e9\n\
0 12 4 20000 20000 8192 20021 aedfe657314a8796\n\
1 15 0 70000 1000 8192 30269 54ea6fb0acd54e09\n\
1 9 0 3000 77 100 1689 1a42fd6fa9994a79\n\
1 12 0 20000 20000 8192 9547 1532a8a4db9df4bc\n\
1 15 1 70000 1000 8192 30269 54ea6fb0acd54e09\n\
1 9 1 3000 77 100 1689 1a42fd6fa9994a79\n\
1 12 1 20000 20000 8192 9547 1532a8a4db9df4bc\n\
1 15 2 70000 1000 8192 70031 411078c9a1bc1ba9\n\
1 9 2 3000 77 100 3027 368b7096889f8ef0\n\
1 12 2 20000 20000 8192 20045 3eb54c41e51a5bd7\n\
1 15 3 70000 1000 8192 38249 3e4f844071922547\n\
1 9 3 3000 77 100 1687 6f40ff3af35cc962\n\
1 12 3 20000 20000 8192 10940 ee17c573f80c461b\n\
1 15 4 70000 1000 8192 31746 771bc96704a20df0\n\
1 9 4 3000 77 100 1933 a803d57956456d4c\n\
1 12 4 20000 20000 8192 10416 26e2c1b3ca94a59a\n\
2 15 0 70000 1000 8192 30254 f484583c8dd62bb2\n\
2 9 0 3000 77 100 1689 c63b45bdd340f51f\n\
2 12 0 20000 20000 8192 9537 215e07542c79fa89\n\
2 15 1 70000 1000 8192 30254 f484583c8dd62bb2\n\
2 9 1 3000 77 100 1689 c63b45bdd340f51f\n\
2 12 1 20000 20000 8192 9537 215e07542c79fa89\n\
2 15 2 70000 1000 8192 70031 411078c9a1bc1ba9\n\
2 9 2 3000 77 100 3027 368b7096889f8ef0\n\
2 12 2 20000 20000 8192 20045 3eb54c41e51a5bd7\n\
2 15 3 70000 1000 8192 38249 3e4f844071922547\n\
2 9 3 3000 77 100 1687 6f40ff3af35cc962\n\
2 12 3 20000 20000 8192 10940 ee17c573f80c461b\n\
2 15 4 70000 1000 8192 31728 a61e0ecccec18872\n\
2 9 4 3000 77 100 1933 a803d57956456d4c\n\
2 12 4 20000 20000 8192 10407 be0c2f981f5d4f32\n\
3 15 0 70000 1000 8192 30251 0cfd2949b289318a\n\
3 9 0 3000 77 100 1689 c63b45bdd340f51f\n\
3 12 0 20000 20000 8192 9516 8054c0c2a16d4bf7\n\
3 15 1 70000 1000 8192 30251 0cfd2949b289318a\n\
3 9 1 3000 77 100 1689 c63b45bdd340f51f\n\
3 12 1 20000 20000 8192 9516 8054c0c2a16d4bf7\n\
3 15 2 70000 1000 8192 70031 411078c9a1bc1ba9\n\
3 9 2 3000 77 100 3027 368b7096889f8ef0\n\
3 12 2 20000 20000 8192 20045 3eb54c41e51a5bd7\n\
3 15 3 70000 1000 8192 38249 3e4f844071922547\n\
3 9 3 3000 77 100 1687 6f40ff3af35cc962\n\
3 12 3 20000 20000 8192 10940 ee17c573f80c461b\n\
3 15 4 70000 1000 8192 31754 0fc91a2b463d588e\n\
3 9 4 3000 77 100 1933 a803d57956456d4c\n\
3 12 4 20000 20000 8192 10405 833c69ba85ab38d7\n\
4 15 0 70000 1000 8192 29281 490cb7a9b8dc9ffe\n\
4 9 0 3000 77 100 1689 c63b45bdd340f51f\n\
4 12 0 20000 20000 8192 9458 540314387c0245ea\n\
4 15 1 70000 1000 8192 29282 3c6267444e879f93\n\
4 9 1 3000 77 100 1697 9328c6705a568e5b\n\
4 12 1 20000 20000 8192 9474 2b5034edd1a7d16a\n\
4 15 2 70000 1000 8192 70031 411078c9a1bc1ba9\n\
4 9 2 3000 77 100 3027 368b7096889f8ef0\n\
4 12 2 20000 20000 8192 20045 3eb54c41e51a5bd7\n\
4 15 3 70000 1000 8192 38249 3e4f844071922547\n\
4 9 3 3000 77 100 1687 6f40ff3af35cc962\n\
4 12 3 20000 20000 8192 10940 ee17c573f80c461b\n\
4 15 4 70000 1000 8192 30847 042467080e4a9f45\n\
4 9 4 3000 77 100 1933 a803d57956456d4c\n\
4 12 4 20000 20000 8192 10333 e7a2470522dfefac\n\
5 15 0 70000 1000 8192 29278 86c6a0ba4cbcc2b3\n\
5 9 0 3000 77 100 1689 c63b45bdd340f51f\n\
5 12 0 20000 20000 8192 9449 bdce9581635f2cb5\n\
5 15 1 70000 1000 8192 29278 9efad8ecc3363170\n\
5 9 1 3000 77 100 1697 9328c6705a568e5b\n\
5 12 1 20000 20000 8192 9471 d759a125d17906c1\n\
5 15 2 70000 1000 8192 70031 411078c9a1bc1ba9\n\
5 9 2 3000 77 100 3027 368b7096889f8ef0\n\
5 12 2 20000 20000 8192 20045 3eb54c41e51a5bd7\n\
5 15 3 70000 1000 8192 38249 3e4f844071922547\n\
5 9 3 3000 77 100 1687 6f40ff3af35cc962\n\
5 12 3 20000 20000 8192 10940 ee17c573f80c461b\n\
5 15 4 70000 1000 8192 30845 0b0e95db741e430c\n\
5 9 4 3000 77 100 1933 a803d57956456d4c\n\
5 12 4 20000 20000 8192 10327 0c441d88493390ef\n\
6 15 0 70000 1000 8192 29278 8663ab8bebf036e9\n\
6 9 0 3000 77 100 1689 7d73ed981040c885\n\
6 12 0 20000 20000 8192 9449 952c11406122bba3\n\
6 15 1 70000 1000 8192 29278 c70c3c03e69d6a82\n\
6 9 1 3000 77 100 1697 7e3583199b325795\n\
6 12 1 20000 20000 8192 9471 65f5942f15caa1df\n\
6 15 2 70000 1000 8192 70031 411078c9a1bc1ba9\n\
6 9 2 3000 77 100 3027 368b7096889f8ef0\n\
6 12 2 20000 20000 8192 20045 3eb54c41e51a5bd7\n\
6 15 3 70000 1000 8192 38249 3e4f844071922547\n\
6 9 3 3000 77 100 1687 6f40ff3af35cc962\n\
6 12 3 20000 20000 8192 10940 ee17c573f80c461b\n\
6 15 4 70000 1000 8192 30845 0b0e95db741e430c\n\
6 9 4 3000 77 100 1933 a803d57956456d4c\n\
6 12 4 20000 20000 8192 10327 0c441d88493390ef\n\
7 15 0 70000 1000 8192 29278 8e25c1b2a9627b77\n\
7 9 0 3000 77 100 1689 0c55dba9eaf296fb\n\
7 12 0 20000 20000 8192 9449 71fae33e521f4529\n\
7 15 1 70000 1000 8192 29278 8a0abe1dd0bdf144\n\
7 9 1 3000 77 100 1697 5c58f1ddbea44b87\n\
7 12 1 20000 20000 8192 9471 2edd52bd178c0a15\n\
7 15 2 70000 1000 8192 70031 411078c9a1bc1ba9\n\
7 9 2 3000 77 100 3027 368b7096889f8ef0\n\
7 12 2 20000 20000 8192 20045 3eb54c41e51a5bd7\n\
7 15 3 70000 1000 8192 38249 3e4f844071922547\n\
7 9 3 3000 77 100 1687 6f40ff3af35cc962\n\
7 12 3 20000 20000 8192 10940 ee17c573f80c461b\n\
7 15 4 70000 1000 8192 30845 0b0e95db741e430c\n\
7 9 4 3000 77 100 1933 a803d57956456d4c\n\
7 12 4 20000 20000 8192 10327 0c441d88493390ef\n\
8 15 0 70000 1000 8192 29251 ed3d816f82dd85dd\n\
8 9 0 3000 77 100 1689 0c55dba9eaf296fb\n\
8 12 0 20000 20000 8192 9449 71fae33e521f4529\n\
8 15 1 70000 1000 8192 29253 752503a5de0f60c1\n\
8 9 1 3000 77 100 1697 5c58f1ddbea44b87\n\
8 12 1 20000 20000 8192 9471 2edd52bd178c0a15\n\
8 15 2 70000 1000 8192 70031 411078c9a1bc1ba9\n\
8 9 2 3000 77 100 3027 368b7096889f8ef0\n\
8 12 2 20000 20000 8192 20045 3eb54c41e51a5bd7\n\
8 15 3 70000 1000 8192 38249 3e4f844071922547\n\
8 9 3 3000 77 100 1687 6f40ff3af35cc962\n\
8 12 3 20000 20000 8192 10940 ee17c573f80c461b\n\
8 15 4 70000 1000 8192 30806 780107bd2f788ba9\n\
8 9 4 3000 77 100 1933 a803d57956456d4c\n\
8 12 4 20000 20000 8192 10327 0c441d88493390ef\n\
9 15 0 70000 1000 8192 29251 ed3d816f82dd85dd\n\
9 9 0 3000 77 100 1689 0c55dba9eaf296fb\n\
9 12 0 20000 20000 8192 9449 71fae33e521f4529\n\
9 15 1 70000 1000 8192 29253 752503a5de0f60c1\n\
9 9 1 3000 77 100 1697 5c58f1ddbea44b87\n\
9 12 1 20000 20000 8192 9471 2edd52bd178c0a15\n\
9 15 2 70000 1000 8192 70031 411078c9a1bc1ba9\n\
9 9 2 3000 77 100 3027 368b7096889f8ef0\n\
9 12 2 20000 20000 8192 20045 3eb54c41e51a5bd7\n\
9 15 3 70000 1000 8192 38249 3e4f844071922547\n\
9 9 3 3000 77 100 1687 6f40ff3af35cc962\n\
9 12 3 20000 20000 8192 10940 ee17c573f80c461b\n\
9 15 4 70000 1000 8192 30806 780107bd2f788ba9\n\
9 9 4 3000 77 100 1933 a803d57956456d4c\n\
9 12 4 20000 20000 8192 10327 0c441d88493390ef\n\
";

    #[test]
    fn deflate_matches_zlib() {
        let data = test_data();
        let mut lines = String::new();
        for level in 0..=9 {
            for strategy in 0..=Z_FIXED {
                for (window_bits, n, step, out_size) in [
                    (15, 70000, 1000, 8192),
                    (9, 3000, 77, 100),
                    (12, 20000, 20000, 8192),
                ] {
                    let mut strm = ZStream::default();
                    assert_eq!(
                        deflate_init2(&mut strm, level, Z_DEFLATED, window_bits, 8, strategy),
                        Z_OK
                    );
                    let mut out = vec![0u8; out_size];
                    let (mut pos, mut total) = (0, 0);
                    let mut h: u64 = 14695981039346656037;
                    loop {
                        let k = step.min(n - pos);
                        let input = &data[pos..pos + k];
                        pos += k;
                        strm.next_in = 0;
                        strm.avail_in = k as u32;
                        let flush = if pos == n { Z_FINISH } else { Z_NO_FLUSH };
                        loop {
                            strm.next_out = 0;
                            strm.avail_out = out_size as u32;
                            let ret = deflate(&mut strm, input, &mut out, flush);
                            let got = out_size - strm.avail_out as usize;
                            for &b in &out[..got] {
                                h ^= b as u64;
                                h = h.wrapping_mul(1099511628211);
                            }
                            total += got;
                            if ret == Z_STREAM_END || (strm.avail_out != 0 && flush == Z_NO_FLUSH) {
                                break;
                            }
                        }
                        if flush == Z_FINISH {
                            break;
                        }
                    }
                    deflate_end(&mut strm);
                    lines += &format!(
                        "{level} {window_bits} {strategy} {n} {step} {out_size} {total} {h:016x}\n"
                    );
                }
            }
        }
        assert_eq!(lines, DEFLATE_REFERENCE);
    }
}
