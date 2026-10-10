// Rust translation of trees.c (and the tables of trees.h) from zlib 1.3.1.
// Copyright (C) 1995-2024 Jean-loup Gailly
// detect_data_type() function provided freely by Cosmin Truta, 2006
// This is an altered (translated) version of the original software; zlib is
// used under the zlib license (see zlib.h and LICENSE.txt).

//! trees.c -- output deflated data using Huffman coding
//!
//! ```text
//!  ALGORITHM
//!
//!      The "deflation" process uses several Huffman trees. The more
//!      common source values are represented by shorter bit sequences.
//!
//!      Each code tree is stored in a compressed form which is itself
//! a Huffman encoding of the lengths of all the code strings (in
//! ascending order by source values).  The actual code strings are
//! reconstructed from the lengths in the inflate process, as described
//! in the deflate specification.
//!
//!  REFERENCES
//!
//!      Deutsch, L.P.,"'Deflate' Compressed Data Format Specification".
//!      Available in ftp.uu.net:/pub/archiving/zip/doc/deflate-1.1.doc
//!
//!      Storer, James A.
//!          Data Compression:  Methods and Theory, pp. 49-50.
//!          Computer Science Press, 1988.  ISBN 0-7167-8156-5.
//!
//!      Sedgewick, R.
//!          Algorithms, p290.
//!          Addison-Wesley, 1983. ISBN 0-201-06672-6.
//! ```
//!
//! (The static tables, which zlib ships in trees.h as `tr_static_init()`
//! generates them with `GEN_TREES_H`, are computed by that code at compile
//! time.)

use super::deflate::{put_byte, DeflateState};
use super::{
    DYN_TREES, MAX_MATCH, MIN_MATCH, STATIC_TREES, STORED_BLOCK, Z_BINARY, Z_FIXED, Z_TEXT,
    Z_UNKNOWN,
};

/* deflate.h: internal compression state */

pub(crate) const LENGTH_CODES: usize = 29;
/* number of length codes, not counting the special END_BLOCK code */

pub(crate) const LITERALS: usize = 256;
/* number of literal bytes 0..255 */

pub(crate) const L_CODES: usize = LITERALS + 1 + LENGTH_CODES;
/* number of Literal or Length codes, including the END_BLOCK code */

pub(crate) const D_CODES: usize = 30;
/* number of distance codes */

pub(crate) const BL_CODES: usize = 19;
/* number of codes used to transfer the bit lengths */

pub(crate) const HEAP_SIZE: usize = 2 * L_CODES + 1;
/* maximum heap size */

pub(crate) const MAX_BITS: usize = 15;
/* All codes must not exceed MAX_BITS bits */

const BUF_SIZE: i32 = 16;
/* size of bit buffer in bi_buf */

/// `ct_data`: data structure describing a single value and its code
/// string (`fc` is the frequency count or the bit string, `dl` the father
/// node in the Huffman tree or the length of the bit string).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CtData {
    pub(crate) fc: u16,
    pub(crate) dl: u16,
}

/* trees.c */

/* ===========================================================================
 * Constants
 */

const MAX_BL_BITS: i32 = 7;
/* Bit length codes must not exceed MAX_BL_BITS bits */

const END_BLOCK: usize = 256;
/* end of block literal code */

const REP_3_6: usize = 16;
/* repeat previous bit length 3-6 times (2 bits of repeat count) */

const REPZ_3_10: usize = 17;
/* repeat a zero length 3-10 times  (3 bits of repeat count) */

const REPZ_11_138: usize = 18;
/* repeat a zero length 11-138 times  (7 bits of repeat count) */

static EXTRA_LBITS: [i32; LENGTH_CODES] /* extra bits for each length code */
   = [0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4,5,5,5,5,0];

static EXTRA_DBITS: [i32; D_CODES] /* extra bits for each distance code */
   = [0,0,0,0,1,1,2,2,3,3,4,4,5,5,6,6,7,7,8,8,9,9,10,10,11,11,12,12,13,13];

static EXTRA_BLBITS: [i32; BL_CODES]/* extra bits for each bit length code */
   = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2,3,7];

static BL_ORDER: [u8; BL_CODES] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];
/* The lengths of the bit length codes are sent in order of decreasing
 * probability, to avoid transmitting the lengths for unused bit length codes.
 */

/* ===========================================================================
 * Local data. These are initialized only once.
 */

const DIST_CODE_LEN: usize = 512; /* see definition of array dist_code below */

/// The tables of trees.h.
struct StaticTables {
    static_ltree: [CtData; L_CODES + 2],
    /* The static literal tree. Since the bit lengths are imposed, there is no
     * need for the L_CODES extra codes used during heap construction. However
     * The codes 286 and 287 are needed to build a canonical tree (see _tr_init
     * below).
     */
    static_dtree: [CtData; D_CODES],
    /* The static distance tree. (Actually a trivial tree since all codes use
     * 5 bits.)
     */
    dist_code: [u8; DIST_CODE_LEN],
    /* Distance codes. The first 256 values correspond to the distances
     * 3 .. 258, the last 256 values correspond to the top 8 bits of
     * the 15 bit distances.
     */
    length_code: [u8; (MAX_MATCH - MIN_MATCH + 1) as usize],
    /* length code for each normalized match length (0 == MIN_MATCH) */
    base_length: [i32; LENGTH_CODES],
    /* First normalized length for each code (0 = MIN_MATCH) */
    base_dist: [i32; D_CODES],
    /* First normalized distance for each code (0 = distance of 1) */
}

