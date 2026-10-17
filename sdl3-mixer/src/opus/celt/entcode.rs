// Rust translation of celt/entcode.h, celt/entcode.c, celt/entdec.h,
// celt/entdec.c, celt/mfrngcod.h and celt/ecintrin.h from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2001-2011 Timothy B. Terriberry
// Copyright (c) 2008-2009 Xiph.Org Foundation
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The range decoder (the decoding half of the entropy coder; the encoder,
//! entenc.c, isn't translated). `ec_dec` borrows the packet it decodes.
//!
//! `USE_SMALL_DIV_TABLE` is only defined for ARM assembly builds, so
//! `celt_udiv()` is a plain division (and `SMALL_DIV_TABLE` isn't needed).
//! `EC_ILOG()` is `__builtin_clz()` based, which is undefined for 0; it's
//! never called with 0 here (and gives 0 for it, like the portable
//! `ec_ilog()`).

/*OPT: ec_window must be at least 32 bits, but if you have fast arithmetic on a
larger type, you can speed up the decoder by using it here.*/
pub(crate) type EcWindow = u32;

pub(crate) const EC_WINDOW_SIZE: i32 = 32;

/*The number of bits to use for the range-coded part of unsigned integers.*/
pub(crate) const EC_UINT_BITS: i32 = 8;

/*The resolution of fractional-precision bit usage measurements, i.e.,
3 => 1/8th bits.*/
pub(crate) const BITRES: i32 = 3;

/*Constants used by the entropy encoder/decoder.*/

/*The number of bits to output at a time.*/
pub(crate) const EC_SYM_BITS: i32 = 8;
/*The total number of bits in each of the state registers.*/
pub(crate) const EC_CODE_BITS: i32 = 32;
/*The maximum symbol value.*/
pub(crate) const EC_SYM_MAX: u32 = (1u32 << EC_SYM_BITS) - 1;
/*Carry bit of the high-order range symbol.*/
pub(crate) const EC_CODE_TOP: u32 = 1u32 << (EC_CODE_BITS - 1);
/*Low-order bit of the high-order range symbol.*/
pub(crate) const EC_CODE_BOT: u32 = EC_CODE_TOP >> EC_SYM_BITS;
/*The number of bits available for the last, partial symbol in the code field.*/
pub(crate) const EC_CODE_EXTRA: i32 = (EC_CODE_BITS - 2) % EC_SYM_BITS + 1;

/*Modern gcc (4.x) can compile the naive versions of min and max with cmov if
given an appropriate architecture, but the branchless bit-twiddling versions
are just as fast, and do not require any special target architecture.
Earlier gcc versions (3.x) compiled both code to the same assembly
instructions, because of the way they represented ((_b)>(_a)) internally.*/
#[inline(always)]
pub(crate) fn ec_mini(a: u32, b: u32) -> u32 {
    a.wrapping_add(b.wrapping_sub(a) & 0u32.wrapping_sub((b < a) as u32))
}

/// Translation of `EC_ILOG()`: the number of bits needed to hold `x`.
#[inline(always)]
pub(crate) fn ec_ilog(x: u32) -> i32 {
    32 - x.leading_zeros() as i32
}

/*The entropy encoder/decoder context.
We use the same structure for both, so that common functions like ec_tell()
can be used on either one.*/
/// Translation of `ec_ctx` (`ec_dec`).
#[derive(Clone)]
pub(crate) struct EcCtx<'a> {
    /*Buffered input/output.*/
    pub buf: &'a [u8],
    /*The size of the buffer.*/
    pub storage: u32,
    /*The offset at which the last byte containing raw bits was read/written.*/
    pub end_offs: u32,
    /*Bits that will be read from/written at the end.*/
    pub end_window: EcWindow,
    /*Number of valid bits in end_window.*/
    pub nend_bits: i32,
    /*The total number of whole bits read/written.
    This does not include partial bits currently in the range coder.*/
    pub nbits_total: i32,
    /*The offset at which the next range coder byte will be read/written.*/
    pub offs: u32,
    /*The number of values in the current range.*/
    pub rng: u32,
    /*In the decoder: the difference between the top of the current range and
    the input value, minus one.
    In the encoder: the low end of the current range.*/
    pub val: u32,
    /*In the decoder: the saved normalization factor from ec_decode().
    In the encoder: the number of oustanding carry propagating symbols.*/
    pub ext: u32,
    /*A buffered input/output symbol, awaiting carry propagation.*/
    pub rem: i32,
    /*Nonzero if an error occurred.*/
    pub error: i32,
}

