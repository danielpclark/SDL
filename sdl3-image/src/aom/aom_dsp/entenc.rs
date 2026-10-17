// Rust translation of aom_dsp/entenc.c, aom_dsp/entenc.h and
// aom_dsp/entcode.h from aom (https://aomedia.googlesource.com/aom, at the
// revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's patches).
// Copyright (c) 2001-2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The range encoder (`od_ec_enc`), without the bit-level functions the
//! AV1 bitstream writer doesn't use (`od_ec_enc_bits()`,
//! `od_ec_enc_patch_initial_bits()`, `od_ec_enc_tell_frac()`, the
//! checkpoint and rollback); OD_MEASURE_EC_OVERHEAD is off.

use super::prob::{CDF_PROB_TOP, CDF_SHIFT};

// (entcode.h)
pub(crate) const EC_PROB_SHIFT: u32 = 6;
pub(crate) const EC_MIN_PROB: u32 = 4; // must be <= (1<<EC_PROB_SHIFT)/16

/*OPT: od_ec_window must be at least 32 bits, but if you have fast arithmetic
on a larger type, you can speed up the decoder by using it here.*/
pub(crate) type OdEcWindow = u32;

/*The entropy encoder context.*/
#[derive(Debug, Default)]
pub(crate) struct OdEcEnc {
    /*Buffered output.
    This contains only the raw bits until the final call to od_ec_enc_done(),
     where all the arithmetic-coded data gets prepended to it.*/
    pub(crate) buf: Vec<u8>,
    /*The size of the buffer.*/
    pub(crate) storage: u32,
    /*A buffer for output bytes with their associated carry flags.*/
    pub(crate) precarry_buf: Vec<u16>,
    /*The size of the pre-carry buffer.*/
    pub(crate) precarry_storage: u32,
    /*The offset at which the next entropy-coded byte will be written.*/
    pub(crate) offs: u32,
    /*The low end of the current range.*/
    pub(crate) low: OdEcWindow,
    /*The number of values in the current range.*/
    pub(crate) rng: u16,
    /*The number of bits of data in the current value.*/
    pub(crate) cnt: i16,
    /*Nonzero if an error occurred.*/
    pub(crate) error: i32,
}

/*OD_ILOG_NZ(x) = (int)floor(log2(x)) + 1.*/
#[inline]
fn od_ilog_nz(x: u32) -> i32 {
    32 - x.leading_zeros() as i32
}

/*A range encoder.
See entdec.c and the references for implementation details \cite{Mar79,MNW98}.

@INPROCEEDINGS{Mar79,
 author="Martin, G.N.N.",
 title="Range encoding: an algorithm for removing redundancy from a digitised
  message",
 booktitle="Video \& Data Recording Conference",
 year=1979,
 address="Southampton",
 month=Jul,
 URL="http://www.compressconsult.com/rangecoder/rngcod.pdf.gz"
}
@ARTICLE{MNW98,
 author="Alistair Moffat and Radford Neal and Ian H. Witten",
 title="Arithmetic Coding Revisited",
 journal="{ACM} Transactions on Information Systems",
 year=1998,
 volume=16,
 number=3,
 pages="256--294",
 month=Jul,
 URL="http://researchcommons.waikato.ac.nz/bitstream/handle/10289/78/content.pdf"
}*/

/*Takes updated low and range values, renormalizes them so that
 32768 <= rng < 65536 (flushing bytes from low to the pre-carry buffer if
 necessary), and stores them back in the encoder context.
low: The new value of low.
rng: The new value of the range.*/
fn od_ec_enc_normalize(enc: &mut OdEcEnc, mut low: OdEcWindow, rng: u32) {
    let d: i32;
    let mut c: i32;
    let mut s: i32;
    c = enc.cnt as i32;
    debug_assert!(rng <= 65535);
    /*The number of leading zeros in the 16-bit binary representation of rng.*/
    d = 16 - od_ilog_nz(rng);
    s = c + d;
    /*TODO: Right now we flush every time we have at least one byte available.
    Instead we should use an od_ec_window and flush right before we're about to
     shift bits off the end of the window.
    For a 32-bit window this is about the same amount of work, but for a 64-bit
     window it should be a fair win.*/
    if s >= 0 {
        let mut storage: u32;
        let mut offs: u32;
        let mut m: u32;
        storage = enc.precarry_storage;
        offs = enc.offs;
        if offs + 2 > storage {
            storage = 2 * storage + 2;
            // (realloc: a failed allocation aborts)
            enc.precarry_buf.resize(storage as usize, 0);
            enc.precarry_storage = storage;
        }
        let buf = &mut enc.precarry_buf;
        c += 16;
        m = (1u32 << c) - 1;
        if s >= 8 {
            debug_assert!(offs < storage);
            buf[offs as usize] = (low >> c) as u16;
            offs += 1;
            low &= m;
            c -= 8;
            m >>= 8;
        }
        debug_assert!(offs < storage);
        buf[offs as usize] = (low >> c) as u16;
        offs += 1;
        s = c + d - 24;
        low &= m;
        enc.offs = offs;
    }
    enc.low = low << d;
    enc.rng = (rng << d) as u16;
    enc.cnt = s as i16;
}