static TABLES: StaticTables = tr_static_init();

/// `static_tree_desc`
struct StaticTreeDesc {
    static_tree: Option<&'static [CtData]>, /* static tree or NULL */
    extra_bits: &'static [i32],             /* extra bits for each code or NULL */
    extra_base: i32,                        /* base index for extra_bits */
    elems: i32,                             /* max number of elements in the tree */
    max_length: i32,                        /* max bit length for the codes */
}

static STATIC_L_DESC: StaticTreeDesc = StaticTreeDesc {
    static_tree: Some(&TABLES.static_ltree),
    extra_bits: &EXTRA_LBITS,
    extra_base: LITERALS as i32 + 1,
    elems: L_CODES as i32,
    max_length: MAX_BITS as i32,
};

static STATIC_D_DESC: StaticTreeDesc = StaticTreeDesc {
    static_tree: Some(&TABLES.static_dtree),
    extra_bits: &EXTRA_DBITS,
    extra_base: 0,
    elems: D_CODES as i32,
    max_length: MAX_BITS as i32,
};

static STATIC_BL_DESC: StaticTreeDesc = StaticTreeDesc {
    static_tree: None,
    extra_bits: &EXTRA_BLBITS,
    extra_base: 0,
    elems: BL_CODES as i32,
    max_length: MAX_BL_BITS,
};

/// Which of the state's trees a `tree_desc` describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tree {
    L,
    D,
    Bl,
}

/* ===========================================================================
 * Output a short LSB first on the stream.
 * IN assertion: there is enough room in pendingBuf.
 */
fn put_short(s: &mut DeflateState, w: u16) {
    put_byte(s, (w & 0xff) as u8);
    put_byte(s, (w >> 8) as u8);
}

/* ===========================================================================
 * Reverse the first len bits of a code, using straightforward code (a faster
 * method would use a table)
 * IN assertion: 1 <= len <= 15
 */
const fn bi_reverse(mut code: u32, mut len: i32) -> u32 {
    let mut res: u32 = 0;
    loop {
        res |= code & 1;
        code >>= 1;
        res <<= 1;
        len -= 1;
        if len <= 0 {
            break;
        }
    }
    res >> 1
}

/* ===========================================================================
 * Flush the bit buffer, keeping at most 7 bits in it.
 */
fn bi_flush(s: &mut DeflateState) {
    if s.bi_valid == 16 {
        put_short(s, s.bi_buf);
        s.bi_buf = 0;
        s.bi_valid = 0;
    } else if s.bi_valid >= 8 {
        put_byte(s, s.bi_buf as u8);
        s.bi_buf >>= 8;
        s.bi_valid -= 8;
    }
}

/* ===========================================================================
 * Flush the bit buffer and align the output on a byte boundary
 */
fn bi_windup(s: &mut DeflateState) {
    if s.bi_valid > 8 {
        put_short(s, s.bi_buf);
    } else if s.bi_valid > 0 {
        put_byte(s, s.bi_buf as u8);
    }
    s.bi_buf = 0;
    s.bi_valid = 0;
}

/* ===========================================================================
 * Generate the codes for a given tree and bit counts (which need not be
 * optimal).
 * IN assertion: the array bl_count contains the bit length statistics for
 * the given tree and the field len is set for all tree elements.
 * OUT assertion: the field code is set for all tree elements of non
 *     zero code length.
 */
const fn gen_codes(tree: &mut [CtData], max_code: i32, bl_count: &[u16; MAX_BITS + 1]) {
    let mut next_code = [0u16; MAX_BITS + 1]; /* next code value for each bit length */
    let mut code: u32 = 0; /* running code value */

    /* The distribution counts are first used to generate the code values
     * without bit reversal.
     */
    let mut bits = 1;
    while bits <= MAX_BITS {
        code = (code + bl_count[bits - 1] as u32) << 1;
        next_code[bits] = code as u16;
        bits += 1;
    }
    /* Check that the bit counts in bl_count are consistent. The last code
     * must be all ones.
     */

    let mut n = 0;
    while n <= max_code as usize {
        let len = tree[n].dl as usize;
        if len != 0 {
            /* Now reverse the bits */
            tree[n].fc = bi_reverse(next_code[len] as u32, len as i32) as u16;
            next_code[len] += 1;
        }
        n += 1;
    }
}

/// `send_bits`: send a value on a given number of bits.
/// IN assertion: length <= 16 and value fits in length bits.
fn send_bits(s: &mut DeflateState, value: i32, length: i32) {
    let len = length;
    if s.bi_valid > BUF_SIZE - len {
        let val = value;
        s.bi_buf |= ((val as u16) as u32).wrapping_shl(s.bi_valid as u32) as u16;
        put_short(s, s.bi_buf);
        s.bi_buf = ((val as u16) as u32 >> (BUF_SIZE - s.bi_valid)) as u16;
        s.bi_valid += len - BUF_SIZE;
    } else {
        s.bi_buf |= ((value as u16) as u32).wrapping_shl(s.bi_valid as u32) as u16;
        s.bi_valid += len;
    }
}

/// `send_code`: send a code of the given tree.
fn send_code(s: &mut DeflateState, c: usize, tree: &[CtData]) {
    let t = tree[c];
    send_bits(s, t.fc as i32, t.dl as i32);
}

/* ===========================================================================
 * Initialize the various 'constant' tables.
 */
