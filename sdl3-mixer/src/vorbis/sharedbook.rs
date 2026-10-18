// Rust translation of lib/sharedbook.c and the unpacking half of
// lib/codebook.c from libvorbis 1.3.7 (https://xiph.org/vorbis/, at the
// revision SDL_mixer's external/vorbis pins).
// THE OggVorbis SOURCE CODE IS (C) COPYRIGHT 1994-2015
// by the Xiph.Org Foundation https://xiph.org/
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! function: basic shared codebook operations
//!
//! `_float32_pack()`, `vorbis_book_init_encode()`,
//! `vorbis_book_codeword()`/`vorbis_book_codelen()` and the self test are
//! the encoder's and aren't translated; `vorbis_staticbook_unpack()`,
//! which codebook.c has, is here with the rest of the codebook setup.

use super::*;
use crate::ogg::bitwise::*;

/**** pack/unpack helpers ******************************************/

/// Translation of `ov_ilog()`.
pub(crate) fn ov_ilog(mut v: u32) -> i32 {
    let mut ret = 0;
    while v != 0 {
        ret += 1;
        v >>= 1;
    }
    ret
}

/* 32 bit float (not IEEE; nonnormalized mantissa +
biased exponent) : neeeeeee eeemmmmm mmmmmmmm mmmmmmmm
Why not IEEE?  It's just not that important here. */

#[allow(dead_code)] // (only the encoder's _float32_pack() uses it)
const VQ_FEXP: i32 = 10;
const VQ_FMAN: i32 = 21;
const VQ_FEXP_BIAS: i64 = 768; /* bias toward values smaller than 1. */

/// Translation of `_float32_unpack()`.
pub(crate) fn _float32_unpack(val: i64) -> f32 {
    let mut mant: f64 = (val & 0x1fffff) as f64;
    let sign: i32 = (val & 0x80000000) as i32;
    let mut exp: i64 = (val & 0x7fe00000) >> VQ_FMAN;
    if sign != 0 {
        mant = -mant;
    }
    exp = exp - (VQ_FMAN - 1) as i64 - VQ_FEXP_BIAS;
    /* clamp excessive exponent values */
    if exp > 63 {
        exp = 63;
    }
    if exp < -63 {
        exp = -63;
    }
    sdl3::stdlib::math::scalbn(mant, exp as i32) as f32 /* ldexp(mant,exp) */
}

/* given a list of word lengths, generate a list of codewords.  Works
for length ordered or unordered, always assigns the lowest valued
codewords first.  Extended to handle unused entries (length 0) */
/// Translation of `_make_words()`.
pub(crate) fn _make_words(l: &[i8], n: i64, sparsecount: i64) -> Option<Vec<u32>> {
    let mut count: i64 = 0;
    let mut marker = [0u32; 33];
    let mut r = try_vec::<u32>((if sparsecount != 0 { sparsecount } else { n }) as usize)?;

    for i in 0..n as usize {
        let length = l[i] as i64;
        if length > 0 {
            let mut entry = marker[length as usize];

            /* when we claim a node for an entry, we also claim the nodes
            below it (pruning off the imagined tree that may have dangled
            from it) as well as blocking the use of any nodes directly
            above for leaves */

            /* update ourself */
            if length < 32 && (entry >> length) != 0 {
                /* error condition; the lengths must specify an overpopulated tree */
                return None;
            }
            r[count as usize] = entry;
            count += 1;

            /* Look to see if the next shorter marker points to the node
            above. if so, update it and repeat.  */
            {
                let mut j = length as usize;
                while j > 0 {
                    if marker[j] & 1 != 0 {
                        /* have to jump branches */
                        if j == 1 {
                            marker[1] = marker[1].wrapping_add(1);
                        } else {
                            marker[j] = marker[j - 1] << 1;
                        }
                        break; /* invariant says next upper marker would already
                               have been moved if it was on the same path */
                    }
                    marker[j] = marker[j].wrapping_add(1);
                    j -= 1;
                }
            }

            /* prune the tree; the implicit invariant says all the longer
            markers were dangling from our just-taken node.  Dangle them
            from our *new* node. */
            for j in length as usize + 1..33 {
                if (marker[j] >> 1) == entry {
                    entry = marker[j];
                    marker[j] = marker[j - 1] << 1;
                } else {
                    break;
                }
            }
        } else if sparsecount == 0 {
            count += 1;
        }
    }

    /* any underpopulated tree must be rejected. */
    /* Single-entry codebooks are a retconned extension to the spec.
    They have a single codeword '0' of length 1 that results in an
    underpopulated tree.  Shield that case from the underformed tree check. */
    if !(count == 1 && marker[2] == 2) {
        for i in 1..33 {
            if marker[i] & (0xffffffffu32 >> (32 - i)) != 0 {
                return None;
            }
        }
    }

    /* bitreverse the words because our bitwise packer/unpacker is LSb
    endian */
    count = 0;
    for i in 0..n as usize {
        let mut temp: u32 = 0;
        for j in 0..l[i].max(0) as u32 {
            temp <<= 1;
            temp |= (r[count as usize] >> j) & 1;
        }

        if sparsecount != 0 {
            if l[i] != 0 {
                r[count as usize] = temp;
                count += 1;
            }
        } else {
            r[count as usize] = temp;
            count += 1;
        }
    }

    Some(r)
}