/*Initializes the encoder.
size: The initial size of the buffer, in bytes.*/
pub(crate) fn od_ec_enc_init(enc: &mut OdEcEnc, size: u32) {
    od_ec_enc_reset(enc);
    enc.buf = vec![0; size as usize];
    enc.storage = size;
    enc.precarry_buf = vec![0; size as usize];
    enc.precarry_storage = size;
}

/*Reinitializes the encoder.*/
pub(crate) fn od_ec_enc_reset(enc: &mut OdEcEnc) {
    enc.offs = 0;
    enc.low = 0;
    enc.rng = 0x8000;
    /*This is initialized to -9 so that it crosses zero after we've accumulated
    one byte + one carry bit.*/
    enc.cnt = -9;
    enc.error = 0;
}

/*Frees the buffers used by the encoder.*/
pub(crate) fn od_ec_enc_clear(enc: &mut OdEcEnc) {
    enc.precarry_buf = Vec::new();
    enc.buf = Vec::new();
}

/*Encodes a symbol given its frequency in Q15.
fl: CDF_PROB_TOP minus the cumulative frequency of all symbols that come
 before the
     one to be encoded.
fh: CDF_PROB_TOP minus the cumulative frequency of all symbols up to and
 including
     the one to be encoded.*/
fn od_ec_encode_q15(enc: &mut OdEcEnc, fl: u32, fh: u32, s: i32, nsyms: i32) {
    let mut l: OdEcWindow;
    let mut r: u32;
    let u: u32;
    let v: u32;
    l = enc.low;
    r = enc.rng as u32;
    debug_assert!(32768 <= r);
    debug_assert!(fh <= fl);
    debug_assert!(fl <= 32768);
    let n = nsyms - 1;
    if fl < CDF_PROB_TOP {
        u = ((r >> 8) * (fl >> EC_PROB_SHIFT) >> (7 - EC_PROB_SHIFT - CDF_SHIFT))
            .wrapping_add(EC_MIN_PROB.wrapping_mul((n - (s - 1)) as u32));
        v = ((r >> 8) * (fh >> EC_PROB_SHIFT) >> (7 - EC_PROB_SHIFT - CDF_SHIFT))
            .wrapping_add(EC_MIN_PROB.wrapping_mul((n - s) as u32));
        l = l.wrapping_add(r.wrapping_sub(u));
        r = u.wrapping_sub(v);
    } else {
        r = r.wrapping_sub(
            ((r >> 8) * (fh >> EC_PROB_SHIFT) >> (7 - EC_PROB_SHIFT - CDF_SHIFT))
                .wrapping_add(EC_MIN_PROB.wrapping_mul((n - s) as u32)),
        );
    }
    od_ec_enc_normalize(enc, l, r);
}

/*Encode a single binary value.
val: The value to encode (0 or 1).
f: The probability that the val is one, scaled by 32768.*/
pub(crate) fn od_ec_encode_bool_q15(enc: &mut OdEcEnc, val: i32, f: u32) {
    let mut l: OdEcWindow;
    let r: u32;
    let mut v: u32;
    debug_assert!(0 < f);
    debug_assert!(f < 32768);
    l = enc.low;
    r = enc.rng as u32;
    debug_assert!(32768 <= r);
    v = (r >> 8) * (f >> EC_PROB_SHIFT) >> (7 - EC_PROB_SHIFT);
    v += EC_MIN_PROB;
    if val != 0 {
        l = l.wrapping_add(r - v);
    }
    let r = if val != 0 { v } else { r - v };
    od_ec_enc_normalize(enc, l, r);
}

