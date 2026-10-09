// Rust translation of src/hb-ot-layout-common.hh and the
// OT/Layout/Common/*.hh headers (Coverage, RangeRecord) from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it), without the subsetter
// and the glyph closure.
// Copyright © 2007,2008,2009  Red Hat, Inc.
// Copyright © 2010,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod, Garret Rieger

//! OpenType Layout Common Table Formats: script, feature and lookup
//! lists, coverage and class definition tables, device tables, the item
//! variation store and feature variations.
//!
//! Each structure is the slice of its table that starts at it (see
//! [`super::hb_open_type`]); `sanitize` functions check the structure at a
//! position of the blob being sanitized. As HarfBuzz is built with
//! `HB_NO_BEYOND_64K`, only the 16-bit formats exist.

#![allow(non_snake_case)]

use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_open_type::*;
use super::hb_sanitize::HbSanitizeContext;
use super::hb_set::HbSet;

/// `NOT_COVERED`
pub(crate) const NOT_COVERED: u32 = u32::MAX;

/// `Index::NOT_FOUND_INDEX`
pub(crate) const NOT_FOUND_INDEX: u32 = 0xFFFF;

/* RangeRecord */

/// `RangeRecord<SmallTypes>` (first, last, value: 6 bytes)
#[derive(Clone, Copy)]
pub(crate) struct RangeRecord<'a>(pub(crate) &'a [u8]);

impl<'a> RangeRecord<'a> {
    pub(crate) const SIZE: usize = 6;

    /// The record `i` of the array `arr` of `len` records (the Null
    /// `RangeRecord`, whose first is 0x0100 and last 0, out of range).
    #[inline]
    pub(crate) fn at(arr: &'a [u8], len: u32, i: u32) -> (u32, u32, u32) {
        if i >= len {
            return (0x0100, 0, 0);
        }
        let o = i as usize * Self::SIZE;
        (
            u16_at(arr, o) as u32,
            u16_at(arr, o + 2) as u32,
            u16_at(arr, o + 4) as u32,
        )
    }

    /// `cmp`
    #[inline]
    pub(crate) fn cmp(g: u32, first: u32, last: u32) -> i32 {
        if g < first {
            -1
        } else if g <= last {
            0
        } else {
            1
        }
    }

    /// `bsearch` of the sorted array: the record containing `g`, or the
    /// Null record.
    #[inline]
    pub(crate) fn bsearch(arr: &'a [u8], len: u32, g: u32) -> (u32, u32, u32) {
        match hb_bsearch_impl(len as usize, |mid| {
            let o = mid * Self::SIZE;
            Self::cmp(g, u16_at(arr, o) as u32, u16_at(arr, o + 2) as u32)
        }) {
            Ok(i) => Self::at(arr, len, i),
            Err(_) => (0x0100, 0, 0),
        }
    }
}

/* Coverage */

/// `Coverage`
#[derive(Clone, Copy)]
pub(crate) struct Coverage<'a>(pub(crate) &'a [u8]);

