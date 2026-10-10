// Rust translation of inffast.h and inffast.c from zlib 1.3.1.
// Copyright (C) 1995-2017 Mark Adler
// This is an altered (translated) version of the original software; zlib is
// used under the zlib license (see zlib.h and LICENSE.txt).

//! inffast.c -- fast decoding

use super::inflate::{InflateMode, InflateState};
use super::inftrees::Code;
use super::ZStream;

/* inffast.c -- fast decoding */

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
pub(crate) fn inflate_fast(
    strm: &mut ZStream,
    state: &mut InflateState,
    input: &[u8],
    output: &mut [u8],
    start: u32,
) {
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