/* there might be a straightforward one-line way to do the below
that's portable and totally safe against roundoff, but I haven't
thought of it.  Therefore, we opt on the side of caution */
/// Translation of `_book_maptype1_quantvals()`.
///
/// Note (upstream): with a `dim` of 0, the search never ends (no `vals`
/// passes `acc1>b->entries` when the loop over the dimensions is empty),
/// so libvorbis hangs setting up a value mapping 1 book of no dimensions
/// (`vorbis_staticbook_unpack()` guards its own call, but not
/// `_book_unquantize()`). Here (and in the reference C, patched the same
/// way) such a book has no quantized values, as
/// `vorbis_staticbook_unpack()` has it.
pub(crate) fn _book_maptype1_quantvals(b: &StaticCodebook) -> i64 {
    let mut vals: i64;
    if b.entries < 1 {
        return 0;
    }
    if b.dim == 0 {
        return 0;
    }
    vals = c_long(sdl3::stdlib::math::floor(sdl3::stdlib::math::pow(
        b.entries as f32 as f64,
        (1.0f32 / b.dim as f32) as f64,
    )));

    /* the above *should* be reliable, but we'll not assume that FP is
    ever reliable when bitstream sync is at stake; verify via integer
    means that vals really is the greatest value of dim for which
    vals^b->bim <= b->entries */
    /* treat the above as an initial guess */
    if vals < 1 {
        vals = 1;
    }
    loop {
        let mut acc: i64 = 1;
        let mut acc1: i64 = 1;
        let mut i: i64 = 0;
        while i < b.dim {
            if b.entries / vals < acc {
                break;
            }
            acc *= vals;
            if i64::MAX / (vals + 1) < acc1 {
                acc1 = i64::MAX;
            } else {
                acc1 *= vals + 1;
            }
            i += 1;
        }
        if i >= b.dim && acc <= b.entries && acc1 > b.entries {
            return vals;
        } else if i < b.dim || acc > b.entries {
            vals -= 1;
        } else {
            vals += 1;
        }
    }
}

