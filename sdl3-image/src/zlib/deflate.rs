// Rust translation of deflate.h and deflate.c from zlib 1.3.1.
// Copyright (C) 1995-2024 Jean-loup Gailly and Mark Adler
// This is an altered (translated) version of the original software; zlib is
// used under the zlib license (see zlib.h and LICENSE.txt).

//! deflate.c -- compress data using the deflation algorithm
//!
//! ```text
//!  ALGORITHM
//!
//!      The "deflation" process depends on being able to identify portions
//!      of the input text which are identical to earlier input (within a
//!      sliding window trailing behind the input currently being processed).
//!
//!      The most straightforward technique turns out to be the fastest for
//!      most input files: try all possible matches and select the longest.
//!      The key feature of this algorithm is that insertions into the string
//!      dictionary are very simple and thus fast, and deletions are avoided
//!      completely. Insertions are performed at each input character, whereas
//!      string matches are performed only when the previous match ends. So it
//!      is preferable to spend more time in matches to allow very fast string
//!      insertions and avoid deletions. The matching algorithm for small
//!      strings is inspired from that of Rabin & Karp. A brute force approach
//!      is used to find longer strings when a small match has been found.
//!      A similar algorithm is used in comic (by Jan-Mark Wams) and freeze
//!      (by Leonid Broukhis).
//!         A previous version of this file used a more sophisticated algorithm
//!      (by Fiala and Greene) which is guaranteed to run in linear amortized
//!      time, but has a larger average cost, uses more memory and is patented.
//!      However the F&G algorithm may be faster for some highly redundant
//!      files if the parameter max_chain_length (described below) is too large.
//!
//!  ACKNOWLEDGEMENTS
//!
//!      The idea of lazy evaluation of matches is due to Jan-Mark Wams, and
//!      I found it in 'freeze' written by Leonid Broukhis.
//!      Thanks to many people for bug reports and testing.
//!
//!  REFERENCES
//!
//!      Deutsch, L.P.,"DEFLATE Compressed Data Format Specification".
//!      Available in <http://tools.ietf.org/html/rfc1951>
//!
//!      A description of the Rabin and Karp algorithm is given in the book
//!         "Algorithms" by R. Sedgewick, Addison-Wesley, p252.
//!
//!      Fiala,E.R., and Greene,D.H.
//!         Data Compression with Finite Windows, Comm.ACM, 32,4 (1989) 490-595
//! ```
//!
//! (Built as zlib's CMake builds it: not `FASTEST`, without `LIT_MEM`, and
//! here without the gzip wrapper, which libpng never asks for.)

use super::trees::{
    tr_align, tr_flush_bits, tr_flush_block, tr_init, tr_stored_block, tr_tally_dist, tr_tally_lit,
    CtData, BL_CODES, D_CODES, HEAP_SIZE, L_CODES, MAX_BITS,
};
use super::{
    adler32, err_msg, zalloc, ZStream, MAX_MATCH, MAX_MEM_LEVEL, MIN_MATCH, PRESET_DICT, Z_BLOCK,
    Z_BUF_ERROR, Z_DATA_ERROR, Z_DEFAULT_COMPRESSION, Z_DEFLATED, Z_FILTERED, Z_FINISH, Z_FIXED,
    Z_FULL_FLUSH, Z_HUFFMAN_ONLY, Z_MEM_ERROR, Z_NO_FLUSH, Z_OK, Z_PARTIAL_FLUSH, Z_RLE,
    Z_STREAM_END, Z_STREAM_ERROR, Z_UNKNOWN,
};

/* deflate.h -- internal compression state */

pub(crate) const INIT_STATE: i32 = 42; /* zlib header -> BUSY_STATE */
pub(crate) const BUSY_STATE: i32 = 113; /* deflate -> FINISH_STATE */
pub(crate) const FINISH_STATE: i32 = 666; /* stream complete */
/* Stream status */

/// `tree_desc`: the dynamic tree is named by the descriptor's place in
/// the state (`l_desc`, `d_desc`, `bl_desc`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct TreeDesc {
    pub(crate) max_code: i32, /* largest code with non zero frequency */
}

/// `deflate_state` (`internal_state`).
#[derive(Debug)]
pub(crate) struct DeflateState {
    pub(crate) status: i32,           /* as the name implies */
    pub(crate) pending_buf: Vec<u8>,  /* output still pending */
    pub(crate) pending_buf_size: u64, /* size of pending_buf */
    pub(crate) pending_out: usize,    /* next pending byte to output to the stream */
    pub(crate) pending: u64,          /* nb of bytes in the pending buffer */
    pub(crate) wrap: i32,             /* bit 0 true for zlib, bit 1 true for gzip */
    pub(crate) method: u8,            /* can only be DEFLATED */
    pub(crate) last_flush: i32,       /* value of flush param for previous deflate call */

    /* used by deflate.c: */
    pub(crate) w_size: u32, /* LZ77 window size (32K by default) */
    pub(crate) w_bits: u32, /* log2(w_size)  (8..16) */
    pub(crate) w_mask: u32, /* w_size - 1 */

    pub(crate) window: Vec<u8>,
    /* Sliding window. Input bytes are read into the second half of the window,
     * and move to the first half later to keep a dictionary of at least wSize
     * bytes. With this organization, matches are limited to a distance of
     * wSize-MAX_MATCH bytes, but this ensures that IO is always
     * performed with a length multiple of the block size. Also, it limits
     * the window size to 64K, which is quite useful on MSDOS.
     * To do: use the user input buffer as sliding window.
     */
    pub(crate) window_size: u64,
    /* Actual size of window: 2*wSize, except when the user input buffer
     * is directly used as sliding window.
     */
    pub(crate) prev: Vec<u16>,
    /* Link to older string with same hash index. To limit the size of this
     * array to 64K, this link is maintained only for the last 32K strings.
     * An index in this array is thus a window index modulo 32K.
     */
    pub(crate) head: Vec<u16>, /* Heads of the hash chains or NIL. */

    pub(crate) ins_h: u32,     /* hash index of string to be inserted */
    pub(crate) hash_size: u32, /* number of elements in hash table */
    pub(crate) hash_bits: u32, /* log2(hash_size) */
    pub(crate) hash_mask: u32, /* hash_size-1 */

    pub(crate) hash_shift: u32,
    /* Number of bits by which ins_h must be shifted at each input
     * step. It must be such that after MIN_MATCH steps, the oldest
     * byte no longer takes part in the hash key, that is:
     *   hash_shift * MIN_MATCH >= hash_bits
     */
    pub(crate) block_start: i64,
    /* Window position at the beginning of the current output block. Gets
     * negative when the window is moved backwards.
     */
    pub(crate) match_length: u32,    /* length of best match */
    pub(crate) prev_match: u32,      /* previous match */
    pub(crate) match_available: i32, /* set if previous match exists */
    pub(crate) strstart: u32,        /* start of string to insert */
    pub(crate) match_start: u32,     /* start of matching string */
    pub(crate) lookahead: u32,       /* number of valid bytes ahead in window */

    pub(crate) prev_length: u32,
    /* Length of the best match at previous step. Matches not greater than this
     * are discarded. This is used in the lazy match evaluation.
     */
    pub(crate) max_chain_length: u32,
    /* To speed up deflation, hash chains are never searched beyond this
     * length.  A higher limit improves compression ratio but degrades the
     * speed.
     */
    pub(crate) max_lazy_match: u32,
    /* Attempt to find a better match only when the current match is strictly
     * smaller than this value. This mechanism is used only for compression
     * levels >= 4.
     */
    /* (max_insert_length is max_lazy_match:)
     * Insert new strings in the hash table only if the match length is not
     * greater than this length. This saves time but degrades compression.
     * max_insert_length is used only for compression levels <= 3.
     */
    pub(crate) level: i32,    /* compression level (1..9) */
    pub(crate) strategy: i32, /* favor or force Huffman coding*/

    pub(crate) good_match: u32,
    /* Use a faster search when the previous match is longer than this */
    pub(crate) nice_match: i32, /* Stop searching when current match exceeds this */

    /* used by trees.c: */
    pub(crate) dyn_ltree: Vec<CtData>, /* literal and length tree */
    pub(crate) dyn_dtree: Vec<CtData>, /* distance tree */
    pub(crate) bl_tree: Vec<CtData>,   /* Huffman tree for bit lengths */