const fn tr_static_init() -> StaticTables {
    let mut t = StaticTables {
        static_ltree: [CtData { fc: 0, dl: 0 }; L_CODES + 2],
        static_dtree: [CtData { fc: 0, dl: 0 }; D_CODES],
        dist_code: [0; DIST_CODE_LEN],
        length_code: [0; (MAX_MATCH - MIN_MATCH + 1) as usize],
        base_length: [0; LENGTH_CODES],
        base_dist: [0; D_CODES],
    };
    let mut n: usize; /* iterates over tree elements */
    let mut length: usize; /* length value */
    let mut code: usize; /* code value */
    let mut dist: usize; /* distance index */
    let mut bl_count = [0u16; MAX_BITS + 1];
    /* number of codes at each bit length for an optimal tree */

    /* Initialize the mapping length (0..255) -> length code (0..28) */
    length = 0;
    code = 0;
    while code < LENGTH_CODES - 1 {
        t.base_length[code] = length as i32;
        n = 0;
        while n < (1 << EXTRA_LBITS[code]) {
            t.length_code[length] = code as u8;
            length += 1;
            n += 1;
        }
        code += 1;
    }
    /* Note that the length 255 (match length 258) can be represented
     * in two different ways: code 284 + 5 bits or code 285, so we
     * overwrite length_code[255] to use the best encoding:
     */
    t.length_code[length - 1] = code as u8;

    /* Initialize the mapping dist (0..32K) -> dist code (0..29) */
    dist = 0;
    code = 0;
    while code < 16 {
        t.base_dist[code] = dist as i32;
        n = 0;
        while n < (1 << EXTRA_DBITS[code]) {
            t.dist_code[dist] = code as u8;
            dist += 1;
            n += 1;
        }
        code += 1;
    }
    dist >>= 7; /* from now on, all distances are divided by 128 */
    while code < D_CODES {
        t.base_dist[code] = (dist << 7) as i32;
        n = 0;
        while n < (1 << (EXTRA_DBITS[code] - 7)) {
            t.dist_code[256 + dist] = code as u8;
            dist += 1;
            n += 1;
        }
        code += 1;
    }

    /* Construct the codes of the static literal tree */
    n = 0;
    while n <= 143 {
        t.static_ltree[n].dl = 8;
        bl_count[8] += 1;
        n += 1;
    }
    while n <= 255 {
        t.static_ltree[n].dl = 9;
        bl_count[9] += 1;
        n += 1;
    }
    while n <= 279 {
        t.static_ltree[n].dl = 7;
        bl_count[7] += 1;
        n += 1;
    }
    while n <= 287 {
        t.static_ltree[n].dl = 8;
        bl_count[8] += 1;
        n += 1;
    }
    /* Codes 286 and 287 do not exist, but we must include them in the
     * tree construction to get a canonical Huffman tree (longest code
     * all ones)
     */
    gen_codes(&mut t.static_ltree, L_CODES as i32 + 1, &bl_count);

    /* The static distance tree is trivial: */
    n = 0;
    while n < D_CODES {
        t.static_dtree[n].dl = 5;
        t.static_dtree[n].fc = bi_reverse(n as u32, 5) as u16;
        n += 1;
    }
    t
}

/// `d_code(dist)`: mapping from a distance to a distance code. dist is
/// the distance - 1 and must not have side effects. _dist_code[256] and
/// _dist_code[257] are never used.
fn d_code(dist: u32) -> usize {
    if dist < 256 {
        TABLES.dist_code[dist as usize] as usize
    } else {
        TABLES.dist_code[256 + (dist >> 7) as usize] as usize
    }
}

/* ===========================================================================
 * Initialize a new block.
 */
fn init_block(s: &mut DeflateState) {
    /* Initialize the trees. */
    for n in 0..L_CODES {
        s.dyn_ltree[n].fc = 0;
    }
    for n in 0..D_CODES {
        s.dyn_dtree[n].fc = 0;
    }
    for n in 0..BL_CODES {
        s.bl_tree[n].fc = 0;
    }

    s.dyn_ltree[END_BLOCK].fc = 1;
    s.opt_len = 0;
    s.static_len = 0;
    s.sym_next = 0;
    s.matches = 0;
}

/* ===========================================================================
 * Initialize the tree data structures for a new zlib stream.
 */
pub(crate) fn tr_init(s: &mut DeflateState) {
    /* (tr_static_init(): the tables are constants; the descriptors' trees
     * and static descriptors are named by Tree) */
    s.bi_buf = 0;
    s.bi_valid = 0;

    /* Initialize the first block of the first file: */
    init_block(s);
}

const SMALLEST: usize = 1;
/* Index within the heap array of least frequent node in the Huffman tree */

/// `pqremove`: remove the smallest element from the heap and recreate the
/// heap with one less element. Updates heap and heap_len.
fn pqremove(s: &mut DeflateState, tree: &[CtData]) -> i32 {
    let top = s.heap[SMALLEST];
    s.heap[SMALLEST] = s.heap[s.heap_len as usize];
    s.heap_len -= 1;
    pqdownheap(s, tree, SMALLEST as i32);
    top
}

/// `smaller`: compares to subtrees, using the tree depth as tie breaker
/// when the subtrees have equal frequency. This minimizes the worst case
/// length.
fn smaller(tree: &[CtData], n: i32, m: i32, depth: &[u8]) -> bool {
    let (n, m) = (n as usize, m as usize);
    tree[n].fc < tree[m].fc || (tree[n].fc == tree[m].fc && depth[n] <= depth[m])
}