/* unpack the quantized list of values for encode/decode ***********/
/* we need to deal with two map types: in map type 1, the values are
generated algorithmically (each column of the vector counts through
the values in the quant vector). in map type 2, all the values came
in in an explicit list.  Both value lists must be unpacked */
/// Translation of `_book_unquantize()`: `None` is C's NULL (no value
/// mapping); `Some(Err(()))` is the allocation failure C would crash on.
pub(crate) fn _book_unquantize(
    b: &StaticCodebook,
    n: i32,
    sparsemap: Option<&[i32]>,
) -> Option<Result<Vec<f32>, ()>> {
    let mut count: usize = 0;
    if b.maptype == 1 || b.maptype == 2 {
        let quantvals: i32;
        let mindel = _float32_unpack(b.q_min);
        let delta = _float32_unpack(b.q_delta);
        let Some(mut r) = try_vec::<f32>((n as i64 * b.dim) as usize) else {
            return Some(Err(()));
        };
        let dim = b.dim as usize;

        /* maptype 1 and 2 both use a quantized value vector, but
        different sizes */
        match b.maptype {
            1 => {
                /* most of the time, entries%dimensions == 0, but we need to be
                well defined.  We define that the possible vales at each
                scalar is values == entries/dim.  If entries%dim != 0, we'll
                have 'too few' values (values*dim<entries), which means that
                we'll have 'left over' entries; left over entries use zeroed
                values (and are wasted).  So don't generate codebooks like
                that */
                quantvals = _book_maptype1_quantvals(b) as i32;
                for j in 0..b.entries {
                    if (sparsemap.is_some() && b.lengthlist[j as usize] != 0) || sparsemap.is_none()
                    {
                        let mut last: f32 = 0.0;
                        let mut indexdiv: i32 = 1;
                        for k in 0..dim {
                            let index = ((j / indexdiv as i64) % quantvals as i64) as i32;
                            let mut val = b.quantlist[index as usize] as f32;
                            val = ((val as f64).abs() * delta as f64 + mindel as f64 + last as f64)
                                as f32;
                            if b.q_sequencep != 0 {
                                last = val;
                            }
                            match sparsemap {
                                Some(map) => r[map[count] as usize * dim + k] = val,
                                None => r[count * dim + k] = val,
                            }
                            indexdiv = indexdiv.wrapping_mul(quantvals);
                        }
                        count += 1;
                    }
                }
            }
            _ => {
                for j in 0..b.entries as usize {
                    if (sparsemap.is_some() && b.lengthlist[j] != 0) || sparsemap.is_none() {
                        let mut last: f32 = 0.0;

                        for k in 0..dim {
                            let mut val = b.quantlist[j * dim + k] as f32;
                            val = ((val as f64).abs() * delta as f64 + mindel as f64 + last as f64)
                                as f32;
                            if b.q_sequencep != 0 {
                                last = val;
                            }
                            match sparsemap {
                                Some(map) => r[map[count] as usize * dim + k] = val,
                                None => r[count * dim + k] = val,
                            }
                        }
                        count += 1;
                    }
                }
            }
        }

        return Some(Ok(r));
    }
    None
}

/// Translation of `vorbis_book_clear()`.
pub(crate) fn vorbis_book_clear(b: &mut Codebook) {
    /* static book is not cleared; we're likely called on the lookup and
    the static codebook belongs to the info struct */
    *b = Codebook::default();
}

/// Translation of `bitreverse()`.
pub(crate) fn bitreverse(mut x: u32) -> u32 {
    x = ((x >> 16) & 0x0000ffff) | ((x << 16) & 0xffff0000);
    x = ((x >> 8) & 0x00ff00ff) | ((x << 8) & 0xff00ff00);
    x = ((x >> 4) & 0x0f0f0f0f) | ((x << 4) & 0xf0f0f0f0);
    x = ((x >> 2) & 0x33333333) | ((x << 2) & 0xcccccccc);
    ((x >> 1) & 0x55555555) | ((x << 1) & 0xaaaaaaaa)
}

