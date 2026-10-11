// Rust translation of the zlib 1.3 inflater as FreeType's src/gzip/ bundles
// it (zutil.c, inffast.c, inflate.c, inftrees.c, adler32.c, crc32.c, built
// with Z_SOLO and Z_FREETYPE).
// Copyright (C) 1995-2023 Jean-loup Gailly and Mark Adler.
// This is an altered (translated) version of the original software; zlib is
// used under the zlib license (see zlib.h and LICENSE.txt).

//! The parts of zlib that FreeType's `ftgzip.c` compiles: `inflate()`
//! with zlib and gzip wrappers, and raw (for the gzip stream).
//!
//! Translation notes:
//! - The fixed Huffman tables are built on first use, as zlib does with
//!   `BUILDFIXED` (they are identical to `inffixed.h`, which `makefixed()`
//!   generates from the same code).
//! - `crc32()` uses zlib's byte-wise table loop rather than its braided
//!   word-at-a-time variant; both compute the same CRC.
//! - Memory comes from Rust allocations (FreeType routes zlib's `zalloc`
//!   to its own allocator); allocation failure maps to `Z_MEM_ERROR`.

#![allow(clippy::too_many_lines)]

use std::sync::OnceLock;

/* zlib.h */
pub const Z_NO_FLUSH: i32 = 0;
pub const Z_PARTIAL_FLUSH: i32 = 1;
pub const Z_SYNC_FLUSH: i32 = 2;
pub const Z_FULL_FLUSH: i32 = 3;
pub const Z_FINISH: i32 = 4;
pub const Z_BLOCK: i32 = 5;
pub const Z_TREES: i32 = 6;

pub const Z_OK: i32 = 0;
pub const Z_STREAM_END: i32 = 1;
pub const Z_NEED_DICT: i32 = 2;
pub const Z_ERRNO: i32 = -1;
pub const Z_STREAM_ERROR: i32 = -2;
pub const Z_DATA_ERROR: i32 = -3;
pub const Z_MEM_ERROR: i32 = -4;
pub const Z_BUF_ERROR: i32 = -5;
pub const Z_VERSION_ERROR: i32 = -6;

pub const Z_DEFLATED: u32 = 8;

/* zconf.h */
pub const MAX_WBITS: i32 = 15; /* 32K LZ77 window */

/// `z_stream`: the input and output buffers are slices; `next_in` and
/// `next_out` are positions in them.
#[derive(Debug)]
pub struct ZStream<'a> {
    pub input: &'a [u8],
    pub next_in: usize, /* next input byte */
    pub avail_in: u32,  /* number of bytes available at next_in */
    pub total_in: u64,  /* total number of input bytes read so far */

    pub output: &'a mut [u8],
    pub next_out: usize, /* next output byte will go here */
    pub avail_out: u32,  /* remaining free space at next_out */
    pub total_out: u64,  /* total number of bytes output so far */

    pub msg: Option<&'static str>, /* last error message, NULL if no error */
    state: Option<Box<InflateState>>, /* not visible by applications */

    pub data_type: i32, /* best guess about the data type: binary or text
                        for deflate, or the decoding state for inflate */
    pub adler: u64, /* Adler-32 or CRC-32 value of the uncompressed data */
}

impl<'a> ZStream<'a> {
    /// A stream reading all of `input` and writing into `output`.
    pub fn new(input: &'a [u8], output: &'a mut [u8]) -> Self {
        let avail_in = input.len() as u32;
        let avail_out = output.len() as u32;
        ZStream {
            input,
            next_in: 0,
            avail_in,
            total_in: 0,
            output,
            next_out: 0,
            avail_out,
            total_out: 0,
            msg: None,
            state: None,
            data_type: 0,
            adler: 0,
        }
    }
}

/// The state of a stream between `inflate()` calls that change its
/// buffers (C assigns new `next_in` and `next_out` pointers): the inflate
/// state and the counters.
#[derive(Debug, Default)]
pub struct ZStreamState {
    state: Option<Box<InflateState>>,
    pub total_in: u64,
    pub total_out: u64,
    pub msg: Option<&'static str>,
    pub data_type: i32,
    pub adler: u64,
}

impl<'a> ZStream<'a> {
    /// A stream over new buffers, with the state of an earlier one.
    pub fn with_state(
        input: &'a [u8],
        next_in: usize,
        avail_in: u32,
        output: &'a mut [u8],
        next_out: usize,
        avail_out: u32,
        st: ZStreamState,
    ) -> Self {
        ZStream {
            input,
            next_in,
            avail_in,
            total_in: st.total_in,
            output,
            next_out,
            avail_out,
            total_out: st.total_out,
            msg: st.msg,
            state: st.state,
            data_type: st.data_type,
            adler: st.adler,
        }
    }

    /// The stream's state, for [`ZStream::with_state`].
    pub fn into_state(self) -> ZStreamState {
        ZStreamState {
            state: self.state,
            total_in: self.total_in,
            total_out: self.total_out,
            msg: self.msg,
            data_type: self.data_type,
            adler: self.adler,
        }
    }
}

/*************************************************************************
 * inftrees.h
 */

/// `code`: structure for decoding tables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Code {
    pub op: u8,   /* operation, extra bits, table bits */
    pub bits: u8, /* bits in this part of the code */
    pub val: u16, /* offset in table or code value */
}

/* op values as set by inflate_table():
   00000000 - literal
   0000tttt - table link, tttt != 0 is the number of table index bits
   0001eeee - length or distance, eeee is the number of extra bits
   01100000 - end of block
   01000000 - invalid code
*/

/* Maximum size of the dynamic table.  The maximum number of code structures is
1444, which is the sum of 852 for literal/length codes and 592 for distance
codes.  These values were found by exhaustive searches using the program
examples/enough.c found in the zlib distribution.  The arguments to that
program are the number of symbols, the initial root table size, and the
maximum bit length of a code.  "enough 286 9 15" for literal/length codes
returns returns 852, and "enough 30 6 15" for distance codes returns 592.
The initial root table size (9 or 6) is found in the fifth argument of the
inflate_table() calls in inflate.c and infback.c.  If the root table size is
changed, then these maximum sizes would be need to be recalculated and
updated. */
const ENOUGH_LENS: u32 = 852;
const ENOUGH_DISTS: u32 = 592;
const ENOUGH: usize = (ENOUGH_LENS + ENOUGH_DISTS) as usize;

/// `codetype`: type of code to build for inflate_table()
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeType {
    Codes,
    Lens,
    Dists,
}

/*************************************************************************
 * inftrees.c
 */

const MAXBITS: usize = 15;

#[allow(dead_code)]
static INFLATE_COPYRIGHT: &str = " inflate 1.3 Copyright 1995-2023 Mark Adler ";
/*
 If you use the zlib library in a product, an acknowledgment is welcome
 in the documentation of your product. If for some reason you cannot
 include such an acknowledgment, I would appreciate that you keep this
 copyright string in the executable of your product.
*/