/* ===========================================================================
 * Restore the heap property by moving down the tree starting at node k,
 * exchanging a node with the smallest of its two sons if necessary, stopping
 * when the heap property is re-established (each father smaller than its
 * two sons).
 */
fn pqdownheap(s: &mut DeflateState, tree: &[CtData], mut k: i32) {
    let v = s.heap[k as usize];
    let mut j = k << 1; /* left son of k */
    while j <= s.heap_len {
        /* Set j to the smallest of the two sons: */
        if j < s.heap_len && smaller(tree, s.heap[j as usize + 1], s.heap[j as usize], &s.depth) {
            j += 1;
        }
        /* Exit if v is smaller than both sons */
        if smaller(tree, v, s.heap[j as usize], &s.depth) {
            break;
        }

        /* Exchange v with the smallest son */
        s.heap[k as usize] = s.heap[j as usize];
        k = j;

        /* And continue down the tree, setting j to the left son of k */
        j <<= 1;
    }
    s.heap[k as usize] = v;
}

/// The descriptor of a tree, and its static descriptor.
fn desc(
    s: &mut DeflateState,
    which: Tree,
) -> (&mut super::deflate::TreeDesc, &'static StaticTreeDesc) {
    match which {
        Tree::L => (&mut s.l_desc, &STATIC_L_DESC),
        Tree::D => (&mut s.d_desc, &STATIC_D_DESC),
        Tree::Bl => (&mut s.bl_desc, &STATIC_BL_DESC),
    }
}

/* ===========================================================================
 * Compute the optimal bit lengths for a tree and update the total bit length
 * for the current block.
 * IN assertion: the fields freq and dad are set, heap[heap_max] and
 *    above are the tree nodes sorted by increasing frequency.
 * OUT assertions: the field len is set to the optimal bit length, the
 *     array bl_count contains the frequencies for each bit length.
 *     The length opt_len is updated; static_len is also updated if stree is
 *     not null.
 */
fn gen_bitlen(s: &mut DeflateState, tree: &mut [CtData], which: Tree) {
    let (d, stat) = desc(s, which);
    let max_code = d.max_code;
    let stree = stat.static_tree;
    let extra = stat.extra_bits;
    let base = stat.extra_base;
    let max_length = stat.max_length;
    let mut h: usize; /* heap index */
    let mut n: i32; /* iterate over the tree elements */
    let mut m: i32;
    let mut bits: i32; /* bit length */
    let mut xbits: i32; /* extra bits */
    let mut f: u16; /* frequency */
    let mut overflow = 0; /* number of elements with bit length too large */

    for bits in 0..=MAX_BITS {
        s.bl_count[bits] = 0;
    }

    /* In a first pass, compute the optimal bit lengths (which may
     * overflow in the case of the bit length tree).
     */
    tree[s.heap[s.heap_max as usize] as usize].dl = 0; /* root of the heap */

    h = s.heap_max as usize + 1;
    while h < HEAP_SIZE {
        n = s.heap[h];
        bits = tree[tree[n as usize].dl as usize].dl as i32 + 1;
        if bits > max_length {
            bits = max_length;
            overflow += 1;
        }
        tree[n as usize].dl = bits as u16;
        /* We overwrite tree[n].Dad which is no longer needed */

        h += 1;
        if n > max_code {
            continue; /* not a leaf node */
        }

        s.bl_count[bits as usize] += 1;
        xbits = 0;
        if n >= base {
            xbits = extra[(n - base) as usize];
        }
        f = tree[n as usize].fc;
        s.opt_len = s
            .opt_len
            .wrapping_add(f as u64 * (bits + xbits) as u32 as u64);
        if let Some(stree) = stree {
            s.static_len = s
                .static_len
                .wrapping_add(f as u64 * (stree[n as usize].dl as i32 + xbits) as u32 as u64);
        }
    }
    if overflow == 0 {
        return;
    }

    /* This happens for example on obj2 and pic of the Calgary corpus */

    /* Find the first bit length which could increase: */
    loop {
        bits = max_length - 1;
        while s.bl_count[bits as usize] == 0 {
            bits -= 1;
        }
        s.bl_count[bits as usize] -= 1; /* move one leaf down the tree */
        s.bl_count[bits as usize + 1] += 2; /* move one overflow item as its brother */
        s.bl_count[max_length as usize] -= 1;
        /* The brother of the overflow item also moves one step up,
         * but this does not affect bl_count[max_length]
         */
        overflow -= 2;
        if overflow <= 0 {
            break;
        }
    }

    /* Now recompute all bit lengths, scanning in increasing frequency.
     * h is still equal to HEAP_SIZE. (It is simpler to reconstruct all
     * lengths instead of fixing only the wrong ones. This idea is taken
     * from 'ar' written by Haruhiko Okumura.)
     */
    bits = max_length;
    while bits != 0 {
        n = s.bl_count[bits as usize] as i32;
        while n != 0 {
            h -= 1;
            m = s.heap[h];
            if m > max_code {
                continue;
            }
            if tree[m as usize].dl as u32 != bits as u32 {
                s.opt_len = s.opt_len.wrapping_add(
                    (bits as u64)
                        .wrapping_sub(tree[m as usize].dl as u64)
                        .wrapping_mul(tree[m as usize].fc as u64),
                );
                tree[m as usize].dl = bits as u16;
            }
            n -= 1;
        }
        bits -= 1;
    }
}