    pub(crate) l_desc: TreeDesc,  /* desc. for literal tree */
    pub(crate) d_desc: TreeDesc,  /* desc. for distance tree */
    pub(crate) bl_desc: TreeDesc, /* desc. for bit length tree */

    pub(crate) bl_count: [u16; MAX_BITS + 1],
    /* number of codes at each bit length for an optimal tree */
    pub(crate) heap: [i32; 2 * L_CODES + 1], /* heap used to build the Huffman trees */
    pub(crate) heap_len: i32,                /* number of elements in the heap */
    pub(crate) heap_max: i32,                /* element of largest frequency */
    /* The sons of heap[n] are heap[2*n] and heap[2*n+1]. heap[0] is not used.
     * The same heap array is used to build all trees.
     */
    pub(crate) depth: [u8; 2 * L_CODES + 1],
    /* Depth of each subtree used as tie breaker for trees of equal frequency
     */
    pub(crate) sym_buf: usize, /* buffer for distances and literals/lengths
                               (an offset in pending_buf) */

    pub(crate) lit_bufsize: u32,
    /* Size of match buffer for literals/lengths.  There are 4 reasons for
     * limiting lit_bufsize to 64K:
     *   - frequencies can be kept in 16 bit counters
     *   - if compression is not successful for the first block, all input
     *     data is still in the window so we can still emit a stored block even
     *     when input comes from standard input.  (This can also be done for
     *     all blocks if lit_bufsize is not greater than 32K.)
     *   - if compression is not successful for a file smaller than 64K, we can
     *     even emit a stored file instead of a stored block (saving 5 bytes).
     *     This is applicable only for zip (not gzip or zlib).
     *   - creating new Huffman trees less frequently may not provide fast
     *     adaptation to changes in the input data statistics. (Take for
     *     example a binary file with poorly compressible code followed by
     *     a highly compressible string table.) Smaller buffer sizes give
     *     fast adaptation but have of course the overhead of transmitting
     *     trees more frequently.
     *   - I can't count above 4
     */
    pub(crate) sym_next: u32, /* running index in symbol buffer */
    pub(crate) sym_end: u32,  /* symbol table full when sym_next reaches this */

    pub(crate) opt_len: u64, /* bit length of current block with optimal trees */
    pub(crate) static_len: u64, /* bit length of current block with static trees */
    pub(crate) matches: u32, /* number of string matches in current block */
    pub(crate) insert: u32,  /* bytes at end of window left to insert */

    pub(crate) bi_buf: u16,
    /* Output buffer. bits are inserted starting at the bottom (least
     * significant bits).
     */
    pub(crate) bi_valid: i32,
    /* Number of valid bits in bi_buf.  All bits above the last valid bit
     * are always zero.
     */
    pub(crate) high_water: u64,
    /* High water mark offset in window for initialized bytes -- bytes above
     * this are set to zero in order to avoid memory check warnings when
     * longest match routines access bytes past the input.  This is then
     * updated to the new high water mark.
     */
}

/// Output a byte on the stream. (`put_byte`)
/// IN assertion: there is enough room in pending_buf.
pub(crate) fn put_byte(s: &mut DeflateState, c: u8) {
    s.pending_buf[s.pending as usize] = c;
    s.pending += 1;
}

pub(crate) const MIN_LOOKAHEAD: u32 = MAX_MATCH + MIN_MATCH + 1;
/* Minimum amount of lookahead, except at the end of the input file.
 * See deflate.c for comments about the MIN_MATCH+1.
 */

/// `MAX_DIST(s)`
/// In order to simplify the code, particularly on 16 bit machines, match
/// distances are limited to MAX_DIST instead of WSIZE.
fn max_dist(s: &DeflateState) -> u32 {
    s.w_size - MIN_LOOKAHEAD
}

const WIN_INIT: u64 = MAX_MATCH as u64;
/* Number of bytes after end of data in window to initialize in order to avoid
memory checker errors from longest match routines */

/* deflate.c */

#[allow(dead_code)]
pub(crate) static DEFLATE_COPYRIGHT: &str =
    " deflate 1.3.1 Copyright 1995-2024 Jean-loup Gailly and Mark Adler ";
/*
 If you use the zlib library in a product, an acknowledgment is welcome
 in the documentation of your product. If for some reason you cannot
 include such an acknowledgment, I would appreciate that you keep this
 copyright string in the executable of your product.
*/

/// `block_state`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockState {
    NeedMore,      /* block not completed, need more input or more output */
    BlockDone,     /* block flush performed */
    FinishStarted, /* finish started, need only more output at next deflate */
    FinishDone,    /* finish done, accept no more input or output */
}

/// `compress_func`: the compression function of a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompressFunc {
    Stored,
    Fast,
    Slow,
}

/// The stream a deflate call works on: the stream's fields (C's
/// `s->strm`) and the buffers its positions index.
pub(crate) struct Z<'a> {
    pub(crate) strm: &'a mut ZStream,
    pub(crate) input: &'a [u8],
    pub(crate) output: &'a mut [u8],
}

/* ===========================================================================
 * Local data
 */

const NIL: u32 = 0;
/* Tail of hash chains */

const TOO_FAR: u32 = 4096;
/* Matches of length 3 are discarded if their distance exceeds TOO_FAR */

/// `config`: values for max_lazy_match, good_match and max_chain_length,
/// depending on the desired pack level (0..9). The values given below have
/// been tuned to exclude worst case performance for pathological files.
/// Better values may be found for specific files.
struct Config {
    good_length: u16, /* reduce lazy search above this match length */
    max_lazy: u16,    /* do not perform lazy search above this match length */
    nice_length: u16, /* quit search above this match length */
    max_chain: u16,
    func: CompressFunc,
}

const fn config(
    good_length: u16,
    max_lazy: u16,
    nice_length: u16,
    max_chain: u16,
    func: CompressFunc,
) -> Config {
    Config {
        good_length,
        max_lazy,
        nice_length,
        max_chain,
        func,
    }
}

static CONFIGURATION_TABLE: [Config; 10] = [
    /*      good lazy nice chain */
    /* 0 */
    config(0, 0, 0, 0, CompressFunc::Stored), /* store only */
    /* 1 */ config(4, 4, 8, 4, CompressFunc::Fast), /* max speed, no lazy matches */
    /* 2 */ config(4, 5, 16, 8, CompressFunc::Fast),
    /* 3 */ config(4, 6, 32, 32, CompressFunc::Fast),
    /* 4 */ config(4, 4, 16, 16, CompressFunc::Slow), /* lazy matches */
    /* 5 */ config(8, 16, 32, 32, CompressFunc::Slow),
    /* 6 */ config(8, 16, 128, 128, CompressFunc::Slow),
    /* 7 */ config(8, 32, 128, 256, CompressFunc::Slow),
    /* 8 */ config(32, 128, 258, 1024, CompressFunc::Slow),
    /* 9 */ config(32, 258, 258, 4096, CompressFunc::Slow), /* max compression */
];

/* Note: the deflate() code requires max_lazy >= MIN_MATCH and max_chain >= 4
 * For deflate_fast() (levels <= 3) good is ignored and lazy has a different
 * meaning.
 */

/// rank Z_BLOCK between Z_NO_FLUSH and Z_PARTIAL_FLUSH (`RANK`)
fn rank(f: i32) -> i32 {
    (f * 2) - if f > 4 { 9 } else { 0 }
}

/// `UPDATE_HASH`: update a hash value with the given input byte
/// IN  assertion: all calls to UPDATE_HASH are made with consecutive input
///    characters, so that a running hash key can be computed from the previous
///    key instead of complete recalculation each time.
fn update_hash(s: &DeflateState, h: u32, c: u8) -> u32 {
    ((h << s.hash_shift) ^ c as u32) & s.hash_mask
}

/// `INSERT_STRING`: insert string str in the dictionary and return the
/// previous head of the hash chain (the most recent string with same hash
/// key).
/// IN  assertion: all calls to INSERT_STRING are made with consecutive input
///    characters and the first MIN_MATCH bytes of str are valid (except for
///    the last MIN_MATCH-1 bytes of the input file).
fn insert_string(s: &mut DeflateState, str: u32) -> u32 {
    s.ins_h = update_hash(s, s.ins_h, s.window[(str + (MIN_MATCH - 1)) as usize]);
    let match_head = s.head[s.ins_h as usize];
    s.prev[(str & s.w_mask) as usize] = match_head;
    s.head[s.ins_h as usize] = str as u16;
    match_head as u32
}