/*
  Build a set of tables to decode the provided canonical Huffman code.
  The code lengths are lens[0..codes-1].  The result starts at *table,
  whose indices are 0..2^bits-1.  work is a writable array of at least
  lens shorts, which is used as a work area.  type is the type of code
  to be generated, CODES, LENS, or DISTS.  On return, zero is success,
  -1 is an invalid code, and +1 means that ENOUGH isn't enough.  table
  on return points to the next available entry's address.  bits is the
  requested root table index bits, and on return it is the actual root
  table index bits.  It will differ if the request is greater than the
  longest code or if it is less than the shortest code.

  (Here `table` is `tables[*next..]`, with `*next` advanced on return.)
*/
fn inflate_table(
    type_: CodeType,
    lens: &[u16],
    codes: u32,
    tables: &mut [Code],
    next_index: &mut usize,
    bits: &mut u32,
    work: &mut [u16],
) -> i32 {
    let mut len: u32; /* a code's length in bits */
    let mut sym: u32; /* index of code symbols */
    let mut min: u32; /* minimum and maximum code lengths */
    let mut max: u32;
    let mut root: u32; /* number of index bits for root table */
    let mut curr: u32; /* number of index bits for current table */
    let mut drop: u32; /* code bits to drop for sub-table */
    let mut left: i32; /* number of prefix codes available */
    let mut used: u32; /* code entries in table used */
    let mut huff: u32; /* Huffman code */
    let mut incr: u32; /* for incrementing code, index */
    let mut fill: u32; /* index for replicating entries */
    let mut low: u32; /* low bits for current root entry */
    let mask: u32; /* mask for low root bits */
    let mut here: Code; /* table entry for duplication */
    let mut next: usize; /* next available space in table */
    let base: &[u16]; /* base value table to use */
    let extra: &[u16]; /* extra bits table to use */
    let match_: u32; /* use base and extra for symbol >= match */
    let mut count = [0u16; MAXBITS + 1]; /* number of codes of each length */
    let mut offs = [0u16; MAXBITS + 1]; /* offsets in table for each length */
    static LBASE: [u16; 31] = [
        /* Length codes 257..285 base */
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258, 0, 0,
    ];
    static LEXT: [u16; 31] = [
        /* Length codes 257..285 extra */
        16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 18, 18, 18, 18, 19, 19, 19, 19, 20, 20, 20,
        20, 21, 21, 21, 21, 16, 198, 203,
    ];
    static DBASE: [u16; 32] = [
        /* Distance codes 0..29 base */
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 0, 0,
    ];
    static DEXT: [u16; 32] = [
        /* Distance codes 0..29 extra */
        16, 16, 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21, 22, 22, 23, 23, 24, 24, 25, 25, 26,
        26, 27, 27, 28, 28, 29, 29, 64, 64,
    ];

    /*
      Process a set of code lengths to create a canonical Huffman code.  The
      code lengths are lens[0..codes-1].  Each length corresponds to the
      symbols 0..codes-1.  The Huffman code is generated by first sorting the
      symbols by length from short to long, and retaining the symbol order
      for codes with equal lengths.  Then the code starts with all zero bits
      for the first code of the shortest length, and the codes are integer
      increments for the same length, and zeros are appended as the length
      increases.  For the deflate format, these bits are stored backwards
      from their more natural integer increment ordering, and so when the
      decoding tables are built in the large loop below, the integer codes
      are incremented backwards.

      This routine assumes, but does not check, that all of the entries in
      lens[] are in the range 0..MAXBITS.  The caller must assure this.
      1..MAXBITS is interpreted as that code length.  zero means that that
      symbol does not occur in this code.

      The codes are sorted by computing a count of codes for each length,
      creating from that a table of starting indices for each length in the
      sorted table, and then entering the symbols in order in the sorted
      table.  The sorted table is work[], with that space being provided by
      the caller.

      The length counts are used for other purposes as well, i.e. finding
      the minimum and maximum length codes, determining if there are any
      codes at all, checking for a valid set of lengths, and looking ahead
      at length counts to determine sub-table sizes when building the
      decoding tables.
    */

    /* accumulate lengths for codes (assumes lens[] all in 0..MAXBITS) */
    for sym in 0..codes as usize {
        count[lens[sym] as usize] += 1;
    }

    /* bound code lengths, force root to be within code lengths */
    root = *bits;
    max = MAXBITS as u32;
    while max >= 1 {
        if count[max as usize] != 0 {
            break;
        }
        max -= 1;
    }
    if root > max {
        root = max;
    }
    if max == 0 {
        /* no symbols to code at all */
        here = Code {
            op: 64,
            bits: 1,
            val: 0,
        }; /* invalid code marker */
        tables[*next_index] = here; /* make a table to force an error */
        tables[*next_index + 1] = here;
        *next_index += 2;
        *bits = 1;
        return 0; /* no symbols, but wait for decoding to report error */
    }
    min = 1;
    while min < max {
        if count[min as usize] != 0 {
            break;
        }
        min += 1;
    }
    if root < min {
        root = min;
    }

    /* check for an over-subscribed or incomplete set of lengths */
    left = 1;
    for len in 1..=MAXBITS {
        left <<= 1;
        left -= count[len] as i32;
        if left < 0 {
            return -1; /* over-subscribed */
        }
    }
    if left > 0 && (type_ == CodeType::Codes || max != 1) {
        return -1; /* incomplete set */
    }

    /* generate offsets into symbol table for each length for sorting */
    offs[1] = 0;
    for len in 1..MAXBITS {
        offs[len + 1] = offs[len] + count[len];
    }

    /* sort symbols by length, by symbol order within each length */
    for sym in 0..codes as usize {
        if lens[sym] != 0 {
            let l = lens[sym] as usize;
            work[offs[l] as usize] = sym as u16;
            offs[l] += 1;
        }
    }

    /*
      Create and fill in decoding tables.  In this loop, the table being
      filled is at next and has curr index bits.  The code being used is huff
      with length len.  That code is converted to an index by dropping drop
      bits off of the bottom.  For codes where len is less than drop + curr,
      those top drop + curr - len bits are incremented through all values to
      fill the table with replicated entries.

      root is the number of index bits for the root table.  When len exceeds
      root, sub-tables are created pointed to by the root entry with an index
      of the low root bits of huff.  This is saved in low to check for when a
      new sub-table should be started.  drop is zero when the root table is
      being filled, and drop is root when sub-tables are being filled.

      When a new sub-table is needed, it is necessary to look ahead in the
      code lengths to determine what size sub-table is needed.  The length
      counts are used for this, and so count[] is decremented as codes are
      entered in the tables.

      used keeps track of how many table entries have been allocated from the
      provided *table space.  It is checked for LENS and DIST tables against
      the constants ENOUGH_LENS and ENOUGH_DISTS to guard against changes in
      the initial root table size constants.  See the comments in inftrees.h
      for more information.

      sym increments through all symbols, and the loop terminates when
      all codes of length max, i.e. all codes, have been processed.  This
      routine permits incomplete codes, so another loop after this one fills
      in the rest of the decoding tables with invalid code markers.
    */

    /* set up for code type */
    match type_ {
        CodeType::Codes => {
            base = &[]; /* dummy value--not used */
            extra = &[];
            match_ = 20;
        }
        CodeType::Lens => {
            base = &LBASE;
            extra = &LEXT;
            match_ = 257;
        }
        CodeType::Dists => {
            /* DISTS */
            base = &DBASE;
            extra = &DEXT;
            match_ = 0;
        }
    }

    /* initialize state for loop */
    huff = 0; /* starting code */
    sym = 0; /* starting code symbol */
    len = min; /* starting code length */
    let table = *next_index; /* (the start of the root table) */
    next = table; /* current table to fill in */
    curr = root; /* current table index bits */
    drop = 0; /* current bits to drop from code for index */
    low = u32::MAX; /* trigger new sub-table when len > root */
    used = 1u32 << root; /* use root table entries */
    mask = used - 1; /* mask for comparing low */

    /* check available table space */
    if (type_ == CodeType::Lens && used > ENOUGH_LENS)
        || (type_ == CodeType::Dists && used > ENOUGH_DISTS)
    {
        return 1;
    }

    /* process all codes and make table entries */
    loop {
        /* create table entry */
        here = Code {
            op: 0,
            bits: (len - drop) as u8,
            val: 0,
        };
        let w = work[sym as usize] as u32;
        if w + 1 < match_ {
            here.op = 0;
            here.val = w as u16;
        } else if w >= match_ {
            here.op = extra[(w - match_) as usize] as u8;
            here.val = base[(w - match_) as usize];
        } else {
            here.op = 32 + 64; /* end of block */
            here.val = 0;
        }

        /* replicate for those indices with low len bits equal to huff */
        incr = 1u32 << (len - drop);
        fill = 1u32 << curr;
        min = fill; /* save offset to next table */
        loop {
            fill -= incr;
            tables[next + ((huff >> drop) + fill) as usize] = here;
            if fill == 0 {
                break;
            }
        }

        /* backwards increment the len-bit code huff */
        incr = 1u32 << (len - 1);
        while huff & incr != 0 {
            incr >>= 1;
        }
        if incr != 0 {
            huff &= incr - 1;
            huff += incr;
        } else {
            huff = 0;
        }

        /* go to next symbol, update count, len */
        sym += 1;
        count[len as usize] -= 1;
        if count[len as usize] == 0 {
            if len == max {
                break;
            }
            len = lens[work[sym as usize] as usize] as u32;
        }

        /* create new sub-table if needed */
        if len > root && (huff & mask) != low {
            /* if first time, transition to sub-tables */
            if drop == 0 {
                drop = root;
            }

            /* increment past last table */
            next += min as usize; /* here min is 1 << curr */

            /* determine length of next table */
            curr = len - drop;
            left = 1 << curr;
            while curr + drop < max {
                left -= count[(curr + drop) as usize] as i32;
                if left <= 0 {
                    break;
                }
                curr += 1;
                left <<= 1;
            }

            /* check for enough space */
            used += 1u32 << curr;
            if (type_ == CodeType::Lens && used > ENOUGH_LENS)
                || (type_ == CodeType::Dists && used > ENOUGH_DISTS)
            {
                return 1;
            }

            /* point entry in root table to sub-table */
            low = huff & mask;
            tables[table + low as usize] = Code {
                op: curr as u8,
                bits: root as u8,
                val: (next - table) as u16,
            };
        }
    }

    /* fill in remaining table entry if code is incomplete (guaranteed to have
    at most one remaining entry, since if the code is incomplete, the
    maximum code length that was allowed to get this far is one bit) */
    if huff != 0 {
        here = Code {
            op: 64,
            bits: (len - drop) as u8,
            val: 0,
        }; /* invalid code marker */
        tables[next + huff as usize] = here;
    }

    /* set return parameters */
    *next_index += used as usize;
    *bits = root;
    0
}

/*************************************************************************
 * inflate.h
 */