/// Take a tree out of the state (to build or send it while the state is
/// written).
fn take_tree(s: &mut DeflateState, which: Tree) -> Vec<CtData> {
    match which {
        Tree::L => std::mem::take(&mut s.dyn_ltree),
        Tree::D => std::mem::take(&mut s.dyn_dtree),
        Tree::Bl => std::mem::take(&mut s.bl_tree),
    }
}

/// Put back a tree [`take_tree`] took.
fn put_tree(s: &mut DeflateState, which: Tree, tree: Vec<CtData>) {
    match which {
        Tree::L => s.dyn_ltree = tree,
        Tree::D => s.dyn_dtree = tree,
        Tree::Bl => s.bl_tree = tree,
    }
}

/* ===========================================================================
 * Construct one Huffman tree and assigns the code bit strings and lengths.
 * Update the total bit length for the current block.
 * IN assertion: the field freq is set for all tree elements.
 * OUT assertions: the fields len and code are set to the optimal bit length
 *     and corresponding code. The length opt_len is updated; static_len is
 *     also updated if stree is not null. The field max_code is set.
 */
fn build_tree(s: &mut DeflateState, which: Tree) {
    let mut tree = take_tree(s, which);
    let stat = desc(s, which).1;
    let stree = stat.static_tree;
    let elems = stat.elems;
    let mut n: i32; /* iterate over heap elements */
    let mut m: i32;
    let mut max_code: i32 = -1; /* largest code with non zero frequency */
    let mut node: i32; /* new node being created */

    /* Construct the initial heap, with least frequent element in
     * heap[SMALLEST]. The sons of heap[n] are heap[2*n] and heap[2*n + 1].
     * heap[0] is not used.
     */
    s.heap_len = 0;
    s.heap_max = HEAP_SIZE as i32;

    n = 0;
    while n < elems {
        if tree[n as usize].fc != 0 {
            s.heap_len += 1;
            max_code = n;
            s.heap[s.heap_len as usize] = max_code;
            s.depth[n as usize] = 0;
        } else {
            tree[n as usize].dl = 0;
        }
        n += 1;
    }

    /* The pkzip format requires that at least one distance code exists,
     * and that at least one bit should be sent even if there is only one
     * possible code. So to avoid special checks later on we force at least
     * two codes of non zero frequency.
     */
    while s.heap_len < 2 {
        s.heap_len += 1;
        node = if max_code < 2 {
            max_code += 1;
            max_code
        } else {
            0
        };
        s.heap[s.heap_len as usize] = node;
        tree[node as usize].fc = 1;
        s.depth[node as usize] = 0;
        s.opt_len = s.opt_len.wrapping_sub(1);
        if let Some(stree) = stree {
            s.static_len = s.static_len.wrapping_sub(stree[node as usize].dl as u64);
        }
        /* node is 0 or 1 so it does not have extra bits */
    }
    desc(s, which).0.max_code = max_code;

    /* The elements heap[heap_len/2 + 1 .. heap_len] are leaves of the tree,
     * establish sub-heaps of increasing lengths:
     */
    n = s.heap_len / 2;
    while n >= 1 {
        pqdownheap(s, &tree, n);
        n -= 1;
    }

    /* Construct the Huffman tree by repeatedly combining the least two
     * frequent nodes.
     */
    node = elems; /* next internal node of the tree */
    loop {
        n = pqremove(s, &tree); /* n = node of least frequency */
        m = s.heap[SMALLEST]; /* m = node of next least frequency */

        s.heap_max -= 1;
        s.heap[s.heap_max as usize] = n; /* keep the nodes sorted by frequency */
        s.heap_max -= 1;
        s.heap[s.heap_max as usize] = m;

        /* Create a new node father of n and m */
        tree[node as usize].fc = tree[n as usize].fc.wrapping_add(tree[m as usize].fc);
        s.depth[node as usize] = (if s.depth[n as usize] >= s.depth[m as usize] {
            s.depth[n as usize]
        } else {
            s.depth[m as usize]
        })
        .wrapping_add(1);
        tree[n as usize].dl = node as u16;
        tree[m as usize].dl = node as u16;
        /* and insert the new node in the heap */
        s.heap[SMALLEST] = node;
        node += 1;
        pqdownheap(s, &tree, SMALLEST as i32);

        if s.heap_len < 2 {
            break;
        }
    }

    s.heap_max -= 1;
    s.heap[s.heap_max as usize] = s.heap[SMALLEST];

    /* At this point, the fields freq and dad are set. We can now
     * generate the bit lengths.
     */
    gen_bitlen(s, &mut tree, which);

    /* The field len is now set, we can generate the bit codes */
    gen_codes(&mut tree, max_code, &s.bl_count);
    put_tree(s, which, tree);
}

/* ===========================================================================
 * Scan a literal or distance tree to determine the frequencies of the codes
 * in the bit length tree.
 */