/// `CLEAR_HASH`: initialize the hash table (avoiding 64K overflow for 16
/// bit systems). prev[] will be initialized on the fly.
fn clear_hash(s: &mut DeflateState) {
    let n = s.hash_size as usize;
    s.head[n - 1] = NIL as u16;
    s.head[..n - 1].fill(0);
}

/* ===========================================================================
 * Slide the hash table when sliding the window down (could be avoided with 32
 * bit values at the expense of memory usage). We slide even when level == 0 to
 * keep the hash table consistent if we switch back to level > 0 later.
 */
fn slide_hash(s: &mut DeflateState) {
    let wsize = s.w_size;

    for p in s.head.iter_mut() {
        let m = *p as u32;
        *p = if m >= wsize {
            (m - wsize) as u16
        } else {
            NIL as u16
        };
    }
    for p in s.prev[..wsize as usize].iter_mut() {
        let m = *p as u32;
        *p = if m >= wsize {
            (m - wsize) as u16
        } else {
            NIL as u16
        };
        /* If n is not on any hash chain, prev[n] is garbage but
         * its value will never be used.
         */
    }
}

/* ===========================================================================
 * Read a new buffer from the current input stream, update the adler32
 * and total number of bytes read.  All deflate() input goes through
 * this function so some applications may wish to modify it to avoid
 * allocating a large strm->next_in buffer and copying from it.
 * (See also flush_pending()).
 * (`buf` is a position in `dst`: the window or the output buffer.)
 */
fn read_buf(
    strm: &mut ZStream,
    input: &[u8],
    wrap: i32,
    dst: &mut [u8],
    buf: usize,
    size: u32,
) -> u32 {
    let mut len = strm.avail_in;

    if len > size {
        len = size;
    }
    if len == 0 {
        return 0;
    }

    strm.avail_in -= len;

    let src = &input[strm.next_in..strm.next_in + len as usize];
    dst[buf..buf + len as usize].copy_from_slice(src);
    if wrap == 1 {
        strm.adler = adler32(strm.adler, Some(&dst[buf..buf + len as usize]));
    }
    strm.next_in += len as usize;
    strm.total_in += len as u64;

    len
}

/* ===========================================================================
 * Fill the window when the lookahead becomes insufficient.
 * Updates strstart and lookahead.
 *
 * IN assertion: lookahead < MIN_LOOKAHEAD
 * OUT assertions: strstart <= window_size-MIN_LOOKAHEAD
 *    At least one byte has been read, or avail_in == 0; reads are
 *    performed for at least two bytes (required for the zip translate_eol
 *    option -- not supported here).
 */
fn fill_window(s: &mut DeflateState, z: &mut Z<'_>) {
    let mut n: u32;
    let mut more: u32; /* Amount of free space at the end of the window. */
    let wsize = s.w_size;

    loop {
        more = (s.window_size - s.lookahead as u64 - s.strstart as u64) as u32;

        /* (the 64K limit of 16-bit machines doesn't apply) */

        /* If the window is almost full and there is insufficient lookahead,
         * move the upper half to the lower one to make room in the upper half.
         */
        if s.strstart >= wsize + max_dist(s) {
            s.window
                .copy_within(wsize as usize..(2 * wsize - more) as usize, 0);
            s.match_start = s.match_start.wrapping_sub(wsize);
            s.strstart -= wsize; /* we now have strstart >= MAX_DIST */
            s.block_start -= wsize as i64;
            if s.insert > s.strstart {
                s.insert = s.strstart;
            }
            slide_hash(s);
            more += wsize;
        }
        if z.strm.avail_in == 0 {
            break;
        }

        /* If there was no sliding:
         *    strstart <= WSIZE+MAX_DIST-1 && lookahead <= MIN_LOOKAHEAD - 1 &&
         *    more == window_size - lookahead - strstart
         * => more >= window_size - (MIN_LOOKAHEAD-1 + WSIZE + MAX_DIST-1)
         * => more >= window_size - 2*WSIZE + 2
         * In the BIG_MEM or MMAP case (not yet supported),
         *   window_size == input_size + MIN_LOOKAHEAD  &&
         *   strstart + s->lookahead <= input_size => more >= MIN_LOOKAHEAD.
         * Otherwise, window_size == 2*WSIZE so more >= 2.
         * If there was sliding, more >= WSIZE. So in all cases, more >= 2.
         */
        let at = (s.strstart + s.lookahead) as usize;
        n = read_buf(z.strm, z.input, s.wrap, &mut s.window, at, more);
        s.lookahead += n;

        /* Initialize the hash value now that we have some input: */
        if s.lookahead + s.insert >= MIN_MATCH {
            let mut str = s.strstart - s.insert;
            s.ins_h = s.window[str as usize] as u32;
            s.ins_h = update_hash(s, s.ins_h, s.window[(str + 1) as usize]);
            while s.insert != 0 {
                s.ins_h = update_hash(s, s.ins_h, s.window[(str + MIN_MATCH - 1) as usize]);
                s.prev[(str & s.w_mask) as usize] = s.head[s.ins_h as usize];
                s.head[s.ins_h as usize] = str as u16;
                str += 1;
                s.insert -= 1;
                if s.lookahead + s.insert < MIN_MATCH {
                    break;
                }
            }
        }
        /* If the whole input has less than MIN_MATCH bytes, ins_h is garbage,
         * but this is not important since only literal bytes will be emitted.
         */

        if !(s.lookahead < MIN_LOOKAHEAD && z.strm.avail_in != 0) {
            break;
        }
    }

    /* If the WIN_INIT bytes after the end of the current data have never been
     * written, then zero those bytes in order to avoid memory check reports of
     * the use of uninitialized (or uninitialised as Julian writes) bytes by
     * the longest match routines.  Update the high water mark for the next
     * time through here.  WIN_INIT is set to MAX_MATCH since the longest match
     * routines allow scanning to strstart + MAX_MATCH, ignoring lookahead.
     */
    if s.high_water < s.window_size {
        let curr = s.strstart as u64 + s.lookahead as u64;
        let mut init: u64;

        if s.high_water < curr {
            /* Previous high water mark below current data -- zero WIN_INIT
             * bytes or up to end of window, whichever is less.
             */
            init = s.window_size - curr;
            if init > WIN_INIT {
                init = WIN_INIT;
            }
            s.window[curr as usize..(curr + init) as usize].fill(0);
            s.high_water = curr + init;
        } else if s.high_water < curr + WIN_INIT {
            /* High water mark at or above current data, but below current data
             * plus WIN_INIT -- zero out to current data plus WIN_INIT, or up
             * to end of window, whichever is less.
             */
            init = curr + WIN_INIT - s.high_water;
            if init > s.window_size - s.high_water {
                init = s.window_size - s.high_water;
            }
            s.window[s.high_water as usize..(s.high_water + init) as usize].fill(0);
            s.high_water += init;
        }
    }
}

