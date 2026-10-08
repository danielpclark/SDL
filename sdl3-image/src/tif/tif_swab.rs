// Rust translation of libtiff/tif_swab.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! TIFF Library Bit & Byte Swapping Support.
//!
//! XXX We assume short = 16-bits and long = 32-bits XXX
//!
//! The array functions swap the elements stored in a byte buffer (the C
//! casts its buffers to arrays of the element type); `n` is the number of
//! elements, and a buffer too short for them is swapped as far as it goes.

use super::tiffiop::TmSize;

/// Swap the bytes of `n` elements of `size` bytes in `buf`.
fn swab_elements(buf: &mut [u8], n: TmSize, size: usize) {
    let n = n.max(0) as usize;
    for e in buf.chunks_exact_mut(size).take(n) {
        e.reverse();
    }
}

/// Translation of `TIFFSwabArrayOfShort()`.
pub(crate) fn tiff_swab_array_of_short(wp: &mut [u8], n: TmSize) {
    swab_elements(wp, n, 2);
}

/// Translation of `TIFFSwabArrayOfTriples()`: swap 1st and 3rd bytes
pub(crate) fn tiff_swab_array_of_triples(tp: &mut [u8], n: TmSize) {
    swab_elements(tp, n, 3);
}

/// Translation of `TIFFSwabArrayOfLong()`.
pub(crate) fn tiff_swab_array_of_long(lp: &mut [u8], n: TmSize) {
    swab_elements(lp, n, 4);
}

/// Translation of `TIFFSwabArrayOfLong8()`.
pub(crate) fn tiff_swab_array_of_long8(lp: &mut [u8], n: TmSize) {
    swab_elements(lp, n, 8);
}

/// Translation of `TIFFSwabArrayOfDouble()`.
pub(crate) fn tiff_swab_array_of_double(dp: &mut [u8], n: TmSize) {
    swab_elements(dp, n, 8);
}

/*
 * Bit reversal tables.  TIFFBitRevTable[<byte>] gives
 * the bit reversed value of <byte>.  Used in various
 * places in the library when the FillOrder requires
 * bit reversal of byte values (e.g. CCITT Fax 3
 * encoding/decoding).  TIFFNoBitRevTable is provided
 * for algorithms that want an equivalent table that
 * do not reverse bit values.
 */
static TIFF_BIT_REV_TABLE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as u8).reverse_bits();
        i += 1;
    }
    t
};
static TIFF_NO_BIT_REV_TABLE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = i as u8;
        i += 1;
    }
    t
};

/// Translation of `TIFFGetBitRevTable()`.
pub(crate) fn tiff_get_bit_rev_table(reversed: bool) -> &'static [u8; 256] {
    if reversed {
        &TIFF_BIT_REV_TABLE
    } else {
        &TIFF_NO_BIT_REV_TABLE
    }
}

/// Translation of `TIFFReverseBits()`.
pub(crate) fn tiff_reverse_bits(cp: &mut [u8], n: TmSize) {
    let n = n.max(0) as usize;
    for c in cp.iter_mut().take(n) {
        *c = TIFF_BIT_REV_TABLE[*c as usize];
    }
}