/// `inflate_mode`: possible inflate modes between inflate() calls
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum InflateMode {
    Head = 16180, /* i: waiting for magic header */
    Flags,        /* i: waiting for method and flags (gzip) */
    Time,         /* i: waiting for modification time (gzip) */
    Os,           /* i: waiting for extra flags and operating system (gzip) */
    Exlen,        /* i: waiting for extra length (gzip) */
    Extra,        /* i: waiting for extra bytes (gzip) */
    Name,         /* i: waiting for end of file name (gzip) */
    Comment,      /* i: waiting for end of comment (gzip) */
    Hcrc,         /* i: waiting for header crc (gzip) */
    Dictid,       /* i: waiting for dictionary check value */
    Dict,         /* waiting for inflateSetDictionary() call */
    Type,         /* i: waiting for type bits, including last-flag bit */
    Typedo,       /* i: same, but skip check to exit inflate on new block */
    Stored,       /* i: waiting for stored size (length and complement) */
    Copy_,        /* i/o: same as COPY below, but only first time in */
    Copy,         /* i/o: waiting for input or output to copy stored block */
    Table,        /* i: waiting for dynamic block table lengths */
    Lenlens,      /* i: waiting for code length code lengths */
    Codelens,     /* i: waiting for length/lit and distance code lengths */
    Len_,         /* i: same as LEN below, but only first time in */
    Len,          /* i: waiting for length/lit/eob code */
    Lenext,       /* i: waiting for length extra bits */
    Dist,         /* i: waiting for distance code */
    Distext,      /* i: waiting for distance extra bits */
    Match,        /* o: waiting for output space to copy string */
    Lit,          /* o: waiting for output space to write literal */
    Check,        /* i: waiting for 32-bit check value */
    Length,       /* i: waiting for 32-bit length (gzip) */
    Done,         /* finished check, done -- remain here until reset */
    Bad,          /* got a data error -- remain here until reset */
    Mem,          /* got an inflate() memory error -- remain here until reset */
    Sync,         /* looking for synchronization bytes to restart inflate() */
}

/*
   State transitions between above modes -

   (most modes can go to BAD or MEM on error -- not shown for clarity)

   Process header:
       HEAD -> (gzip) or (zlib) or (raw)
       (gzip) -> FLAGS -> TIME -> OS -> EXLEN -> EXTRA -> NAME -> COMMENT ->
                 HCRC -> TYPE
       (zlib) -> DICTID or TYPE
       DICTID -> DICT -> TYPE
       (raw) -> TYPEDO
   Read deflate blocks:
           TYPE -> TYPEDO -> STORED or TABLE or LEN_ or CHECK
           STORED -> COPY_ -> COPY -> TYPE
           TABLE -> LENLENS -> CODELENS -> LEN_
           LEN_ -> LEN
   Read deflate codes in fixed or dynamic block:
               LEN -> LENEXT or LIT or TYPE
               LENEXT -> DIST -> DISTEXT -> MATCH -> LEN
               LIT -> LEN
   Process trailer:
       CHECK -> LENGTH -> DONE
*/

/// Where a decoding table starts (C's `code const FAR *`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodePtr {
    /// `state->codes + n`
    Codes(usize),
    /// `lenfix`
    LenFix,
    /// `distfix`
    DistFix,
}

/// `struct inflate_state`: state maintained between inflate() calls --
/// approximately 7K bytes, not including the allocated sliding window,
/// which is up to 32K bytes.
#[derive(Debug)]
struct InflateState {
    mode: InflateMode, /* current inflate mode */
    last: i32,         /* true if processing last block */
    wrap: i32,         /* bit 0 true for zlib, bit 1 true for gzip,
                       bit 2 true to validate check value */
    havedict: i32, /* true if dictionary provided */
    flags: i32,    /* gzip header method and flags, 0 if zlib, or
                   -1 if raw or no header yet */
    dmax: u32,  /* zlib header max distance (INFLATE_STRICT) */
    check: u64, /* protected copy of check value */
    total: u64, /* protected copy of output count */
    /* (`head`: no gzip header is ever requested) */
    /* sliding window */
    wbits: u32,              /* log base 2 of requested window size */
    wsize: u32,              /* window size or zero if not using window */
    whave: u32,              /* valid bytes in the window */
    wnext: u32,              /* window write index */
    window: Option<Vec<u8>>, /* allocated sliding window, if needed */
    /* bit accumulator */
    hold: u64, /* input bit accumulator */
    bits: u32, /* number of bits in "in" */
    /* for string and stored block copying */
    length: u32, /* literal or length of data to copy */
    offset: u32, /* distance back to copy string from */
    /* for table and code decoding */
    extra: u32, /* extra bits needed */
    /* fixed and dynamic code tables */
    lencode: CodePtr,  /* starting table for length/literal codes */
    distcode: CodePtr, /* starting table for distance codes */
    lenbits: u32,      /* index bits for lencode */
    distbits: u32,     /* index bits for distcode */
    /* dynamic table building */
    ncode: u32,            /* number of code length code lengths */
    nlen: u32,             /* number of length code lengths */
    ndist: u32,            /* number of distance code lengths */
    have: u32,             /* number of code lengths in lens[] */
    next: usize,           /* next available space in codes[] */
    lens: [u16; 320],      /* temporary storage for code lengths */
    work: [u16; 288],      /* work area for code table building */
    codes: [Code; ENOUGH], /* space for code tables */
    sane: i32,             /* if false, allow invalid distance too far */
    back: i32,             /* bits back of last unprocessed length/lit */
    was: u32,              /* initial length of match */
}

impl InflateState {
    /// `code` lookup through a table pointer
    fn code(&self, table: CodePtr, index: usize) -> Code {
        match table {
            CodePtr::Codes(n) => self.codes[n + index],
            CodePtr::LenFix => fixed_tables().0[index],
            CodePtr::DistFix => fixed_tables().1[index],
        }
    }
}

/*************************************************************************
 * inflate.c
 */

/// `inflateStateCheck`
fn inflate_state_check(strm: &ZStream) -> bool {
    match &strm.state {
        None => true,
        Some(state) => state.mode < InflateMode::Head || state.mode > InflateMode::Sync,
    }
}

/// `inflateResetKeep`
fn inflate_reset_keep(strm: &mut ZStream) -> i32 {
    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    let state = strm.state.as_mut().unwrap();
    strm.total_in = 0;
    strm.total_out = 0;
    state.total = 0;
    strm.msg = None;
    if state.wrap != 0 {
        /* to support ill-conceived Java test suite */
        strm.adler = (state.wrap & 1) as u64;
    }
    state.mode = InflateMode::Head;
    state.last = 0;
    state.havedict = 0;
    state.flags = -1;
    state.dmax = 32768;
    state.hold = 0;
    state.bits = 0;
    state.lencode = CodePtr::Codes(0);
    state.distcode = CodePtr::Codes(0);
    state.next = 0;
    state.sane = 1;
    state.back = -1;
    Z_OK
}

/// `inflateReset`
pub fn inflate_reset(strm: &mut ZStream) -> i32 {
    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    let state = strm.state.as_mut().unwrap();
    state.wsize = 0;
    state.whave = 0;
    state.wnext = 0;
    inflate_reset_keep(strm)
}

/// `inflateReset2`
fn inflate_reset2(strm: &mut ZStream, mut window_bits: i32) -> i32 {
    let wrap: i32;

    /* get the state */
    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    let state = strm.state.as_mut().unwrap();

    /* extract wrap request from windowBits parameter */
    if window_bits < 0 {
        if window_bits < -15 {
            return Z_STREAM_ERROR;
        }
        wrap = 0;
        window_bits = -window_bits;
    } else {
        wrap = (window_bits >> 4) + 5;
        /* GUNZIP */
        if window_bits < 48 {
            window_bits &= 15;
        }
    }

    /* set number of window bits, free window if different */
    if window_bits != 0 && !(8..=15).contains(&window_bits) {
        return Z_STREAM_ERROR;
    }
    if state.window.is_some() && state.wbits != window_bits as u32 {
        state.window = None;
    }

    /* update state and reset the rest of it */
    state.wrap = wrap;
    state.wbits = window_bits as u32;
    inflate_reset(strm)
}

/// `inflateInit2_` (via the `inflateInit2` macro)
pub fn inflate_init2(strm: &mut ZStream, window_bits: i32) -> i32 {
    strm.msg = None; /* in case we return an error */
    let state = match try_box_state() {
        Some(s) => s,
        None => return Z_MEM_ERROR,
    };
    strm.state = Some(state);
    /* (state->window = Z_NULL; state->mode = HEAD: see try_box_state) */
    let ret = inflate_reset2(strm, window_bits);
    if ret != Z_OK {
        strm.state = None;
    }
    ret
}

/// `ZALLOC(strm, 1, sizeof(struct inflate_state))`
fn try_box_state() -> Option<Box<InflateState>> {
    let mut v: Vec<InflateState> = Vec::new();
    if v.try_reserve_exact(1).is_err() {
        return None;
    }
    v.push(InflateState {
        mode: InflateMode::Head, /* to pass state test in inflateReset2() */
        last: 0,
        wrap: 0,
        havedict: 0,
        flags: 0,
        dmax: 0,
        check: 0,
        total: 0,
        wbits: 0,
        wsize: 0,
        whave: 0,
        wnext: 0,
        window: None,
        hold: 0,
        bits: 0,
        length: 0,
        offset: 0,
        extra: 0,
        lencode: CodePtr::Codes(0),
        distcode: CodePtr::Codes(0),
        lenbits: 0,
        distbits: 0,
        ncode: 0,
        nlen: 0,
        ndist: 0,
        have: 0,
        next: 0,
        lens: [0; 320],
        work: [0; 288],
        codes: [Code::default(); ENOUGH],
        sane: 0,
        back: 0,
        was: 0,
    });
    v.pop().map(Box::new)
}

