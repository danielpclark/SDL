// Rust translation of src/msac.c and src/msac.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins), the plain-C functions.
// Copyright © 2018, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The multi-symbol arithmetic (entropy) decoder.
//!
//! `ec_win` is `size_t` upstream; it is 64 bits here on every target (the
//! decoded symbols don't depend on the window size). The buffer pointers
//! are offsets into the tile's data.

use std::sync::Arc;

use super::intops::{inv_recenter, ulog2};

type EcWin = u64;

const EC_PROB_SHIFT: u32 = 6;
const EC_MIN_PROB: u32 = 4; // must be <= (1<<EC_PROB_SHIFT)/16

const EC_WIN_SIZE: i32 = (std::mem::size_of::<EcWin>() << 3) as i32;

/// Translation of `MsacContext`.
#[derive(Clone, Default)]
pub(crate) struct MsacContext {
    buf: Option<Arc<[u8]>>,
    buf_pos: usize,
    buf_end: usize,
    dif: EcWin,
    pub(crate) rng: u32,
    pub(crate) cnt: i32,
    allow_update_cdf: bool,
}

impl MsacContext {
    #[inline]
    fn ctx_refill(&mut self) {
        let mut buf_pos = self.buf_pos;
        let buf_end = self.buf_end;
        let mut c = EC_WIN_SIZE - self.cnt - 24;
        let mut dif = self.dif;
        if let Some(buf) = &self.buf {
            while c >= 0 && buf_pos < buf_end {
                dif ^= (buf[buf_pos] as EcWin) << c;
                buf_pos += 1;
                c -= 8;
            }
        }
        self.dif = dif;
        self.cnt = EC_WIN_SIZE - c - 24;
        self.buf_pos = buf_pos;
    }

    /// Takes updated dif and range values, renormalizes them so that
    /// 32768 <= rng < 65536 (reading more bytes from the stream into dif if
    /// necessary), and stores them back in the decoder context.
    /// dif: The new value of dif.
    /// rng: The new value of the range.
    #[inline]
    fn ctx_norm(&mut self, dif: EcWin, rng: u32) {
        let d = 15 ^ (31 ^ rng.leading_zeros() as i32);
        debug_assert!(rng <= 65535);
        self.cnt -= d;
        self.dif = (dif.wrapping_add(1) << d).wrapping_sub(1); /* Shift in 1s in the LSBs */
        self.rng = rng << d;
        if self.cnt < 0 {
            self.ctx_refill();
        }
    }

    /// Translation of `dav1d_msac_decode_bool_equi_c()`.
    pub(crate) fn decode_bool_equi(&mut self) -> u32 {
        let r = self.rng;
        let mut dif = self.dif;
        debug_assert!((dif >> (EC_WIN_SIZE - 16)) < r as EcWin);
        // When the probability is 1/2, f = 16384 >> EC_PROB_SHIFT = 256 and we can
        // replace the multiply with a simple shift.
        let mut v = ((r >> 8) << 7) + EC_MIN_PROB;
        let vw = (v as EcWin) << (EC_WIN_SIZE - 16);
        let ret = (dif >= vw) as u32;
        dif = dif.wrapping_sub(ret as EcWin * vw);
        v = v.wrapping_add(ret.wrapping_mul(r.wrapping_sub(2 * v)));
        self.ctx_norm(dif, v);
        (ret == 0) as u32
    }

    /// Translation of `dav1d_msac_decode_bool_c()`: decode a single binary
    /// value.
    /// f: The probability that the bit is one
    /// Return: The value decoded (0 or 1).
    pub(crate) fn decode_bool(&mut self, f: u32) -> u32 {
        let r = self.rng;
        let mut dif = self.dif;
        debug_assert!((dif >> (EC_WIN_SIZE - 16)) < r as EcWin);
        let mut v = ((r >> 8) * (f >> EC_PROB_SHIFT) >> (7 - EC_PROB_SHIFT)) + EC_MIN_PROB;
        let vw = (v as EcWin) << (EC_WIN_SIZE - 16);
        let ret = (dif >= vw) as u32;
        dif = dif.wrapping_sub(ret as EcWin * vw);
        v = v.wrapping_add(ret.wrapping_mul(r.wrapping_sub(2 * v)));
        self.ctx_norm(dif, v);
        (ret == 0) as u32
    }

    /// Translation of `dav1d_msac_decode_subexp()`.
    pub(crate) fn decode_subexp(&mut self, r: i32, n: i32, mut k: u32) -> i32 {
        debug_assert!(n >> k == 8);

        let mut a: u32 = 0;
        if self.decode_bool_equi() != 0 {
            if self.decode_bool_equi() != 0 {
                k += self.decode_bool_equi() + 1;
            }
            a = 1 << k;
        }
        let v = self.decode_bools(k) + a;
        if r * 2 <= n {
            inv_recenter(r as u32, v) as i32
        } else {
            n - 1 - inv_recenter((n - 1 - r) as u32, v) as i32
        }
    }