fn scan_tree(s: &mut DeflateState, tree: &mut [CtData], max_code: i32) {
    let mut prevlen: i32 = -1; /* last emitted length */
    let mut curlen: i32; /* length of current code */
    let mut nextlen: i32 = tree[0].dl as i32; /* length of next code */
    let mut count: i32 = 0; /* repeat count of the current code */
    let mut max_count: i32 = 7; /* max repeat count */
    let mut min_count: i32 = 4; /* min repeat count */

    if nextlen == 0 {
        max_count = 138;
        min_count = 3;
    }
    tree[(max_code + 1) as usize].dl = 0xffff; /* guard */

    for n in 0..=max_code as usize {
        curlen = nextlen;
        nextlen = tree[n + 1].dl as i32;
        count += 1;
        if count < max_count && curlen == nextlen {
            continue;
        } else if count < min_count {
            s.bl_tree[curlen as usize].fc =
                s.bl_tree[curlen as usize].fc.wrapping_add(count as u16);
        } else if curlen != 0 {
            if curlen != prevlen {
                s.bl_tree[curlen as usize].fc = s.bl_tree[curlen as usize].fc.wrapping_add(1);
            }
            s.bl_tree[REP_3_6].fc = s.bl_tree[REP_3_6].fc.wrapping_add(1);
        } else if count <= 10 {
            s.bl_tree[REPZ_3_10].fc = s.bl_tree[REPZ_3_10].fc.wrapping_add(1);
        } else {
            s.bl_tree[REPZ_11_138].fc = s.bl_tree[REPZ_11_138].fc.wrapping_add(1);
        }
        count = 0;
        prevlen = curlen;
        if nextlen == 0 {
            max_count = 138;
            min_count = 3;
        } else if curlen == nextlen {
            max_count = 6;
            min_count = 3;
        } else {
            max_count = 7;
            min_count = 4;
        }
    }
}

/* ===========================================================================
 * Send a literal or distance tree in compressed form, using the codes in
 * bl_tree.
 */
fn send_tree(s: &mut DeflateState, tree: &[CtData], max_code: i32) {
    let mut prevlen: i32 = -1; /* last emitted length */
    let mut curlen: i32; /* length of current code */
    let mut nextlen: i32 = tree[0].dl as i32; /* length of next code */
    let mut count: i32 = 0; /* repeat count of the current code */
    let mut max_count: i32 = 7; /* max repeat count */
    let mut min_count: i32 = 4; /* min repeat count */
    let bl_tree = s.bl_tree.clone();

    /* tree[max_code + 1].Len = -1; */
    /* guard already set */
    if nextlen == 0 {
        max_count = 138;
        min_count = 3;
    }

    for n in 0..=max_code as usize {
        curlen = nextlen;
        nextlen = tree[n + 1].dl as i32;
        count += 1;
        if count < max_count && curlen == nextlen {
            continue;
        } else if count < min_count {
            loop {
                send_code(s, curlen as usize, &bl_tree);
                count -= 1;
                if count == 0 {
                    break;
                }
            }
        } else if curlen != 0 {
            if curlen != prevlen {
                send_code(s, curlen as usize, &bl_tree);
                count -= 1;
            }
            send_code(s, REP_3_6, &bl_tree);
            send_bits(s, count - 3, 2);
        } else if count <= 10 {
            send_code(s, REPZ_3_10, &bl_tree);
            send_bits(s, count - 3, 3);
        } else {
            send_code(s, REPZ_11_138, &bl_tree);
            send_bits(s, count - 11, 7);
        }
        count = 0;
        prevlen = curlen;
        if nextlen == 0 {
            max_count = 138;
            min_count = 3;
        } else if curlen == nextlen {
            max_count = 6;
            min_count = 3;
        } else {
            max_count = 7;
            min_count = 4;
        }
    }
}

/* ===========================================================================
 * Construct the Huffman tree for the bit lengths and return the index in
 * bl_order of the last bit length code to send.
 */
fn build_bl_tree(s: &mut DeflateState) -> i32 {
    let mut max_blindex: i32; /* index of last bit length code of non zero freq */

    /* Determine the bit length frequencies for literal and distance trees */
    let mut ltree = take_tree(s, Tree::L);
    let max_code = s.l_desc.max_code;
    scan_tree(s, &mut ltree, max_code);
    put_tree(s, Tree::L, ltree);
    let mut dtree = take_tree(s, Tree::D);
    let max_code = s.d_desc.max_code;
    scan_tree(s, &mut dtree, max_code);
    put_tree(s, Tree::D, dtree);

    /* Build the bit length tree: */
    build_tree(s, Tree::Bl);
    /* opt_len now includes the length of the tree representations, except the
     * lengths of the bit lengths codes and the 5 + 5 + 4 bits for the counts.
     */

    /* Determine the number of bit length codes to send. The pkzip format
     * requires that at least 4 bit length codes be sent. (appnote.txt says
     * 3 but the actual value used is 4.)
     */
    max_blindex = BL_CODES as i32 - 1;
    while max_blindex >= 3 {
        if s.bl_tree[BL_ORDER[max_blindex as usize] as usize].dl != 0 {
            break;
        }
        max_blindex -= 1;
    }
    /* Update opt_len to include the bit length tree and counts */
    s.opt_len = s
        .opt_len
        .wrapping_add(3 * (max_blindex as u64 + 1) + 5 + 5 + 4);

    max_blindex
}

/* ===========================================================================
 * Send the header for a block using dynamic Huffman trees: the counts, the
 * lengths of the bit length codes, the literal tree and the distance tree.
 * IN assertion: lcodes >= 257, dcodes >= 1, blcodes >= 4.
 */
fn send_all_trees(s: &mut DeflateState, lcodes: i32, dcodes: i32, blcodes: i32) {
    send_bits(s, lcodes - 257, 5); /* not +255 as stated in appnote.txt */
    send_bits(s, dcodes - 1, 5);
    send_bits(s, blcodes - 4, 4); /* not -3 as stated in appnote.txt */
    for rank in 0..blcodes as usize {
        let len = s.bl_tree[BL_ORDER[rank] as usize].dl;
        send_bits(s, len as i32, 3);
    }

    let ltree = take_tree(s, Tree::L);
    send_tree(s, &ltree, lcodes - 1); /* literal tree */
    put_tree(s, Tree::L, ltree);

    let dtree = take_tree(s, Tree::D);
    send_tree(s, &dtree, dcodes - 1); /* distance tree */
    put_tree(s, Tree::D, dtree);
}

