// Rust translation of the decoding half of lib/codebook.c from libvorbis
// 1.3.7 (https://xiph.org/vorbis/, at the revision SDL_mixer's
// external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: basic codebook pack/unpack/code/decode operations
//!
//! `vorbis_staticbook_pack()` and `vorbis_book_encode()` are the
//! encoder's; `vorbis_staticbook_unpack()` is in sharedbook.rs, with the
//! rest of the codebook setup.

use super::sharedbook::bitreverse;
use super::*;
use crate::ogg::bitwise::*;

/* the 'eliminate the decode tree' optimization actually requires the
codewords to be MSb first, not LSb.  This is an annoying inelegancy
(and one of the first places where carefully thought out design
turned out to be wrong; Vorbis II and future Ogg codecs should go
to an MSb bitpacker), but not actually the huge hit it appears to
be.  The first-stage decode table catches most words so that
bitreverse is not in the main execution path. */

/// Translation of `decode_packed_entry_number()`.
fn decode_packed_entry_number(book: &Codebook, b: &mut OggpackBuffer<'_>) -> i64 {
    let mut read = book.dec_maxlength;
    let mut lo: i64;
    let mut hi: i64;
    let mut lok = oggpack_look(b, book.dec_firsttablen);

    if lok >= 0 {
        let entry = book.dec_firsttable[lok as usize] as i64;
        if entry & 0x80000000 != 0 {
            lo = (entry >> 15) & 0x7fff;
            hi = book.used_entries - (entry & 0x7fff);
        } else {
            oggpack_adv(b, book.dec_codelengths[(entry - 1) as usize] as i32);
            return entry - 1;
        }
    } else {
        lo = 0;
        hi = book.used_entries;
    }

    /* Single entry codebooks use a firsttablen of 1 and a
    dec_maxlength of 1.  If a single-entry codebook gets here (due to
    failure to read one bit above), the next look attempt will also
    fail and we'll correctly kick out instead of trying to walk the
    underformed tree */

    lok = oggpack_look(b, read);

    while lok < 0 && read > 1 {
        read -= 1;
        lok = oggpack_look(b, read);
    }
    if lok < 0 {
        return -1;
    }

    /* bisect search for the codeword in the ordered list */
    {
        let testword = bitreverse(lok as u32);

        while hi - lo > 1 {
            let p = (hi - lo) >> 1;
            let test = (book.codelist[(lo + p) as usize] > testword) as i64;
            lo += p & (test - 1);
            hi -= p & (-test);
        }

        if book.dec_codelengths[lo as usize] as i32 <= read {
            oggpack_adv(b, book.dec_codelengths[lo as usize] as i32);
            return lo;
        }
    }

    oggpack_adv(b, read);

    -1
}

/* Decode side is specced and easier, because we don't need to find
matches using different criteria; we simply read and map.  There are
two things we need to do 'depending':

We may need to support interleave.  We don't really, but it's
convenient to do it here rather than rebuild the vector later.

Cascades may be additive or multiplicitive; this is not inherent in
the codebook, but set in the code using the codebook.  Like
interleaving, it's easiest to do it here.
addmul==0 -> declarative (set the value)
addmul==1 -> additive
addmul==2 -> multiplicitive */

/* returns the [original, not compacted] entry number or -1 on eof *********/
/// Translation of `vorbis_book_decode()`.
pub(crate) fn vorbis_book_decode(book: &Codebook, b: &mut OggpackBuffer<'_>) -> i64 {
    if book.used_entries > 0 {
        let packed_entry = decode_packed_entry_number(book, b);
        if packed_entry >= 0 {
            return book.dec_index[packed_entry as usize] as i64;
        }
    }

    /* if there's no dec_index, the codebook unpacking isn't collapsed */
    -1
}

