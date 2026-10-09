// Rust translation of src/hb-ucd.cc from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).

//! HarfBuzz's own Unicode functions, on its Unicode Character Database
//! tables: the default (and, as SDL_ttf builds HarfBuzz without GLib and
//! ICU, only) Unicode functions.

use super::hb_common::*;
use super::hb_ucd_table::*;

/// `hb_ucd_combining_class`
pub(crate) fn hb_ucd_combining_class(unicode: HbCodepoint) -> u32 {
    _hb_ucd_ccc(unicode) as u32
}

/// `hb_ucd_general_category`
pub(crate) fn hb_ucd_general_category(unicode: HbCodepoint) -> u32 {
    _hb_ucd_gc(unicode) as u32
}

/// `hb_ucd_mirroring`
pub(crate) fn hb_ucd_mirroring(unicode: HbCodepoint) -> HbCodepoint {
    unicode.wrapping_add(_hb_ucd_bmg(unicode) as i32 as u32)
}

/// `hb_ucd_script`
pub(crate) fn hb_ucd_script(unicode: HbCodepoint) -> HbScript {
    _hb_ucd_sc_map[_hb_ucd_sc(unicode) as usize]
}

const SBASE: u32 = 0xAC00;
const LBASE: u32 = 0x1100;
const VBASE: u32 = 0x1161;
const TBASE: u32 = 0x11A7;
const SCOUNT: u32 = 11172;
const LCOUNT: u32 = 19;
const VCOUNT: u32 = 21;
const TCOUNT: u32 = 28;
const NCOUNT: u32 = VCOUNT * TCOUNT;

/// `_hb_ucd_decompose_hangul`
#[inline]
fn _hb_ucd_decompose_hangul(ab: HbCodepoint, a: &mut HbCodepoint, b: &mut HbCodepoint) -> bool {
    let si = ab.wrapping_sub(SBASE);

    if si >= SCOUNT {
        return false;
    }

    if si % TCOUNT != 0 {
        /* LV,T */
        *a = SBASE + (si / TCOUNT) * TCOUNT;
        *b = TBASE + (si % TCOUNT);
        true
    } else {
        /* L,V */
        *a = LBASE + (si / NCOUNT);
        *b = VBASE + (si % NCOUNT) / TCOUNT;
        true
    }
}

/// `_hb_ucd_compose_hangul`
#[inline]
fn _hb_ucd_compose_hangul(a: HbCodepoint, b: HbCodepoint, ab: &mut HbCodepoint) -> bool {
    if (SBASE..SBASE + SCOUNT).contains(&a)
        && b > TBASE
        && b < (TBASE + TCOUNT)
        && (a - SBASE) % TCOUNT == 0
    {
        /* LV,T */
        *ab = a + (b - TBASE);
        true
    } else if (LBASE..LBASE + LCOUNT).contains(&a) && (VBASE..VBASE + VCOUNT).contains(&b) {
        /* L,V */
        let li = a - LBASE;
        let vi = b - VBASE;
        *ab = SBASE + li * NCOUNT + vi * TCOUNT;
        true
    } else {
        false
    }
}

/// `HB_CODEPOINT_DECODE3_1`
#[inline]
fn hb_codepoint_decode3_1(v: u64) -> HbCodepoint {
    (v >> 42) as HbCodepoint
}
/// `HB_CODEPOINT_DECODE3_2`
#[inline]
fn hb_codepoint_decode3_2(v: u64) -> HbCodepoint {
    ((v >> 21) as HbCodepoint) & 0x1FFFFF
}
/// `HB_CODEPOINT_DECODE3_3`
#[inline]
fn hb_codepoint_decode3_3(v: u64) -> HbCodepoint {
    (v as HbCodepoint) & 0x1FFFFF
}
/// `HB_CODEPOINT_DECODE3_11_7_14_1`
#[inline]
fn hb_codepoint_decode3_11_7_14_1(v: u32) -> HbCodepoint {
    v >> 21
}
/// `HB_CODEPOINT_DECODE3_11_7_14_2`
#[inline]
fn hb_codepoint_decode3_11_7_14_2(v: u32) -> HbCodepoint {
    ((v >> 14) & 0x007F) | 0x0300
}
/// `HB_CODEPOINT_DECODE3_11_7_14_3`
#[inline]
fn hb_codepoint_decode3_11_7_14_3(v: u32) -> HbCodepoint {
    v & 0x3FFF
}