impl<'a> Coverage<'a> {
    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        if !c.sanitize_u16(p) {
            return false;
        }
        match c.u16(p) {
            /* CoverageFormat1_3::sanitize: glyphArray */
            1 => c.sanitize_array16_shallow(p + 2, 2),
            /* CoverageFormat2_4::sanitize: rangeRecord */
            2 => c.sanitize_array16_shallow(p + 2, RangeRecord::SIZE as u32),
            _ => true,
        }
    }

    #[inline]
    pub(crate) fn format(&self) -> u16 {
        u16_at(self.0, 0)
    }

    /// `get_coverage`
    pub(crate) fn get_coverage(&self, glyph_id: HbCodepoint) -> u32 {
        match self.format() {
            1 => {
                /* CoverageFormat1_3::get_coverage */
                let (len, arr) = array16(self.0);
                match bfind_u16(arr, len, glyph_id) {
                    Ok(i) => i,
                    Err(_) => NOT_COVERED,
                }
            }
            2 => {
                /* CoverageFormat2_4::get_coverage */
                let (len, arr) = array16(self.0);
                let (first, last, value) = RangeRecord::bsearch(arr, len, glyph_id);
                if first <= last {
                    value.wrapping_add(glyph_id.wrapping_sub(first))
                } else {
                    NOT_COVERED
                }
            }
            _ => NOT_COVERED,
        }
    }

    /// `get` (`operator []`)
    #[inline]
    pub(crate) fn get(&self, k: HbCodepoint) -> u32 {
        self.get_coverage(k)
    }

    /// `has`
    #[inline]
    pub(crate) fn has(&self, k: HbCodepoint) -> bool {
        self.get(k) != NOT_COVERED
    }

    /// `collect_coverage`: might return false if the array looks unsorted.
    pub(crate) fn collect_coverage(&self, glyphs: &mut HbSet) -> bool {
        match self.format() {
            1 => {
                let (len, arr) = array16(self.0);
                glyphs.add_sorted_array((0..len as usize).map(|i| u16_at(arr, i * 2) as u32))
            }
            2 => {
                let (len, arr) = array16(self.0);
                for i in 0..len {
                    let (first, last, _) = RangeRecord::at(arr, len, i);
                    if !glyphs.add_range(first, last) {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// The covered glyphs in coverage order (`iter ()`), as the
    /// iterators of the two formats produce them (format 2 stops at a
    /// broken table).
    pub(crate) fn iter(&self) -> Vec<HbCodepoint> {
        let mut out = Vec::new();
        match self.format() {
            1 => {
                let (len, arr) = array16(self.0);
                for i in 0..len as usize {
                    out.push(u16_at(arr, i * 2) as u32);
                }
            }
            2 => {
                /* CoverageFormat2_4::iter_t */
                let (len, arr) = array16(self.0);
                let mut i = 0u32;
                let mut coverage = 0u32;
                let mut j = if len != 0 {
                    RangeRecord::at(arr, len, 0).0
                } else {
                    0
                };
                let r0 = RangeRecord::at(arr, len, 0);
                if r0.0 > r0.1 {
                    /* Broken table. Skip. */
                    i = len;
                }
                while i < len {
                    out.push(j);
                    let r = RangeRecord::at(arr, len, i);
                    if j >= r.1 {
                        i += 1;
                        if i < len {
                            let old = coverage;
                            let ri = RangeRecord::at(arr, len, i);
                            j = ri.0;
                            coverage = ri.2;
                            if coverage != old.wrapping_add(1) {
                                /* Broken table. Skip. Important to avoid DoS.
                                 * Also, our callers depend on coverage being
                                 * consecutive and monotonically increasing,
                                 * ie. iota(). */
                                break;
                            }
                        }
                        continue;
                    }
                    coverage += 1;
                    j += 1;
                }
            }
            _ => {}
        }
        out
    }
}

/* ClassDef */

/// `ClassDef`
#[derive(Clone, Copy)]
pub(crate) struct ClassDef<'a>(pub(crate) &'a [u8]);

impl<'a> ClassDef<'a> {
    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        if !c.sanitize_u16(p) {
            return false;
        }
        match c.u16(p) {
            /* ClassDefFormat1_3::sanitize */
            1 => c.check_struct(p, 6) && c.sanitize_array16_shallow(p + 4, 2),
            /* ClassDefFormat2_4::sanitize */
            2 => c.sanitize_array16_shallow(p + 2, RangeRecord::SIZE as u32),
            _ => true,
        }
    }

    #[inline]
    pub(crate) fn format(&self) -> u16 {
        u16_at(self.0, 0)
    }

    /// `get_class`
    pub(crate) fn get_class(&self, glyph_id: HbCodepoint) -> u32 {
        match self.format() {
            1 => {
                /* ClassDefFormat1_3::get_class */
                let start_glyph = u16_at(self.0, 2) as u32;
                let (len, arr) = array16(struct_at(self.0, 4));
                let i = glyph_id.wrapping_sub(start_glyph);
                if i >= len {
                    0
                } else {
                    u16_at(arr, i as usize * 2) as u32
                }
            }
            2 => {
                /* ClassDefFormat2_4::get_class */
                let (len, arr) = array16(struct_at(self.0, 2));
                RangeRecord::bsearch(arr, len, glyph_id).2
            }
            _ => 0,
        }
    }

    /// `get` (`operator []`)
    #[inline]
    pub(crate) fn get(&self, k: HbCodepoint) -> u32 {
        self.get_class(k)
    }

    /// `collect_coverage`
    pub(crate) fn collect_coverage(&self, glyphs: &mut HbSet) -> bool {
        match self.format() {
            1 => {
                let start_glyph = u16_at(self.0, 2) as u32;
                let (count, arr) = array16(struct_at(self.0, 4));
                let mut start = 0u32;
                for i in 0..count {
                    if u16_at(arr, i as usize * 2) != 0 {
                        continue;
                    }
                    if start != i && !glyphs.add_range(start_glyph + start, start_glyph + i) {
                        return false;
                    }
                    start = i + 1;
                }
                if start != count && !glyphs.add_range(start_glyph + start, start_glyph + count) {
                    return false;
                }
                true
            }
            2 => {
                let (len, arr) = array16(struct_at(self.0, 2));
                for i in 0..len {
                    let (first, last, value) = RangeRecord::at(arr, len, i);
                    if value != 0 && !glyphs.add_range(first, last) {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// `collect_class`
    pub(crate) fn collect_class(&self, glyphs: &mut HbSet, klass: u32) -> bool {
        match self.format() {
            1 => {
                let start_glyph = u16_at(self.0, 2) as u32;
                let (count, arr) = array16(struct_at(self.0, 4));
                for i in 0..count {
                    if u16_at(arr, i as usize * 2) as u32 == klass {
                        glyphs.add(start_glyph + i);
                    }
                }
                true
            }
            2 => {
                let (len, arr) = array16(struct_at(self.0, 2));
                for i in 0..len {
                    let (first, last, value) = RangeRecord::at(arr, len, i);
                    if value == klass && !glyphs.add_range(first, last) {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }
}

/* IndexArray */

/// `IndexArray`: an `Array16Of<Index>` at the start of the slice.
#[derive(Clone, Copy)]
pub(crate) struct IndexArray<'a>(pub(crate) &'a [u8]);

impl<'a> IndexArray<'a> {
    #[inline]
    pub(crate) fn len(&self) -> u32 {
        u16_at(self.0, 0) as u32
    }
    /// `operator []` (the Null `Index`, 0xFFFF, out of range)
    #[inline]
    pub(crate) fn get(&self, i: u32) -> u32 {
        if i >= self.len() {
            return NOT_FOUND_INDEX;
        }
        u16_at(self.0, 2 + i as usize * 2) as u32
    }
    /// `get_indexes`: the indexes from `start_offset`, at most
    /// `max_count` (C's in/out count); returns the array length.
    pub(crate) fn get_indexes(&self, start_offset: u32, max_count: u32, out: &mut Vec<u32>) -> u32 {
        let len = self.len();
        let mut i = start_offset;
        let mut n = 0;
        while i < len && n < max_count {
            out.push(self.get(i));
            i += 1;
            n += 1;
        }
        len
    }
    /// `add_indexes_to`
    pub(crate) fn add_indexes_to(&self, output: &mut HbSet) {
        for i in 0..self.len() {
            output.add(self.get(i));
        }
    }
}

/* FeatureParams */

/// `FeatureParams::sanitize` (with the feature's tag)
fn feature_params_sanitize(c: &mut HbSanitizeContext, p: usize, tag: HbTag) -> bool {
    if tag == hb_tag(b's', b'i', b'z', b'e') {
        /* FeatureParamsSize::sanitize */
        if !c.check_struct(p, 10) {
            return false;
        }

        /* This subtable has some "history", if you will.  Some earlier versions of
         * Adobe tools calculated the offset of the FeatureParams subtable from the
         * beginning of the FeatureList table!  Now, that is dealt with in the
         * Feature implementation.  But we still need to be able to tell junk from
         * real data.  Note: We don't check that the nameID actually exists.
         * (See hb-ot-layout-common.hh for the rules quoted from Read Roberts.) */

        let design_size = c.u16(p);
        let subfamily_id = c.u16(p + 2);
        let subfamily_name_id = c.u16(p + 4);
        let range_start = c.u16(p + 6);
        let range_end = c.u16(p + 8);
        if design_size == 0 {
            return false;
        } else if subfamily_id == 0 && subfamily_name_id == 0 && range_start == 0 && range_end == 0
        {
            return true;
        } else if design_size < range_start
            || design_size > range_end
            || subfamily_name_id < 256
            || subfamily_name_id > 32767
        {
            return false;
        } else {
            return true;
        }
    }
    if (tag & 0xFFFF0000) == hb_tag(b's', b's', 0, 0) {
        /* ssXX */
        /* FeatureParamsStylisticSet::sanitize */
        /* Right now minorVersion is at zero.  Which means, any table supports
         * the uiNameID field. */
        return c.check_struct(p, 4);
    }
    if (tag & 0xFFFF0000) == hb_tag(b'c', b'v', 0, 0) {
        /* cvXX */
        /* FeatureParamsCharacterVariants::sanitize */
        return c.check_struct(p, 14) && c.sanitize_array16_shallow(p + 12, 3);
    }
    true
}

/* Feature */

/// `Record_sanitize_closure_t`
#[derive(Clone, Copy)]
pub(crate) struct RecordSanitizeClosure {
    pub(crate) tag: HbTag,
    pub(crate) list_base: usize,
}

/// `Feature`
#[derive(Clone, Copy)]
pub(crate) struct Feature<'a>(pub(crate) &'a [u8]);

impl<'a> Feature<'a> {
    /// `lookupIndex`
    #[inline]
    pub(crate) fn lookup_index(&self) -> IndexArray<'a> {
        IndexArray(struct_at(self.0, 2))
    }
    /// `get_lookup_count`
    #[inline]
    pub(crate) fn get_lookup_count(&self) -> u32 {
        self.lookup_index().len()
    }
    /// `get_lookup_index`
    #[inline]
    pub(crate) fn get_lookup_index(&self, i: u32) -> u32 {
        self.lookup_index().get(i)
    }
    /// `get_lookup_indexes`
    pub(crate) fn get_lookup_indexes(
        &self,
        start_index: u32,
        max_count: u32,
        out: &mut Vec<u32>,
    ) -> u32 {
        self.lookup_index().get_indexes(start_index, max_count, out)
    }
    /// `add_lookup_indexes_to`
    pub(crate) fn add_lookup_indexes_to(&self, lookup_indexes: &mut HbSet) {
        self.lookup_index().add_indexes_to(lookup_indexes)
    }

    /// `sanitize`
    pub(crate) fn sanitize(
        c: &mut HbSanitizeContext,
        p: usize,
        closure: Option<RecordSanitizeClosure>,
    ) -> bool {
        if !(c.check_struct(p, 4) && c.sanitize_array16_shallow(p + 2, 2)) {
            return false;
        }

        /* Some earlier versions of Adobe tools calculated the offset of the
         * FeatureParams subtable from the beginning of the FeatureList table!
         *
         * If sanitizing "failed" for the FeatureParams subtable, try it with the
         * alternative location.  We would know sanitize "failed" if old value
         * of the offset was non-zero, but it's zeroed now.
         *
         * Only do this for the 'size' feature, since at the time of the faulty
         * Adobe tools, only the 'size' feature had FeatureParams defined.
         */

        if c.u16(p) == 0 {
            return true;
        }

        let orig_offset = c.u16(p) as u32;
        let tag = closure.map(|cl| cl.tag).unwrap_or(HB_TAG_NONE);
        if !c.sanitize_offset16(p, p, |c, q| feature_params_sanitize(c, q, tag)) {
            return false;
        }

        if let Some(closure) = closure {
            if c.u16(p) == 0
                && closure.tag == hb_tag(b's', b'i', b'z', b'e')
                && closure.list_base != usize::MAX
                && closure.list_base < p
            {
                let new_offset_int = orig_offset.wrapping_sub((p - closure.list_base) as u32);
                /* Check that it would not overflow. */
                let new_offset = new_offset_int as u16;
                if new_offset as u32 == new_offset_int
                    && c.try_set_u16(p, new_offset)
                    && !c.sanitize_offset16(p, p, |c, q| feature_params_sanitize(c, q, tag))
                {
                    return false;
                }
            }
        }

        true
    }
}

/* Record, RecordArrayOf, RecordListOf */

/// `Record<Type>::sanitize`: the record at `p` (tag, offset from `base`).
fn record_sanitize(
    c: &mut HbSanitizeContext,
    p: usize,
    base: usize,
    f: impl FnOnce(&mut HbSanitizeContext, usize, Option<RecordSanitizeClosure>) -> bool,
) -> bool {
    let closure = RecordSanitizeClosure {
        tag: c.u32(p),
        list_base: base,
    };
    c.check_struct(p, 6) && c.sanitize_offset16(p + 4, base, |c, q| f(c, q, Some(closure)))
}

/// `RecordArrayOf<Type>` / `RecordListOf<Type>`: a sorted array of
/// (tag, offset) records at the start of the slice; offsets are from
/// `base` (the list itself for a `RecordListOf`).
#[derive(Clone, Copy)]
pub(crate) struct RecordArray<'a> {
    /// the object holding the array (the offsets' base)
    pub(crate) base: &'a [u8],
    /// the position of the array in `base`
    pub(crate) arr_off: usize,
}

impl<'a> RecordArray<'a> {
    #[inline]
    fn arr(&self) -> &'a [u8] {
        struct_at(self.base, self.arr_off)
    }
    #[inline]
    pub(crate) fn len(&self) -> u32 {
        u16_at(self.arr(), 0) as u32
    }
    /// `get_tag` (`HB_TAG_NONE` out of range, the Null record)
    #[inline]
    pub(crate) fn get_tag(&self, i: u32) -> HbTag {
        if i >= self.len() {
            return HB_TAG_NONE;
        }
        u32_at(self.arr(), 2 + i as usize * 6)
    }
    /// The object at `get_offset (i)`.
    #[inline]
    pub(crate) fn get(&self, i: u32) -> &'a [u8] {
        if i >= self.len() {
            return NULL;
        }
        offset16_to(self.base, self.arr_off + 2 + i as usize * 6 + 4)
    }
    /// `get_tags`
    pub(crate) fn get_tags(&self, start_offset: u32, max_count: u32, out: &mut Vec<HbTag>) -> u32 {
        let len = self.len();
        let mut i = start_offset;
        let mut n = 0;
        while i < len && n < max_count {
            out.push(self.get_tag(i));
            i += 1;
            n += 1;
        }
        len
    }
    /// `find_index`: bfind of the tag (`Index::NOT_FOUND_INDEX` stored if
    /// not found).
    pub(crate) fn find_index(&self, tag: HbTag, index: &mut u32) -> bool {
        let arr = self.arr();
        match hb_bsearch_impl(self.len() as usize, |mid| {
            cmp_tag(tag, u32_at(arr, 2 + mid * 6))
        }) {
            Ok(i) => {
                *index = i;
                true
            }
            Err(_) => {
                *index = NOT_FOUND_INDEX;
                false
            }
        }
    }

    /// `RecordArrayOf<Type>::sanitize (c, base)`
    pub(crate) fn sanitize(
        c: &mut HbSanitizeContext,
        p: usize,
        base: usize,
        mut f: impl FnMut(&mut HbSanitizeContext, usize, Option<RecordSanitizeClosure>) -> bool,
    ) -> bool {
        c.sanitize_array16(p, 6, |c, q| record_sanitize(c, q, base, &mut f))
    }
}

/// `RecordListOf<Type>` at the start of `d`.
#[inline]
pub(crate) fn record_list(d: &[u8]) -> RecordArray<'_> {
    RecordArray {
        base: d,
        arr_off: 0,
    }
}

/* LangSys */

/// `LangSys`
#[derive(Clone, Copy)]
pub(crate) struct LangSys<'a>(pub(crate) &'a [u8]);

impl<'a> LangSys<'a> {
    /// `reqFeatureIndex` (0xFFFF for the Null `LangSys`)
    #[inline]
    fn req_feature_index(&self) -> u32 {
        if self.0.is_empty() {
            return 0xFFFF;
        }
        u16_at(self.0, 2) as u32
    }
    /// `featureIndex`
    #[inline]
    pub(crate) fn feature_index(&self) -> IndexArray<'a> {
        IndexArray(struct_at(self.0, 4))
    }
    /// `get_feature_count`
    #[inline]
    pub(crate) fn get_feature_count(&self) -> u32 {
        self.feature_index().len()
    }
    /// `get_feature_index`
    #[inline]
    pub(crate) fn get_feature_index(&self, i: u32) -> u32 {
        self.feature_index().get(i)
    }
    /// `get_feature_indexes`
    pub(crate) fn get_feature_indexes(
        &self,
        start_offset: u32,
        max_count: u32,
        out: &mut Vec<u32>,
    ) -> u32 {
        self.feature_index()
            .get_indexes(start_offset, max_count, out)
    }
    /// `add_feature_indexes_to`
    pub(crate) fn add_feature_indexes_to(&self, feature_indexes: &mut HbSet) {
        self.feature_index().add_indexes_to(feature_indexes)
    }
    /// `has_required_feature`
    #[inline]
    pub(crate) fn has_required_feature(&self) -> bool {
        self.req_feature_index() != 0xFFFF
    }
    /// `get_required_feature_index`
    #[inline]
    pub(crate) fn get_required_feature_index(&self) -> u32 {
        let r = self.req_feature_index();
        if r == 0xFFFF {
            return NOT_FOUND_INDEX;
        }
        r
    }

    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.check_struct(p, 6) && c.sanitize_array16_shallow(p + 4, 2)
    }
}

/* Script */

/// `Script`
#[derive(Clone, Copy)]
pub(crate) struct Script<'a>(pub(crate) &'a [u8]);