/// The fixed tables (`lenfix`, `distfix`), built once (`BUILDFIXED`).
fn fixed_tables() -> &'static (Vec<Code>, Vec<Code>) {
    static FIXED: OnceLock<(Vec<Code>, Vec<Code>)> = OnceLock::new();
    FIXED.get_or_init(|| {
        let mut lens = [0u16; 320];
        let mut work = [0u16; 288];
        let mut fixed = vec![Code::default(); 544];
        let mut next = 0usize;

        /* literal/length table */
        let mut sym = 0usize;
        while sym < 144 {
            lens[sym] = 8;
            sym += 1;
        }
        while sym < 256 {
            lens[sym] = 9;
            sym += 1;
        }
        while sym < 280 {
            lens[sym] = 7;
            sym += 1;
        }
        while sym < 288 {
            lens[sym] = 8;
            sym += 1;
        }
        let lenfix = next;
        let mut bits = 9;
        inflate_table(
            CodeType::Lens,
            &lens,
            288,
            &mut fixed,
            &mut next,
            &mut bits,
            &mut work,
        );

        /* distance table */
        for l in lens.iter_mut().take(32) {
            *l = 5;
        }
        let distfix = next;
        bits = 5;
        inflate_table(
            CodeType::Dists,
            &lens,
            32,
            &mut fixed,
            &mut next,
            &mut bits,
            &mut work,
        );

        (
            fixed[lenfix..distfix].to_vec(),
            fixed[distfix..next].to_vec(),
        )
    })
}

/*
  Return state with length and distance decoding tables and index sizes set to
  fixed code decoding.  Normally this returns fixed tables from inffixed.h.
  If BUILDFIXED is defined, then instead this routine builds the tables the
  first time it's called, and returns those tables the first time and
  thereafter.  This reduces the size of the code by about 2K bytes, in
  exchange for a little execution time.  However, BUILDFIXED should not be
  used for threaded applications, since the rewriting of the tables and virgin
  may not be thread-safe.
*/
fn fixedtables(state: &mut InflateState) {
    state.lencode = CodePtr::LenFix;
    state.lenbits = 9;
    state.distcode = CodePtr::DistFix;
    state.distbits = 5;
}

/*
  Update the window with the last wsize (normally 32K) bytes written before
  returning.  If window does not exist yet, create it.  This is only called
  when a window is already in use, or when output has been written during this
  inflate call, but the end of the deflate stream has not been reached yet.
  It is also called to create a window for dictionary data when a dictionary
  is loaded.

  Providing output buffers larger than 32K to inflate() should provide a speed
  advantage, since only the last 32K of output is copied to the sliding window
  upon return from inflate(), and since all distances after the first 32K of
  output will fall in the output data, making match copies simpler and faster.
  The advantage may be dependent on the size of the processor's data caches.
*/
fn updatewindow(state: &mut InflateState, output: &[u8], end: usize, mut copy: u32) -> i32 {
    /* if it hasn't been done already, allocate space for the window */
    if state.window.is_none() {
        let mut w = Vec::new();
        if w.try_reserve_exact(1usize << state.wbits).is_err() {
            return 1;
        }
        w.resize(1usize << state.wbits, 0);
        state.window = Some(w);
    }

    /* if window not in use yet, initialize */
    if state.wsize == 0 {
        state.wsize = 1u32 << state.wbits;
        state.wnext = 0;
        state.whave = 0;
    }

    let wsize = state.wsize as usize;
    let window = state.window.as_mut().unwrap();

    /* copy state->wsize or less output bytes into the circular window */
    if copy >= state.wsize {
        window[..wsize].copy_from_slice(&output[end - wsize..end]);
        state.wnext = 0;
        state.whave = state.wsize;
    } else {
        let mut dist = state.wsize - state.wnext;
        if dist > copy {
            dist = copy;
        }
        let wn = state.wnext as usize;
        let c = copy as usize;
        window[wn..wn + dist as usize].copy_from_slice(&output[end - c..end - c + dist as usize]);
        copy -= dist;
        if copy != 0 {
            let c = copy as usize;
            window[..c].copy_from_slice(&output[end - c..end]);
            state.wnext = copy;
            state.whave = state.wsize;
        } else {
            state.wnext += dist;
            if state.wnext == state.wsize {
                state.wnext = 0;
            }
            if state.whave < state.wsize {
                state.whave += dist;
            }
        }
    }
    0
}

/* Macros for inflate(): */

/* check function to use adler32() for zlib or crc32() for gzip */
fn update_check(state: &InflateState, check: u64, buf: &[u8]) -> u64 {
    if state.flags != 0 {
        crc32(check, buf)
    } else {
        adler32(check, buf)
    }
}

/* check macros for header crc */
fn crc2(check: u64, word: u64) -> u64 {
    let hbuf = [word as u8, (word >> 8) as u8];
    crc32(check, &hbuf)
}

fn crc4(check: u64, word: u64) -> u64 {
    let hbuf = [
        word as u8,
        (word >> 8) as u8,
        (word >> 16) as u8,
        (word >> 24) as u8,
    ];
    crc32(check, &hbuf)
}

/// `ZSWAP32`
fn zswap32(q: u64) -> u64 {
    ((q >> 24) & 0xff) + ((q >> 8) & 0xff00) + ((q & 0xff00) << 8) + ((q & 0xff) << 24)
}

/*
  inflate() uses a state machine to process as much input data and generate as
  much output data as possible before returning.  The state machine is
  structured roughly as follows:

   for (;;) switch (state) {
   ...
   case STATEn:
       if (not enough input data or output space to make progress)
           return;
       ... make progress ...
       state = STATEm;
       break;
   ...
   }

  so when inflate() is called again, the same case is attempted again, and
  if the appropriate resources are provided, the machine proceeds to the
  next state.  The NEEDBITS() macro is usually the way the state evaluates
  whether it can proceed or should return.  NEEDBITS() does the return if
  the requested bits are not available.  The typical use of the BITS macros
  is:

       NEEDBITS(n);
       ... do something with BITS(n) ...
       DROPBITS(n);

  where NEEDBITS(n) either returns from inflate() if there isn't enough
  input left to load n bits into the accumulator, or it continues.  BITS(n)
  gives the low n bits in the accumulator.  When done, DROPBITS(n) drops
  the low n bits off the accumulator.  INITBITS() clears the accumulator
  and sets the number of available bits to zero.  BYTEBITS() discards just
  enough bits to put the accumulator on a byte boundary.  After BYTEBITS()
  and a NEEDBITS(8), then BITS(8) would return the next byte in the stream.

  NEEDBITS(n) uses PULLBYTE() to get an available byte of input, or to return
  if there is no input available.  The decoding of variable length codes uses
  PULLBYTE() directly in order to pull just enough bytes to decode the next
  code, and no more.

  Some states loop until they get enough input, making sure that enough
  state information is maintained to continue the loop where it left off
  if NEEDBITS() returns in the loop.  For example, want, need, and keep
  would all have to actually be part of the saved state in case NEEDBITS()
  returns:

   case STATEw:
       while (want < need) {
           NEEDBITS(n);
           keep[want++] = BITS(n);
           DROPBITS(n);
       }
       state = STATEx;
   case STATEx:

  As shown above, if the next state is also the next case, then the break
  is omitted.

  A state may also return if there is not enough output space available to
  complete that state.  Those states are copying stored data, writing a
  literal byte, and copying a matching string.

  When returning, a "goto inf_leave" is used to update the total counters,
  update the check value, and determine whether any progress has been made
  during that inflate() call in order to return the proper return code.
  Progress is defined as a change in either strm->avail_in or strm->avail_out.
  When there is a window, goto inf_leave will update the window with the last
  output written.  If a goto inf_leave occurs in the middle of decompression
  and there is no window currently, goto inf_leave will create one and copy
  output to the window for the next call of inflate().

  In this implementation, the flush parameter of inflate() only affects the
  return code (per zlib.h).  inflate() always writes as much as possible to
  strm->next_out, given the space available and the provided input--the effect
  documented in zlib.h of Z_SYNC_FLUSH.  Furthermore, inflate() always defers
  the allocation of and copying into a sliding window until necessary, which
  provides the effect documented in zlib.h for Z_FINISH when the entire input
  stream available.  So the only thing the flush parameter actually does is:
  when flush is set to Z_FINISH, inflate() cannot return Z_OK.  Instead it
  will return Z_BUF_ERROR if it has not reached the end of the stream.
*/

/// `inflate`
pub fn inflate(strm: &mut ZStream, flush: i32) -> i32 {
    static ORDER: [u16; 19] = /* permutation of code lengths */
        [
            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
        ];

    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    let mut state = strm.state.take().unwrap();
    let ret = inflate_body(strm, &mut state, flush, &ORDER);
    strm.state = Some(state);
    ret
}

