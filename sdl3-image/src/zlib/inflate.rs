// Rust translation of inflate.h and inflate.c from zlib 1.3.1.
// Copyright (C) 1995-2022 Mark Adler
// This is an altered (translated) version of the original software; zlib is
// used under the zlib license (see zlib.h and LICENSE.txt).

//! inflate.c -- zlib decompression

use std::sync::OnceLock;

use super::inffast::inflate_fast;
use super::inftrees::{inflate_table, Code, CodeType, ENOUGH};
use super::{
    adler32, crc32, ZStream, Z_BLOCK, Z_BUF_ERROR, Z_DATA_ERROR, Z_DEFLATED, Z_FINISH, Z_MEM_ERROR,
    Z_NEED_DICT, Z_OK, Z_STREAM_END, Z_STREAM_ERROR, Z_TREES,
};

/* inflate.h -- internal inflate state definition */

/// `inflate_mode`: possible inflate modes between inflate() calls
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum InflateMode {
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
pub(crate) enum CodePtr {
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
pub(crate) struct InflateState {
    pub(crate) mode: InflateMode, /* current inflate mode */
    pub(crate) last: i32,         /* true if processing last block */
    pub(crate) wrap: i32,         /* bit 0 true for zlib, bit 1 true for gzip,
                                  bit 2 true to validate check value */
    pub(crate) havedict: i32, /* true if dictionary provided */
    pub(crate) flags: i32,    /* gzip header method and flags, 0 if zlib, or
                              -1 if raw or no header yet */
    pub(crate) dmax: u32,  /* zlib header max distance (INFLATE_STRICT) */
    pub(crate) check: u64, /* protected copy of check value */
    pub(crate) total: u64, /* protected copy of output count */
    /* (`head`: no gzip header is ever requested) */
    /* sliding window */
    pub(crate) wbits: u32, /* log base 2 of requested window size */
    pub(crate) wsize: u32, /* window size or zero if not using window */
    pub(crate) whave: u32, /* valid bytes in the window */
    pub(crate) wnext: u32, /* window write index */
    pub(crate) window: Option<Vec<u8>>, /* allocated sliding window, if needed */
    /* bit accumulator */
    pub(crate) hold: u64, /* input bit accumulator */
    pub(crate) bits: u32, /* number of bits in "in" */
    /* for string and stored block copying */
    pub(crate) length: u32, /* literal or length of data to copy */
    pub(crate) offset: u32, /* distance back to copy string from */
    /* for table and code decoding */
    pub(crate) extra: u32, /* extra bits needed */
    /* fixed and dynamic code tables */
    pub(crate) lencode: CodePtr, /* starting table for length/literal codes */
    pub(crate) distcode: CodePtr, /* starting table for distance codes */
    pub(crate) lenbits: u32,     /* index bits for lencode */
    pub(crate) distbits: u32,    /* index bits for distcode */
    /* dynamic table building */
    pub(crate) ncode: u32,            /* number of code length code lengths */
    pub(crate) nlen: u32,             /* number of length code lengths */
    pub(crate) ndist: u32,            /* number of distance code lengths */
    pub(crate) have: u32,             /* number of code lengths in lens[] */
    pub(crate) next: usize,           /* next available space in codes[] */
    pub(crate) lens: [u16; 320],      /* temporary storage for code lengths */
    pub(crate) work: [u16; 288],      /* work area for code table building */
    pub(crate) codes: [Code; ENOUGH], /* space for code tables */
    pub(crate) sane: i32,             /* if false, allow invalid distance too far */
    pub(crate) back: i32,             /* bits back of last unprocessed length/lit */
    pub(crate) was: u32,              /* initial length of match */
}

impl InflateState {
    /// `code` lookup through a table pointer
    pub(crate) fn code(&self, table: CodePtr, index: usize) -> Code {
        match table {
            CodePtr::Codes(n) => self.codes[n + index],
            CodePtr::LenFix => fixed_tables().0[index],
            CodePtr::DistFix => fixed_tables().1[index],
        }
    }
}

/* inflate.c -- zlib decompression */

/// `inflateStateCheck`
fn inflate_state_check(strm: &ZStream) -> bool {
    match &strm.istate {
        None => true,
        Some(state) => state.mode < InflateMode::Head || state.mode > InflateMode::Sync,
    }
}

/// `inflateResetKeep`
fn inflate_reset_keep(strm: &mut ZStream) -> i32 {
    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    let state = strm.istate.as_mut().unwrap();
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
pub(crate) fn inflate_reset(strm: &mut ZStream) -> i32 {
    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    let state = strm.istate.as_mut().unwrap();
    state.wsize = 0;
    state.whave = 0;
    state.wnext = 0;
    inflate_reset_keep(strm)
}

/// `inflateReset2`
pub(crate) fn inflate_reset2(strm: &mut ZStream, mut window_bits: i32) -> i32 {
    let wrap: i32;

    /* get the state */
    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    let state = strm.istate.as_mut().unwrap();

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
pub(crate) fn inflate_init2(strm: &mut ZStream, window_bits: i32) -> i32 {
    strm.msg = None; /* in case we return an error */
    let state = match try_box_state() {
        Some(s) => s,
        None => return Z_MEM_ERROR,
    };
    strm.istate = Some(state);
    /* (state->window = Z_NULL; state->mode = HEAD: see try_box_state) */
    let ret = inflate_reset2(strm, window_bits);
    if ret != Z_OK {
        strm.istate = None;
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
        crc32(check, Some(buf))
    } else {
        adler32(check, Some(buf))
    }
}

/* check macros for header crc */
fn crc2(check: u64, word: u64) -> u64 {
    let hbuf = [word as u8, (word >> 8) as u8];
    crc32(check, Some(&hbuf))
}

fn crc4(check: u64, word: u64) -> u64 {
    let hbuf = [
        word as u8,
        (word >> 8) as u8,
        (word >> 16) as u8,
        (word >> 24) as u8,
    ];
    crc32(check, Some(&hbuf))
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
pub(crate) fn inflate(strm: &mut ZStream, input: &[u8], output: &mut [u8], flush: i32) -> i32 {
    static ORDER: [u16; 19] = /* permutation of code lengths */
        [
            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
        ];

    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    let mut state = strm.istate.take().unwrap();
    let ret = inflate_body(strm, &mut state, input, output, flush, &ORDER);
    strm.istate = Some(state);
    ret
}

fn inflate_body(
    strm: &mut ZStream,
    state: &mut InflateState,
    input: &[u8],
    output: &mut [u8],
    flush: i32,
    order: &[u16; 19],
) -> i32 {
    use InflateMode::*;

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
                    state.check = crc32(0, None);
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
                if bits_!(4) != Z_DEFLATED as u32 {
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
                state.check = adler32(0, None);
                strm.adler = state.check;
                state.mode = if hold & 0x200 != 0 { Dictid } else { Type };
                initbits!();
            }
            /* GUNZIP */
            Flags => {
                needbits!('inf_leave, 16);
                state.flags = hold as i32;
                if (state.flags & 0xff) as u32 != Z_DEFLATED as u32 {
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
                            state.check =
                                crc32(state.check, Some(&input[next..next + copy as usize]));
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
                        state.check = crc32(state.check, Some(&input[next..next + copy as usize]));
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
                        state.check = crc32(state.check, Some(&input[next..next + copy as usize]));
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
                state.check = crc32(0, None);
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
                state.check = adler32(0, None);
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
                    output[put..put + c].copy_from_slice(&input[next..next + c]);
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
                    inflate_fast(strm, state, input, output, out);
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
                        output[put] = window[from + i];
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
                        output[put] = output[from];
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
                output[put] = state.length as u8;
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
                            update_check(state, state.check, &output[put - out as usize..put]);
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
        && updatewindow(state, output, strm.next_out, out - strm.avail_out) != 0
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
            &output[strm.next_out - out as usize..strm.next_out],
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
pub(crate) fn inflate_end(strm: &mut ZStream) -> i32 {
    if inflate_state_check(strm) {
        return Z_STREAM_ERROR;
    }
    strm.istate = None;
    Z_OK
}