impl<'a> Script<'a> {
    /// `langSys`
    #[inline]
    pub(crate) fn lang_sys(&self) -> RecordArray<'a> {
        RecordArray {
            base: self.0,
            arr_off: 2,
        }
    }
    /// `get_lang_sys_count`
    #[inline]
    pub(crate) fn get_lang_sys_count(&self) -> u32 {
        self.lang_sys().len()
    }
    /// `get_lang_sys_tag`
    #[inline]
    pub(crate) fn get_lang_sys_tag(&self, i: u32) -> HbTag {
        self.lang_sys().get_tag(i)
    }
    /// `get_lang_sys`
    pub(crate) fn get_lang_sys(&self, i: u32) -> LangSys<'a> {
        if i == NOT_FOUND_INDEX {
            return self.get_default_lang_sys();
        }
        LangSys(self.lang_sys().get(i))
    }
    /// `find_lang_sys_index`
    pub(crate) fn find_lang_sys_index(&self, tag: HbTag, index: &mut u32) -> bool {
        self.lang_sys().find_index(tag, index)
    }
    /// `has_default_lang_sys`
    #[inline]
    pub(crate) fn has_default_lang_sys(&self) -> bool {
        u16_at(self.0, 0) != 0
    }
    /// `get_default_lang_sys`
    #[inline]
    pub(crate) fn get_default_lang_sys(&self) -> LangSys<'a> {
        LangSys(offset16_to(self.0, 0))
    }

    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.sanitize_offset16(p, p, LangSys::sanitize)
            && RecordArray::sanitize(c, p + 2, p, |c, q, _| LangSys::sanitize(c, q))
    }
}