/// `deflateInit2_` (via the `deflateInit2` macro; the version and
/// structure size checks have nothing to check here)
pub(crate) fn deflate_init2(
    strm: &mut ZStream,
    mut level: i32,
    method: i32,
    mut window_bits: i32,
    mem_level: i32,
    strategy: i32,
) -> i32 {
    let mut wrap = 1;

    strm.msg = None;
    /* (zalloc and zfree: Rust allocations) */

    if level == Z_DEFAULT_COMPRESSION {
        level = 6;
    }

    if window_bits < 0 {
        /* suppress zlib wrapper */
        wrap = 0;
        if window_bits < -15 {
            return Z_STREAM_ERROR;
        }
        window_bits = -window_bits;
    } else if window_bits > 15 {
        /* (the gzip wrapper: not translated) */
        return Z_STREAM_ERROR;
    }
    if !(1..=MAX_MEM_LEVEL).contains(&mem_level)
        || method != Z_DEFLATED
        || !(8..=15).contains(&window_bits)
        || !(0..=9).contains(&level)
        || !(0..=Z_FIXED).contains(&strategy)
        || (window_bits == 8 && wrap != 1)
    {
        return Z_STREAM_ERROR;
    }
    if window_bits == 8 {
        window_bits = 9; /* until 256-byte window bug fixed */
    }

    let w_bits = window_bits as u32;
    let w_size = 1u32 << w_bits;
    let hash_bits = mem_level as u32 + 7;
    let hash_size = 1u32 << hash_bits;
    let lit_bufsize = 1u32 << (mem_level + 6); /* 16K elements by default */

    let window = zalloc::<u8>(w_size as usize * 2);
    let prev = zalloc::<u16>(w_size as usize);
    let head = zalloc::<u16>(hash_size as usize);

    /* We overlay pending_buf and sym_buf. This works since the average size
     * for length/distance pairs over any compressed block is assured to be 31
     * bits or less.
     *
     * Analysis: The longest fixed codes are a length code of 8 bits plus 5
     * extra bits, for lengths 131 to 257. The longest fixed distance codes are
     * 5 bits plus 13 extra bits, for distances 16385 to 32768. The longest
     * possible fixed-codes length/distance pair is then 31 bits total.
     *
     * sym_buf starts one-fourth of the way into pending_buf. So there are
     * three bytes in sym_buf for every four bytes in pending_buf. Each symbol
     * in sym_buf is three bytes -- two for the distance and one for the
     * literal/length. As each symbol is consumed, the pointer to the next
     * sym_buf value to read moves forward three bytes. From that symbol, up to
     * 31 bits are written to pending_buf. The closest the written pending_buf
     * bits gets to the next sym_buf symbol to read is just before the last
     * code is written. At that time, 31*(n - 2) bits have been written, just
     * after 24*(n - 2) bits have been consumed from sym_buf. sym_buf starts at
     * 8*n bits into pending_buf. (Note that the symbol buffer fills when n - 1
     * symbols are written.) The closest the writing gets to what is unread is
     * then n + 14 bits. Here n is lit_bufsize, which is 16384 by default, and
     * can range from 128 to 32768.
     *
     * Therefore, at a minimum, there are 142 bits of space between what is
     * written and what is read in the overlain buffers, so the symbols cannot
     * be overwritten by the compressed data. That space is actually 139 bits,
     * due to the three-bit fixed-code block header.
     *
     * That covers the case where either Z_FIXED is specified, forcing fixed
     * codes, or when the use of fixed codes is chosen, because that choice
     * results in a smaller compressed block than dynamic codes. That latter
     * condition then assures that the above analysis also covers all dynamic
     * blocks. A dynamic-code block will only be chosen to be emitted if it has
     * fewer bits than a fixed-code block would for the same set of symbols.
     * Therefore its average symbol length is assured to be less than 31. So
     * the compressed data for a dynamic block also cannot overwrite the
     * symbols from which it is being constructed.
     */
    let pending_buf = zalloc::<u8>(lit_bufsize as usize * 4);
    let dyn_ltree = zalloc::<CtData>(HEAP_SIZE);
    let dyn_dtree = zalloc::<CtData>(2 * D_CODES + 1);
    let bl_tree = zalloc::<CtData>(2 * BL_CODES + 1);

    let (
        Some(window),
        Some(prev),
        Some(head),
        Some(pending_buf),
        Some(dyn_ltree),
        Some(dyn_dtree),
        Some(bl_tree),
    ) = (
        window,
        prev,
        head,
        pending_buf,
        dyn_ltree,
        dyn_dtree,
        bl_tree,
    )
    else {
        /* (s->status = FINISH_STATE; deflateEnd (strm): nothing is kept) */
        strm.msg = Some(err_msg(Z_MEM_ERROR));
        return Z_MEM_ERROR;
    };

    let s = DeflateState {
        status: INIT_STATE, /* to pass state test in deflateReset() */
        pending_buf,
        pending_buf_size: lit_bufsize as u64 * 4,
        pending_out: 0,
        pending: 0,
        wrap,
        method: method as u8,
        last_flush: 0,
        w_size,
        w_bits,
        w_mask: w_size - 1,
        window,
        window_size: 0,
        prev,
        head,
        ins_h: 0,
        hash_size,
        hash_bits,
        hash_mask: hash_size - 1,
        hash_shift: (hash_bits + MIN_MATCH - 1) / MIN_MATCH,
        block_start: 0,
        match_length: 0,
        prev_match: 0,
        match_available: 0,
        strstart: 0,
        match_start: 0,
        lookahead: 0,
        prev_length: 0,
        max_chain_length: 0,
        max_lazy_match: 0,
        level,
        strategy,
        good_match: 0,
        nice_match: 0,
        dyn_ltree,
        dyn_dtree,
        bl_tree,
        l_desc: TreeDesc { max_code: 0 },
        d_desc: TreeDesc { max_code: 0 },
        bl_desc: TreeDesc { max_code: 0 },
        bl_count: [0; MAX_BITS + 1],
        heap: [0; 2 * L_CODES + 1],
        heap_len: 0,
        heap_max: 0,
        depth: [0; 2 * L_CODES + 1],
        sym_buf: lit_bufsize as usize,
        lit_bufsize,
        sym_next: 0,
        sym_end: (lit_bufsize - 1) * 3,
        /* We avoid equality with lit_bufsize*3 because of wraparound at 64K
         * on 16 bit machines and because stored blocks are restricted to
         * 64K-1 bytes.
         */
        opt_len: 0,
        static_len: 0,
        matches: 0,
        insert: 0,
        bi_buf: 0,
        bi_valid: 0,
        high_water: 0, /* nothing written to s->window yet */
    };
    let mut boxed = Vec::new();
    if boxed.try_reserve_exact(1).is_err() {
        return Z_MEM_ERROR;
    }
    boxed.push(s);
    strm.dstate = boxed.pop().map(Box::new);

    deflate_reset(strm)
}

/* =========================================================================
 * Check for a valid deflate stream state. Return 0 if ok, 1 if not.
 * (`deflateStateCheck`: the states this translation makes)
 */
fn deflate_state_check(strm: &ZStream) -> bool {
    match &strm.dstate {
        None => true,
        Some(s) => s.status != INIT_STATE && s.status != BUSY_STATE && s.status != FINISH_STATE,
    }
}

/// `deflateResetKeep`
fn deflate_reset_keep(strm: &mut ZStream) -> i32 {
    if deflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }

    strm.total_in = 0;
    strm.total_out = 0;
    strm.msg = None; /* use zfree if we ever allocate msg dynamically */
    strm.data_type = Z_UNKNOWN;

    let s = strm.dstate.as_mut().unwrap();
    s.pending = 0;
    s.pending_out = 0;

    if s.wrap < 0 {
        s.wrap = -s.wrap; /* was made negative by deflate(..., Z_FINISH); */
    }
    s.status = INIT_STATE;
    strm.adler = adler32(0, None);
    s.last_flush = -2;

    tr_init(s);

    Z_OK
}

/* ===========================================================================
 * Initialize the "longest match" routines for a new zlib stream
 */
fn lm_init(s: &mut DeflateState) {
    s.window_size = 2u64 * s.w_size as u64;

    clear_hash(s);

    /* Set the default configuration parameters:
     */
    let c = &CONFIGURATION_TABLE[s.level as usize];
    s.max_lazy_match = c.max_lazy as u32;
    s.good_match = c.good_length as u32;
    s.nice_match = c.nice_length as i32;
    s.max_chain_length = c.max_chain as u32;

    s.strstart = 0;
    s.block_start = 0;
    s.lookahead = 0;
    s.insert = 0;
    s.match_length = MIN_MATCH - 1;
    s.prev_length = MIN_MATCH - 1;
    s.match_available = 0;
    s.ins_h = 0;
}

/// `deflateReset`
pub(crate) fn deflate_reset(strm: &mut ZStream) -> i32 {
    let ret = deflate_reset_keep(strm);
    if ret == Z_OK {
        lm_init(strm.dstate.as_mut().unwrap());
    }
    ret
}

/* =========================================================================
 * Put a short in the pending buffer. The 16-bit value is put in MSB order.
 * IN assertion: the stream state is correct and there is enough room in
 * pending_buf.
 */
fn put_short_msb(s: &mut DeflateState, b: u32) {
    put_byte(s, (b >> 8) as u8);
    put_byte(s, (b & 0xff) as u8);
}

/* =========================================================================
 * Flush as much pending output as possible. All deflate() output, except for
 * some deflate_stored() output, goes through this function so some
 * applications may wish to modify it to avoid allocating a large
 * strm->next_out buffer and copying into it. (See also read_buf()).
 */
