// Rust translation of the parts of src/hb-set.hh, src/hb-bit-set.hh and
// src/hb-bit-page.hh that the OpenType layout code uses, from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2012,2017  Google, Inc.
// Copyright © 2021 Behdad Esfahbod
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! `hb_set_t`: a set of code points or glyph indices, as 512-bit pages
//! (as HarfBuzz's `hb_bit_set_t`), keyed by their major number.
//!
//! Translation notes: the pages are kept in a `BTreeMap` instead of the
//! page map and page vector; allocation never fails, and the inversion
//! (`hb_bit_set_invertible_t`) is not translated (the layout code does not
//! invert sets).

use std::collections::BTreeMap;

use super::hb_common::HbCodepoint;

const PAGE_BITS: u32 = 512;
const PAGE_WORDS: usize = (PAGE_BITS / 64) as usize;

/// `HB_SET_VALUE_INVALID`
pub const HB_SET_VALUE_INVALID: HbCodepoint = HbCodepoint::MAX;

/// `hb_set_t`
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HbSet {
    pages: BTreeMap<u32, [u64; PAGE_WORDS]>,
}

impl HbSet {
    /// `INVALID`
    pub const INVALID: HbCodepoint = HB_SET_VALUE_INVALID;

    /// `hb_set_create`
    pub fn new() -> HbSet {
        HbSet::default()
    }

    #[inline]
    fn get_major(g: HbCodepoint) -> u32 {
        g / PAGE_BITS
    }

    /// `add`
    pub fn add(&mut self, g: HbCodepoint) {
        if g == Self::INVALID {
            return;
        }
        let page = self
            .pages
            .entry(Self::get_major(g))
            .or_insert([0; PAGE_WORDS]);
        let i = g % PAGE_BITS;
        page[(i / 64) as usize] |= 1u64 << (i % 64);
    }

    /// `add_range`: adds `a..=b`; false (and nothing added) if the range
    /// is invalid.
    pub fn add_range(&mut self, a: HbCodepoint, b: HbCodepoint) -> bool {
        if a > b || a == Self::INVALID || b == Self::INVALID {
            return false;
        }
        let mut g = a;
        loop {
            let major = Self::get_major(g);
            let page_end =
                (major as u64 * PAGE_BITS as u64 + PAGE_BITS as u64 - 1).min(b as u64) as u32;
            let page = self.pages.entry(major).or_insert([0; PAGE_WORDS]);
            for x in g..=page_end {
                let i = x % PAGE_BITS;
                page[(i / 64) as usize] |= 1u64 << (i % 64);
            }
            if page_end == b {
                break;
            }
            g = page_end + 1;
        }
        true
    }

    /// `add_array`
    pub fn add_array(&mut self, array: impl IntoIterator<Item = HbCodepoint>) {
        for g in array {
            self.add(g);
        }
    }

    /// `add_sorted_array`: might return false if the array looks unsorted
    /// (the elements before the first out-of-order one are added).
    pub fn add_sorted_array(&mut self, array: impl IntoIterator<Item = HbCodepoint>) -> bool {
        let mut last_g: Option<HbCodepoint> = None;
        for g in array {
            if let Some(l) = last_g {
                if g < l {
                    return false;
                }
            }
            last_g = Some(g);
            if g != Self::INVALID {
                self.add(g);
            }
        }
        true
    }

    /// `del`
    pub fn del(&mut self, g: HbCodepoint) {
        let major = Self::get_major(g);
        if let Some(page) = self.pages.get_mut(&major) {
            let i = g % PAGE_BITS;
            page[(i / 64) as usize] &= !(1u64 << (i % 64));
        }
    }

    /// `has`
    pub fn has(&self, g: HbCodepoint) -> bool {
        if g == Self::INVALID {
            return false;
        }
        match self.pages.get(&Self::get_major(g)) {
            Some(page) => {
                let i = g % PAGE_BITS;
                page[(i / 64) as usize] & (1u64 << (i % 64)) != 0
            }
            None => false,
        }
    }

    /// `is_empty`
    pub fn is_empty(&self) -> bool {
        self.pages.values().all(|p| p.iter().all(|&w| w == 0))
    }

    /// `get_population`
    pub fn get_population(&self) -> u32 {
        self.pages
            .values()
            .map(|p| p.iter().map(|w| w.count_ones()).sum::<u32>())
            .sum()
    }

    /// `next`: the smallest element greater than `*codepoint` (all
    /// elements after `HB_SET_VALUE_INVALID`); false at the end.
    pub fn next(&self, codepoint: &mut HbCodepoint) -> bool {
        let start: u64 = if *codepoint == Self::INVALID {
            0
        } else {
            *codepoint as u64 + 1
        };
        if start > u32::MAX as u64 {
            *codepoint = Self::INVALID;
            return false;
        }
        let start = start as u32;
        for (&major, page) in self.pages.range(Self::get_major(start)..) {
            let base = major * PAGE_BITS;
            let first = if major == Self::get_major(start) {
                start % PAGE_BITS
            } else {
                0
            };
            for i in first..PAGE_BITS {
                if page[(i / 64) as usize] & (1u64 << (i % 64)) != 0 {
                    *codepoint = base + i;
                    return true;
                }
            }
        }
        *codepoint = Self::INVALID;
        false
    }

    /// The elements, in increasing order.
    pub fn iter(&self) -> impl Iterator<Item = HbCodepoint> + '_ {
        self.pages.iter().flat_map(|(&major, page)| {
            (0..PAGE_BITS).filter_map(move |i| {
                if page[(i / 64) as usize] & (1u64 << (i % 64)) != 0 {
                    Some(major * PAGE_BITS + i)
                } else {
                    None
                }
            })
        })
    }

    /// `subtract`
    pub fn subtract(&mut self, other: &HbSet) {
        for (major, page) in self.pages.iter_mut() {
            if let Some(o) = other.pages.get(major) {
                for (w, ow) in page.iter_mut().zip(o.iter()) {
                    *w &= !ow;
                }
            }
        }
    }

    /// `union_`
    pub fn union(&mut self, other: &HbSet) {
        for (major, o) in other.pages.iter() {
            let page = self.pages.entry(*major).or_insert([0; PAGE_WORDS]);
            for (w, ow) in page.iter_mut().zip(o.iter()) {
                *w |= ow;
            }
        }
    }

    /// `clear`
    pub fn clear(&mut self) {
        self.pages.clear();
    }
}