/* LookupFlag */

pub(crate) const LOOKUP_FLAG_RIGHT_TO_LEFT: u32 = 0x0001;
pub(crate) const LOOKUP_FLAG_IGNORE_BASE_GLYPHS: u32 = 0x0002;
pub(crate) const LOOKUP_FLAG_IGNORE_LIGATURES: u32 = 0x0004;
pub(crate) const LOOKUP_FLAG_IGNORE_MARKS: u32 = 0x0008;
pub(crate) const LOOKUP_FLAG_IGNORE_FLAGS: u32 = 0x000E;
pub(crate) const LOOKUP_FLAG_USE_MARK_FILTERING_SET: u32 = 0x0010;
pub(crate) const LOOKUP_FLAG_RESERVED: u32 = 0x00E0;
pub(crate) const LOOKUP_FLAG_MARK_ATTACHMENT_TYPE: u32 = 0xFF00;

/* Lookup */

/// `Lookup`
#[derive(Clone, Copy)]
pub(crate) struct Lookup<'a>(pub(crate) &'a [u8]);

impl<'a> Lookup<'a> {
    /// `get_subtable_count`
    #[inline]
    pub(crate) fn get_subtable_count(&self) -> u32 {
        u16_at(self.0, 4) as u32
    }
    /// `get_subtable (i)`
    #[inline]
    pub(crate) fn get_subtable(&self, i: u32) -> &'a [u8] {
        if i >= self.get_subtable_count() {
            return NULL;
        }
        offset16_to(self.0, 6 + i as usize * 2)
    }
    /// `get_type`
    #[inline]
    pub(crate) fn get_type(&self) -> u32 {
        u16_at(self.0, 0) as u32
    }
    /// `lookupFlag`
    #[inline]
    pub(crate) fn lookup_flag(&self) -> u32 {
        u16_at(self.0, 2) as u32
    }

    /// `get_props`: lookup_props is a 32-bit integer where the lower 16-bit
    /// is LookupFlag and higher 16-bit is mark-filtering-set if the lookup
    /// uses one. Not to be confused with glyph_props which is very similar.
    pub(crate) fn get_props(&self) -> u32 {
        let mut flag = self.lookup_flag();
        if flag & LOOKUP_FLAG_USE_MARK_FILTERING_SET != 0 {
            let mark_filtering_set =
                u16_at(self.0, 6 + self.get_subtable_count() as usize * 2) as u32;
            flag += mark_filtering_set << 16;
        }
        flag
    }

    /// `sanitize`, with the table's subtable sanitize (by lookup type) and
    /// its Extension lookup type and extension type reader.
    pub(crate) fn sanitize(
        c: &mut HbSanitizeContext,
        p: usize,
        extension_type: u32,
        subtable_sanitize: fn(&mut HbSanitizeContext, usize, u32) -> bool,
    ) -> bool {
        if !(c.check_struct(p, 6) && c.sanitize_array16_shallow(p + 4, 2)) {
            return false;
        }

        let subtables = c.u16(p + 4) as u32;
        if !c.visit_subtables(subtables) {
            return false;
        }

        let lookup_flag = c.u16(p + 2) as u32;
        if lookup_flag & LOOKUP_FLAG_USE_MARK_FILTERING_SET != 0 {
            let mfs = p + 6 + subtables as usize * 2;
            if !c.sanitize_u16(mfs) {
                return false;
            }
        }

        let lookup_type = c.u16(p) as u32;
        if !c.sanitize_array16_of_offset16(p + 4, p, |c, q| subtable_sanitize(c, q, lookup_type)) {
            return false;
        }

        if lookup_type == extension_type && c.get_edit_count() == 0 {
            /* The spec says all subtables of an Extension lookup should
             * have the same type, which shall not be the Extension type
             * itself (but we already checked for that).
             * This is specially important if one has a reverse type!
             *
             * We only do this if sanitizer edit_count is zero.  Otherwise,
             * some of the subtables might have become insane after they
             * were sanity-checked by the edits of subsequent subtables.
             * https://bugs.chromium.org/p/chromium/issues/detail?id=960331
             */
            let ext_type = |c: &HbSanitizeContext, i: usize| -> u32 {
                let off = c.u16(p + 6 + i * 2) as usize;
                if off == 0 {
                    return 0;
                }
                /* Extension::get_type (): format 1's extensionLookupType */
                let q = p + off;
                if c.u16(q) == 1 {
                    c.u16(q + 2) as u32
                } else {
                    0
                }
            };
            let t = ext_type(c, 0);
            for i in 1..subtables as usize {
                if ext_type(c, i) != t {
                    return false;
                }
            }
        }

        true
    }
}