fn flush_pending(s: &mut DeflateState, z: &mut Z<'_>) {
    tr_flush_bits(s);
    let mut len = s.pending as u32;
    if len > z.strm.avail_out {
        len = z.strm.avail_out;
    }
    if len == 0 {
        return;
    }

    let out = z.strm.next_out;
    z.output[out..out + len as usize]
        .copy_from_slice(&s.pending_buf[s.pending_out..s.pending_out + len as usize]);
    z.strm.next_out += len as usize;
    s.pending_out += len as usize;
    z.strm.total_out += len as u64;
    z.strm.avail_out -= len;
    s.pending -= len as u64;
    if s.pending == 0 {
        s.pending_out = 0;
    }
}

/// `ERR_RETURN`
fn err_return(strm: &mut ZStream, err: i32) -> i32 {
    strm.msg = Some(err_msg(err));
    err
}

/// `deflate`: compress `input[strm.next_in..]` (`strm.avail_in` bytes)
/// into `output[strm.next_out..]` (`strm.avail_out` bytes).
pub(crate) fn deflate(strm: &mut ZStream, input: &[u8], output: &mut [u8], flush: i32) -> i32 {
    if deflate_state_check(strm) || !(0..=Z_BLOCK).contains(&flush) {
        return Z_STREAM_ERROR;
    }
    let mut s = strm.dstate.take().unwrap();
    let mut z = Z {
        strm,
        input,
        output,
    };
    let ret = deflate_body(&mut s, &mut z, flush);
    z.strm.dstate = Some(s);
    ret
}

fn deflate_body(s: &mut DeflateState, z: &mut Z<'_>, flush: i32) -> i32 {
    /* (next_out and next_in can't be Z_NULL) */
    if s.status == FINISH_STATE && flush != Z_FINISH {
        return err_return(z.strm, Z_STREAM_ERROR);
    }
    if z.strm.avail_out == 0 {
        return err_return(z.strm, Z_BUF_ERROR);
    }

    let old_flush = s.last_flush; /* value of flush param for previous deflate call */
    s.last_flush = flush;

    /* Flush as much pending output as possible */
    if s.pending != 0 {
        flush_pending(s, z);
        if z.strm.avail_out == 0 {
            /* Since avail_out is 0, deflate will be called again with
             * more output space, but possibly with both pending and
             * avail_in equal to zero. There won't be anything to do,
             * but this is not an error situation so make sure we
             * return OK instead of BUF_ERROR at next call of deflate:
             */
            s.last_flush = -1;
            return Z_OK;
        }

        /* Make sure there is something to do and avoid duplicate consecutive
         * flushes. For repeated and useless calls with Z_FINISH, we keep
         * returning Z_STREAM_END instead of Z_BUF_ERROR.
         */
    } else if z.strm.avail_in == 0 && rank(flush) <= rank(old_flush) && flush != Z_FINISH {
        return err_return(z.strm, Z_BUF_ERROR);
    }

    /* User must not provide more input after the first FINISH: */
    if s.status == FINISH_STATE && z.strm.avail_in != 0 {
        return err_return(z.strm, Z_BUF_ERROR);
    }

    /* Write the header */
    if s.status == INIT_STATE && s.wrap == 0 {
        s.status = BUSY_STATE;
    }
    if s.status == INIT_STATE {
        /* zlib header */
        let mut header: u32 = (Z_DEFLATED as u32 + ((s.w_bits - 8) << 4)) << 8;
        let level_flags: u32 = if s.strategy >= Z_HUFFMAN_ONLY || s.level < 2 {
            0
        } else if s.level < 6 {
            1
        } else if s.level == 6 {
            2
        } else {
            3
        };
        header |= level_flags << 6;
        if s.strstart != 0 {
            header |= PRESET_DICT;
        }
        header += 31 - (header % 31);

        put_short_msb(s, header);

        /* Save the adler32 of the preset dictionary: */
        if s.strstart != 0 {
            put_short_msb(s, (z.strm.adler >> 16) as u32);
            put_short_msb(s, (z.strm.adler & 0xffff) as u32);
        }
        z.strm.adler = adler32(0, None);
        s.status = BUSY_STATE;

        /* Compression must start with an empty pending buffer */
        flush_pending(s, z);
        if s.pending != 0 {
            s.last_flush = -1;
            return Z_OK;
        }
    }
    /* (GZIP_STATE to HCRC_STATE: the gzip header, not translated) */

    /* Start a new block or continue the current one.
     */
    if z.strm.avail_in != 0 || s.lookahead != 0 || (flush != Z_NO_FLUSH && s.status != FINISH_STATE)
    {
        let bstate = if s.level == 0 {
            deflate_stored(s, z, flush)
        } else if s.strategy == Z_HUFFMAN_ONLY {
            deflate_huff(s, z, flush)
        } else if s.strategy == Z_RLE {
            deflate_rle(s, z, flush)
        } else {
            match CONFIGURATION_TABLE[s.level as usize].func {
                CompressFunc::Stored => deflate_stored(s, z, flush),
                CompressFunc::Fast => deflate_fast(s, z, flush),
                CompressFunc::Slow => deflate_slow(s, z, flush),
            }
        };

        if bstate == BlockState::FinishStarted || bstate == BlockState::FinishDone {
            s.status = FINISH_STATE;
        }
        if bstate == BlockState::NeedMore || bstate == BlockState::FinishStarted {
            if z.strm.avail_out == 0 {
                s.last_flush = -1; /* avoid BUF_ERROR next call, see above */
            }
            return Z_OK;
            /* If flush != Z_NO_FLUSH && avail_out == 0, the next call
             * of deflate should use the same flush parameter to make sure
             * that the flush is complete. So we don't have to output an
             * empty block here, this will be done at next call. This also
             * ensures that for a very small output buffer, we emit at most
             * one empty block.
             */
        }
        if bstate == BlockState::BlockDone {
            if flush == Z_PARTIAL_FLUSH {
                tr_align(s);
            } else if flush != Z_BLOCK {
                /* FULL_FLUSH or SYNC_FLUSH */
                tr_stored_block(s, None, 0, 0);
                /* For a full flush, this empty block will be recognized
                 * as a special marker by inflate_sync().
                 */
                if flush == Z_FULL_FLUSH {
                    clear_hash(s); /* forget history */
                    if s.lookahead == 0 {
                        s.strstart = 0;
                        s.block_start = 0;
                        s.insert = 0;
                    }
                }
            }
            flush_pending(s, z);
            if z.strm.avail_out == 0 {
                s.last_flush = -1; /* avoid BUF_ERROR at next call, see above */
                return Z_OK;
            }
        }
    }

    if flush != Z_FINISH {
        return Z_OK;
    }
    if s.wrap <= 0 {
        return Z_STREAM_END;
    }

    /* Write the trailer */
    put_short_msb(s, (z.strm.adler >> 16) as u32);
    put_short_msb(s, (z.strm.adler & 0xffff) as u32);
    flush_pending(s, z);
    /* If avail_out is zero, the application will call deflate again
     * to flush the rest.
     */
    if s.wrap > 0 {
        s.wrap = -s.wrap; /* write the trailer only once! */
    }
    if s.pending != 0 {
        Z_OK
    } else {
        Z_STREAM_END
    }
}

/// `deflateEnd`
pub(crate) fn deflate_end(strm: &mut ZStream) -> i32 {
    if deflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }

    let status = strm.dstate.as_ref().unwrap().status;

    /* Deallocate in reverse order of allocations: */
    strm.dstate = None;

    if status == BUSY_STATE {
        Z_DATA_ERROR
    } else {
        Z_OK
    }
}

/* ===========================================================================
 * Set match_start to the longest match starting at the given string and
 * return its length. Matches shorter or equal to prev_length are discarded,
 * in which case the result is equal to prev_length and match_start is
 * garbage.
 * IN assertions: cur_match is the head of the hash chain for the current
 *   string (strstart) and its distance is <= MAX_DIST, and prev_length >= 1
 * OUT assertion: the match length is not greater than s->lookahead.
 */