/// `hb_ucd_compose`
pub(crate) fn hb_ucd_compose(a: HbCodepoint, b: HbCodepoint, ab: &mut HbCodepoint) -> bool {
    // Hangul is handled algorithmically.
    if _hb_ucd_compose_hangul(a, b, ab) {
        return true;
    }

    let u;

    if (a & 0xFFFFF800) == 0x0000 && (b & 0xFFFFFF80) == 0x0300 {
        /* If "a" is small enough and "b" is in the U+0300 range,
         * the composition data is encoded in a 32bit array sorted
         * by "a,b" pair. */
        let k = hb_codepoint_encode3_11_7_14(a, b, 0);
        let mask = hb_codepoint_encode3_11_7_14(0x1FFFFF, 0x1FFFFF, 0);
        let Ok(i) = _hb_ucd_dm2_u32_map.binary_search_by(|&item| (item & mask).cmp(&k)) else {
            return false;
        };
        u = hb_codepoint_decode3_11_7_14_3(_hb_ucd_dm2_u32_map[i]);
    } else {
        /* Otherwise it is stored in a 64bit array sorted by
         * "a,b" pair. */
        let k = hb_codepoint_encode3(a, b, 0);
        let mask = hb_codepoint_encode3(0x1FFFFF, 0x1FFFFF, 0);
        let Ok(i) = _hb_ucd_dm2_u64_map.binary_search_by(|&item| (item & mask).cmp(&k)) else {
            return false;
        };
        u = hb_codepoint_decode3_3(_hb_ucd_dm2_u64_map[i]);
    }

    if u == 0 {
        return false;
    }
    *ab = u;
    true
}

/// `hb_ucd_decompose`
pub(crate) fn hb_ucd_decompose(ab: HbCodepoint, a: &mut HbCodepoint, b: &mut HbCodepoint) -> bool {
    if _hb_ucd_decompose_hangul(ab, a, b) {
        return true;
    }

    let mut i = _hb_ucd_dm(ab) as usize;

    /* If no data, there's no decomposition. */
    if i == 0 {
        return false;
    }
    i -= 1;

    /* Check if it's a single-character decomposition. */
    if i < _hb_ucd_dm1_p0_map.len() + _hb_ucd_dm1_p2_map.len() {
        /* Single-character decompositions currently are only in plane 0 or plane 2. */
        if i < _hb_ucd_dm1_p0_map.len() {
            /* Plane 0. */
            *a = _hb_ucd_dm1_p0_map[i] as u32;
        } else {
            /* Plane 2. */
            i -= _hb_ucd_dm1_p0_map.len();
            *a = 0x20000 | _hb_ucd_dm1_p2_map[i] as u32;
        }
        *b = 0;
        return true;
    }
    i -= _hb_ucd_dm1_p0_map.len() + _hb_ucd_dm1_p2_map.len();

    /* Otherwise they are encoded either in a 32bit array or a 64bit array. */
    if i < _hb_ucd_dm2_u32_map.len() {
        /* 32bit array. */
        let v = _hb_ucd_dm2_u32_map[i];
        *a = hb_codepoint_decode3_11_7_14_1(v);
        *b = hb_codepoint_decode3_11_7_14_2(v);
        return true;
    }
    i -= _hb_ucd_dm2_u32_map.len();

    /* 64bit array. */
    let v = _hb_ucd_dm2_u64_map[i];
    *a = hb_codepoint_decode3_1(v);
    *b = hb_codepoint_decode3_2(v);
    true
}