/* VarRegionAxis, VarRegionList, VarData, ItemVariationStore */

/// `VarRegionAxis::evaluate` (start, peak, end are F2DOT14)
#[inline]
fn var_region_axis_evaluate(axis: &[u8], coord: i32) -> f32 {
    let peak = i16_at(axis, 2) as i32;
    if peak == 0 || coord == peak {
        return 1.0;
    }

    let start = i16_at(axis, 0) as i32;
    let end = i16_at(axis, 4) as i32;

    /* TODO Move these to sanitize(). */
    if start > peak || peak > end {
        return 1.0;
    }
    if start < 0 && end > 0 && peak != 0 {
        return 1.0;
    }

    if coord <= start || end <= coord {
        return 0.0;
    }

    /* Interpolate */
    if coord < peak {
        (coord - start) as f32 / (peak - start) as f32
    } else {
        (end - coord) as f32 / (end - peak) as f32
    }
}

/// `REGION_CACHE_ITEM_CACHE_INVALID`
pub(crate) const REGION_CACHE_ITEM_CACHE_INVALID: f32 = 2.0;

/// `VarRegionList`
#[derive(Clone, Copy)]
pub(crate) struct VarRegionList<'a>(pub(crate) &'a [u8]);

impl<'a> VarRegionList<'a> {
    #[inline]
    fn axis_count(&self) -> u32 {
        u16_at(self.0, 0) as u32
    }
    #[inline]
    pub(crate) fn region_count(&self) -> u32 {
        u16_at(self.0, 2) as u32
    }