/*Encodes a symbol given a cumulative distribution function (CDF) table in Q15.
s: The index of the symbol to encode.
icdf: 32768 minus the CDF, such that symbol s falls in the range
       [s > 0 ? (32768 - icdf[s - 1]) : 0, 32768 - icdf[s]).
      The values must be monotonically decreasing, and icdf[nsyms - 1] must
       be 0.
nsyms: The number of symbols in the alphabet.
       This should be at most 16.*/
pub(crate) fn od_ec_encode_cdf_q15(enc: &mut OdEcEnc, s: i32, icdf: &[u16], nsyms: i32) {
    debug_assert!(s >= 0);
    debug_assert!(s < nsyms);
    debug_assert!(icdf[(nsyms - 1) as usize] as u32 == CDF_PROB_TOP - CDF_PROB_TOP);
    od_ec_encode_q15(
        enc,
        if s > 0 {
            icdf[(s - 1) as usize] as u32
        } else {
            CDF_PROB_TOP
        },
        icdf[s as usize] as u32,
        s,
        nsyms,
    );
}

/*Indicates that there are no more symbols to encode.
All remaining output bytes are flushed to the output buffer.
od_ec_enc_reset() should be called before using the encoder again.
bytes: Returns the size of the encoded data in the returned buffer.
Return: A pointer to the start of the final buffer, or NULL if there was an
         encoding error.*/
/// (returns the offset of the data in `enc.buf`)
pub(crate) fn od_ec_enc_done(enc: &mut OdEcEnc, nbytes: &mut u32) -> Option<usize> {
    let mut storage: u32;
    let mut offs: u32;
    let m: OdEcWindow;
    let mut e: OdEcWindow;
    let l: OdEcWindow;
    let mut c: i32;
    let mut s: i32;
    if enc.error != 0 {
        return None;
    }
    /*We output the minimum number of bits that ensures that the symbols encoded
    thus far will be decoded correctly regardless of the bits that follow.*/
    l = enc.low;
    c = enc.cnt as i32;
    s = 10;
    m = 0x3FFF;
    e = (l.wrapping_add(m) & !m) | (m + 1);
    s += c;
    offs = enc.offs;
    if s > 0 {
        let mut n: u32;
        storage = enc.precarry_storage;
        if offs + ((s + 7) >> 3) as u32 > storage {
            storage = storage * 2 + ((s + 7) >> 3) as u32;
            enc.precarry_buf.resize(storage as usize, 0);
            enc.precarry_storage = storage;
        }
        n = (1u32 << (c + 16)) - 1;
        loop {
            debug_assert!(offs < storage);
            enc.precarry_buf[offs as usize] = (e >> (c + 16)) as u16;
            offs += 1;
            e &= n;
            s -= 8;
            c -= 8;
            n >>= 8;
            if s <= 0 {
                break;
            }
        }
    }
    /*Make sure there's enough room for the entropy-coded bits.*/
    storage = enc.storage;
    c = ((s + 7) >> 3).max(0);
    if offs + c as u32 > storage {
        storage = offs + c as u32;
        enc.buf.resize(storage as usize, 0);
        enc.storage = storage;
    }
    *nbytes = offs;
    /*Perform carry propagation.*/
    debug_assert!(offs <= storage);
    let out = (storage - offs) as usize;
    c = 0;
    while offs > 0 {
        offs -= 1;
        c += enc.precarry_buf[offs as usize] as i32;
        enc.buf[out + offs as usize] = c as u8;
        c >>= 8;
    }
    /*Note: Unless there's an allocation error, if you keep encoding into the
    current buffer and call this function again later, everything will work
    just fine (you won't get a new packet out, but you will get a single
    buffer with the new data appended to the old).
    However, this function is O(N) where N is the amount of data coded so far,
    so calling it more than once for a given packet is a bad idea.*/
    Some(out)
}

/*Returns the number of bits "used" by the encoded symbols so far.
This same number can be computed in either the encoder or the decoder, and is
 suitable for making coding decisions.
Warning: The value returned by this function can decrease compared to an
 earlier call, even after encoding more data, if there is an encoding error
 (i.e., a failure to allocate enough space for the output buffer).
Return: The number of bits.
        This will always be slightly larger than the exact value (e.g., all
         rounding error is in the positive direction).*/
pub(crate) fn od_ec_enc_tell(enc: &OdEcEnc) -> i32 {
    /*The 10 here counteracts the offset of -9 baked into cnt, and adds 1 extra
    bit, which we reserve for terminating the stream.*/
    (enc.cnt as i32 + 10) + enc.offs as i32 * 8
}