/* decode codebook arrangement is more heavily optimized than encode */
/// Translation of `vorbis_book_init_decode()`.
///
/// C sorts the codewords with `qsort()`, which isn't stable; a stable sort
/// gives the same order, as the words of a tree that passes
/// `_make_words()`'s checks are all different.
pub(crate) fn vorbis_book_init_decode(c: &mut Codebook, s: &StaticCodebook) -> i32 {
    let mut n: i32 = 0;
    let tabn: i32;

    *c = Codebook::default();

    'err_out: {
        /* count actually used entries and find max length */
        for i in 0..s.entries as usize {
            if s.lengthlist[i] > 0 {
                n += 1;
            }
        }

        c.entries = s.entries;
        c.used_entries = n as i64;
        c.dim = s.dim;

        if n > 0 {
            /* two different remappings go on here.

            First, we collapse the likely sparse codebook down only to
            actually represented values/words.  This collapsing needs to be
            indexed as map-valueless books are used to encode original entry
            positions as integers.

            Second, we reorder all vectors, including the entry index above,
            by sorted bitreversed codeword to allow treeless decode. */

            /* perform sort */
            let Some(mut codes) = _make_words(&s.lengthlist, s.entries, c.used_entries) else {
                break 'err_out;
            };
            let Some(mut codep) = try_vec::<usize>(n as usize) else {
                break 'err_out;
            };

            for i in 0..n as usize {
                codes[i] = bitreverse(codes[i]);
                codep[i] = i;
            }

            codep.sort_by(|&a, &b| codes[a].cmp(&codes[b]));

            let Some(mut sortindex) = try_vec::<i32>(n as usize) else {
                break 'err_out;
            };
            let Some(mut codelist) = try_vec::<u32>(n as usize) else {
                break 'err_out;
            };
            /* the index is a reverse index */
            for i in 0..n as usize {
                let position = codep[i];
                sortindex[position] = i as i32;
            }

            for i in 0..n as usize {
                codelist[sortindex[i] as usize] = codes[i];
            }
            c.codelist = codelist;
            drop(codes);

            match _book_unquantize(s, n, Some(&sortindex)) {
                None => {}
                Some(Ok(v)) => c.valuelist = v,
                Some(Err(())) => break 'err_out,
            }
            let Some(mut dec_index) = try_vec::<i32>(n as usize) else {
                break 'err_out;
            };

            n = 0;
            for i in 0..s.entries as usize {
                if s.lengthlist[i] > 0 {
                    dec_index[sortindex[n as usize] as usize] = i as i32;
                    n += 1;
                }
            }
            c.dec_index = dec_index;

            let Some(mut dec_codelengths) = try_vec::<i8>(n as usize) else {
                break 'err_out;
            };
            c.dec_maxlength = 0;
            n = 0;
            for i in 0..s.entries as usize {
                if s.lengthlist[i] > 0 {
                    dec_codelengths[sortindex[n as usize] as usize] = s.lengthlist[i];
                    n += 1;
                    if s.lengthlist[i] as i32 > c.dec_maxlength {
                        c.dec_maxlength = s.lengthlist[i] as i32;
                    }
                }
            }
            c.dec_codelengths = dec_codelengths;

            if n == 1 && c.dec_maxlength == 1 {
                /* special case the 'single entry codebook' with a single bit
                fastpath table (that always returns entry 0 )in order to use
                unmodified decode paths. */
                c.dec_firsttablen = 1;
                c.dec_firsttable = vec![1, 1];
            } else {
                c.dec_firsttablen = ov_ilog(c.used_entries as u32) - 4; /* this is magic */
                if c.dec_firsttablen < 5 {
                    c.dec_firsttablen = 5;
                }
                if c.dec_firsttablen > 8 {
                    c.dec_firsttablen = 8;
                }

                tabn = 1 << c.dec_firsttablen;
                c.dec_firsttable = vec![0u32; tabn as usize];

                for i in 0..n as usize {
                    if c.dec_codelengths[i] as i32 <= c.dec_firsttablen {
                        let orig = bitreverse(c.codelist[i]);
                        for j in 0..(1u32 << (c.dec_firsttablen - c.dec_codelengths[i] as i32)) {
                            c.dec_firsttable
                                [(orig | (j << c.dec_codelengths[i] as u32)) as usize] =
                                i as u32 + 1;
                        }
                    }
                }

                /* now fill in 'unused' entries in the firsttable with hi/lo search
                hints for the non-direct-hits */
                {
                    let mask: u32 = 0xfffffffeu32 << (31 - c.dec_firsttablen);
                    let mut lo: i64 = 0;
                    let mut hi: i64 = 0;

                    for i in 0..tabn as u32 {
                        let word: u32 = i << (32 - c.dec_firsttablen);
                        if c.dec_firsttable[bitreverse(word) as usize] == 0 {
                            while (lo + 1) < n as i64 && c.codelist[(lo + 1) as usize] <= word {
                                lo += 1;
                            }
                            while hi < n as i64 && word >= (c.codelist[hi as usize] & mask) {
                                hi += 1;
                            }

                            /* we only actually have 15 bits per hint to play with here.
                            In order to overflow gracefully (nothing breaks, efficiency
                            just drops), encode as the difference from the extremes. */
                            {
                                let mut loval: u64 = lo as u64;
                                let mut hival: u64 = (n as i64 - hi) as u64;

                                if loval > 0x7fff {
                                    loval = 0x7fff;
                                }
                                if hival > 0x7fff {
                                    hival = 0x7fff;
                                }
                                c.dec_firsttable[bitreverse(word) as usize] =
                                    (0x80000000u64 | (loval << 15) | hival) as u32;
                            }
                        }
                    }
                }
            }
        }

        return 0;
    }
    vorbis_book_clear(c);
    -1
}