    /// `evaluate`
    pub(crate) fn evaluate(
        &self,
        region_index: u32,
        coords: &[i32],
        cache: Option<&mut [f32]>,
    ) -> f32 {
        if region_index >= self.region_count() {
            return 0.0;
        }

        let mut cache = cache;
        if let Some(cache) = cache.as_deref_mut() {
            let cached_value = cache[region_index as usize];
            if cached_value != REGION_CACHE_ITEM_CACHE_INVALID {
                return cached_value;
            }
        }

        let axis_count = self.axis_count();
        let axes = struct_at(self.0, 4 + (region_index * axis_count) as usize * 6);

        let mut v = 1.0f32;
        for i in 0..axis_count as usize {
            let coord = if i < coords.len() { coords[i] } else { 0 };
            let factor = var_region_axis_evaluate(struct_at(axes, i * 6), coord);
            if factor == 0.0 {
                if let Some(cache) = cache.as_deref_mut() {
                    cache[region_index as usize] = 0.0;
                }
                return 0.0;
            }
            v *= factor;
        }

        if let Some(cache) = cache.as_deref_mut() {
            cache[region_index as usize] = v;
        }
        v
    }

    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.check_struct(p, 4) && {
            let axis_count = c.u16(p) as u32;
            let region_count = c.u16(p + 2) as u32;
            c.sanitize_unsized_array_shallow(p + 4, axis_count.wrapping_mul(region_count), 6)
        }
    }
}

/// `VarData`
#[derive(Clone, Copy)]
pub(crate) struct VarData<'a>(pub(crate) &'a [u8]);

impl<'a> VarData<'a> {
    #[inline]
    fn item_count(&self) -> u32 {
        u16_at(self.0, 0) as u32
    }
    #[inline]
    fn word_size_count(&self) -> u32 {
        u16_at(self.0, 2) as u32
    }
    #[inline]
    fn region_indices_len(&self) -> u32 {
        u16_at(self.0, 4) as u32
    }
    /// `longWords`
    #[inline]
    fn long_words(&self) -> bool {
        self.word_size_count() & 0x8000 != 0 /* LONG_WORDS */
    }
    /// `wordCount`
    #[inline]
    fn word_count(&self) -> u32 {
        self.word_size_count() & 0x7FFF /* WORD_DELTA_COUNT_MASK */
    }
    /// `get_row_size`
    #[inline]
    fn get_row_size(&self) -> u32 {
        (self.word_count() + self.region_indices_len()) * if self.long_words() { 2 } else { 1 }
    }

