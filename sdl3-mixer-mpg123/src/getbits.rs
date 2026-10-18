/*
    getbits

    copyright ?-2009 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Michael Hipp

    All code is in the header to suggest/force inlining of these small often-used functions.
    This indeed has some impact on performance.
*/
// Modified 2026-10-17: translated into Rust from getbits.h in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The bit reader over `fr->wordpointer`, an offset into `fr->bsspace`
//! here. Upstream's readers look ahead up to two bytes past the bits they
//! take; where corrupt data makes them go past the end of `bsspace` (into
//! the handle's next members in upstream's C), they read zeros here.
//! Note (upstream): that would be a read past `bsspace`.

use crate::frame::Mpg123Handle;

impl Mpg123Handle<'_> {
    /// `fr->wordpointer[i]`
    #[inline]
    pub(crate) fn wp(&self, i: isize) -> u8 {
        let at = self.wordpointer as isize + i;
        if at < 0 {
            return 0;
        }
        self.bsspace.get(at as usize).copied().unwrap_or(0)
    }

    /// `fr->wordpointer += n`
    #[inline]
    fn wp_add(&mut self, n: isize) {
        self.wordpointer = (self.wordpointer as isize).wrapping_add(n) as usize;
    }
}

/// `backbits(fr, nob)`
#[inline]
pub(crate) fn backbits(fr: &mut Mpg123Handle<'_>, nob: i32) {
    fr.bits_avail += nob as i64;
    fr.bitindex -= nob;
    fr.wp_add((fr.bitindex >> 3) as isize);
    fr.bitindex &= 0x7;
}

/// `getbitoffset(fr)`
#[inline]
pub(crate) fn getbitoffset(fr: &Mpg123Handle<'_>) -> i32 {
    (-fr.bitindex) & 0x7
}

/* Precomputing the bytes to be read is error-prone, and some over-read
is even expected for Huffman. Just play safe and return zeros in case
of overflow. This assumes you made bitindex zero already! */
/// `getbyte(fr)`
#[inline]
pub(crate) fn getbyte(fr: &mut Mpg123Handle<'_>) -> u8 {
    fr.bits_avail -= 8;
    if fr.bits_avail >= 0 {
        let b = fr.wp(0);
        fr.wp_add(1);
        b
    } else {
        0
    }
}

/// Translation of `getbits()`.
#[inline]
pub(crate) fn getbits(fr: &mut Mpg123Handle<'_>, number_of_bits: i32) -> u32 {
    fr.bits_avail -= number_of_bits as i64;
    /* Safety catch until we got the nasty code fully figured out. */
    /* No, that catch stays here, even if we think we got it figured out! */
    if fr.bits_avail < 0 {
        // Tried to read %i bits with %li available.
        return 0;
    }
    /*  This is actually slow: if(!number_of_bits)
    return 0; */

    let mut rval: u64;
    {
        rval = fr.wp(0) as u64;
        rval <<= 8;
        rval |= fr.wp(1) as u64;
        rval <<= 8;
        rval |= fr.wp(2) as u64;

        rval <<= fr.bitindex;
        rval &= 0xffffff;

        fr.bitindex += number_of_bits;

        rval >>= 24 - number_of_bits;

        fr.wp_add((fr.bitindex >> 3) as isize);
        fr.bitindex &= 7;
    }

    rval as u32
}

/// `skipbits(fr, nob)`
#[inline]
pub(crate) fn skipbits(fr: &mut Mpg123Handle<'_>, nob: i32) {
    fr.ultmp = fr.wp(0) as u64;
    fr.ultmp <<= 8;
    fr.ultmp |= fr.wp(1) as u64;
    fr.ultmp <<= 8;
    fr.ultmp |= fr.wp(2) as u64;
    fr.ultmp <<= fr.bitindex;
    fr.ultmp &= 0xffffff;
    fr.bitindex += nob;
    fr.bits_avail -= nob as i64;
    fr.ultmp >>= 24 - nob;
    fr.wp_add((fr.bitindex >> 3) as isize);
    fr.bitindex &= 7;
}

/// `getbits_fast(fr, nob)`
#[inline]
pub(crate) fn getbits_fast(fr: &mut Mpg123Handle<'_>, nob: i32) -> u32 {
    fr.ultmp = ((fr.wp(0) as u32) << fr.bitindex) as u8 as u64;
    fr.ultmp |= ((fr.wp(1) as u64) << fr.bitindex) >> 8;
    fr.ultmp <<= nob;
    fr.ultmp >>= 8;
    fr.bitindex += nob;
    fr.bits_avail -= nob as i64;
    fr.wp_add((fr.bitindex >> 3) as isize);
    fr.bitindex &= 7;
    fr.ultmp as u32
}

/// `get1bit(fr)`
#[inline]
pub(crate) fn get1bit(fr: &mut Mpg123Handle<'_>) -> u32 {
    fr.uctmp = ((fr.wp(0) as u32) << fr.bitindex) as u8;
    fr.bitindex += 1;
    fr.bits_avail -= 1;
    fr.wp_add((fr.bitindex >> 3) as isize);
    fr.bitindex &= 7;
    (fr.uctmp >> 7) as u32
}