fn inflate_body(
    strm: &mut ZStream,
    state: &mut InflateState,
    flush: i32,
    order: &[u16; 19],
) -> i32 {
    use InflateMode::*;

    let input = strm.input;
    let mut next: usize; /* next input */
    let mut put: usize; /* next output */
    let mut have: u32; /* available input and output */
    let mut left: u32;
    let mut hold: u64; /* bit buffer */
    let mut bits: u32; /* bits in bit buffer */
    let mut in_: u32; /* save starting available input and output */
    let mut out: u32;
    let mut copy: u32; /* number of stored or match bytes to copy */
    let mut here: Code; /* current decoding table entry */
    let mut last: Code; /* parent table entry */
    let mut len: u32; /* length to copy for repeats, bits to drop */
    let mut ret: i32; /* return code */

    if state.mode == Type {
        state.mode = Typedo; /* skip check */
    }

    /* Load registers with state in inflate() for speed */
    macro_rules! load {
        () => {
            put = strm.next_out;
            left = strm.avail_out;
            next = strm.next_in;
            have = strm.avail_in;
            hold = state.hold;
            bits = state.bits;
        };
    }

    /* Restore state from registers in inflate() */
    macro_rules! restore {
        () => {
            strm.next_out = put;
            strm.avail_out = left;
            strm.next_in = next;
            strm.avail_in = have;
            state.hold = hold;
            state.bits = bits;
        };
    }

    /* Clear the input bit accumulator */
    macro_rules! initbits {
        () => {
            hold = 0;
            bits = 0;
        };
    }

    /* Get a byte of input into the bit accumulator, or return from inflate()
    if there is no input available. */
    macro_rules! pullbyte {
        ($leave:lifetime) => {
            if have == 0 {
                break $leave;
            }
            have -= 1;
            hold += (input[next] as u64) << bits;
            next += 1;
            bits += 8;
        };
    }

    /* Assure that there are at least n bits in the bit accumulator.  If there is
    not enough available input to do that, then return from inflate(). */
    macro_rules! needbits {
        ($leave:lifetime, $n:expr) => {
            while bits < ($n) as u32 {
                pullbyte!($leave);
            }
        };
    }

    /* Return the low n bits of the bit accumulator (n < 16) */
    macro_rules! bits_ {
        ($n:expr) => {
            ((hold as u32) & ((1u32 << ($n)) - 1))
        };
    }

    /* Remove n bits from the bit accumulator */
    macro_rules! dropbits {
        ($n:expr) => {
            hold >>= ($n);
            bits -= ($n) as u32;
        };
    }

    /* Remove zero to seven bits as needed to go to a byte boundary */
    macro_rules! bytebits {
        () => {
            hold >>= bits & 7;
            bits -= bits & 7;
        };
    }

    load!();
    in_ = have;
    out = left;
    ret = Z_OK;
    'inf_leave: loop {
        match state.mode {
            Head => {
                if state.wrap == 0 {
                    state.mode = Typedo;
                    continue;
                }
                needbits!('inf_leave, 16);
                /* GUNZIP */
                if (state.wrap & 2) != 0 && hold == 0x8b1f {
                    /* gzip header */
                    if state.wbits == 0 {
                        state.wbits = 15;
                    }
                    state.check = crc32(0, &[]);
                    state.check = crc2(state.check, hold);
                    initbits!();
                    state.mode = Flags;
                    continue;
                }
                if (state.wrap & 1) == 0 ||   /* check if zlib header allowed */
                    ((bits_!(8) << 8) as u64 + (hold >> 8)) % 31 != 0
                {
                    strm.msg = Some("incorrect header check");
                    state.mode = Bad;
                    continue;
                }
                if bits_!(4) != Z_DEFLATED {
                    strm.msg = Some("unknown compression method");
                    state.mode = Bad;
                    continue;
                }
                dropbits!(4);
                len = bits_!(4) + 8;
                if state.wbits == 0 {
                    state.wbits = len;
                }
                if len > 15 || len > state.wbits {
                    strm.msg = Some("invalid window size");
                    state.mode = Bad;
                    continue;
                }
                state.dmax = 1u32 << len;
                state.flags = 0; /* indicate zlib header */
                state.check = adler32(0, &[]);
                strm.adler = state.check;
                state.mode = if hold & 0x200 != 0 { Dictid } else { Type };
                initbits!();
            }
            /* GUNZIP */
            Flags => {
                needbits!('inf_leave, 16);
                state.flags = hold as i32;
                if (state.flags & 0xff) as u32 != Z_DEFLATED {
                    strm.msg = Some("unknown compression method");
                    state.mode = Bad;
                    continue;
                }
                if state.flags & 0xe000 != 0 {
                    strm.msg = Some("unknown header flags set");
                    state.mode = Bad;
                    continue;
                }
                if (state.flags & 0x0200) != 0 && (state.wrap & 4) != 0 {
                    state.check = crc2(state.check, hold);
                }
                initbits!();
                state.mode = Time;
                /* fallthrough */
            }
            Time => {
                needbits!('inf_leave, 32);
                if (state.flags & 0x0200) != 0 && (state.wrap & 4) != 0 {
                    state.check = crc4(state.check, hold);
                }
                initbits!();
                state.mode = Os;
                /* fallthrough */
            }
            Os => {
                needbits!('inf_leave, 16);
                if (state.flags & 0x0200) != 0 && (state.wrap & 4) != 0 {
                    state.check = crc2(state.check, hold);
                }
                initbits!();
                state.mode = Exlen;
                /* fallthrough */
            }
            Exlen => {
                if state.flags & 0x0400 != 0 {
                    needbits!('inf_leave, 16);
                    state.length = hold as u32;
                    if (state.flags & 0x0200) != 0 && (state.wrap & 4) != 0 {
                        state.check = crc2(state.check, hold);
                    }
                    initbits!();
                }
                state.mode = Extra;
                /* fallthrough */
            }
            Extra => {
                if state.flags & 0x0400 != 0 {
                    copy = state.length;
                    if copy > have {
                        copy = have;
                    }
                    if copy != 0 {
                        if (state.flags & 0x0200) != 0 && (state.wrap & 4) != 0 {
                            state.check = crc32(state.check, &input[next..next + copy as usize]);
                        }
                        have -= copy;
                        next += copy as usize;
                        state.length -= copy;
                    }
                    if state.length != 0 {
                        break 'inf_leave;
                    }
                }
                state.length = 0;
                state.mode = Name;
                /* fallthrough */
            }
            Name => {
                if state.flags & 0x0800 != 0 {
                    if have == 0 {
                        break 'inf_leave;
                    }
                    copy = 0;
                    loop {
                        len = input[next + copy as usize] as u32;
                        copy += 1;
                        if !(len != 0 && copy < have) {
                            break;
                        }
                    }
                    if (state.flags & 0x0200) != 0 && (state.wrap & 4) != 0 {
                        state.check = crc32(state.check, &input[next..next + copy as usize]);
                    }
                    have -= copy;
                    next += copy as usize;
                    if len != 0 {
                        break 'inf_leave;
                    }
                }
                state.length = 0;
                state.mode = Comment;
                /* fallthrough */
            }
            Comment => {
                if state.flags & 0x1000 != 0 {
                    if have == 0 {
                        break 'inf_leave;
                    }
                    copy = 0;
                    loop {
                        len = input[next + copy as usize] as u32;
                        copy += 1;
                        if !(len != 0 && copy < have) {
                            break;
                        }
                    }
                    if (state.flags & 0x0200) != 0 && (state.wrap & 4) != 0 {
                        state.check = crc32(state.check, &input[next..next + copy as usize]);
                    }
                    have -= copy;
                    next += copy as usize;
                    if len != 0 {
                        break 'inf_leave;
                    }
                }
                state.mode = Hcrc;
                /* fallthrough */
            }
            Hcrc => {
                if state.flags & 0x0200 != 0 {
                    needbits!('inf_leave, 16);
                    if (state.wrap & 4) != 0 && hold != (state.check & 0xffff) {
                        strm.msg = Some("header crc mismatch");
                        state.mode = Bad;
                        continue;
                    }
                    initbits!();
                }
                state.check = crc32(0, &[]);
                strm.adler = state.check;
                state.mode = Type;
            }
            Dictid => {
                needbits!('inf_leave, 32);
                state.check = zswap32(hold);
                strm.adler = state.check;
                initbits!();
                state.mode = Dict;
                /* fallthrough */
            }
            Dict => {
                if state.havedict == 0 {
                    restore!();
                    return Z_NEED_DICT;
                }
                state.check = adler32(0, &[]);
                strm.adler = state.check;
                state.mode = Type;
                /* fallthrough */
            }
            Type | Typedo => {
                if state.mode == Type && (flush == Z_BLOCK || flush == Z_TREES) {
                    break 'inf_leave;
                }
                /* fallthrough */
                /* case TYPEDO: */
                if state.last != 0 {
                    bytebits!();
                    state.mode = Check;
                    continue;
                }
                needbits!('inf_leave, 3);
                state.last = bits_!(1) as i32;
                dropbits!(1);
                match bits_!(2) {
                    0 => {
                        /* stored block */
                        state.mode = Stored;
                    }
                    1 => {
                        /* fixed block */
                        fixedtables(state);
                        state.mode = Len_; /* decode codes */
                        if flush == Z_TREES {
                            dropbits!(2);
                            break 'inf_leave;
                        }
                    }
                    2 => {
                        /* dynamic block */
                        state.mode = Table;
                    }
                    _ => {
                        strm.msg = Some("invalid block type");
                        state.mode = Bad;
                    }
                }
                dropbits!(2);
            }
            Stored => {
                bytebits!(); /* go to byte boundary */
                needbits!('inf_leave, 32);
                if (hold & 0xffff) != ((hold >> 16) ^ 0xffff) {
                    strm.msg = Some("invalid stored block lengths");
                    state.mode = Bad;
                    continue;
                }
                state.length = (hold as u32) & 0xffff;
                initbits!();
                state.mode = Copy_;
                if flush == Z_TREES {
                    break 'inf_leave;
                }
                /* fallthrough */
            }
            Copy_ => {
                state.mode = Copy;
                /* fallthrough */
            }
            Copy => {
                copy = state.length;
                if copy != 0 {
                    if copy > have {
                        copy = have;
                    }
                    if copy > left {
                        copy = left;
                    }
                    if copy == 0 {
                        break 'inf_leave;
                    }
                    let c = copy as usize;
                    strm.output[put..put + c].copy_from_slice(&input[next..next + c]);
                    have -= copy;
                    next += c;
                    left -= copy;
                    put += c;
                    state.length -= copy;
                    continue;
                }
                state.mode = Type;
            }
            Table => {
                needbits!('inf_leave, 14);
                state.nlen = bits_!(5) + 257;
                dropbits!(5);
                state.ndist = bits_!(5) + 1;
                dropbits!(5);
                state.ncode = bits_!(4) + 4;
                dropbits!(4);
                /* !PKZIP_BUG_WORKAROUND */
                if state.nlen > 286 || state.ndist > 30 {
                    strm.msg = Some("too many length or distance symbols");
                    state.mode = Bad;
                    continue;
                }
                state.have = 0;
                state.mode = Lenlens;
                /* fallthrough */
            }
            Lenlens => {
                while state.have < state.ncode {
                    needbits!('inf_leave, 3);
                    state.lens[order[state.have as usize] as usize] = bits_!(3) as u16;
                    state.have += 1;
                    dropbits!(3);
                }
                while state.have < 19 {
                    state.lens[order[state.have as usize] as usize] = 0;
                    state.have += 1;
                }
                state.next = 0;
                state.lencode = CodePtr::Codes(state.next);
                state.lenbits = 7;
                ret = {
                    let s = &mut *state;
                    inflate_table(
                        CodeType::Codes,
                        &s.lens,
                        19,
                        &mut s.codes,
                        &mut s.next,
                        &mut s.lenbits,
                        &mut s.work,
                    )
                };
                if ret != 0 {
                    strm.msg = Some("invalid code lengths set");
                    state.mode = Bad;
                    continue;
                }
                state.have = 0;
                state.mode = Codelens;
                /* fallthrough */
            }
            Codelens => {
                while state.have < state.nlen + state.ndist {
                    loop {
                        here = state.code(state.lencode, bits_!(state.lenbits) as usize);
                        if here.bits as u32 <= bits {
                            break;
                        }
                        pullbyte!('inf_leave);
                    }
                    if here.val < 16 {
                        dropbits!(here.bits);
                        state.lens[state.have as usize] = here.val;
                        state.have += 1;
                    } else {
                        if here.val == 16 {
                            needbits!('inf_leave, here.bits as u32 + 2);
                            dropbits!(here.bits);
                            if state.have == 0 {
                                strm.msg = Some("invalid bit length repeat");
                                state.mode = Bad;
                                break;
                            }
                            len = state.lens[state.have as usize - 1] as u32;
                            copy = 3 + bits_!(2);
                            dropbits!(2);
                        } else if here.val == 17 {
                            needbits!('inf_leave, here.bits as u32 + 3);
                            dropbits!(here.bits);
                            len = 0;
                            copy = 3 + bits_!(3);
                            dropbits!(3);
                        } else {
                            needbits!('inf_leave, here.bits as u32 + 7);
                            dropbits!(here.bits);
                            len = 0;
                            copy = 11 + bits_!(7);
                            dropbits!(7);
                        }
                        if state.have + copy > state.nlen + state.ndist {
                            strm.msg = Some("invalid bit length repeat");
                            state.mode = Bad;
                            break;
                        }
                        while copy != 0 {
                            copy -= 1;
                            state.lens[state.have as usize] = len as u16;
                            state.have += 1;
                        }
                    }
                }

                /* handle error breaks in while */
                if state.mode == Bad {
                    continue;
                }

                /* check for end-of-block code (better have one) */
                if state.lens[256] == 0 {
                    strm.msg = Some("invalid code -- missing end-of-block");
                    state.mode = Bad;
                    continue;
                }

                /* build code tables -- note: do not change the lenbits or distbits
                values here (9 and 6) without reading the comments in inftrees.h
                concerning the ENOUGH constants, which depend on those values */
                state.next = 0;
                state.lencode = CodePtr::Codes(state.next);
                state.lenbits = 9;
                ret = {
                    let s = &mut *state;
                    inflate_table(
                        CodeType::Lens,
                        &s.lens,
                        s.nlen,
                        &mut s.codes,
                        &mut s.next,
                        &mut s.lenbits,
                        &mut s.work,
                    )
                };
                if ret != 0 {
                    strm.msg = Some("invalid literal/lengths set");
                    state.mode = Bad;
                    continue;
                }
                state.distcode = CodePtr::Codes(state.next);
                state.distbits = 6;
                ret = {
                    let s = &mut *state;
                    let nlen = s.nlen as usize;
                    inflate_table(
                        CodeType::Dists,
                        &s.lens[nlen..],
                        s.ndist,
                        &mut s.codes,
                        &mut s.next,
                        &mut s.distbits,
                        &mut s.work,
                    )
                };
                if ret != 0 {
                    strm.msg = Some("invalid distances set");
                    state.mode = Bad;
                    continue;
                }
                state.mode = Len_;
                if flush == Z_TREES {
                    break 'inf_leave;
                }
                /* fallthrough */
            }
            Len_ => {
                state.mode = Len;
                /* fallthrough */
            }
            Len => {
                if have >= 6 && left >= 258 {
                    restore!();
                    inflate_fast(strm, state, out);
                    load!();
                    if state.mode == Type {
                        state.back = -1;
                    }
                    continue;
                }
                state.back = 0;
                loop {
                    here = state.code(state.lencode, bits_!(state.lenbits) as usize);
                    if here.bits as u32 <= bits {
                        break;
                    }
                    pullbyte!('inf_leave);
                }
                if here.op != 0 && (here.op & 0xf0) == 0 {
                    last = here;
                    loop {
                        here = state.code(
                            state.lencode,
                            last.val as usize
                                + (bits_!(last.bits as u32 + last.op as u32) >> last.bits) as usize,
                        );
                        if (last.bits as u32 + here.bits as u32) <= bits {
                            break;
                        }
                        pullbyte!('inf_leave);
                    }
                    dropbits!(last.bits);
                    state.back += last.bits as i32;
                }
                dropbits!(here.bits);
                state.back += here.bits as i32;
                state.length = here.val as u32;
                if here.op == 0 {
                    state.mode = Lit;
                    continue;
                }
                if here.op & 32 != 0 {
                    state.back = -1;
                    state.mode = Type;
                    continue;
                }
                if here.op & 64 != 0 {
                    strm.msg = Some("invalid literal/length code");
                    state.mode = Bad;
                    continue;
                }
                state.extra = (here.op as u32) & 15;
                state.mode = Lenext;
                /* fallthrough */
            }
            Lenext => {
                if state.extra != 0 {
                    needbits!('inf_leave, state.extra);
                    state.length += bits_!(state.extra);
                    dropbits!(state.extra);
                    state.back += state.extra as i32;
                }
                state.was = state.length;
                state.mode = Dist;
                /* fallthrough */
            }
            Dist => {
                loop {
                    here = state.code(state.distcode, bits_!(state.distbits) as usize);
                    if here.bits as u32 <= bits {
                        break;
                    }
                    pullbyte!('inf_leave);
                }
                if (here.op & 0xf0) == 0 {
                    last = here;
                    loop {
                        here = state.code(
                            state.distcode,
                            last.val as usize
                                + (bits_!(last.bits as u32 + last.op as u32) >> last.bits) as usize,
                        );
                        if (last.bits as u32 + here.bits as u32) <= bits {
                            break;
                        }
                        pullbyte!('inf_leave);
                    }
                    dropbits!(last.bits);
                    state.back += last.bits as i32;
                }
                dropbits!(here.bits);
                state.back += here.bits as i32;
                if here.op & 64 != 0 {
                    strm.msg = Some("invalid distance code");
                    state.mode = Bad;
                    continue;
                }
                state.offset = here.val as u32;
                state.extra = (here.op as u32) & 15;
                state.mode = Distext;
                /* fallthrough */
            }
            Distext => {
                if state.extra != 0 {
                    needbits!('inf_leave, state.extra);
                    state.offset += bits_!(state.extra);
                    dropbits!(state.extra);
                    state.back += state.extra as i32;
                }
                state.mode = Match;
                /* fallthrough */
            }
            Match => {
                if left == 0 {
                    break 'inf_leave;
                }
                copy = out - left;
                if state.offset > copy {
                    /* copy from window */
                    copy = state.offset - copy;
                    if copy > state.whave && state.sane != 0 {
                        strm.msg = Some("invalid distance too far back");
                        state.mode = Bad;
                        continue;
                    }
                    let from: usize;
                    if copy > state.wnext {
                        copy -= state.wnext;
                        from = (state.wsize - copy) as usize;
                    } else {
                        from = (state.wnext - copy) as usize;
                    }
                    if copy > state.length {
                        copy = state.length;
                    }
                    if copy > left {
                        copy = left;
                    }
                    left -= copy;
                    state.length -= copy;
                    let window = state.window.as_ref().unwrap();
                    for i in 0..copy as usize {
                        strm.output[put] = window[from + i];
                        put += 1;
                    }
                } else {
                    /* copy from output */
                    let mut from = put - state.offset as usize;
                    copy = state.length;
                    if copy > left {
                        copy = left;
                    }
                    left -= copy;
                    state.length -= copy;
                    loop {
                        strm.output[put] = strm.output[from];
                        put += 1;
                        from += 1;
                        copy -= 1;
                        if copy == 0 {
                            break;
                        }
                    }
                }
                if state.length == 0 {
                    state.mode = Len;
                }
            }
            Lit => {
                if left == 0 {
                    break 'inf_leave;
                }
                strm.output[put] = state.length as u8;
                put += 1;
                left -= 1;
                state.mode = Len;
            }
            Check => {
                if state.wrap != 0 {
                    needbits!('inf_leave, 32);
                    out -= left;
                    strm.total_out += out as u64;
                    state.total += out as u64;
                    if (state.wrap & 4) != 0 && out != 0 {
                        state.check =
                            update_check(state, state.check, &strm.output[put - out as usize..put]);
                        strm.adler = state.check;
                    }
                    out = left;
                    if (state.wrap & 4) != 0
                        && (if state.flags != 0 {
                            hold
                        } else {
                            zswap32(hold)
                        }) != state.check
                    {
                        strm.msg = Some("incorrect data check");
                        state.mode = Bad;
                        continue;
                    }
                    initbits!();
                }
                /* GUNZIP */
                state.mode = Length;
                /* fallthrough */
            }
            Length => {
                if state.wrap != 0 && state.flags != 0 {
                    needbits!('inf_leave, 32);
                    if (state.wrap & 4) != 0 && hold != (state.total & 0xffffffff) {
                        strm.msg = Some("incorrect length check");
                        state.mode = Bad;
                        continue;
                    }
                    initbits!();
                }
                state.mode = Done;
                /* fallthrough */
            }
            Done => {
                ret = Z_STREAM_END;
                break 'inf_leave;
            }
            Bad => {
                ret = Z_DATA_ERROR;
                break 'inf_leave;
            }
            Mem => return Z_MEM_ERROR,
            Sync => return Z_STREAM_ERROR,
        }
    }

    /*
      Return from inflate(), updating the total counts and the check value.
      If there was no progress during the inflate() call, return a buffer
      error.  Call updatewindow() to create and/or update the window state.
      Note: a memory error from inflate() is non-recoverable.
    */
    /* inf_leave: */
    restore!();
    if (state.wsize != 0
        || (out != strm.avail_out && state.mode < Bad && (state.mode < Check || flush != Z_FINISH)))
        && updatewindow(state, strm.output, strm.next_out, out - strm.avail_out) != 0
    {
        state.mode = Mem;
        return Z_MEM_ERROR;
    }
    in_ -= strm.avail_in;
    out -= strm.avail_out;
    strm.total_in += in_ as u64;
    strm.total_out += out as u64;
    state.total += out as u64;
    if (state.wrap & 4) != 0 && out != 0 {
        state.check = update_check(
            state,
            state.check,
            &strm.output[strm.next_out - out as usize..strm.next_out],
        );
        strm.adler = state.check;
    }
    strm.data_type = state.bits as i32
        + if state.last != 0 { 64 } else { 0 }
        + if state.mode == Type { 128 } else { 0 }
        + if state.mode == Len_ || state.mode == Copy_ {
            256
        } else {
            0
        };
    if ((in_ == 0 && out == 0) || flush == Z_FINISH) && ret == Z_OK {
        ret = Z_BUF_ERROR;
    }
    ret
}

/// `inflateEnd`
pub fn inflate_end(strm: &mut ZStream) -> i32 {
    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    strm.state = None;
    Z_OK
}

/*************************************************************************
 * inffast.c
 */

/*
  Decode literal, length, and distance codes and write out the resulting
  literal and match bytes until either not enough input or output is
  available, an end-of-block is encountered, or a data error is encountered.
  When large enough input and output buffers are supplied to inflate(), for
  example, a 16K input buffer and a 64K output buffer, more than 95% of the
  inflate execution time is spent in this routine.

  Entry assumptions:

       state->mode == LEN
       strm->avail_in >= 6
       strm->avail_out >= 258
       start >= strm->avail_out
       state->bits < 8

  On return, state->mode is one of:

       LEN -- ran out of enough output space or enough available input
       TYPE -- reached end of block code, inflate() to interpret next block
       BAD -- error in block data

  Notes:

   - The maximum input bits used by a length/distance pair is 15 bits for the
     length code, 5 bits for the length extra, 15 bits for the distance code,
     and 13 bits for the distance extra.  This totals 48 bits, or six bytes.
     Therefore if strm->avail_in >= 6, then there is enough input to avoid
     checking for available input while decoding.

   - The maximum bytes that a single length/distance pair can output is 258
     bytes, which is the maximum length that can be coded.  inflate_fast()
     requires strm->avail_out >= 258 for each loop to avoid checking for
     output space.
*/
fn inflate_fast(strm: &mut ZStream, state: &mut InflateState, start: u32) {
    let input = strm.input;
    let output = &mut *strm.output;

    /* copy state to local variables */
    let mut in_ = strm.next_in; /* local strm->next_in */
    let last = in_ + (strm.avail_in as usize - 5); /* have enough input while in < last */
    let mut out = strm.next_out; /* local strm->next_out */
    let beg = out - (start - strm.avail_out) as usize; /* inflate()'s initial strm->next_out */
    let end = out + (strm.avail_out as usize - 257); /* while out < end, enough space available */
    let wsize = state.wsize; /* window size or zero if not using window */
    let whave = state.whave; /* valid bytes in the window */
    let wnext = state.wnext; /* window write index */
    let mut hold = state.hold; /* local strm->hold */
    let mut bits = state.bits; /* local strm->bits */
    let lcode = state.lencode; /* local strm->lencode */
    let dcode = state.distcode; /* local strm->distcode */
    let lmask = (1u64 << state.lenbits) - 1; /* mask for first level of length codes */
    let dmask = (1u64 << state.distbits) - 1; /* mask for first level of distance codes */
    let mut here: Code; /* retrieved table entry */
    let mut op: u32; /* code bits, operation, extra bits, or */
    /*  window position, window bytes to copy */
    let mut len: u32; /* match length, unused bytes */
    let mut dist: u32; /* match distance */

    /* decode literals and length/distances until end-of-block or not enough
    input data or output space */
    'outer: loop {
        if bits < 15 {
            hold += (input[in_] as u64) << bits;
            in_ += 1;
            bits += 8;
            hold += (input[in_] as u64) << bits;
            in_ += 1;
            bits += 8;
        }
        here = state.code(lcode, (hold & lmask) as usize);
        /* dolen: */
        loop {
            op = here.bits as u32;
            hold >>= op;
            bits -= op;
            op = here.op as u32;
            if op == 0 {
                /* literal */
                output[out] = here.val as u8;
                out += 1;
            } else if op & 16 != 0 {
                /* length base */
                len = here.val as u32;
                op &= 15; /* number of extra bits */
                if op != 0 {
                    if bits < op {
                        hold += (input[in_] as u64) << bits;
                        in_ += 1;
                        bits += 8;
                    }
                    len += (hold as u32) & ((1u32 << op) - 1);
                    hold >>= op;
                    bits -= op;
                }
                if bits < 15 {
                    hold += (input[in_] as u64) << bits;
                    in_ += 1;
                    bits += 8;
                    hold += (input[in_] as u64) << bits;
                    in_ += 1;
                    bits += 8;
                }
                here = state.code(dcode, (hold & dmask) as usize);
                /* dodist: */
                loop {
                    op = here.bits as u32;
                    hold >>= op;
                    bits -= op;
                    op = here.op as u32;
                    if op & 16 != 0 {
                        /* distance base */
                        dist = here.val as u32;
                        op &= 15; /* number of extra bits */
                        if bits < op {
                            hold += (input[in_] as u64) << bits;
                            in_ += 1;
                            bits += 8;
                            if bits < op {
                                hold += (input[in_] as u64) << bits;
                                in_ += 1;
                                bits += 8;
                            }
                        }
                        dist += (hold as u32) & ((1u32 << op) - 1);
                        hold >>= op;
                        bits -= op;
                        op = (out - beg) as u32; /* max distance in output */
                        if dist > op {
                            /* see if copy from window */
                            op = dist - op; /* distance back in window */
                            if op > whave && state.sane != 0 {
                                strm.msg = Some("invalid distance too far back");
                                state.mode = InflateMode::Bad;
                                break 'outer;
                            }
                            let window = state.window.as_deref().unwrap_or(&[]);
                            let mut from: usize; /* where to copy match from */
                            let mut from_window = true;
                            if wnext == 0 {
                                /* very common case */
                                from = (wsize - op) as usize;
                                if op < len {
                                    /* some from window */
                                    len -= op;
                                    loop {
                                        output[out] = window[from];
                                        out += 1;
                                        from += 1;
                                        op -= 1;
                                        if op == 0 {
                                            break;
                                        }
                                    }
                                    from = out - dist as usize; /* rest from output */
                                    from_window = false;
                                }
                            } else if wnext < op {
                                /* wrap around window */
                                from = (wsize + wnext - op) as usize;
                                op -= wnext;
                                if op < len {
                                    /* some from end of window */
                                    len -= op;
                                    loop {
                                        output[out] = window[from];
                                        out += 1;
                                        from += 1;
                                        op -= 1;
                                        if op == 0 {
                                            break;
                                        }
                                    }
                                    from = 0;
                                    if wnext < len {
                                        /* some from start of window */
                                        op = wnext;
                                        len -= op;
                                        loop {
                                            output[out] = window[from];
                                            out += 1;
                                            from += 1;
                                            op -= 1;
                                            if op == 0 {
                                                break;
                                            }
                                        }
                                        from = out - dist as usize; /* rest from output */
                                        from_window = false;
                                    }
                                }
                            } else {
                                /* contiguous in window */
                                from = (wnext - op) as usize;
                                if op < len {
                                    /* some from window */
                                    len -= op;
                                    loop {
                                        output[out] = window[from];
                                        out += 1;
                                        from += 1;
                                        op -= 1;
                                        if op == 0 {
                                            break;
                                        }
                                    }
                                    from = out - dist as usize; /* rest from output */
                                    from_window = false;
                                }
                            }
                            while len > 2 {
                                for _ in 0..3 {
                                    output[out] = if from_window {
                                        window[from]
                                    } else {
                                        output[from]
                                    };
                                    out += 1;
                                    from += 1;
                                }
                                len -= 3;
                            }
                            if len != 0 {
                                output[out] = if from_window {
                                    window[from]
                                } else {
                                    output[from]
                                };
                                out += 1;
                                from += 1;
                                if len > 1 {
                                    output[out] = if from_window {
                                        window[from]
                                    } else {
                                        output[from]
                                    };
                                    out += 1;
                                }
                            }
                        } else {
                            let mut from = out - dist as usize; /* copy direct from output */
                            loop {
                                /* minimum length is three */
                                output[out] = output[from];
                                output[out + 1] = output[from + 1];
                                output[out + 2] = output[from + 2];
                                out += 3;
                                from += 3;
                                len -= 3;
                                if len <= 2 {
                                    break;
                                }
                            }
                            if len != 0 {
                                output[out] = output[from];
                                out += 1;
                                from += 1;
                                if len > 1 {
                                    output[out] = output[from];
                                    out += 1;
                                }
                            }
                        }
                        break;
                    } else if (op & 64) == 0 {
                        /* 2nd level distance code */
                        here = state.code(
                            dcode,
                            here.val as usize + (hold & ((1u64 << op) - 1)) as usize,
                        );
                        continue; /* goto dodist */
                    } else {
                        strm.msg = Some("invalid distance code");
                        state.mode = InflateMode::Bad;
                        break 'outer;
                    }
                }
            } else if (op & 64) == 0 {
                /* 2nd level length code */
                here = state.code(
                    lcode,
                    here.val as usize + (hold & ((1u64 << op) - 1)) as usize,
                );
                continue; /* goto dolen */
            } else if op & 32 != 0 {
                /* end-of-block */
                state.mode = InflateMode::Type;
                break 'outer;
            } else {
                strm.msg = Some("invalid literal/length code");
                state.mode = InflateMode::Bad;
                break 'outer;
            }
            break;
        }
        if !(in_ < last && out < end) {
            break;
        }
    }

    /* return unused bytes (on entry, bits < 8, so in won't go too far back) */
    len = bits >> 3;
    in_ -= len as usize;
    bits -= len << 3;
    hold &= (1u64 << bits) - 1;

    /* update state and return */
    strm.next_in = in_;
    strm.next_out = out;
    strm.avail_in = if in_ < last {
        5 + (last - in_) as u32
    } else {
        5 - (in_ - last) as u32
    };
    strm.avail_out = if out < end {
        257 + (end - out) as u32
    } else {
        257 - (out - end) as u32
    };
    state.hold = hold;
    state.bits = bits;
}

/*
  inflate_fast() speedups that turned out slower (on a PowerPC G3 750CXe):
  - Using bit fields for code structure
  - Different op definition to avoid & for extra bits (do & for table bits)
  - Three separate decoding do-loops for direct, window, and wnext == 0
  - Special case for distance > 1 copies to do overlapped load and store copy
  - Explicit branch predictions (based on measured branch probabilities)
  - Deferring match copy and interspersed it with decoding subsequent codes
  - Swapping literal/length else
  - Swapping window/direct else
  - Larger unrolled copy loops (three is about right)
  - Moving len -= 3 statement into middle of loop
*/

/*************************************************************************
 * adler32.c
 */

const BASE: u64 = 65521; /* largest prime smaller than 65536 */
const NMAX: usize = 5552;
/* NMAX is the largest n such that 255n(n+1)/2 + (n+1)(BASE-1) <= 2^32-1 */

/// `adler32_z` (and `adler32`)
pub fn adler32(mut adler: u64, buf: &[u8]) -> u64 {
    /* split Adler-32 into component sums */
    let mut sum2 = (adler >> 16) & 0xffff;
    adler &= 0xffff;

    let mut len = buf.len();

    /* in case user likes doing a byte at a time, keep it fast */
    if len == 1 {
        adler += buf[0] as u64;
        if adler >= BASE {
            adler -= BASE;
        }
        sum2 += adler;
        if sum2 >= BASE {
            sum2 -= BASE;
        }
        return adler | (sum2 << 16);
    }

    /* initial Adler-32 value (deferred check for len == 1 speed) */
    if buf.is_empty() {
        /* (C tests for a NULL `buf'; all empty-buffer calls pass NULL) */
        return 1;
    }

    let mut p = 0usize;

    /* in case short lengths are provided, keep it somewhat fast */
    if len < 16 {
        while len > 0 {
            len -= 1;
            adler += buf[p] as u64;
            p += 1;
            sum2 += adler;
        }
        if adler >= BASE {
            adler -= BASE;
        }
        sum2 %= BASE; /* only added so many BASE's */
        return adler | (sum2 << 16);
    }

    /* do length NMAX blocks -- requires just one modulo operation */
    while len >= NMAX {
        len -= NMAX;
        for &b in &buf[p..p + NMAX] {
            adler += b as u64;
            sum2 += adler;
        }
        p += NMAX;
        adler %= BASE;
        sum2 %= BASE;
    }

    /* do remaining bytes (less than NMAX, still just one modulo) */
    if len != 0 {
        /* avoid modulos if none remaining */
        for &b in &buf[p..p + len] {
            adler += b as u64;
            sum2 += adler;
        }
        adler %= BASE;
        sum2 %= BASE;
    }

    /* return recombined sums */
    adler | (sum2 << 16)
}

/*************************************************************************
 * crc32.c
 */

/// The CRC-32 table (`crc_table`, polynomial 0xedb88320, reflected).
const fn make_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut p = i as u32;
        let mut k = 0;
        while k < 8 {
            p = if p & 1 != 0 {
                (p >> 1) ^ 0xedb88320
            } else {
                p >> 1
            };
            k += 1;
        }
        table[i] = p;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = make_crc_table();

/// `crc32_z` (and `crc32`)
pub fn crc32(crc: u64, buf: &[u8]) -> u64 {
    /* Return initial CRC, if requested. */
    if buf.is_empty() {
        /* (C tests for a NULL `buf'; all empty-buffer calls pass NULL) */
        return 0;
    }

    /* Pre-condition the CRC */
    let mut crc = (crc as u32) ^ 0xffffffff;

    /* Complete the computation of the CRC on any remaining bytes. */
    for &b in buf {
        crc = (crc >> 8) ^ CRC_TABLE[((crc ^ b as u32) & 0xff) as usize];
    }

    /* Return the CRC, post-conditioned. */
    (crc ^ 0xffffffff) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        assert_eq!(adler32(1, b"Wikipedia"), 0x11E60398);
        assert_eq!(crc32(0, b"123456789"), 0xCBF43926);
    }

    #[test]
    fn inflate_zlib_stream() {
        /* zlib.compress(b"hello hello hello hello") */
        let data = [
            0x78, 0x9c, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01, 0x68, 0x03,
            0x08, 0xb1,
        ];
        let mut out = [0u8; 64];
        let mut strm = ZStream::new(&data, &mut out);
        assert_eq!(inflate_init2(&mut strm, MAX_WBITS | 32), Z_OK);
        assert_eq!(inflate(&mut strm, Z_FINISH), Z_STREAM_END);
        let n = strm.total_out as usize;
        assert_eq!(inflate_end(&mut strm), Z_OK);
        assert_eq!(&out[..n], b"hello hello hello hello");
    }

    #[test]
    fn inflate_gzip_stream() {
        /* gzip.compress(b"<svg>abc abc abc</svg>", mtime=0) */
        let data = [
            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0xb3, 0x29, 0x2e, 0x4b,
            0xb7, 0x4b, 0x4c, 0x4a, 0x56, 0x80, 0x62, 0x1b, 0x7d, 0x90, 0x00, 0x00, 0x30, 0x84,
            0x83, 0xeb, 0x16, 0x00, 0x00, 0x00,
        ];
        let mut out = [0u8; 22];
        let mut strm = ZStream::new(&data, &mut out);
        assert_eq!(inflate_init2(&mut strm, MAX_WBITS | 32), Z_OK);
        assert_eq!(inflate(&mut strm, Z_FINISH), Z_STREAM_END);
        assert_eq!(strm.total_out, 22);
        assert_eq!(inflate_end(&mut strm), Z_OK);
        assert_eq!(&out[..], b"<svg>abc abc abc</svg>");

        /* a corrupted check value */
        let mut bad = data;
        bad[27] ^= 1;
        let mut out = [0u8; 22];
        let mut strm = ZStream::new(&bad, &mut out);
        assert_eq!(inflate_init2(&mut strm, MAX_WBITS | 32), Z_OK);
        assert_eq!(inflate(&mut strm, Z_FINISH), Z_DATA_ERROR);
        assert_eq!(strm.msg, Some("incorrect data check"));
    }
}