/* ===========================================================================
 * Send a stored block
 * (`buf`, if any, is a position in the window)
 */
pub(crate) fn tr_stored_block(
    s: &mut DeflateState,
    buf: Option<usize>,
    stored_len: u64,
    last: i32,
) {
    send_bits(s, (STORED_BLOCK << 1) + last, 3); /* send block type */
    bi_windup(s); /* align on byte boundary */
    put_short(s, stored_len as u16);
    put_short(s, !stored_len as u16);
    if stored_len != 0 {
        let p = s.pending as usize;
        let b = buf.unwrap_or(0);
        let len = stored_len as usize;
        s.pending_buf[p..p + len].copy_from_slice(&s.window[b..b + len]);
    }
    s.pending += stored_len;
}

/* ===========================================================================
 * Flush the bits in the bit buffer to pending output (leaves at most 7 bits)
 */
pub(crate) fn tr_flush_bits(s: &mut DeflateState) {
    bi_flush(s);
}

/* ===========================================================================
 * Send one empty static block to give enough lookahead for inflate.
 * This takes 10 bits, of which 7 may remain in the bit buffer.
 */
pub(crate) fn tr_align(s: &mut DeflateState) {
    send_bits(s, STATIC_TREES << 1, 3);
    send_code(s, END_BLOCK, &TABLES.static_ltree);
    bi_flush(s);
}

/* ===========================================================================
 * Send the block data compressed using the given Huffman trees
 */
fn compress_block(s: &mut DeflateState, ltree: &[CtData], dtree: &[CtData]) {
    let mut dist: u32; /* distance of matched string */
    let mut lc: i32; /* match length or unmatched char (if dist == 0) */
    let mut sx: usize = 0; /* running index in symbol buffers */
    let mut code: usize; /* the code to send */
    let mut extra: i32; /* number of extra bits to send */

    if s.sym_next != 0 {
        loop {
            let sym = s.sym_buf;
            dist = s.pending_buf[sym + sx] as u32 & 0xff;
            sx += 1;
            dist += (s.pending_buf[sym + sx] as u32 & 0xff) << 8;
            sx += 1;
            lc = s.pending_buf[sym + sx] as i32;
            sx += 1;
            if dist == 0 {
                send_code(s, lc as usize, ltree); /* send a literal byte */
            } else {
                /* Here, lc is the match length - MIN_MATCH */
                code = TABLES.length_code[lc as usize] as usize;
                send_code(s, code + LITERALS + 1, ltree); /* send length code */
                extra = EXTRA_LBITS[code];
                if extra != 0 {
                    lc -= TABLES.base_length[code];
                    send_bits(s, lc, extra); /* send the extra length bits */
                }
                dist -= 1; /* dist is now the match distance - 1 */
                code = d_code(dist);

                send_code(s, code, dtree); /* send the distance code */
                extra = EXTRA_DBITS[code];
                if extra != 0 {
                    dist -= TABLES.base_dist[code] as u32;
                    send_bits(s, dist as i32, extra); /* send the extra distance bits */
                }
            } /* literal or match pair ? */

            /* Check for no overlay of pending_buf on needed symbols */

            if sx >= s.sym_next as usize {
                break;
            }
        }
    }

    send_code(s, END_BLOCK, ltree);
}

/* ===========================================================================
 * Check if the data type is TEXT or BINARY, using the following algorithm:
 * - TEXT if the two conditions below are satisfied:
 *    a) There are no non-portable control characters belonging to the
 *       "block list" (0..6, 14..25, 28..31).
 *    b) There is at least one printable character belonging to the
 *       "allow list" (9 {TAB}, 10 {LF}, 13 {CR}, 32..255).
 * - BINARY otherwise.
 * - The following partially-portable control characters form a
 *   "gray list" that is ignored in this detection algorithm:
 *   (7 {BEL}, 8 {BS}, 11 {VT}, 12 {FF}, 26 {SUB}, 27 {ESC}).
 * IN assertion: the fields Freq of dyn_ltree are set.
 */
fn detect_data_type(s: &DeflateState) -> i32 {
    /* block_mask is the bit mask of block-listed bytes
     * set bits 0..6, 14..25, and 28..31
     * 0xf3ffc07f = binary 11110011111111111100000001111111
     */
    let mut block_mask: u64 = 0xf3ffc07f;

    /* Check for non-textual ("block-listed") bytes. */
    for n in 0..=31 {
        if (block_mask & 1) != 0 && s.dyn_ltree[n].fc != 0 {
            return Z_BINARY;
        }
        block_mask >>= 1;
    }

    /* Check for textual ("allow-listed") bytes. */
    if s.dyn_ltree[9].fc != 0 || s.dyn_ltree[10].fc != 0 || s.dyn_ltree[13].fc != 0 {
        return Z_TEXT;
    }
    for n in 32..LITERALS {
        if s.dyn_ltree[n].fc != 0 {
            return Z_TEXT;
        }
    }

    /* There are no "block-listed" or "allow-listed" bytes:
     * this stream either is empty or has tolerated ("gray-listed") bytes only.
     */
    Z_BINARY
}

/* ===========================================================================
 * Determine the best encoding for the current block: dynamic trees, static
 * trees or store, and write out the encoded block.
 * (`buf`, if any, is a position in the window; `data_type` is
 * `s->strm->data_type`.)
 */
