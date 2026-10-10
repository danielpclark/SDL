// Rust translation of adler32.c from zlib 1.3.1.
// Copyright (C) 1995-2011, 2016 Mark Adler
// This is an altered (translated) version of the original software; zlib is
// used under the zlib license (see zlib.h and LICENSE.txt).

//! adler32.c -- compute the Adler-32 checksum of a data stream

const BASE: u64 = 65521; /* largest prime smaller than 65536 */
const NMAX: usize = 5552;
/* NMAX is the largest n such that 255n(n+1)/2 + (n+1)(BASE-1) <= 2^32-1 */

/// `adler32_z` (and `adler32`): `None` is C's `Z_NULL` buffer.
pub(crate) fn adler32(mut adler: u64, buf: Option<&[u8]>) -> u64 {
    /* split Adler-32 into component sums */
    let mut sum2 = (adler >> 16) & 0xffff;
    adler &= 0xffff;

    let Some(buf) = buf else {
        /* initial Adler-32 value (deferred check for len == 1 speed) */
        return 1;
    };
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