    /// `get_delta`
    pub(crate) fn get_delta(
        &self,
        inner: u32,
        coords: &[i32],
        regions: VarRegionList,
        mut cache: Option<&mut [f32]>,
    ) -> f32 {
        if inner >= self.item_count() {
            return 0.0;
        }

        let count = self.region_indices_len();
        let is_long = self.long_words();
        let word_count = self.word_count();
        let scount = if is_long { count } else { word_count };
        let lcount = if is_long { word_count } else { 0 };

        let region_indices = struct_at(self.0, 6);
        let bytes = struct_at(self.0, 6 + count as usize * 2);
        let row = struct_at(bytes, (inner * self.get_row_size()) as usize);

        let mut delta = 0.0f32;
        let mut i = 0u32;
        let mut cursor = 0usize;

        while i < lcount {
            let scalar = regions.evaluate(
                u16_at(region_indices, i as usize * 2) as u32,
                coords,
                cache.as_deref_mut(),
            );
            delta += scalar * i32_at(row, cursor) as f32;
            cursor += 4;
            i += 1;
        }
        while i < scount {
            let scalar = regions.evaluate(
                u16_at(region_indices, i as usize * 2) as u32,
                coords,
                cache.as_deref_mut(),
            );
            delta += scalar * i16_at(row, cursor) as f32;
            cursor += 2;
            i += 1;
        }
        while i < count {
            let scalar = regions.evaluate(
                u16_at(region_indices, i as usize * 2) as u32,
                coords,
                cache.as_deref_mut(),
            );
            delta += scalar * i8_at(row, cursor) as f32;
            cursor += 1;
            i += 1;
        }

        delta
    }

    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        if !(c.check_struct(p, 6) && c.sanitize_array16_shallow(p + 4, 2)) {
            return false;
        }
        let item_count = c.u16(p) as u32;
        let word_size_count = c.u16(p + 2) as u32;
        let region_indices_len = c.u16(p + 4) as u32;
        let word_count = word_size_count & 0x7FFF;
        let row_size =
            (word_count + region_indices_len) * if word_size_count & 0x8000 != 0 { 2 } else { 1 };
        word_count <= region_indices_len
            && c.check_range2(
                p + 6 + region_indices_len as usize * 2,
                item_count,
                row_size,
            )
    }
}

/// `ItemVariationStore`
#[derive(Clone, Copy)]
pub(crate) struct ItemVariationStore<'a>(pub(crate) &'a [u8]);

impl<'a> ItemVariationStore<'a> {
    /// `regions`
    #[inline]
    fn regions(&self) -> VarRegionList<'a> {
        VarRegionList(offset32_to(self.0, 2))
    }

    /// `create_cache`
    pub(crate) fn create_cache(&self) -> Vec<f32> {
        let count = self.regions().region_count();
        vec![REGION_CACHE_ITEM_CACHE_INVALID; count as usize]
    }

    /// `get_delta (outer, inner, ...)`
    fn get_delta_outer_inner(
        &self,
        outer: u32,
        inner: u32,
        coords: &[i32],
        cache: Option<&mut [f32]>,
    ) -> f32 {
        let data_sets_len = u16_at(self.0, 6) as u32;
        if outer >= data_sets_len {
            return 0.0;
        }
        VarData(offset32_to(self.0, 8 + outer as usize * 4)).get_delta(
            inner,
            coords,
            self.regions(),
            cache,
        )
    }

    /// `get_delta (index, ...)`
    pub(crate) fn get_delta(&self, index: u32, coords: &[i32], cache: Option<&mut [f32]>) -> f32 {
        let outer = index >> 16;
        let inner = index & 0xFFFF;
        self.get_delta_outer_inner(outer, inner, coords, cache)
    }

    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.check_struct(p, 8)
            && c.u16(p) == 1
            && c.sanitize_offset32(p + 2, p, VarRegionList::sanitize)
            && c.sanitize_array16_of_offset32(p + 6, p, VarData::sanitize)
    }
}

/* Feature Variations */

/// `ConditionSet::evaluate`
fn condition_set_evaluate(set: &[u8], coords: &[i32]) -> bool {
    let count = u16_at(set, 0) as usize;
    for i in 0..count {
        let cond = offset32_to(set, 2 + i * 4);
        /* Condition::evaluate */
        let ok = match u16_at(cond, 0) {
            1 => {
                /* ConditionFormat1::evaluate */
                let axis_index = u16_at(cond, 2) as usize;
                let coord = if axis_index < coords.len() {
                    coords[axis_index]
                } else {
                    0
                };
                (i16_at(cond, 4) as i32) <= coord && coord <= i16_at(cond, 6) as i32
            }
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// `Condition::sanitize` (as dispatched: the format, then the format's)
fn condition_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        1 => c.check_struct(p, 8),
        _ => true,
    }
}

/// `ConditionSet::sanitize`
fn condition_set_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    c.sanitize_array16_of_offset32(p, p, condition_sanitize)
}

/// `FeatureTableSubstitution::sanitize`
fn feature_table_substitution_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    c.check_struct(p, 4)
        && c.u16(p) == 1
        && c.sanitize_array16(p + 4, 6, |c, q| {
            /* FeatureTableSubstitutionRecord::sanitize */
            c.check_struct(q, 6)
                && c.sanitize_offset32(q + 2, p, |c, r| Feature::sanitize(c, r, None))
        })
}

/// `FeatureVariations`
#[derive(Clone, Copy)]
pub(crate) struct FeatureVariations<'a>(pub(crate) &'a [u8]);

/// `FeatureVariations::NOT_FOUND_INDEX`
pub(crate) const FEATURE_VARIATIONS_NOT_FOUND_INDEX: u32 = 0xFFFFFFFF;

impl<'a> FeatureVariations<'a> {
    /// `find_index`
    pub(crate) fn find_index(&self, coords: &[i32], index: &mut u32) -> bool {
        let count = u32_at(self.0, 4);
        for i in 0..count as usize {
            if condition_set_evaluate(offset32_to(self.0, 8 + i * 8), coords) {
                *index = i as u32;
                return true;
            }
        }
        *index = FEATURE_VARIATIONS_NOT_FOUND_INDEX;
        false
    }