fn longest_match(s: &mut DeflateState, mut cur_match: u32) -> u32 {
    let mut chain_length = s.max_chain_length; /* max hash chain length */
    let scan = s.strstart as usize; /* current string */
    let mut len: i32; /* length of current match */
    let mut best_len = s.prev_length as i32; /* best match length so far */
    let mut nice_match = s.nice_match; /* stop if match long enough */
    let limit = if s.strstart > max_dist(s) {
        s.strstart - max_dist(s)
    } else {
        NIL
    };
    /* Stop when cur_match becomes <= limit. To simplify the code,
     * we prevent matches with the string of window index 0.
     */
    let wmask = s.w_mask;
    let window = &s.window;

    let strend = s.strstart as usize + MAX_MATCH as usize;
    let mut scan_end1 = window[scan + best_len as usize - 1];
    let mut scan_end = window[scan + best_len as usize];

    /* The code is optimized for HASH_BITS >= 8 and MAX_MATCH-2 multiple of 16.
     * It is easy to get rid of this optimization if necessary.
     */

    /* Do not waste too much time if we already have a good match: */
    if s.prev_length >= s.good_match {
        chain_length >>= 2;
    }
    /* Do not look for matches beyond the end of the input. This is necessary
     * to make deflate deterministic.
     */
    if nice_match as u32 > s.lookahead {
        nice_match = s.lookahead as i32;
    }

    'chain: loop {
        let mut m = cur_match as usize; /* matched string */

        /* Skip to next match if the match length cannot increase
         * or if the match length is less than 2.  Note that the checks below
         * for insufficient lookahead only occur occasionally for performance
         * reasons.  Therefore uninitialized memory will be accessed, and
         * conditional jumps will be made that depend on those values.
         * However the length of the match is limited to the lookahead, so
         * the output of deflate is not affected by the uninitialized values.
         */
        'next: {
            if window[m + best_len as usize] != scan_end
                || window[m + best_len as usize - 1] != scan_end1
                || window[m] != window[scan]
            {
                break 'next;
            }
            m += 1;
            if window[m] != window[scan + 1] {
                break 'next;
            }

            /* The check at best_len - 1 can be removed because it will be made
             * again later. (This heuristic is not always a win.)
             * It is not necessary to compare scan[2] and match[2] since they
             * are always equal when the other bytes match, given that
             * the hash keys are equal and that HASH_BITS >= 8.
             */
            let mut sc = scan + 2;
            m += 1;

            /* We check for insufficient lookahead only every 8th comparison;
             * the 256th check will be made at strstart + 258.
             */
            'scan: loop {
                for _ in 0..8 {
                    sc += 1;
                    m += 1;
                    if window[sc] != window[m] {
                        break 'scan;
                    }
                }
                if sc >= strend {
                    break;
                }
            }

            len = MAX_MATCH as i32 - (strend - sc) as i32;
            /* (scan = strend - MAX_MATCH: scan is unchanged here) */

            if len > best_len {
                s.match_start = cur_match;
                best_len = len;
                if len >= nice_match {
                    break 'chain;
                }
                scan_end1 = window[scan + best_len as usize - 1];
                scan_end = window[scan + best_len as usize];
            }
        }
        cur_match = s.prev[(cur_match & wmask) as usize] as u32;
        if cur_match <= limit {
            break;
        }
        chain_length -= 1;
        if chain_length == 0 {
            break;
        }
    }

    if best_len as u32 <= s.lookahead {
        return best_len as u32;
    }
    s.lookahead
}

/// `FLUSH_BLOCK_ONLY`: flush the current block, with given end-of-file
/// flag.
/// IN assertion: strstart is set to the end of the current match.
fn flush_block_only(s: &mut DeflateState, z: &mut Z<'_>, last: i32) {
    let buf = if s.block_start >= 0 {
        Some(s.block_start as usize)
    } else {
        None
    };
    let stored_len = (s.strstart as i64 - s.block_start) as u64;
    tr_flush_block(s, &mut z.strm.data_type, buf, stored_len, last);
    s.block_start = s.strstart as i64;
    flush_pending(s, z);
}

/// `FLUSH_BLOCK`: the same but force premature exit if necessary (`Some`
/// is the state to return).
fn flush_block(s: &mut DeflateState, z: &mut Z<'_>, last: i32) -> Option<BlockState> {
    flush_block_only(s, z, last);
    if z.strm.avail_out == 0 {
        return Some(if last != 0 {
            BlockState::FinishStarted
        } else {
            BlockState::NeedMore
        });
    }
    None
}

/* Maximum stored block length in deflate format (not including header). */
const MAX_STORED: u32 = 65535;

/* ===========================================================================
 * Copy without compression as much as possible from the input stream, return
 * the current block state.
 *
 * In case deflateParams() is used to later switch to a non-zero compression
 * level, s->matches (otherwise unused when storing) keeps track of the number
 * of hash table slides to perform. If s->matches is 1, then one hash table
 * slide will be done when switching. If s->matches is 2, the maximum value
 * allowed here, then the hash table will be cleared, since two or more slides
 * is the same as a clear.
 *
 * deflate_stored() is written to minimize the number of times an input byte is
 * copied. It is most efficient with large input and output buffers, which
 * maximizes the opportunities to have a single copy from next_in to next_out.
 */
