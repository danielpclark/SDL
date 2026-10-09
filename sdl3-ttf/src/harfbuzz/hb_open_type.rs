// Rust translation of the parts of src/hb-open-type.hh, src/hb-algs.hh
// (hb_bsearch_impl) and src/hb-blob.hh that the OpenType layout tables use,
// from HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2007,2008,2009,2010  Red Hat, Inc.
// Copyright © 2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! The OpenType font file data types.
//!
//! "The following data types are used in the OpenType font file. All
//! OpenType fonts use Motorola-style byte ordering (Big Endian)."
//!
//! Translation notes: HarfBuzz overlays its table structures on the
//! (sanitized) font data; here a structure is the slice of the table data
//! that starts at it (C's `this`), read with the big-endian accessors
//! below. Reading past the end of the slice yields zeros: that is the
//! Null object HarfBuzz uses for null offsets and out-of-range array
//! indices (an empty slice here), whose bytes are all zeros except for a
//! few types (`Index`, `VarIdx`, `LangSys` and `RangeRecord`), which their
//! accessors handle. Sanitized data never makes a non-Null structure read
//! past its table.

/// The Null object (all zeros).
pub(crate) const NULL: &[u8] = &[];

/// A read of an `HBUINT8`.
#[inline]
pub(crate) fn u8_at(d: &[u8], off: usize) -> u8 {
    d.get(off).copied().unwrap_or(0)
}
/// A read of an `HBINT8`.
#[inline]
pub(crate) fn i8_at(d: &[u8], off: usize) -> i8 {
    u8_at(d, off) as i8
}
/// A read of an `HBUINT16`.
#[inline]
pub(crate) fn u16_at(d: &[u8], off: usize) -> u16 {
    match d.get(off..off.wrapping_add(2)) {
        Some(b) => u16::from_be_bytes([b[0], b[1]]),
        None => 0,
    }
}
/// A read of an `HBINT16`.
#[inline]
pub(crate) fn i16_at(d: &[u8], off: usize) -> i16 {
    u16_at(d, off) as i16
}
/// A read of an `HBUINT24`.
#[inline]
pub(crate) fn u24_at(d: &[u8], off: usize) -> u32 {
    match d.get(off..off.wrapping_add(3)) {
        Some(b) => u32::from_be_bytes([0, b[0], b[1], b[2]]),
        None => 0,
    }
}
/// A read of an `HBUINT32`.
#[inline]
pub(crate) fn u32_at(d: &[u8], off: usize) -> u32 {
    match d.get(off..off.wrapping_add(4)) {
        Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        None => 0,
    }
}
/// A read of an `HBINT32`.
#[inline]
pub(crate) fn i32_at(d: &[u8], off: usize) -> i32 {
    u32_at(d, off) as i32
}

/// The structure at `off` bytes from `base` (`StructAtOffset`).
#[inline]
pub(crate) fn struct_at(base: &[u8], off: usize) -> &[u8] {
    base.get(off..).unwrap_or(NULL)
}

/// `base+offset` of an `Offset16To<Type>` at `at`: the Null object for a
/// null offset.
#[inline]
pub(crate) fn offset16_to(base: &[u8], at: usize) -> &[u8] {
    let off = u16_at(base, at) as usize;
    if off == 0 {
        return NULL;
    }
    struct_at(base, off)
}

/// `base+offset` of an `Offset24To<Type>` at `at`.
#[inline]
pub(crate) fn offset24_to(base: &[u8], at: usize) -> &[u8] {
    let off = u24_at(base, at) as usize;
    if off == 0 {
        return NULL;
    }
    struct_at(base, off)
}

/// `base+offset` of an `Offset32To<Type>` at `at`.
#[inline]
pub(crate) fn offset32_to(base: &[u8], at: usize) -> &[u8] {
    let off = u32_at(base, at) as usize;
    if off == 0 {
        return NULL;
    }
    struct_at(base, off)
}

/// `base+offset` of an `NNOffset16To<Type>` (no null: offset 0 is the
/// base itself).
#[inline]
pub(crate) fn nn_offset16_to(base: &[u8], at: usize) -> &[u8] {
    struct_at(base, u16_at(base, at) as usize)
}

/// `hb_bsearch_impl`: the binary search of all of HarfBuzz, over `nmemb`
/// items, `compar (mid)` comparing the key with the item `mid` (negative:
/// the key is smaller). Returns the found index, or the insertion point.
#[inline]
pub(crate) fn hb_bsearch_impl(
    nmemb: usize,
    mut compar: impl FnMut(usize) -> i32,
) -> Result<u32, u32> {
    /* This is our *only* bsearch implementation. */

    let mut min: i32 = 0;
    let mut max: i32 = nmemb as i32 - 1;
    while min <= max {
        let mid = ((min as u32 + max as u32) / 2) as i32;
        let c = compar(mid as usize);
        if c < 0 {
            max = mid - 1;
        } else if c > 0 {
            min = mid + 1;
        } else {
            return Ok(mid as u32);
        }
    }
    Err(min as u32)
}

/// `IntType::cmp` of a key and an item of an unsigned type narrower than
/// `int` (`(int) a - (int) b`).
#[inline]
pub(crate) fn cmp_small(a: u32, b: u32) -> i32 {
    (a as i32).wrapping_sub(b as i32)
}

/// `IntType::cmp` for types as wide as `int` (`a < b ? -1 : a == b ? 0 : +1`).
#[inline]
pub(crate) fn cmp_wide(a: u32, b: u32) -> i32 {
    if a < b {
        -1
    } else if a == b {
        0
    } else {
        1
    }
}

/// An `ArrayOf<Type, HBUINT16>` at the start of `d`: its length and the
/// slice of its items (`arrayZ`).
#[inline]
pub(crate) fn array16(d: &[u8]) -> (u32, &[u8]) {
    (u16_at(d, 0) as u32, struct_at(d, 2))
}

/// An `ArrayOf<Type, HBUINT32>` at the start of `d`.
#[inline]
pub(crate) fn array32(d: &[u8]) -> (u32, &[u8]) {
    (u32_at(d, 0), struct_at(d, 4))
}

/// `SortedArrayOf<HBUINT16 or HBGlyphID16>::bfind` of an `unsigned`
/// key (compared as `a < b ? -1 : a == b ? 0 : +1`): the index of `x` in
/// the sorted 16-bit array of `len` items in `items`.
#[inline]
pub(crate) fn bfind_u16(items: &[u8], len: u32, x: u32) -> Result<u32, u32> {
    hb_bsearch_impl(len as usize, |mid| {
        cmp_wide(x, u16_at(items, mid * 2) as u32)
    })
}

/// `Tag` comparisons (`Tag::cmp`, a 32-bit `IntType`)
#[inline]
pub(crate) fn cmp_tag(a: u32, b: u32) -> i32 {
    cmp_wide(a, b)
}