    /// `find_substitute`
    pub(crate) fn find_substitute(
        &self,
        variations_index: u32,
        feature_index: u32,
    ) -> Option<Feature<'a>> {
        let count = u32_at(self.0, 4);
        if variations_index >= count {
            /* the Null record's substitutions: Null */
            return None;
        }
        let subst = offset32_to(self.0, 8 + variations_index as usize * 8 + 4);
        /* FeatureTableSubstitution::find_substitute */
        let n = u16_at(subst, 4) as usize;
        for i in 0..n {
            let record = 6 + i * 6;
            if u16_at(subst, record) as u32 == feature_index {
                return Some(Feature(offset32_to(subst, record + 2)));
            }
        }
        None
    }

    /// `collect_lookups` (without a feature substitutes map)
    pub(crate) fn collect_lookups(&self, feature_indexes: &HbSet, lookup_indexes: &mut HbSet) {
        let count = u32_at(self.0, 4) as usize;
        for i in 0..count {
            /* FeatureVariationRecord::collect_lookups */
            let subst = offset32_to(self.0, 8 + i * 8 + 4);
            /* FeatureTableSubstitution::collect_lookups */
            let n = u16_at(subst, 4) as usize;
            for j in 0..n {
                let record = 6 + j * 6;
                /* FeatureTableSubstitutionRecord::collect_lookups */
                if feature_indexes.has(u16_at(subst, record) as u32) {
                    Feature(offset32_to(subst, record + 2)).add_lookup_indexes_to(lookup_indexes);
                }
            }
        }
    }

    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.check_struct(p, 4)
            && c.u16(p) == 1
            && c.sanitize_array32(p + 4, 8, |c, q| {
                /* FeatureVariationRecord::sanitize */
                c.sanitize_offset32(q, p, condition_set_sanitize)
                    && c.sanitize_offset32(q + 4, p, feature_table_substitution_sanitize)
            })
    }
}

/* Device Tables */

/// `HintingDevice::get_size`
fn hinting_device_get_size(start_size: u32, end_size: u32, f: u32) -> u32 {
    if !(1..=3).contains(&f) || start_size > end_size {
        return 3 * 2;
    }
    2 * (4 + ((end_size - start_size) >> (4 - f)))
}

/// `Device`
#[derive(Clone, Copy)]
pub(crate) struct Device<'a>(pub(crate) &'a [u8]);

impl<'a> Device<'a> {
    #[inline]
    fn format(&self) -> u32 {
        u16_at(self.0, 4) as u32
    }

    /// `HintingDevice::get_delta_pixels`
    fn get_delta_pixels(&self, ppem_size: u32) -> i32 {
        let f = self.format();
        if !(1..=3).contains(&f) {
            return 0;
        }

        let start_size = u16_at(self.0, 0) as u32;
        let end_size = u16_at(self.0, 2) as u32;
        if ppem_size < start_size || ppem_size > end_size {
            return 0;
        }

        let s = ppem_size - start_size;

        let byte = u16_at(self.0, 6 + (s >> (4 - f)) as usize * 2) as u32;
        let bits = byte >> (16 - (((s & ((1 << (4 - f)) - 1)) + 1) << f));
        let mask = 0xFFFF >> (16 - (1 << f));

        let mut delta = (bits & mask) as i32;

        if delta as u32 >= (mask + 1) >> 1 {
            delta -= (mask + 1) as i32;
        }

        delta
    }

    /// `HintingDevice::get_delta`
    fn hinting_get_delta(&self, ppem: u32, scale: i32) -> i32 {
        if ppem == 0 {
            return 0;
        }

        let pixels = self.get_delta_pixels(ppem);

        if pixels == 0 {
            return 0;
        }

        (pixels as i64 * scale as i64 / ppem as i64) as i32
    }

    /// `VariationDevice::get_delta`
    fn variation_get_delta(
        &self,
        font: &HbFont,
        store: ItemVariationStore,
        cache: Option<&mut [f32]>,
    ) -> f32 {
        let var_idx = if self.0.is_empty() {
            0xFFFFFFFF
        } else {
            u32_at(self.0, 0)
        };
        store.get_delta(var_idx, &font.p.coords, cache)
    }

    /// `get_x_delta`
    pub(crate) fn get_x_delta(
        &self,
        font: &HbFont,
        store: ItemVariationStore,
        cache: Option<&mut [f32]>,
    ) -> HbPosition {
        match self.format() {
            1..=3 => self.hinting_get_delta(font.p.x_ppem, font.p.x_scale),
            0x8000 => font.em_scalef_x(self.variation_get_delta(font, store, cache)),
            _ => 0,
        }
    }

    /// `get_y_delta`
    pub(crate) fn get_y_delta(
        &self,
        font: &HbFont,
        store: ItemVariationStore,
        cache: Option<&mut [f32]>,
    ) -> HbPosition {
        match self.format() {
            1..=3 => self.hinting_get_delta(font.p.y_ppem, font.p.y_scale),
            0x8000 => font.em_scalef_y(self.variation_get_delta(font, store, cache)),
            _ => 0,
        }
    }

    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        /* u.b.format.sanitize */
        if !c.sanitize_u16(p + 4) {
            return false;
        }
        match c.u16(p + 4) {
            1..=3 => {
                /* HintingDevice::sanitize */
                c.check_struct(p, 6) && {
                    let size = hinting_device_get_size(
                        c.u16(p) as u32,
                        c.u16(p + 2) as u32,
                        c.u16(p + 4) as u32,
                    );
                    c.check_range(p, size)
                }
            }
            /* VariationDevice::sanitize */
            0x8000 => c.check_struct(p, 6),
            _ => true,
        }
    }
}