pub(crate) type EcDec<'a> = EcCtx<'a>;

impl<'a> EcCtx<'a> {
    /// Translation of `ec_get_error()`.
    #[inline(always)]
    pub fn ec_get_error(&self) -> i32 {
        self.error
    }

    /*Returns the number of bits "used" by the encoded or decoded symbols so far.
    This same number can be computed in either the encoder or the decoder, and is
    suitable for making coding decisions.
    Return: The number of bits.
    This will always be slightly larger than the exact value (e.g., all
    rounding error is in the positive direction).*/
    /// Translation of `ec_tell()`.
    #[inline(always)]
    pub fn ec_tell(&self) -> i32 {
        self.nbits_total - ec_ilog(self.rng)
    }

    /* This is a faster version of ec_tell_frac() that takes advantage
    of the low (1/8 bit) resolution to use just a linear function
    followed by a lookup to determine the exact transition thresholds. */
    /// Translation of `ec_tell_frac()`.
    pub fn ec_tell_frac(&self) -> u32 {
        const CORRECTION: [u32; 8] = [35733, 38967, 42495, 46340, 50535, 55109, 60097, 65535];
        let nbits = (self.nbits_total as u32) << BITRES;
        let mut l = ec_ilog(self.rng);
        let r = self.rng >> (l - 16);
        let mut b = (r >> 12).wrapping_sub(8);
        b += (r > CORRECTION[b as usize & 7]) as u32;
        l = (l << 3) + b as i32;
        nbits.wrapping_sub(l as u32)
    }

    fn ec_read_byte(&mut self) -> i32 {
        if self.offs < self.storage {
            let b = self.buf[self.offs as usize];
            self.offs += 1;
            b as i32
        } else {
            0
        }
    }

    fn ec_read_byte_from_end(&mut self) -> i32 {
        if self.end_offs < self.storage {
            self.end_offs += 1;
            self.buf[(self.storage - self.end_offs) as usize] as i32
        } else {
            0
        }
    }

    /*Normalizes the contents of val and rng so that rng lies entirely in the
    high-order symbol.*/
    fn ec_dec_normalize(&mut self) {
        /*If the range is too small, rescale it and input some bits.*/
        while self.rng <= EC_CODE_BOT {
            self.nbits_total += EC_SYM_BITS;
            self.rng <<= EC_SYM_BITS;
            /*Use up the remaining bits from our last symbol.*/
            let mut sym = self.rem;
            /*Read the next value from the input.*/
            self.rem = self.ec_read_byte();
            /*Take the rest of the bits we need from this new symbol.*/
            sym = (sym << EC_SYM_BITS | self.rem) >> (EC_SYM_BITS - EC_CODE_EXTRA);
            /*And subtract them from val, capped to be less than EC_CODE_TOP.*/
            self.val = ((self.val << EC_SYM_BITS).wrapping_add(EC_SYM_MAX & !(sym as u32)))
                & (EC_CODE_TOP - 1);
        }
    }

