// Rust translation of inftrees.h and inftrees.c from zlib 1.3.1.
// Copyright (C) 1995-2024 Mark Adler
// This is an altered (translated) version of the original software; zlib is
// used under the zlib license (see zlib.h and LICENSE.txt).

//! inftrees.c -- generate Huffman trees for efficient decoding

/* inftrees.h -- header to use inftrees.c */

/// `code`: structure for decoding tables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Code {
    pub(crate) op: u8,   /* operation, extra bits, table bits */
    pub(crate) bits: u8, /* bits in this part of the code */
    pub(crate) val: u16, /* offset in table or code value */
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
returns 852, and "enough 30 6 15" for distance codes returns 592. The
initial root table size (9 or 6) is found in the fifth argument of the
inflate_table() calls in inflate.c and infback.c.  If the root table size is
changed, then these maximum sizes would be need to be recalculated and
updated. */
pub(crate) const ENOUGH_LENS: u32 = 852;
pub(crate) const ENOUGH_DISTS: u32 = 592;
pub(crate) const ENOUGH: usize = (ENOUGH_LENS + ENOUGH_DISTS) as usize;

/// `codetype`: type of code to build for inflate_table()
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeType {
    Codes,
    Lens,
    Dists,
}

/* inftrees.c -- generate Huffman trees for efficient decoding */

const MAXBITS: usize = 15;

#[allow(dead_code)]
pub(crate) static INFLATE_COPYRIGHT: &str = " inflate 1.3.1 Copyright 1995-2024 Mark Adler ";
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
pub(crate) fn inflate_table(
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
        20, 21, 21, 21, 21, 16, 203, 77,
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
