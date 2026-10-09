// Rust translation of src/hb-set-digest.hh from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it).
// Copyright © 2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The set digests: approximate sets of glyphs (no false negatives).
//!
//! Translation notes: the GSUB/GPOS lookups' digests only make HarfBuzz
//! skip work that would not change anything, so they are not translated;
//! the `kern` table's digests are, as there they decide which glyph pairs
//! are looked up at all.

use super::hb_common::HbCodepoint;

/*
 * The set-digests here implement various "filters" that support
 * "approximate member query".  Conceptually these are like Bloom
 * Filter and Quotient Filter, however, much smaller, faster, and
 * designed to fit the requirements of our uses for glyph coverage
 * queries.
 *
 * Our filters are highly accurate if the lookup covers fairly local
 * set of glyphs, but fully flooded and ineffective if coverage is
 * all over the place.
 *
 * The way these are used is that the filter is first populated by
 * a lookup's or subtable's Coverage table(s), and then when we
 * want to apply the lookup or subtable to a glyph, before trying
 * to apply, we ask the filter if the glyph may be covered. If it's
 * not, we return early.  We can also match a digest against another
 * digest.
 *
 * We use these filters at three levels:
 *   - If the digest for all the glyphs in the buffer as a whole
 *     does not match the digest for the lookup, skip the lookup.
 *   - For each glyph, if it doesn't match the lookup digest,
 *     skip it.
 *   - For each glyph, if it doesn't match the subtable digest,
 *     skip it.
 *
 * The main filter we use is a combination of three bits-pattern
 * filters. A bits-pattern filter checks a number of bits (5 or 6)
 * of the input number (glyph-id in this case) and checks whether
 * its pattern is amongst the patterns of any of the accepted values.
 * The accepted patterns are represented as a "long" integer. The
 * check is done using four bitwise operations only.
 */

/// `hb_set_digest_bits_pattern_t<unsigned long, shift>` (64-bit `long`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct HbSetDigestBitsPattern<const SHIFT: u32> {
    mask: u64,
}

impl<const SHIFT: u32> HbSetDigestBitsPattern<SHIFT> {
    const MASK_BITS: u32 = 64;

    fn full() -> Self {
        HbSetDigestBitsPattern { mask: u64::MAX }
    }

    fn union_(&mut self, o: &Self) {
        self.mask |= o.mask;
    }

    fn add(&mut self, g: HbCodepoint) {
        self.mask |= Self::mask_for(g);
    }

    fn add_range(&mut self, a: HbCodepoint, b: HbCodepoint) -> bool {
        if self.mask == u64::MAX {
            return false;
        }
        if (b >> SHIFT).wrapping_sub(a >> SHIFT) >= Self::MASK_BITS - 1 {
            self.mask = u64::MAX;
            false
        } else {
            let ma = Self::mask_for(a);
            let mb = Self::mask_for(b);
            self.mask |= mb
                .wrapping_add(mb.wrapping_sub(ma))
                .wrapping_sub((mb < ma) as u64);
            true
        }
    }

    fn may_have_digest(&self, o: &Self) -> bool {
        self.mask & o.mask != 0
    }

    fn may_have(&self, g: HbCodepoint) -> bool {
        self.mask & Self::mask_for(g) != 0
    }

    fn mask_for(g: HbCodepoint) -> u64 {
        1u64 << ((g >> SHIFT) & (Self::MASK_BITS - 1))
    }
}

/// `hb_set_digest_t`: the combination of the bits-pattern filters with
/// shifts 4, 0 and 9.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HbSetDigest {
    head: HbSetDigestBitsPattern<4>,
    tail_head: HbSetDigestBitsPattern<0>,
    tail_tail: HbSetDigestBitsPattern<9>,
}

impl HbSetDigest {
    /// `full`
    pub(crate) fn full() -> HbSetDigest {
        HbSetDigest {
            head: HbSetDigestBitsPattern::full(),
            tail_head: HbSetDigestBitsPattern::full(),
            tail_tail: HbSetDigestBitsPattern::full(),
        }
    }

    /// `union_`
    pub(crate) fn union_(&mut self, o: &HbSetDigest) {
        self.head.union_(&o.head);
        self.tail_head.union_(&o.tail_head);
        self.tail_tail.union_(&o.tail_tail);
    }

    /// `add`
    pub(crate) fn add(&mut self, g: HbCodepoint) {
        self.head.add(g);
        self.tail_head.add(g);
        self.tail_tail.add(g);
    }

    /// `add_range`
    pub(crate) fn add_range(&mut self, a: HbCodepoint, b: HbCodepoint) -> bool {
        let h = self.head.add_range(a, b);
        let t = {
            let th = self.tail_head.add_range(a, b);
            let tt = self.tail_tail.add_range(a, b);
            th | tt
        };
        h | t
    }

    /// `may_have (const hb_set_digest_t &)`
    pub(crate) fn may_have_digest(&self, o: &HbSetDigest) -> bool {
        self.head.may_have_digest(&o.head)
            && self.tail_head.may_have_digest(&o.tail_head)
            && self.tail_tail.may_have_digest(&o.tail_tail)
    }

    /// `may_have (hb_codepoint_t)`
    pub(crate) fn may_have(&self, g: HbCodepoint) -> bool {
        self.head.may_have(g) && self.tail_head.may_have(g) && self.tail_tail.may_have(g)
    }
}