    /// Translation of `ec_dec_init()`.
    pub fn ec_dec_init(buf: &'a [u8], storage: u32) -> EcCtx<'a> {
        let mut this = EcCtx {
            buf,
            storage,
            end_offs: 0,
            end_window: 0,
            nend_bits: 0,
            /*This is the offset from which ec_tell() will subtract partial bits.
            The final value after the ec_dec_normalize() call will be the same as in
            the encoder, but we have to compensate for the bits that are added there.*/
            nbits_total: EC_CODE_BITS + 1
                - ((EC_CODE_BITS - EC_CODE_EXTRA) / EC_SYM_BITS) * EC_SYM_BITS,
            offs: 0,
            rng: 1u32 << EC_CODE_EXTRA,
            val: 0,
            ext: 0,
            rem: 0,
            error: 0,
        };
        this.rem = this.ec_read_byte();
        this.val = this.rng - 1 - (this.rem >> (EC_SYM_BITS - EC_CODE_EXTRA)) as u32;
        this.error = 0;
        /*Normalize the interval.*/
        this.ec_dec_normalize();
        this
    }

    /// Translation of `ec_decode()`.
    pub fn ec_decode(&mut self, ft: u32) -> u32 {
        self.ext = celt_udiv(self.rng, ft);
        // (ext is never 0 for the totals CELT and SILK use.)
        let s = self.val / self.ext.max(1);
        ft.wrapping_sub(ec_mini(s.wrapping_add(1), ft))
    }

    /// Translation of `ec_decode_bin()`.
    pub fn ec_decode_bin(&mut self, bits: u32) -> u32 {
        self.ext = self.rng >> bits;
        let s = self.val / self.ext.max(1);
        (1u32 << bits).wrapping_sub(ec_mini(s.wrapping_add(1), 1u32 << bits))
    }

    /// Translation of `ec_dec_update()`.
    pub fn ec_dec_update(&mut self, fl: u32, fh: u32, ft: u32) {
        let s = self.ext.wrapping_mul(ft.wrapping_sub(fh));
        self.val = self.val.wrapping_sub(s);
        self.rng = if fl > 0 {
            self.ext.wrapping_mul(fh.wrapping_sub(fl))
        } else {
            self.rng.wrapping_sub(s)
        };
        self.ec_dec_normalize();
    }

    /*The probability of having a "one" is 1/(1<<_logp).*/
    /// Translation of `ec_dec_bit_logp()`.
    pub fn ec_dec_bit_logp(&mut self, logp: u32) -> i32 {
        let r = self.rng;
        let d = self.val;
        let s = r >> logp;
        let ret = d < s;
        if !ret {
            self.val = d - s;
        }
        self.rng = if ret { s } else { r - s };
        self.ec_dec_normalize();
        ret as i32
    }

    /// Translation of `ec_dec_icdf()`.
    pub fn ec_dec_icdf(&mut self, icdf: &[u8], ftb: u32) -> i32 {
        let mut s = self.rng;
        let d = self.val;
        let r = s >> ftb;
        let mut ret: i32 = -1;
        let mut t;
        loop {
            t = s;
            ret += 1;
            s = r.wrapping_mul(icdf[ret as usize] as u32);
            if d >= s {
                break;
            }
        }
        self.val = d.wrapping_sub(s);
        self.rng = t.wrapping_sub(s);
        self.ec_dec_normalize();
        ret
    }

    /// Translation of `ec_dec_uint()`.
    pub fn ec_dec_uint(&mut self, mut ft: u32) -> u32 {
        /*In order to optimize EC_ILOG(), it is undefined for the value 0.*/
        debug_assert!(ft > 1);
        ft -= 1;
        let mut ftb = ec_ilog(ft);
        if ftb > EC_UINT_BITS {
            ftb -= EC_UINT_BITS;
            let ft1 = (ft >> ftb) + 1;
            let s = self.ec_decode(ft1);
            self.ec_dec_update(s, s + 1, ft1);
            let t = s << ftb | self.ec_dec_bits(ftb as u32);
            if t <= ft {
                return t;
            }
            self.error = 1;
            ft
        } else {
            ft += 1;
            let s = self.ec_decode(ft);
            self.ec_dec_update(s, s + 1, ft);
            s
        }
    }

    /// Translation of `ec_dec_bits()`.
    pub fn ec_dec_bits(&mut self, bits: u32) -> u32 {
        let mut window = self.end_window;
        let mut available = self.nend_bits;
        if (available as u32) < bits {
            loop {
                window |= (self.ec_read_byte_from_end() as EcWindow) << available;
                available += EC_SYM_BITS;
                if available > EC_WINDOW_SIZE - EC_SYM_BITS {
                    break;
                }
            }
        }
        // (bits is at most 25; x86 shifts by the count modulo 32.)
        let ret = window & 1u32.wrapping_shl(bits).wrapping_sub(1);
        window = window.wrapping_shr(bits);
        available -= bits as i32;
        self.end_window = window;
        self.nend_bits = available;
        self.nbits_total += bits as i32;
        ret
    }
}

/* Tested exhaustively for all n and for 1<=d<=256 */
/// Translation of `celt_udiv()`.
#[inline(always)]
pub(crate) fn celt_udiv(n: u32, d: u32) -> u32 {
    debug_assert!(d > 0);
    n / d.max(1)
}

/// Translation of `celt_sudiv()`.
#[inline(always)]
pub(crate) fn celt_sudiv(n: i32, d: i32) -> i32 {
    debug_assert!(d > 0);
    n.wrapping_div(d.max(1))
}