/* returns 0 on OK or -1 on eof *************************************/
/* decode vector / dim granularity gaurding is done in the upper layer */
/// Translation of `vorbis_book_decodevs_add()`.
///
/// FIXME (upstream): a residue 0 partition book of no dimensions divides
/// by zero here (`n/book->dim`); here (and in the reference C, patched the
/// same way) it decodes nothing.
pub(crate) fn vorbis_book_decodevs_add(
    book: &Codebook,
    a: &mut [f32],
    b: &mut OggpackBuffer<'_>,
    n: i32,
) -> i64 {
    if book.used_entries > 0 {
        let step = if book.dim != 0 {
            (n as i64 / book.dim) as i32
        } else {
            0
        };
        let mut entry = vec![0i64; step.max(0) as usize];
        let dim = book.dim as usize;

        for i in 0..step as usize {
            entry[i] = decode_packed_entry_number(book, b);
            if entry[i] == -1 {
                return -1;
            }
        }
        let mut o = 0usize;
        for i in 0..dim {
            let mut j = 0usize;
            while o + j < n as usize && j < step as usize {
                a[o + j] += book.valuelist[entry[j] as usize * dim + i];
                j += 1;
            }
            o += step as usize;
        }
    }
    0
}

/* decode vector / dim granularity gaurding is done in the upper layer */
/// Translation of `vorbis_book_decodev_add()`.
pub(crate) fn vorbis_book_decodev_add(
    book: &Codebook,
    a: &mut [f32],
    b: &mut OggpackBuffer<'_>,
    n: i32,
) -> i64 {
    if book.used_entries > 0 {
        let mut i = 0usize;
        while i < n as usize {
            let entry = decode_packed_entry_number(book, b) as i32;
            if entry == -1 {
                return -1;
            }
            let t = entry as usize * book.dim as usize;
            let mut j = 0usize;
            while i < n as usize && j < book.dim as usize {
                a[i] += book.valuelist[t + j];
                i += 1;
                j += 1;
            }
        }
    }
    0
}

/* unlike the others, we guard against n not being an integer number
of <dim> internally rather than in the upper layer (called only by
floor0) */
/// Translation of `vorbis_book_decodev_set()`.
pub(crate) fn vorbis_book_decodev_set(
    book: &Codebook,
    a: &mut [f32],
    b: &mut OggpackBuffer<'_>,
    n: i32,
) -> i64 {
    if book.used_entries > 0 {
        let mut i = 0usize;
        while i < n as usize {
            let entry = decode_packed_entry_number(book, b) as i32;
            if entry == -1 {
                return -1;
            }
            let t = entry as usize * book.dim as usize;
            let mut j = 0usize;
            while i < n as usize && j < book.dim as usize {
                a[i] = book.valuelist[t + j];
                i += 1;
                j += 1;
            }
        }
    } else {
        for v in a.iter_mut().take(n.max(0) as usize) {
            *v = 0.0;
        }
    }
    0
}

/// Translation of `vorbis_book_decodevv_add()`: `a` is the block's
/// channels and `chans` the ones of C's `a` array.
pub(crate) fn vorbis_book_decodevv_add(
    book: &Codebook,
    a: &mut [Vec<f32>],
    chans: &[usize],
    offset: i64,
    ch: i32,
    b: &mut OggpackBuffer<'_>,
    n: i32,
) -> i64 {
    let mut chptr = 0usize;
    if book.used_entries > 0 {
        let m = (offset + n as i64) / ch as i64;
        let mut i = offset / ch as i64;
        while i < m {
            let entry = decode_packed_entry_number(book, b);
            if entry == -1 {
                return -1;
            }
            {
                let t = entry as usize * book.dim as usize;
                let mut j = 0usize;
                while i < m && j < book.dim as usize {
                    a[chans[chptr]][i as usize] += book.valuelist[t + j];
                    chptr += 1;
                    if chptr == ch as usize {
                        chptr = 0;
                        i += 1;
                    }
                    j += 1;
                }
            }
        }
    }
    0
}