fn deflate_stored(s: &mut DeflateState, z: &mut Z<'_>, flush: i32) -> BlockState {
    /* Smallest worthy block size when not flushing or finishing. By default
     * this is 32K. This can be as small as 507 bytes for memLevel == 1. For
     * large input and output buffers, the stored block size will be larger.
     */
    let mut min_block = ((s.pending_buf_size - 5) as u32).min(s.w_size);

    /* Copy as many min_block or larger stored blocks directly to next_out as
     * possible. If flushing, copy the remaining available input to next_out as
     * stored blocks, if there is enough space.
     */
    let mut len: u32;
    let mut left: u32;
    let mut have: u32;
    let mut last: u32 = 0;
    let mut used = z.strm.avail_in;
    loop {
        /* Set len to the maximum size block that we can copy directly with the
         * available input data and output space. Set left to how much of that
         * would be copied from what's left in the window.
         */
        len = MAX_STORED; /* maximum deflate stored block length */
        have = ((s.bi_valid + 42) >> 3) as u32; /* number of header bytes */
        if z.strm.avail_out < have {
            /* need room for header */
            break;
        }
        /* maximum stored block length that will fit in avail_out: */
        have = z.strm.avail_out - have;
        left = (s.strstart as i64 - s.block_start) as u32; /* bytes left in window */
        if len as u64 > left as u64 + z.strm.avail_in as u64 {
            len = left + z.strm.avail_in; /* limit len to the input */
        }
        if len > have {
            len = have; /* limit len to the output */
        }

        /* If the stored block would be less than min_block in length, or if
         * unable to copy all of the available input when flushing, then try
         * copying to the window and the pending buffer instead. Also don't
         * write an empty block when flushing -- deflate() does that.
         */
        if len < min_block
            && ((len == 0 && flush != Z_FINISH)
                || flush == Z_NO_FLUSH
                || len != left + z.strm.avail_in)
        {
            break;
        }

        /* Make a dummy stored block in pending to get the header bytes,
         * including any pending bits. This also updates the debugging counts.
         */
        last = if flush == Z_FINISH && len == left + z.strm.avail_in {
            1
        } else {
            0
        };
        tr_stored_block(s, None, 0, last as i32);

        /* Replace the lengths in the dummy stored block with len. */
        let p = s.pending as usize;
        s.pending_buf[p - 4] = len as u8;
        s.pending_buf[p - 3] = (len >> 8) as u8;
        s.pending_buf[p - 2] = !len as u8;
        s.pending_buf[p - 1] = (!len >> 8) as u8;

        /* Write the stored block header bytes. */
        flush_pending(s, z);

        /* Copy uncompressed bytes from the window to next_out. */
        if left != 0 {
            if left > len {
                left = len;
            }
            let out = z.strm.next_out;
            let bs = s.block_start as usize;
            z.output[out..out + left as usize].copy_from_slice(&s.window[bs..bs + left as usize]);
            z.strm.next_out += left as usize;
            z.strm.avail_out -= left;
            z.strm.total_out += left as u64;
            s.block_start += left as i64;
            len -= left;
        }

        /* Copy uncompressed bytes directly from next_in to next_out, updating
         * the check value.
         */
        if len != 0 {
            let out = z.strm.next_out;
            read_buf(z.strm, z.input, s.wrap, z.output, out, len);
            z.strm.next_out += len as usize;
            z.strm.avail_out -= len;
            z.strm.total_out += len as u64;
        }
        if last != 0 {
            break;
        }
    }

    /* Update the sliding window with the last s->w_size bytes of the copied
     * data, or append all of the copied data to the existing window if less
     * than s->w_size bytes were copied. Also update the number of bytes to
     * insert in the hash tables, in the event that deflateParams() switches to
     * a non-zero compression level.
     */
    used -= z.strm.avail_in; /* number of input bytes directly copied */
    if used != 0 {
        /* If any input was used, then no unused input remains in the window,
         * therefore s->block_start == s->strstart.
         */
        if used >= s.w_size {
            /* supplant the previous history */
            s.matches = 2; /* clear hash */
            let from = z.strm.next_in - s.w_size as usize;
            s.window[..s.w_size as usize].copy_from_slice(&z.input[from..z.strm.next_in]);
            s.strstart = s.w_size;
            s.insert = s.strstart;
        } else {
            if s.window_size - s.strstart as u64 <= used as u64 {
                /* Slide the window down. */
                s.strstart -= s.w_size;
                let (w, st) = (s.w_size as usize, s.strstart as usize);
                s.window.copy_within(w..w + st, 0);
                if s.matches < 2 {
                    s.matches += 1; /* add a pending slide_hash() */
                }
                if s.insert > s.strstart {
                    s.insert = s.strstart;
                }
            }
            let st = s.strstart as usize;
            let from = z.strm.next_in - used as usize;
            s.window[st..st + used as usize].copy_from_slice(&z.input[from..z.strm.next_in]);
            s.strstart += used;
            s.insert += used.min(s.w_size - s.insert);
        }
        s.block_start = s.strstart as i64;
    }
    if s.high_water < s.strstart as u64 {
        s.high_water = s.strstart as u64;
    }

    /* If the last block was written to next_out, then done. */
    if last != 0 {
        return BlockState::FinishDone;
    }

    /* If flushing and all input has been consumed, then done. */
    if flush != Z_NO_FLUSH
        && flush != Z_FINISH
        && z.strm.avail_in == 0
        && s.strstart as i64 == s.block_start
    {
        return BlockState::BlockDone;
    }

    /* Fill the window with any remaining input. */
    have = (s.window_size - s.strstart as u64) as u32;
    if z.strm.avail_in > have && s.block_start >= s.w_size as i64 {
        /* Slide the window down. */
        s.block_start -= s.w_size as i64;
        s.strstart -= s.w_size;
        let (w, st) = (s.w_size as usize, s.strstart as usize);
        s.window.copy_within(w..w + st, 0);
        if s.matches < 2 {
            s.matches += 1; /* add a pending slide_hash() */
        }
        have += s.w_size; /* more space now */
        if s.insert > s.strstart {
            s.insert = s.strstart;
        }
    }
    if have > z.strm.avail_in {
        have = z.strm.avail_in;
    }
    if have != 0 {
        let st = s.strstart as usize;
        read_buf(z.strm, z.input, s.wrap, &mut s.window, st, have);
        s.strstart += have;
        s.insert += have.min(s.w_size - s.insert);
    }
    if s.high_water < s.strstart as u64 {
        s.high_water = s.strstart as u64;
    }

    /* There was not enough avail_out to write a complete worthy or flushed
     * stored block to next_out. Write a stored block to pending instead, if we
     * have enough input for a worthy block, or if flushing and there is enough
     * room for the remaining input as a stored block in the pending buffer.
     */
    have = ((s.bi_valid + 42) >> 3) as u32; /* number of header bytes */
    /* maximum stored block length that will fit in pending: */
    have = ((s.pending_buf_size - have as u64) as u32).min(MAX_STORED);
    min_block = have.min(s.w_size);
    left = (s.strstart as i64 - s.block_start) as u32;
    if left >= min_block
        || ((left != 0 || flush == Z_FINISH)
            && flush != Z_NO_FLUSH
            && z.strm.avail_in == 0
            && left <= have)
    {
        len = left.min(have);
        last = if flush == Z_FINISH && z.strm.avail_in == 0 && len == left {
            1
        } else {
            0
        };
        tr_stored_block(s, Some(s.block_start as usize), len as u64, last as i32);
        s.block_start += len as i64;
        flush_pending(s, z);
    }

    /* We've done all we can with the available input and output. */
    if last != 0 {
        BlockState::FinishStarted
    } else {
        BlockState::NeedMore
    }
}

/* ===========================================================================
 * Compress as much as possible from the input stream, return the current
 * block state.
 * This function does not perform lazy evaluation of matches and inserts
 * new strings in the dictionary only for unmatched strings or for short
 * matches. It is used only for the fast compression options.
 */
fn deflate_fast(s: &mut DeflateState, z: &mut Z<'_>, flush: i32) -> BlockState {
    let mut hash_head: u32; /* head of the hash chain */
    let mut bflush: bool; /* set if current block must be flushed */

    loop {
        /* Make sure that we always have enough lookahead, except
         * at the end of the input file. We need MAX_MATCH bytes
         * for the next match, plus MIN_MATCH bytes to insert the
         * string following the next match.
         */
        if s.lookahead < MIN_LOOKAHEAD {
            fill_window(s, z);
            if s.lookahead < MIN_LOOKAHEAD && flush == Z_NO_FLUSH {
                return BlockState::NeedMore;
            }
            if s.lookahead == 0 {
                break; /* flush the current block */
            }
        }

        /* Insert the string window[strstart .. strstart + 2] in the
         * dictionary, and set hash_head to the head of the hash chain:
         */
        hash_head = NIL;
        if s.lookahead >= MIN_MATCH {
            hash_head = insert_string(s, s.strstart);
        }

        /* Find the longest match, discarding those <= prev_length.
         * At this point we have always match_length < MIN_MATCH
         */
        if hash_head != NIL && s.strstart - hash_head <= max_dist(s) {
            /* To simplify the code, we prevent matches with the string
             * of window index 0 (in particular we have to avoid a match
             * of the string with itself at the start of the input file).
             */
            s.match_length = longest_match(s, hash_head);
            /* longest_match() sets match_start */
        }
        if s.match_length >= MIN_MATCH {
            bflush = tr_tally_dist(s, s.strstart - s.match_start, s.match_length - MIN_MATCH);

            s.lookahead -= s.match_length;

            /* Insert new strings in the hash table only if the match length
             * is not too large. This saves time but degrades compression.
             */
            if s.match_length <= s.max_lazy_match && s.lookahead >= MIN_MATCH {
                s.match_length -= 1; /* string at strstart already in table */
                loop {
                    s.strstart += 1;
                    insert_string(s, s.strstart);
                    /* strstart never exceeds WSIZE-MAX_MATCH, so there are
                     * always MIN_MATCH bytes ahead.
                     */
                    s.match_length -= 1;
                    if s.match_length == 0 {
                        break;
                    }
                }
                s.strstart += 1;
            } else {
                s.strstart += s.match_length;
                s.match_length = 0;
                s.ins_h = s.window[s.strstart as usize] as u32;
                s.ins_h = update_hash(s, s.ins_h, s.window[s.strstart as usize + 1]);
                /* If lookahead < MIN_MATCH, ins_h is garbage, but it does not
                 * matter since it will be recomputed at next deflate call.
                 */
            }
        } else {
            /* No match, output a literal byte */
            bflush = tr_tally_lit(s, s.window[s.strstart as usize]);
            s.lookahead -= 1;
            s.strstart += 1;
        }
        if bflush {
            if let Some(b) = flush_block(s, z, 0) {
                return b;
            }
        }
    }
    s.insert = if s.strstart < MIN_MATCH - 1 {
        s.strstart
    } else {
        MIN_MATCH - 1
    };
    if flush == Z_FINISH {
        if let Some(b) = flush_block(s, z, 1) {
            return b;
        }
        return BlockState::FinishDone;
    }
    if s.sym_next != 0 {
        if let Some(b) = flush_block(s, z, 0) {
            return b;
        }
    }
    BlockState::BlockDone
}

/* ===========================================================================
 * Same as above, but achieves better compression. We use a lazy
 * evaluation for matches: a match is finally adopted only if there is
 * no better match at the next window position.
 */