/* unpacks a codebook from the packet buffer into the codebook struct,
readies the codebook auxiliary structures for decode *************/
/// Translation of `vorbis_staticbook_unpack()` (codebook.c).
pub(crate) fn vorbis_staticbook_unpack(opb: &mut OggpackBuffer<'_>) -> Option<Box<StaticCodebook>> {
    let mut s = Box::<StaticCodebook>::default();
    s.allocedp = 1;

    'eofout: {
        /* make sure alignment is correct */
        if oggpack_read(opb, 24) != 0x564342 {
            break 'eofout;
        }

        /* first the basic parameters */
        s.dim = oggpack_read(opb, 16);
        s.entries = oggpack_read(opb, 24);
        if s.entries == -1 {
            break 'eofout;
        }

        if ov_ilog(s.dim as u32) + ov_ilog(s.entries as u32) > 24 {
            break 'eofout;
        }

        /* codeword ordering.... length ordered or unordered? */
        match oggpack_read(opb, 1) as i32 {
            0 => {
                /* allocated but unused entries? */
                let unused = oggpack_read(opb, 1);
                if (s.entries * (if unused != 0 { 1 } else { 5 }) + 7) >> 3
                    > opb.storage - oggpack_bytes(opb)
                {
                    break 'eofout;
                }
                /* unordered */
                let Some(lengthlist) = try_vec::<i8>(s.entries as usize) else {
                    break 'eofout;
                };
                s.lengthlist = lengthlist;

                /* allocated but unused entries? */
                if unused != 0 {
                    /* yes, unused entries */

                    for i in 0..s.entries as usize {
                        if oggpack_read(opb, 1) != 0 {
                            let num = oggpack_read(opb, 5);
                            if num == -1 {
                                break 'eofout;
                            }
                            s.lengthlist[i] = (num + 1) as i8;
                        } else {
                            s.lengthlist[i] = 0;
                        }
                    }
                } else {
                    /* all entries used; no tagging */
                    for i in 0..s.entries as usize {
                        let num = oggpack_read(opb, 5);
                        if num == -1 {
                            break 'eofout;
                        }
                        s.lengthlist[i] = (num + 1) as i8;
                    }
                }
            }
            1 => {
                /* ordered */
                let mut length = oggpack_read(opb, 5) + 1;
                if length == 0 {
                    break 'eofout;
                }
                let Some(lengthlist) = try_vec::<i8>(s.entries as usize) else {
                    break 'eofout;
                };
                s.lengthlist = lengthlist;

                let mut i: i64 = 0;
                while i < s.entries {
                    let num = oggpack_read(opb, ov_ilog((s.entries - i) as u32));
                    if num == -1 {
                        break 'eofout;
                    }
                    if length > 32
                        || num > s.entries - i
                        || (num > 0 && (num - 1) >> (length - 1) > 1)
                    {
                        break 'eofout; /* (_errout) */
                    }
                    if length > 32 {
                        break 'eofout; /* (_errout) */
                    }
                    for _ in 0..num {
                        s.lengthlist[i as usize] = length as i8;
                        i += 1;
                    }
                    length += 1;
                }
            }
            _ => {
                /* EOF */
                break 'eofout;
            }
        }

        /* Do we have a mapping to unpack? */
        s.maptype = oggpack_read(opb, 4) as i32;
        match s.maptype {
            0 => { /* no mapping */ }
            1 | 2 => {
                /* implicitly populated value mapping */
                /* explicitly populated value mapping */

                s.q_min = oggpack_read(opb, 32);
                s.q_delta = oggpack_read(opb, 32);
                s.q_quant = oggpack_read(opb, 4) as i32 + 1;
                s.q_sequencep = oggpack_read(opb, 1) as i32;
                if s.q_sequencep == -1 {
                    break 'eofout;
                }

                {
                    let mut quantvals: i32 = 0;
                    match s.maptype {
                        1 => {
                            quantvals = if s.dim == 0 {
                                0
                            } else {
                                _book_maptype1_quantvals(&s) as i32
                            };
                        }
                        2 => {
                            quantvals = (s.entries * s.dim) as i32;
                        }
                        _ => {}
                    }

                    /* quantized values */
                    if ((quantvals as i64 * s.q_quant as i64 + 7) >> 3)
                        > opb.storage - oggpack_bytes(opb)
                    {
                        break 'eofout;
                    }
                    let Some(mut quantlist) = try_vec::<i64>(quantvals.max(0) as usize) else {
                        break 'eofout;
                    };
                    for v in quantlist.iter_mut() {
                        *v = oggpack_read(opb, s.q_quant);
                    }
                    s.quantlist = quantlist;

                    if quantvals != 0 && s.quantlist[quantvals as usize - 1] == -1 {
                        break 'eofout;
                    }
                }
            }
            _ => {
                break 'eofout; /* (_errout) */
            }
        }

        /* all set */
        return Some(s);
    }
    None
}