pub(crate) fn tr_flush_block(
    s: &mut DeflateState,
    data_type: &mut i32,
    buf: Option<usize>,
    stored_len: u64,
    last: i32,
) {
    let mut opt_lenb: u64; /* opt_len and static_len in bytes */
    let static_lenb: u64;
    let mut max_blindex = 0; /* index of last bit length code of non zero freq */

    /* Build the Huffman trees unless a stored block is forced */
    if s.level > 0 {
        /* Check if the file is binary or text */
        if *data_type == Z_UNKNOWN {
            *data_type = detect_data_type(s);
        }

        /* Construct the literal and distance trees */
        build_tree(s, Tree::L);

        build_tree(s, Tree::D);
        /* At this point, opt_len and static_len are the total bit lengths of
         * the compressed block data, excluding the tree representations.
         */

        /* Build the bit length tree for the above two trees, and get the index
         * in bl_order of the last bit length code to send.
         */
        max_blindex = build_bl_tree(s);

        /* Determine the best encoding. Compute the block lengths in bytes. */
        opt_lenb = s.opt_len.wrapping_add(3 + 7) >> 3;
        static_lenb = s.static_len.wrapping_add(3 + 7) >> 3;

        if static_lenb <= opt_lenb || s.strategy == Z_FIXED {
            opt_lenb = static_lenb;
        }
    } else {
        opt_lenb = stored_len + 5; /* force a stored block */
        static_lenb = opt_lenb;
    }

    if stored_len + 4 <= opt_lenb && buf.is_some() {
        /* 4: two words for the lengths */
        /* The test buf != NULL is only necessary if LIT_BUFSIZE > WSIZE.
         * Otherwise we can't have processed more than WSIZE input bytes since
         * the last block flush, because compression would have been
         * successful. If LIT_BUFSIZE <= WSIZE, it is never too late to
         * transform a block into a stored block.
         */
        tr_stored_block(s, buf, stored_len, last);
    } else if static_lenb == opt_lenb {
        send_bits(s, (STATIC_TREES << 1) + last, 3);
        compress_block(s, &TABLES.static_ltree, &TABLES.static_dtree);
    } else {
        send_bits(s, (DYN_TREES << 1) + last, 3);
        send_all_trees(
            s,
            s.l_desc.max_code + 1,
            s.d_desc.max_code + 1,
            max_blindex + 1,
        );
        let ltree = take_tree(s, Tree::L);
        let dtree = take_tree(s, Tree::D);
        compress_block(s, &ltree, &dtree);
        put_tree(s, Tree::L, ltree);
        put_tree(s, Tree::D, dtree);
    }
    /* The above check is made mod 2^32, for files larger than 512 MB
     * and uLong implemented on 32 bits.
     */
    init_block(s);

    if last != 0 {
        bi_windup(s);
    }
}

/// `_tr_tally_lit` (the macro deflate.h uses without `ZLIB_DEBUG`): save
/// a literal and return true if the current block must be flushed.
pub(crate) fn tr_tally_lit(s: &mut DeflateState, c: u8) -> bool {
    let cc = c;
    let at = s.sym_buf + s.sym_next as usize;
    s.pending_buf[at] = 0;
    s.pending_buf[at + 1] = 0;
    s.pending_buf[at + 2] = cc;
    s.sym_next += 3;
    s.dyn_ltree[cc as usize].fc = s.dyn_ltree[cc as usize].fc.wrapping_add(1);
    s.sym_next == s.sym_end
}

/// `_tr_tally_dist` (the macro deflate.h uses without `ZLIB_DEBUG`, which
/// unlike `_tr_tally()` doesn't count `matches`): save a match and return
/// true if the current block must be flushed.
pub(crate) fn tr_tally_dist(s: &mut DeflateState, distance: u32, length: u32) -> bool {
    let len = length as u8;
    let mut dist = distance as u16;
    let at = s.sym_buf + s.sym_next as usize;
    s.pending_buf[at] = dist as u8;
    s.pending_buf[at + 1] = (dist >> 8) as u8;
    s.pending_buf[at + 2] = len;
    s.sym_next += 3;
    dist = dist.wrapping_sub(1);
    let lcode = TABLES.length_code[len as usize] as usize + LITERALS + 1;
    s.dyn_ltree[lcode].fc = s.dyn_ltree[lcode].fc.wrapping_add(1);
    let dcode = d_code(dist as u32);
    s.dyn_dtree[dcode].fc = s.dyn_dtree[dcode].fc.wrapping_add(1);
    s.sym_next == s.sym_end
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables, printed one entry per line, hash as zlib's trees.h does.
    #[test]
    fn static_tables_match_trees_h() {
        let mut o = String::new();
        for c in TABLES.static_ltree.iter().chain(&TABLES.static_dtree) {
            o += &format!("{} {}\n", c.fc, c.dl);
        }
        for c in TABLES.dist_code.iter().chain(&TABLES.length_code) {
            o += &format!("{c}\n");
        }
        for c in TABLES.base_length.iter().chain(&TABLES.base_dist) {
            o += &format!("{c}\n");
        }
        let mut h: u64 = 14695981039346656037;
        for b in o.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        assert_eq!((o.len(), h), (4210, 0x01e54b29721f3466));
        assert_eq!(TABLES.static_ltree[143], CtData { fc: 253, dl: 8 });
        assert_eq!(TABLES.static_ltree[256], CtData { fc: 0, dl: 7 });
    }
}