fn deflate_slow(s: &mut DeflateState, z: &mut Z<'_>, flush: i32) -> BlockState {
    let mut hash_head: u32; /* head of hash chain */
    let mut bflush: bool; /* set if current block must be flushed */

    /* Process the input block. */
    loop {
        /* Make sure that we always have enough lookahead, except
         * at the end of the input file. We need MAX_MATCH bytes
         * for the next match, plus MIN_MATCH bytes to insert the
         * string following the next match.
         */
        if s.lookahead < MIN_LOOKAHEAD {
            fill_window(s, z);
            if s.lookahead < MIN_LOOKAHEAD && flush == Z_NO_FLUSH {
                return BlockState::NeedMore;
            }
            if s.lookahead == 0 {
                break; /* flush the current block */
            }
        }

        /* Insert the string window[strstart .. strstart + 2] in the
         * dictionary, and set hash_head to the head of the hash chain:
         */
        hash_head = NIL;
        if s.lookahead >= MIN_MATCH {
            hash_head = insert_string(s, s.strstart);
        }

        /* Find the longest match, discarding those <= prev_length.
         */
        s.prev_length = s.match_length;
        s.prev_match = s.match_start;
        s.match_length = MIN_MATCH - 1;

        if hash_head != NIL
            && s.prev_length < s.max_lazy_match
            && s.strstart - hash_head <= max_dist(s)
        {
            /* To simplify the code, we prevent matches with the string
             * of window index 0 (in particular we have to avoid a match
             * of the string with itself at the start of the input file).
             */
            s.match_length = longest_match(s, hash_head);
            /* longest_match() sets match_start */

            if s.match_length <= 5
                && (s.strategy == Z_FILTERED
                    || (s.match_length == MIN_MATCH
                        && s.strstart.wrapping_sub(s.match_start) > TOO_FAR))
            {
                /* If prev_match is also MIN_MATCH, match_start is garbage
                 * but we will ignore the current match anyway.
                 */
                s.match_length = MIN_MATCH - 1;
            }
        }
        /* If there was a match at the previous step and the current
         * match is not better, output the previous match:
         */
        if s.prev_length >= MIN_MATCH && s.match_length <= s.prev_length {
            let max_insert = s.strstart + s.lookahead - MIN_MATCH;
            /* Do not insert strings in hash table beyond this. */

            bflush = tr_tally_dist(s, s.strstart - 1 - s.prev_match, s.prev_length - MIN_MATCH);

            /* Insert in hash table all strings up to the end of the match.
             * strstart - 1 and strstart are already inserted. If there is not
             * enough lookahead, the last two strings are not inserted in
             * the hash table.
             */
            s.lookahead -= s.prev_length - 1;
            s.prev_length -= 2;
            loop {
                s.strstart += 1;
                if s.strstart <= max_insert {
                    insert_string(s, s.strstart);
                }
                s.prev_length -= 1;
                if s.prev_length == 0 {
                    break;
                }
            }
            s.match_available = 0;
            s.match_length = MIN_MATCH - 1;
            s.strstart += 1;

            if bflush {
                if let Some(b) = flush_block(s, z, 0) {
                    return b;
                }
            }
        } else if s.match_available != 0 {
            /* If there was no match at the previous position, output a
             * single literal. If there was a match but the current match
             * is longer, truncate the previous match to a single literal.
             */
            bflush = tr_tally_lit(s, s.window[s.strstart as usize - 1]);
            if bflush {
                flush_block_only(s, z, 0);
            }
            s.strstart += 1;
            s.lookahead -= 1;
            if z.strm.avail_out == 0 {
                return BlockState::NeedMore;
            }
        } else {
            /* There is no previous match to compare with, wait for
             * the next step to decide.
             */
            s.match_available = 1;
            s.strstart += 1;
            s.lookahead -= 1;
        }
    }
    /* Assert (flush != Z_NO_FLUSH, "no flush?"); */
    if s.match_available != 0 {
        tr_tally_lit(s, s.window[s.strstart as usize - 1]);
        s.match_available = 0;
    }
    s.insert = if s.strstart < MIN_MATCH - 1 {
        s.strstart
    } else {
        MIN_MATCH - 1
    };
    if flush == Z_FINISH {
        if let Some(b) = flush_block(s, z, 1) {
            return b;
        }
        return BlockState::FinishDone;
    }
    if s.sym_next != 0 {
        if let Some(b) = flush_block(s, z, 0) {
            return b;
        }
    }
    BlockState::BlockDone
}

/* ===========================================================================
 * For Z_RLE, simply look for runs of bytes, generate matches only of distance
 * one.  Do not maintain a hash table.  (It will be regenerated if this run of
 * deflate switches away from Z_RLE.)
 */
fn deflate_rle(s: &mut DeflateState, z: &mut Z<'_>, flush: i32) -> BlockState {
    let mut bflush: bool; /* set if current block must be flushed */

    loop {
        /* Make sure that we always have enough lookahead, except
         * at the end of the input file. We need MAX_MATCH bytes
         * for the longest run, plus one for the unrolled loop.
         */
        if s.lookahead <= MAX_MATCH {
            fill_window(s, z);
            if s.lookahead <= MAX_MATCH && flush == Z_NO_FLUSH {
                return BlockState::NeedMore;
            }
            if s.lookahead == 0 {
                break; /* flush the current block */
            }
        }

        /* See how many times the previous byte repeats */
        s.match_length = 0;
        if s.lookahead >= MIN_MATCH && s.strstart > 0 {
            let w = &s.window;
            let mut scan = s.strstart as usize - 1;
            let prev = w[scan]; /* byte at distance one to match */
            if prev == w[scan + 1] && prev == w[scan + 2] && prev == w[scan + 3] {
                scan += 3;
                let strend = s.strstart as usize + MAX_MATCH as usize;
                'run: loop {
                    for _ in 0..8 {
                        scan += 1;
                        if prev != w[scan] {
                            break 'run;
                        }
                    }
                    if scan >= strend {
                        break;
                    }
                }
                s.match_length = MAX_MATCH - (strend - scan) as u32;
                if s.match_length > s.lookahead {
                    s.match_length = s.lookahead;
                }
            }
        }

        /* Emit match if have run of MIN_MATCH or longer, else emit literal */
        if s.match_length >= MIN_MATCH {
            bflush = tr_tally_dist(s, 1, s.match_length - MIN_MATCH);

            s.lookahead -= s.match_length;
            s.strstart += s.match_length;
            s.match_length = 0;
        } else {
            /* No match, output a literal byte */
            bflush = tr_tally_lit(s, s.window[s.strstart as usize]);
            s.lookahead -= 1;
            s.strstart += 1;
        }
        if bflush {
            if let Some(b) = flush_block(s, z, 0) {
                return b;
            }
        }
    }
    s.insert = 0;
    if flush == Z_FINISH {
        if let Some(b) = flush_block(s, z, 1) {
            return b;
        }
        return BlockState::FinishDone;
    }
    if s.sym_next != 0 {
        if let Some(b) = flush_block(s, z, 0) {
            return b;
        }
    }
    BlockState::BlockDone
}

/* ===========================================================================
 * For Z_HUFFMAN_ONLY, do not look for matches.  Do not maintain a hash table.
 * (It will be regenerated if this run of deflate switches away from Huffman.)
 */
fn deflate_huff(s: &mut DeflateState, z: &mut Z<'_>, flush: i32) -> BlockState {
    let mut bflush: bool; /* set if current block must be flushed */

    loop {
        /* Make sure that we have a literal to write. */
        if s.lookahead == 0 {
            fill_window(s, z);
            if s.lookahead == 0 {
                if flush == Z_NO_FLUSH {
                    return BlockState::NeedMore;
                }
                break; /* flush the current block */
            }
        }

        /* Output a literal byte */
        s.match_length = 0;
        bflush = tr_tally_lit(s, s.window[s.strstart as usize]);
        s.lookahead -= 1;
        s.strstart += 1;
        if bflush {
            if let Some(b) = flush_block(s, z, 0) {
                return b;
            }
        }
    }
    s.insert = 0;
    if flush == Z_FINISH {
        if let Some(b) = flush_block(s, z, 1) {
            return b;
        }
        return BlockState::FinishDone;
    }
    if s.sym_next != 0 {
        if let Some(b) = flush_block(s, z, 0) {
            return b;
        }
    }
    BlockState::BlockDone
}