    /// Translation of `dav1d_msac_decode_symbol_adapt_c()`: decodes a
    /// symbol given an inverse cumulative distribution function (CDF)
    /// table in Q15.
    pub(crate) fn decode_symbol_adapt(&mut self, cdf: &mut [u16], n_symbols: usize) -> u32 {
        let c = (self.dif >> (EC_WIN_SIZE - 16)) as u32;
        let r = self.rng >> 8;
        let mut u;
        let mut v = self.rng;
        let mut val: u32 = u32::MAX;

        debug_assert!(n_symbols <= 15);
        debug_assert!(cdf[n_symbols] <= 32);

        loop {
            val = val.wrapping_add(1);
            u = v;
            v = r * (cdf[val as usize] as u32 >> EC_PROB_SHIFT);
            v >>= 7 - EC_PROB_SHIFT;
            v += EC_MIN_PROB * (n_symbols as u32 - val);
            if c >= v {
                break;
            }
        }

        debug_assert!(u <= self.rng);

        self.ctx_norm(
            self.dif.wrapping_sub((v as EcWin) << (EC_WIN_SIZE - 16)),
            u - v,
        );

        if self.allow_update_cdf {
            let count = cdf[n_symbols] as u32;
            let rate = 4 + (count >> 4) + (n_symbols > 2) as u32;
            let mut i = 0usize;
            while i < val as usize {
                cdf[i] += ((32768 - cdf[i] as u32) >> rate) as u16;
                i += 1;
            }
            while i < n_symbols {
                cdf[i] -= cdf[i] >> rate;
                i += 1;
            }
            cdf[n_symbols] = (count + (count < 32) as u32) as u16;
        }

        val
    }

    /// `dav1d_msac_decode_symbol_adapt4()` (1-4 symbols).
    #[inline]
    pub(crate) fn decode_symbol_adapt4(&mut self, cdf: &mut [u16], n_symbols: usize) -> u32 {
        self.decode_symbol_adapt(cdf, n_symbols)
    }

    /// `dav1d_msac_decode_symbol_adapt8()` (1-7 symbols).
    #[inline]
    pub(crate) fn decode_symbol_adapt8(&mut self, cdf: &mut [u16], n_symbols: usize) -> u32 {
        self.decode_symbol_adapt(cdf, n_symbols)
    }

    /// `dav1d_msac_decode_symbol_adapt16()` (3-15 symbols).
    #[inline]
    pub(crate) fn decode_symbol_adapt16(&mut self, cdf: &mut [u16], n_symbols: usize) -> u32 {
        self.decode_symbol_adapt(cdf, n_symbols)
    }

    /// Translation of `dav1d_msac_decode_bool_adapt_c()`.
    pub(crate) fn decode_bool_adapt(&mut self, cdf: &mut [u16]) -> u32 {
        let bit = self.decode_bool(cdf[0] as u32);

        if self.allow_update_cdf {
            // update_cdf() specialized for boolean CDFs
            let count = cdf[1] as u32;
            let rate = 4 + (count >> 4);
            if bit != 0 {
                cdf[0] += ((32768 - cdf[0] as u32) >> rate) as u16;
            } else {
                cdf[0] -= cdf[0] >> rate;
            }
            cdf[1] = (count + (count < 32) as u32) as u16;
        }

        bit
    }

    /// Translation of `dav1d_msac_decode_hi_tok_c()`.
    pub(crate) fn decode_hi_tok(&mut self, cdf: &mut [u16]) -> u32 {
        let mut tok_br = self.decode_symbol_adapt4(cdf, 3);
        let mut tok = 3 + tok_br;
        if tok_br == 3 {
            tok_br = self.decode_symbol_adapt4(cdf, 3);
            tok = 6 + tok_br;
            if tok_br == 3 {
                tok_br = self.decode_symbol_adapt4(cdf, 3);
                tok = 9 + tok_br;
                if tok_br == 3 {
                    tok = 12 + self.decode_symbol_adapt4(cdf, 3);
                }
            }
        }
        tok
    }

    /// Translation of `dav1d_msac_init()`: `data` is the tile's bytes,
    /// `buf[start..start + sz]`.
    pub(crate) fn init(
        buf: Arc<[u8]>,
        start: usize,
        sz: usize,
        disable_cdf_update_flag: bool,
    ) -> Self {
        let mut s = MsacContext {
            buf: Some(buf),
            buf_pos: start,
            buf_end: start + sz,
            dif: ((1 as EcWin) << (EC_WIN_SIZE - 1)) - 1,
            rng: 0x8000,
            cnt: -15,
            allow_update_cdf: !disable_cdf_update_flag,
        };
        s.ctx_refill();
        s
    }

    /// Translation of `dav1d_msac_decode_bools()`.
    #[inline]
    pub(crate) fn decode_bools(&mut self, mut n: u32) -> u32 {
        let mut v: u32 = 0;
        while n > 0 {
            n -= 1;
            v = (v << 1) | self.decode_bool_equi();
        }
        v
    }

    /// Translation of `dav1d_msac_decode_uniform()`.
    #[inline]
    pub(crate) fn decode_uniform(&mut self, n: u32) -> i32 {
        debug_assert!(n > 0);
        let l = ulog2(n) + 1;
        debug_assert!(l > 1);
        let m = (1u32 << l) - n;
        let v = self.decode_bools((l - 1) as u32);
        (if v < m {
            v
        } else {
            (v << 1) - m + self.decode_bool_equi()
        }) as i32
    }
}
