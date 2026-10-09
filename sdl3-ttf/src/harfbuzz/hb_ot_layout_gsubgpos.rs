// Rust translation of src/hb-ot-layout-gsubgpos.hh from HarfBuzz (8.5.0,
// as SDL_ttf's external/harfbuzz pins it), without the subsetter and the
// glyph closure.
// Copyright © 2007,2008,2009,2010  Red Hat, Inc.
// Copyright © 2010,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! The parts shared by GSUB and GPOS: the lookup application context and
//! its skipping iterators, input/backtrack/lookahead matching, ligation,
//! (chain) context lookups, extension subtables and the GSUB/GPOS table
//! header with its accelerator.
//!
//! Translation notes: the set digests (Bloom-filter-like prefilters of the
//! lookup and subtable coverages and of the buffer's glyphs) and the
//! lookup caches (which keep glyph classes in the `syllable` buffer
//! variable) are not translated: they only skip work whose result is
//! known not to change anything. Matching functions and their data are
//! the `MatchFunc` enum. The buffer message callbacks are not
//! translated.

use std::borrow::Cow;

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_face::*;
use super::hb_font::HbFont;
use super::hb_open_type::*;
use super::hb_ot_layout::*;
use super::hb_ot_layout_common::*;
use super::hb_ot_layout_gdef::{Gdef, GdefAccel};
use super::hb_ot_layout_gpos;
use super::hb_ot_layout_gsub;
use super::hb_sanitize::{hb_sanitize_blob_with, HbSanitizeContext};
use super::hb_set::HbSet;
use super::hb_unicode::*;

/* hb-limits.hh */
pub(crate) const HB_MAX_NESTING_LEVEL: u32 = 64;
pub(crate) const HB_MAX_CONTEXT_LENGTH: u32 = 64;

/// A position that is never in a blob being sanitized: where the Null
/// object is (outside the blob).
pub(crate) const NULL_POS: usize = usize::MAX / 2;

/// The two layout tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GsubGposKind {
    Gsub,
    Gpos,
}

/* match_func_t */

/// `match_func_t` with its data: the functions that match a glyph against
/// a value of a rule.
#[derive(Clone, Copy)]
pub(crate) enum MatchFunc<'a> {
    /// `match_always`
    Always,
    /// `match_glyph`: the value is a glyph index
    Glyph,
    /// `match_class`: the value is a class of the ClassDef
    Class(ClassDef<'a>),
    /// `match_coverage`: the value is an offset, from the data, to a
    /// Coverage table
    Coverage(&'a [u8]),
}

impl<'a> MatchFunc<'a> {
    /// The match function called on `info` and `value`.
    #[inline]
    pub(crate) fn matches(&self, info: &HbGlyphInfo, value: u32) -> bool {
        match *self {
            /* match_always */
            MatchFunc::Always => true,
            /* match_glyph */
            MatchFunc::Glyph => info.codepoint == value,
            /* match_class */
            MatchFunc::Class(class_def) => class_def.get_class(info.codepoint) == value,
            /* match_coverage */
            MatchFunc::Coverage(data) => {
                let coverage = if value == 0 {
                    NULL
                } else {
                    struct_at(data, value as usize)
                };
                Coverage(coverage).get_coverage(info.codepoint) != NOT_COVERED
            }
        }
    }
}

/// `collect_glyphs_func_t` with its data: `collect_glyph`,
/// `collect_class` or `collect_coverage`.
#[derive(Clone, Copy)]
pub(crate) enum CollectFunc<'a> {
    Glyph,
    Class(ClassDef<'a>),
    Coverage(&'a [u8]),
}

impl<'a> CollectFunc<'a> {
    fn collect(&self, glyphs: &mut HbSet, value: u32) {
        match *self {
            /* collect_glyph */
            CollectFunc::Glyph => glyphs.add(value),
            /* collect_class */
            CollectFunc::Class(class_def) => {
                class_def.collect_class(glyphs, value);
            }
            /* collect_coverage */
            CollectFunc::Coverage(data) => {
                let coverage = if value == 0 {
                    NULL
                } else {
                    struct_at(data, value as usize)
                };
                Coverage(coverage).collect_coverage(glyphs);
            }
        }
    }
}

/* hb_would_apply_context_t */

/// `hb_would_apply_context_t`
pub(crate) struct HbWouldApplyContext<'g> {
    pub(crate) glyphs: &'g [HbCodepoint],
    pub(crate) len: u32,
    pub(crate) zero_context: bool,
}

/* hb_collect_glyphs_context_t */

/// Which glyph set of a collect-glyphs context.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectSet {
    Before,
    Input,
    After,
    Output,
}

/// `hb_collect_glyphs_context_t`; a `None` set is C's empty set (which
/// ignores additions).
pub(crate) struct HbCollectGlyphsContext<'f> {
    pub(crate) face: &'f HbFace,
    pub(crate) before: Option<HbSet>,
    pub(crate) input: Option<HbSet>,
    pub(crate) after: Option<HbSet>,
    pub(crate) output: Option<HbSet>,
    /// `recurse_func` (set for GSUB only)
    pub(crate) recurse_gsub: bool,
    pub(crate) recursed_lookups: HbSet,
    pub(crate) nesting_level_left: u32,
}

impl<'f> HbCollectGlyphsContext<'f> {
    pub(crate) fn set(&mut self, which: CollectSet) -> Option<&mut HbSet> {
        match which {
            CollectSet::Before => self.before.as_mut(),
            CollectSet::Input => self.input.as_mut(),
            CollectSet::After => self.after.as_mut(),
            CollectSet::Output => self.output.as_mut(),
        }
    }

    /// Adds a glyph to a set (no-op on the empty set).
    pub(crate) fn add(&mut self, which: CollectSet, g: HbCodepoint) {
        if let Some(s) = self.set(which) {
            s.add(g);
        }
    }

    /// `(coverage).collect_coverage (c->set)`
    pub(crate) fn collect_coverage(&mut self, which: CollectSet, coverage: Coverage) -> bool {
        match self.set(which) {
            Some(s) => coverage.collect_coverage(s),
            /* (the empty set accepts nothing: its add functions return
             * true without adding) */
            None => true,
        }
    }

    /// `recurse`
    pub(crate) fn recurse(&mut self, lookup_index: u32) {
        if self.nesting_level_left == 0 || !self.recurse_gsub {
            return;
        }

        /* Note that GPOS sets recurse_func to nullptr already, so it doesn't get
         * past the previous check.  For GSUB, we only want to collect the output
         * glyphs in the recursion.  If output is not requested, we can go home now.
         *
         * Note further, that the above is not exactly correct.  A recursed lookup
         * is allowed to match input that is not matched in the context, but that's
         * not how most fonts are built.  It's possible to relax that and recurse
         * with all sets here if it proves to be an issue.
         */

        if self.output.is_none() {
            return;
        }

        /* Return if new lookup was recursed to before. */
        if self.recursed_lookups.has(lookup_index) {
            return;
        }

        let old_before = self.before.take();
        let old_input = self.input.take();
        let old_after = self.after.take();

        self.nesting_level_left -= 1;
        /* SubstLookup::dispatch_recurse_func<hb_collect_glyphs_context_t> */
        let face = self.face;
        let l = face.gsub().table().get_lookup(lookup_index);
        hb_ot_layout_gsub::subst_lookup_collect_glyphs(self, l);
        self.nesting_level_left += 1;

        self.before = old_before;
        self.input = old_input;
        self.after = old_after;

        self.recursed_lookups.add(lookup_index);
    }
}

/* hb_ot_apply_context_t */

/// `matcher_t::may_match_t`
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MayMatch {
    No,
    Yes,
    Maybe,
}

/// `matcher_t::may_skip_t`
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaySkip {
    No,
    Yes,
    Maybe,
}

/// `matcher_t`
#[derive(Clone, Copy)]
pub(crate) struct Matcher<'a> {
    lookup_props: u32,
    mask: HbMask,
    ignore_zwnj: bool,
    ignore_zwj: bool,
    per_syllable: bool,
    syllable: u8,
    match_func: Option<MatchFunc<'a>>,
}

impl<'a> Default for Matcher<'a> {
    fn default() -> Self {
        Matcher {
            lookup_props: 0,
            mask: u32::MAX,
            ignore_zwnj: false,
            ignore_zwj: false,
            per_syllable: false,
            syllable: 0,
            match_func: None,
        }
    }
}

impl<'a> Matcher<'a> {
    fn set_syllable(&mut self, syllable: u8) {
        self.syllable = if self.per_syllable { syllable } else { 0 };
    }

    /// `may_match`
    #[inline]
    fn may_match(&self, info: &HbGlyphInfo, glyph_data: HbCodepoint) -> MayMatch {
        if info.mask & self.mask == 0 || (self.syllable != 0 && self.syllable != info.syllable()) {
            return MayMatch::No;
        }

        if let Some(f) = self.match_func {
            return if f.matches(info, glyph_data) {
                MayMatch::Yes
            } else {
                MayMatch::No
            };
        }

        MayMatch::Maybe
    }

    /// `may_skip`
    #[inline]
    fn may_skip(&self, gdef: &GdefAccel, info: &HbGlyphInfo) -> MaySkip {
        if !check_glyph_property(gdef, info, self.lookup_props) {
            return MaySkip::Yes;
        }

        if _hb_glyph_info_is_default_ignorable_and_not_hidden(info)
            && (self.ignore_zwnj || !_hb_glyph_info_is_zwnj(info))
            && (self.ignore_zwj || !_hb_glyph_info_is_zwj(info))
        {
            return MaySkip::Maybe;
        }

        MaySkip::No
    }
}

/// `skipping_iterator_t::match_t`
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum IterMatch {
    Match,
    NotMatch,
    Skip,
}

/// `skipping_iterator_t`
#[derive(Clone, Copy, Default)]
pub(crate) struct SkippingIterator<'a> {
    pub(crate) idx: u32,
    matcher: Matcher<'a>,
    /// `match_glyph_data16`: the u16 array and the current item
    match_glyph_data: Option<(&'a [u8], usize)>,
    end: u32,
}

/// Which skipping iterator of the context.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Iter {
    Input,
    Context,
}

impl<'a> SkippingIterator<'a> {
    /// `set_lookup_props`
    pub(crate) fn set_lookup_props(&mut self, lookup_props: u32) {
        self.matcher.lookup_props = lookup_props;
    }
    /// `set_match_func`
    pub(crate) fn set_match_func(&mut self, match_func: Option<MatchFunc<'a>>) {
        self.matcher.match_func = match_func;
    }
    /// `set_glyph_data`
    pub(crate) fn set_glyph_data(&mut self, glyph_data: Option<&'a [u8]>) {
        self.match_glyph_data = glyph_data.map(|d| (d, 0));
    }

    /// `get_glyph_data`
    #[inline]
    fn get_glyph_data(&self) -> HbCodepoint {
        match self.match_glyph_data {
            Some((d, i)) => u16_at(d, i * 2) as u32,
            None => 0,
        }
    }
    /// `advance_glyph_data`
    #[inline]
    fn advance_glyph_data(&mut self) {
        if let Some((_, i)) = &mut self.match_glyph_data {
            *i += 1;
        }
    }

    /// `may_skip`
    #[inline]
    pub(crate) fn may_skip(&self, gdef: &GdefAccel, info: &HbGlyphInfo) -> MaySkip {
        self.matcher.may_skip(gdef, info)
    }

    /// `match`
    #[inline]
    fn match_(&self, gdef: &GdefAccel, info: &HbGlyphInfo) -> IterMatch {
        let skip = self.matcher.may_skip(gdef, info);
        if skip == MaySkip::Yes {
            return IterMatch::Skip;
        }

        let m = self.matcher.may_match(info, self.get_glyph_data());
        if m == MayMatch::Yes || (m == MayMatch::Maybe && skip == MaySkip::No) {
            return IterMatch::Match;
        }

        if skip == MaySkip::No {
            return IterMatch::NotMatch;
        }

        IterMatch::Skip
    }

    /// `next`
    pub(crate) fn next(
        &mut self,
        buffer: &HbBuffer,
        gdef: &GdefAccel,
        unsafe_to: Option<&mut u32>,
    ) -> bool {
        let stop = self.end as i32 - 1;
        while (self.idx as i32) < stop {
            self.idx += 1;
            match self.match_(gdef, &buffer.info[self.idx as usize]) {
                IterMatch::Match => {
                    self.advance_glyph_data();
                    return true;
                }
                IterMatch::NotMatch => {
                    if let Some(u) = unsafe_to {
                        *u = self.idx + 1;
                    }
                    return false;
                }
                IterMatch::Skip => continue,
            }
        }
        if let Some(u) = unsafe_to {
            *u = self.end;
        }
        false
    }

    /// `prev`
    pub(crate) fn prev(
        &mut self,
        buffer: &HbBuffer,
        gdef: &GdefAccel,
        unsafe_from: Option<&mut u32>,
    ) -> bool {
        let stop = 0;
        while self.idx > stop {
            self.idx -= 1;
            match self.match_(gdef, buffer.out_info(self.idx)) {
                IterMatch::Match => {
                    self.advance_glyph_data();
                    return true;
                }
                IterMatch::NotMatch => {
                    if let Some(u) = unsafe_from {
                        *u = self.idx.max(1) - 1;
                    }
                    return false;
                }
                IterMatch::Skip => continue,
            }
        }
        if let Some(u) = unsafe_from {
            *u = 0;
        }
        false
    }
}

/// `check_glyph_property`
#[inline]
pub(crate) fn check_glyph_property(gdef: &GdefAccel, info: &HbGlyphInfo, match_props: u32) -> bool {
    let glyph_props = _hb_glyph_info_get_glyph_props(info);

    /* Not covered, if, for example, glyph class is ligature and
     * match_props includes LookupFlags::IgnoreLigatures
     */
    if glyph_props & match_props & LOOKUP_FLAG_IGNORE_FLAGS != 0 {
        return false;
    }

    if glyph_props & HB_OT_LAYOUT_GLYPH_PROPS_MARK != 0 {
        return match_properties_mark(gdef, info.codepoint, glyph_props, match_props);
    }

    true
}

/// `match_properties_mark`
#[inline]
fn match_properties_mark(
    gdef: &GdefAccel,
    glyph: HbCodepoint,
    glyph_props: u32,
    match_props: u32,
) -> bool {
    /* If using mark filtering sets, the high short of
     * match_props has the set index.
     */
    if match_props & LOOKUP_FLAG_USE_MARK_FILTERING_SET != 0 {
        return gdef.mark_set_covers(match_props >> 16, glyph);
    }

    /* The second byte of match_props has the meaning
     * "ignore marks of attachment type different than
     * the attachment type specified."
     */
    if match_props & LOOKUP_FLAG_MARK_ATTACHMENT_TYPE != 0 {
        return (match_props & LOOKUP_FLAG_MARK_ATTACHMENT_TYPE)
            == (glyph_props & LOOKUP_FLAG_MARK_ATTACHMENT_TYPE);
    }

    true
}

/// `hb_ot_apply_context_t`
pub(crate) struct HbOtApplyContext<'a, 'b, 'f> {
    pub(crate) iter_input: SkippingIterator<'a>,
    pub(crate) iter_context: SkippingIterator<'a>,

    pub(crate) table_index: u32, /* GSUB/GPOS */
    pub(crate) font: &'b mut HbFont<'f>,
    pub(crate) face: &'a HbFace,
    pub(crate) buffer: &'b mut HbBuffer,
    pub(crate) sanitizer: HbSanitizeContext<'a>,
    /// the table (C's `recurse_func` is its lookups' `dispatch_recurse_func`)
    pub(crate) accel: &'a GsubGposAccel,
    pub(crate) gdef: Gdef<'a>,
    pub(crate) gdef_accel: &'a GdefAccel,
    pub(crate) var_store: ItemVariationStore<'a>,
    pub(crate) var_store_cache: Option<Vec<f32>>,

    pub(crate) direction: HbDirection,
    pub(crate) lookup_mask: HbMask,
    pub(crate) lookup_index: u32,
    pub(crate) lookup_props: u32,
    pub(crate) nesting_level_left: u32,

    pub(crate) has_glyph_classes: bool,
    pub(crate) auto_zwnj: bool,
    pub(crate) auto_zwj: bool,
    pub(crate) per_syllable: bool,
    pub(crate) random: bool,
    pub(crate) new_syllables: u32,

    pub(crate) last_base: i32,       // GPOS uses
    pub(crate) last_base_until: u32, // GPOS uses
}

impl<'a, 'b, 'f> HbOtApplyContext<'a, 'b, 'f> {
    /// `hb_ot_apply_context_t (table_index, font, buffer, table_blob)`
    pub(crate) fn new(
        table_index: u32,
        font: &'b mut HbFont<'f>,
        face: &'a HbFace,
        buffer: &'b mut HbBuffer,
        accel: &'a GsubGposAccel,
    ) -> HbOtApplyContext<'a, 'b, 'f> {
        let gdef_accel = face.gdef();
        let gdef = gdef_accel.table();
        let var_store = gdef.get_var_store();
        let var_store_cache = if table_index == 1 && font.p.num_coords() != 0 {
            Some(var_store.create_cache())
        } else {
            None
        };
        let direction = buffer.props.direction;
        /* (the sanitizer is started on the table blob) */
        let mut sanitizer = HbSanitizeContext::new(Cow::Borrowed(&accel.table[..]));
        if !accel.table.is_empty() {
            sanitizer.start_processing();
        }
        let mut c = HbOtApplyContext {
            iter_input: SkippingIterator::default(),
            iter_context: SkippingIterator::default(),
            table_index,
            font,
            face,
            buffer,
            sanitizer,
            accel,
            gdef,
            gdef_accel,
            var_store,
            var_store_cache,
            direction,
            lookup_mask: 1,
            lookup_index: u32::MAX,
            lookup_props: 0,
            nesting_level_left: HB_MAX_NESTING_LEVEL,
            has_glyph_classes: gdef.has_glyph_classes(),
            auto_zwnj: true,
            auto_zwj: true,
            per_syllable: false,
            random: false,
            new_syllables: u32::MAX,
            last_base: -1,
            last_base_until: 0,
        };
        c.init_iters();
        c
    }

    /// `skipping_iterator_t::init`
    fn init_iter(&self, context_match: bool) -> SkippingIterator<'a> {
        let mut it = SkippingIterator {
            idx: 0,
            matcher: Matcher::default(),
            match_glyph_data: None,
            end: self.buffer.len,
        };
        it.matcher.match_func = None;
        it.matcher.lookup_props = self.lookup_props;
        /* Ignore ZWNJ if we are matching GPOS, or matching GSUB context and asked to. */
        it.matcher.ignore_zwnj = self.table_index == 1 || (context_match && self.auto_zwnj);
        /* Ignore ZWJ if we are matching context, or asked to. */
        it.matcher.ignore_zwj = context_match || self.auto_zwj;
        it.matcher.mask = if context_match {
            u32::MAX
        } else {
            self.lookup_mask
        };
        /* Per syllable matching is only for GSUB. */
        it.matcher.per_syllable = self.table_index == 0 && self.per_syllable;
        it.matcher.set_syllable(0);
        it
    }

    /// `init_iters`
    pub(crate) fn init_iters(&mut self) {
        self.iter_input = self.init_iter(false);
        self.iter_context = self.init_iter(true);
    }

    pub(crate) fn set_lookup_mask(&mut self, mask: HbMask, init: bool) {
        self.lookup_mask = mask;
        self.last_base = -1;
        self.last_base_until = 0;
        if init {
            self.init_iters();
        }
    }
    pub(crate) fn set_auto_zwj(&mut self, auto_zwj: bool, init: bool) {
        self.auto_zwj = auto_zwj;
        if init {
            self.init_iters();
        }
    }
    pub(crate) fn set_auto_zwnj(&mut self, auto_zwnj: bool, init: bool) {
        self.auto_zwnj = auto_zwnj;
        if init {
            self.init_iters();
        }
    }
    pub(crate) fn set_per_syllable(&mut self, per_syllable: bool, init: bool) {
        self.per_syllable = per_syllable;
        if init {
            self.init_iters();
        }
    }
    pub(crate) fn set_random(&mut self, random: bool) {
        self.random = random;
    }
    pub(crate) fn set_lookup_index(&mut self, lookup_index: u32) {
        self.lookup_index = lookup_index;
    }
    pub(crate) fn set_lookup_props(&mut self, lookup_props: u32) {
        self.lookup_props = lookup_props;
        self.init_iters();
    }

    /* The skipping iterators (C's `c->iter_input` and `c->iter_context`) */

    /// The iterator.
    #[inline]
    pub(crate) fn iter(&mut self, which: Iter) -> &mut SkippingIterator<'a> {
        match which {
            Iter::Input => &mut self.iter_input,
            Iter::Context => &mut self.iter_context,
        }
    }

    /// `skippy_iter.reset (start_index)`
    pub(crate) fn iter_reset(&mut self, which: Iter, start_index: u32) {
        let syllable = if start_index == self.buffer.idx {
            self.buffer.cur(0).syllable()
        } else {
            0
        };
        let end = self.buffer.len;
        let it = self.iter(which);
        it.idx = start_index;
        it.end = end;
        it.matcher.set_syllable(syllable);
    }

    /// `skippy_iter.reset_fast (start_index)`
    pub(crate) fn iter_reset_fast(&mut self, which: Iter, start_index: u32) {
        // Doesn't set end or syllable. Used by GPOS which doesn't care / change.
        self.iter(which).idx = start_index;
    }

    /// `skippy_iter.next (unsafe_to)`
    pub(crate) fn iter_next(&mut self, which: Iter, unsafe_to: Option<&mut u32>) -> bool {
        let gdef = self.gdef_accel;
        let it = match which {
            Iter::Input => &mut self.iter_input,
            Iter::Context => &mut self.iter_context,
        };
        it.next(self.buffer, gdef, unsafe_to)
    }

    /// `skippy_iter.prev (unsafe_from)`
    pub(crate) fn iter_prev(&mut self, which: Iter, unsafe_from: Option<&mut u32>) -> bool {
        let gdef = self.gdef_accel;
        let it = match which {
            Iter::Input => &mut self.iter_input,
            Iter::Context => &mut self.iter_context,
        };
        it.prev(self.buffer, gdef, unsafe_from)
    }

    /// `skippy_iter.match (buffer->info[idx])`
    pub(crate) fn iter_match(&self, which: Iter, idx: u32) -> IterMatch {
        let info = &self.buffer.info[idx as usize];
        match which {
            Iter::Input => self.iter_input.match_(self.gdef_accel, info),
            Iter::Context => self.iter_context.match_(self.gdef_accel, info),
        }
    }

    /// `skippy_iter.may_skip (info)`
    pub(crate) fn iter_may_skip(&self, which: Iter, info: &HbGlyphInfo) -> MaySkip {
        match which {
            Iter::Input => self.iter_input.may_skip(self.gdef_accel, info),
            Iter::Context => self.iter_context.may_skip(self.gdef_accel, info),
        }
    }

    /// `recurse`
    pub(crate) fn recurse(&mut self, sub_lookup_index: u32) -> bool {
        if self.nesting_level_left == 0 || {
            let ops = self.buffer.max_ops;
            self.buffer.max_ops = ops.wrapping_sub(1);
            ops <= 0
        } {
            self.buffer.shaping_failed = true;
            return false;
        }

        self.nesting_level_left -= 1;
        let ret = self.dispatch_recurse_func(sub_lookup_index);
        self.nesting_level_left += 1;
        ret
    }

    /// `SubstLookup::dispatch_recurse_func<hb_ot_apply_context_t>` /
    /// `PosLookup::dispatch_recurse_func<hb_ot_apply_context_t>`
    fn dispatch_recurse_func(&mut self, lookup_index: u32) -> bool {
        let accel = self.accel;
        let l = accel.table().get_lookup(lookup_index);
        let saved_lookup_props = self.lookup_props;
        let saved_lookup_index = self.lookup_index;
        self.set_lookup_index(lookup_index);
        self.set_lookup_props(l.get_props());

        let mut ret = false;
        if lookup_index < accel.lookup_count {
            ret = self.apply_lookup_subtables(l);
        }

        self.set_lookup_index(saved_lookup_index);
        self.set_lookup_props(saved_lookup_props);
        ret
    }

    /// `hb_ot_layout_lookup_accelerator_t::apply`: the first subtable of
    /// the lookup that applies.
    pub(crate) fn apply_lookup_subtables(&mut self, l: Lookup<'a>) -> bool {
        let lookup_type = l.get_type();
        let count = l.get_subtable_count();
        for i in 0..count {
            let st = l.get_subtable(i);
            let applied = match self.accel.kind {
                GsubGposKind::Gsub => {
                    hb_ot_layout_gsub::subst_lookup_subtable_apply(self, st, lookup_type)
                }
                GsubGposKind::Gpos => {
                    hb_ot_layout_gpos::pos_lookup_subtable_apply(self, st, lookup_type)
                }
            };
            if applied {
                return true;
            }
        }
        false
    }

    /// `random_number`
    pub(crate) fn random_number(&mut self) -> u32 {
        /* http://www.cplusplus.com/reference/random/minstd_rand/ */
        self.buffer.random_state = ((self.buffer.random_state as u64 * 48271) % 2147483647) as u32;
        self.buffer.random_state
    }

    /// `check_glyph_property`
    #[inline]
    pub(crate) fn check_glyph_property(&self, info: &HbGlyphInfo, match_props: u32) -> bool {
        check_glyph_property(self.gdef_accel, info, match_props)
    }

    /// `_set_glyph_class`
    pub(crate) fn _set_glyph_class(
        &mut self,
        glyph_index: HbCodepoint,
        class_guess: u32,
        ligature: bool,
        component: bool,
    ) {
        if self.new_syllables != u32::MAX {
            let s = self.new_syllables as u8;
            self.buffer.cur_mut(0).set_syllable(s);
        }

        let mut props = _hb_glyph_info_get_glyph_props(self.buffer.cur(0));
        props |= HB_OT_LAYOUT_GLYPH_PROPS_SUBSTITUTED;
        if ligature {
            props |= HB_OT_LAYOUT_GLYPH_PROPS_LIGATED;
            /* In the only place that the MULTIPLIED bit is used, Uniscribe
             * seems to only care about the "last" transformation between
             * Ligature and Multiple substitutions.  Ie. if you ligate, expand,
             * and ligate again, it forgives the multiplication and acts as
             * if only ligation happened.  As such, clear MULTIPLIED bit.
             */
            props &= !HB_OT_LAYOUT_GLYPH_PROPS_MULTIPLIED;
        }
        if component {
            props |= HB_OT_LAYOUT_GLYPH_PROPS_MULTIPLIED;
        }
        if self.has_glyph_classes {
            props &= HB_OT_LAYOUT_GLYPH_PROPS_PRESERVE;
            let p = props | self.gdef_accel.get_glyph_props(glyph_index);
            _hb_glyph_info_set_glyph_props(self.buffer.cur_mut(0), p);
        } else if class_guess != 0 {
            props &= HB_OT_LAYOUT_GLYPH_PROPS_PRESERVE;
            _hb_glyph_info_set_glyph_props(self.buffer.cur_mut(0), props | class_guess);
        } else {
            _hb_glyph_info_set_glyph_props(self.buffer.cur_mut(0), props);
        }
    }

    /// `replace_glyph`
    pub(crate) fn replace_glyph(&mut self, glyph_index: HbCodepoint) {
        self._set_glyph_class(glyph_index, 0, false, false);
        let _ = self.buffer.replace_glyph(glyph_index);
    }
    /// `replace_glyph_inplace`
    pub(crate) fn replace_glyph_inplace(&mut self, glyph_index: HbCodepoint) {
        self._set_glyph_class(glyph_index, 0, false, false);
        self.buffer.cur_mut(0).codepoint = glyph_index;
    }
    /// `replace_glyph_with_ligature`
    pub(crate) fn replace_glyph_with_ligature(
        &mut self,
        glyph_index: HbCodepoint,
        class_guess: u32,
    ) {
        self._set_glyph_class(glyph_index, class_guess, true, false);
        let _ = self.buffer.replace_glyph(glyph_index);
    }
    /// `output_glyph_for_component`
    pub(crate) fn output_glyph_for_component(
        &mut self,
        glyph_index: HbCodepoint,
        class_guess: u32,
    ) {
        self._set_glyph_class(glyph_index, class_guess, false, true);
        let _ = self.buffer.output_glyph(glyph_index);
    }
}

/* Matching */

/// `would_match_input`
fn would_match_input(
    c: &HbWouldApplyContext,
    count: u32,   /* Including the first glyph (not matched) */
    input: &[u8], /* Array of input values--start with second glyph */
    match_func: MatchFunc,
) -> bool {
    if count != c.len {
        return false;
    }

    for i in 1..count as usize {
        let info = HbGlyphInfo {
            codepoint: c.glyphs[i],
            ..Default::default()
        };
        if !match_func.matches(&info, u16_at(input, (i - 1) * 2) as u32) {
            return false;
        }
    }

    true
}

/// `match_input`
#[allow(clippy::too_many_arguments)]
pub(crate) fn match_input<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    count: u32,      /* Including the first glyph (not matched) */
    input: &'a [u8], /* Array of input values--start with second glyph */
    match_func: MatchFunc<'a>,
    end_position: &mut u32,
    match_positions: &mut [u32],
    p_total_component_count: Option<&mut u32>,
) -> bool {
    if count > HB_MAX_CONTEXT_LENGTH {
        return false;
    }

    let idx = c.buffer.idx;
    c.iter_reset(Iter::Input, idx);
    c.iter_input.set_match_func(Some(match_func));
    c.iter_input.set_glyph_data(Some(input));

    /*
     * This is perhaps the trickiest part of OpenType...  Remarks:
     *
     * - If all components of the ligature were marks, we call this a mark ligature.
     *
     * - If there is no GDEF, and the ligature is NOT a mark ligature, we categorize
     *   it as a ligature glyph.
     *
     * - Ligatures cannot be formed across glyphs attached to different components
     *   of previous ligatures.  Eg. the sequence is LAM,SHADDA,LAM,FATHA,HEH, and
     *   LAM,LAM,HEH form a ligature, leaving SHADDA,FATHA next to eachother.
     *   However, it would be wrong to ligate that SHADDA,FATHA sequence.
     *   There are a couple of exceptions to this:
     *
     *   o If a ligature tries ligating with marks that belong to it itself, go ahead,
     *     assuming that the font designer knows what they are doing (otherwise it can
     *     break Indic stuff when a matra wants to ligate with a conjunct,
     *
     *   o If two marks want to ligate and they belong to different components of the
     *     same ligature glyph, and said ligature glyph is to be ignored according to
     *     mark-filtering rules, then allow.
     *     https://github.com/harfbuzz/harfbuzz/issues/545
     */

    let mut total_component_count = 0;

    let first_lig_id = _hb_glyph_info_get_lig_id(c.buffer.cur(0));
    let first_lig_comp = _hb_glyph_info_get_lig_comp(c.buffer.cur(0));

    #[derive(PartialEq, Eq)]
    enum LigBase {
        NotChecked,
        MayNotSkip,
        MaySkip,
    }
    let mut ligbase = LigBase::NotChecked;

    for i in 1..count as usize {
        let mut unsafe_to = 0;
        if !c.iter_next(Iter::Input, Some(&mut unsafe_to)) {
            *end_position = unsafe_to;
            return false;
        }

        let sidx = c.iter_input.idx;
        match_positions[i] = sidx;

        let this_lig_id = _hb_glyph_info_get_lig_id(&c.buffer.info[sidx as usize]);
        let this_lig_comp = _hb_glyph_info_get_lig_comp(&c.buffer.info[sidx as usize]);

        if first_lig_id != 0 && first_lig_comp != 0 {
            /* If first component was attached to a previous ligature component,
             * all subsequent components should be attached to the same ligature
             * component, otherwise we shouldn't ligate them... */
            if first_lig_id != this_lig_id || first_lig_comp != this_lig_comp {
                /* ...unless, we are attached to a base ligature and that base
                 * ligature is ignorable. */
                if ligbase == LigBase::NotChecked {
                    let mut found = false;
                    let mut j = c.buffer.out_len;
                    while j != 0
                        && _hb_glyph_info_get_lig_id(c.buffer.out_info(j - 1)) == first_lig_id
                    {
                        if _hb_glyph_info_get_lig_comp(c.buffer.out_info(j - 1)) == 0 {
                            j -= 1;
                            found = true;
                            break;
                        }
                        j -= 1;
                    }

                    if found && c.iter_may_skip(Iter::Input, c.buffer.out_info(j)) == MaySkip::Yes {
                        ligbase = LigBase::MaySkip;
                    } else {
                        ligbase = LigBase::MayNotSkip;
                    }
                }

                if ligbase == LigBase::MayNotSkip {
                    return false;
                }
            }
        } else {
            /* If first component was NOT attached to a previous ligature component,
             * all subsequent components should also NOT be attached to any ligature
             * component, unless they are attached to the first component itself! */
            if this_lig_id != 0 && this_lig_comp != 0 && this_lig_id != first_lig_id {
                return false;
            }
        }

        total_component_count += _hb_glyph_info_get_lig_num_comps(&c.buffer.info[sidx as usize]);
    }

    *end_position = c.iter_input.idx + 1;

    if let Some(p) = p_total_component_count {
        total_component_count += _hb_glyph_info_get_lig_num_comps(c.buffer.cur(0));
        *p = total_component_count;
    }

    match_positions[0] = c.buffer.idx;

    true
}

/// `ligate_input`
pub(crate) fn ligate_input(
    c: &mut HbOtApplyContext,
    count: u32,              /* Including the first glyph */
    match_positions: &[u32], /* Including the first glyph */
    match_end: u32,
    lig_glyph: HbCodepoint,
    total_component_count: u32,
) -> bool {
    let idx = c.buffer.idx;
    c.buffer.merge_clusters(idx, match_end);

    /* - If a base and one or more marks ligate, consider that as a base, NOT
     *   ligature, such that all following marks can still attach to it.
     *   https://github.com/harfbuzz/harfbuzz/issues/1109
     *
     * - If all components of the ligature were marks, we call this a mark ligature.
     *   If it *is* a mark ligature, we don't allocate a new ligature id, and leave
     *   the ligature to keep its old ligature id.  This will allow it to attach to
     *   a base ligature in GPOS.  Eg. if the sequence is: LAM,LAM,SHADDA,FATHA,HEH,
     *   and LAM,LAM,HEH for a ligature, they will leave SHADDA and FATHA with a
     *   ligature id and component value of 2.  Then if SHADDA,FATHA form a ligature
     *   later, we don't want them to lose their ligature id/component, otherwise
     *   GPOS will fail to correctly position the mark ligature on top of the
     *   LAM,LAM,HEH ligature.  See:
     *     https://bugzilla.gnome.org/show_bug.cgi?id=676343
     *
     * - If a ligature is formed of components that some of which are also ligatures
     *   themselves, and those ligature components had marks attached to *their*
     *   components, we have to attach the marks to the new ligature component
     *   positions!  Now *that*'s tricky!  And these marks may be following the
     *   last component of the whole sequence, so we should loop forward looking
     *   for them and update them.
     *
     *   Eg. the sequence is LAM,LAM,SHADDA,FATHA,HEH, and the font first forms a
     *   'calt' ligature of LAM,HEH, leaving the SHADDA and FATHA with a ligature
     *   id and component == 1.  Now, during 'liga', the LAM and the LAM-HEH ligature
     *   form a LAM-LAM-HEH ligature.  We need to reassign the SHADDA and FATHA to
     *   the new ligature with a component value of 2.
     *
     *   This in fact happened to a font...  See:
     *   https://bugzilla.gnome.org/show_bug.cgi?id=437633
     */

    let mut is_base_ligature =
        _hb_glyph_info_is_base_glyph(&c.buffer.info[match_positions[0] as usize]);
    let mut is_mark_ligature = _hb_glyph_info_is_mark(&c.buffer.info[match_positions[0] as usize]);
    for i in 1..count as usize {
        if !_hb_glyph_info_is_mark(&c.buffer.info[match_positions[i] as usize]) {
            is_base_ligature = false;
            is_mark_ligature = false;
            break;
        }
    }
    let is_ligature = !is_base_ligature && !is_mark_ligature;

    let klass = if is_ligature {
        HB_OT_LAYOUT_GLYPH_PROPS_LIGATURE
    } else {
        0
    };
    let lig_id = if is_ligature {
        _hb_allocate_lig_id(c.buffer) as u32
    } else {
        0
    };
    let mut last_lig_id = _hb_glyph_info_get_lig_id(c.buffer.cur(0));
    let mut last_num_components = _hb_glyph_info_get_lig_num_comps(c.buffer.cur(0));
    let mut components_so_far = last_num_components;

    if is_ligature {
        _hb_glyph_info_set_lig_props_for_ligature(
            c.buffer.cur_mut(0),
            lig_id,
            total_component_count,
        );
        if _hb_glyph_info_get_general_category(c.buffer.cur(0))
            == HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK
        {
            _hb_glyph_info_set_general_category(
                c.buffer.cur_mut(0),
                HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER,
            );
        }
    }
    c.replace_glyph_with_ligature(lig_glyph, klass);

    for i in 1..count as usize {
        while c.buffer.idx < match_positions[i] && c.buffer.successful {
            if is_ligature {
                let mut this_comp = _hb_glyph_info_get_lig_comp(c.buffer.cur(0));
                if this_comp == 0 {
                    this_comp = last_num_components;
                }
                let new_lig_comp =
                    components_so_far - last_num_components + this_comp.min(last_num_components);
                _hb_glyph_info_set_lig_props_for_mark(c.buffer.cur_mut(0), lig_id, new_lig_comp);
            }
            let _ = c.buffer.next_glyph();
        }

        last_lig_id = _hb_glyph_info_get_lig_id(c.buffer.cur(0));
        last_num_components = _hb_glyph_info_get_lig_num_comps(c.buffer.cur(0));
        components_so_far += last_num_components;

        /* Skip the base glyph */
        c.buffer.idx += 1;
    }

    if !is_mark_ligature && last_lig_id != 0 {
        /* Re-adjust components for any marks following. */
        for i in c.buffer.idx..c.buffer.len {
            if last_lig_id != _hb_glyph_info_get_lig_id(&c.buffer.info[i as usize]) {
                break;
            }

            let this_comp = _hb_glyph_info_get_lig_comp(&c.buffer.info[i as usize]);
            if this_comp == 0 {
                break;
            }

            let new_lig_comp =
                components_so_far - last_num_components + this_comp.min(last_num_components);
            _hb_glyph_info_set_lig_props_for_mark(
                &mut c.buffer.info[i as usize],
                lig_id,
                new_lig_comp,
            );
        }
    }
    true
}

/// `match_backtrack`
pub(crate) fn match_backtrack<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    count: u32,
    backtrack: &'a [u8],
    match_func: MatchFunc<'a>,
    match_start: &mut u32,
) -> bool {
    let bl = c.buffer.backtrack_len();
    c.iter_reset(Iter::Context, bl);
    c.iter_context.set_match_func(Some(match_func));
    c.iter_context.set_glyph_data(Some(backtrack));

    for _ in 0..count {
        let mut unsafe_from = 0;
        if !c.iter_prev(Iter::Context, Some(&mut unsafe_from)) {
            *match_start = unsafe_from;
            return false;
        }
    }

    *match_start = c.iter_context.idx;
    true
}

/// `match_lookahead`
pub(crate) fn match_lookahead<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    count: u32,
    lookahead: &'a [u8],
    match_func: MatchFunc<'a>,
    start_index: u32,
    end_index: &mut u32,
) -> bool {
    c.iter_reset(Iter::Context, start_index.wrapping_sub(1));
    c.iter_context.set_match_func(Some(match_func));
    c.iter_context.set_glyph_data(Some(lookahead));

    for _ in 0..count {
        let mut unsafe_to = 0;
        if !c.iter_next(Iter::Context, Some(&mut unsafe_to)) {
            *end_index = unsafe_to;
            return false;
        }
    }

    *end_index = c.iter_context.idx + 1;
    true
}

/* LookupRecord (sequenceIndex, lookupListIndex: 4 bytes) */

/// `LookupRecord`: the (sequence index, lookup list index) of record `i`
/// of the array `records`.
#[inline]
fn lookup_record(records: &[u8], i: u32) -> (u32, u32) {
    let o = i as usize * 4;
    (u16_at(records, o) as u32, u16_at(records, o + 2) as u32)
}

/// `recurse_lookups` (collect glyphs)
fn recurse_lookups(c: &mut HbCollectGlyphsContext, lookup_count: u32, lookup_record_: &[u8]) {
    for i in 0..lookup_count {
        c.recurse(lookup_record(lookup_record_, i).1);
    }
}

/// `apply_lookup`
pub(crate) fn apply_lookup(
    c: &mut HbOtApplyContext,
    count: u32,                     /* Including the first glyph */
    match_positions: &mut Vec<u32>, /* Including the first glyph */
    lookup_count: u32,
    lookup_record_: &[u8], /* Array of LookupRecords--in design order */
    match_end: u32,
) {
    let mut count = count;
    let mut end: i32;

    /* All positions are distance from beginning of *output* buffer.
     * Adjust. */
    {
        let bl = c.buffer.backtrack_len();
        end = (bl + match_end) as i32 - c.buffer.idx as i32;

        let delta = bl as i32 - c.buffer.idx as i32;
        /* Convert positions to new indexing. */
        for j in 0..count as usize {
            match_positions[j] = (match_positions[j] as i32 + delta) as u32;
        }
    }

    let mut i = 0;
    while i < lookup_count && c.buffer.successful {
        let (idx, lookup_list_index) = lookup_record(lookup_record_, i);
        i += 1;
        if idx >= count {
            continue;
        }

        let orig_len = c.buffer.backtrack_len() + c.buffer.lookahead_len();

        /* This can happen if earlier recursed lookups deleted many entries. */
        if match_positions[idx as usize] >= orig_len {
            continue;
        }

        if !c.buffer.move_to(match_positions[idx as usize]) {
            break;
        }

        if c.buffer.max_ops <= 0 {
            break;
        }

        if !c.recurse(lookup_list_index) {
            continue;
        }

        let new_len = c.buffer.backtrack_len() + c.buffer.lookahead_len();
        let mut delta = new_len as i32 - orig_len as i32;

        if delta == 0 {
            continue;
        }

        /* Recursed lookup changed buffer len.  Adjust.
         *
         * TODO:
         *
         * Right now, if buffer length increased by n, we assume n new glyphs
         * were added right after the current position, and if buffer length
         * was decreased by n, we assume n match positions after the current
         * one where removed.  The former (buffer length increased) case is
         * fine, but the decrease case can be improved in at least two ways,
         * both of which are significant:
         *
         *   - If recursed-to lookup is MultipleSubst and buffer length
         *     decreased, then it's current match position that was deleted,
         *     NOT the one after it.
         *
         *   - If buffer length was decreased by n, it does not necessarily
         *     mean that n match positions where removed, as there recursed-to
         *     lookup might had a different LookupFlag.  Here's a constructed
         *     case of that:
         *     https://github.com/harfbuzz/harfbuzz/discussions/3538
         *
         * It should be possible to construct tests for both of these cases.
         */

        end += delta;
        if end < match_positions[idx as usize] as i32 {
            /* End might end up being smaller than match_positions[idx] if the recursed
             * lookup ended up removing many items.
             * Just never rewind end beyond start of current position, since that is
             * not possible in the recursed lookup.  Also adjust delta as such.
             *
             * https://bugs.chromium.org/p/chromium/issues/detail?id=659496
             * https://github.com/harfbuzz/harfbuzz/issues/1611
             */
            delta += match_positions[idx as usize] as i32 - end;
            end = match_positions[idx as usize] as i32;
        }

        let mut next = idx + 1; /* next now is the position after the recursed lookup. */

        if delta > 0 {
            if delta as u32 + count > HB_MAX_CONTEXT_LENGTH {
                break;
            }
            if match_positions.len() < (delta as u32 + count) as usize {
                match_positions.resize((delta as u32 + count) as usize, 0);
            }
        } else {
            /* NOTE: delta is non-positive. */
            delta = delta.max(next as i32 - count as i32);
            next = (next as i32 - delta) as u32;
        }

        /* Shift! */
        let n = (count - next) as usize;
        let dst = (next as i32 + delta) as usize;
        match_positions.copy_within(next as usize..next as usize + n, dst);
        next = (next as i32 + delta) as u32;
        count = (count as i32 + delta) as u32;

        /* Fill in new entries. */
        for j in idx + 1..next {
            match_positions[j as usize] = match_positions[(j - 1) as usize] + 1;
        }

        /* And fixup the rest. */
        while next < count {
            match_positions[next as usize] = (match_positions[next as usize] as i32 + delta) as u32;
            next += 1;
        }
    }

    let _ = c.buffer.move_to(end as u32);
}

/* Contextual lookups */

/// `context_apply_lookup`
fn context_apply_lookup<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    input_count: u32, /* Including the first glyph (not matched) */
    input: &'a [u8],  /* Array of input values--start with second glyph */
    lookup_count: u32,
    lookup_record_: &[u8],
    match_func: MatchFunc<'a>,
) -> bool {
    if input_count > HB_MAX_CONTEXT_LENGTH {
        return false;
    }
    let mut match_positions = vec![0u32; input_count.max(1).max(4) as usize];

    let mut match_end = 0;
    let ret;
    if match_input(
        c,
        input_count,
        input,
        match_func,
        &mut match_end,
        &mut match_positions,
        None,
    ) {
        let idx = c.buffer.idx;
        c.buffer.unsafe_to_break(idx, match_end);
        apply_lookup(
            c,
            input_count,
            &mut match_positions,
            lookup_count,
            lookup_record_,
            match_end,
        );
        ret = true;
    } else {
        let idx = c.buffer.idx;
        c.buffer.unsafe_to_concat(idx, match_end);
        ret = false;
    }

    ret
}

/// `Rule<SmallTypes>`: inputCount, lookupCount, inputZ[inputCount - 1],
/// lookupRecord[lookupCount].
#[derive(Clone, Copy)]
struct Rule<'a>(&'a [u8]);

impl<'a> Rule<'a> {
    fn input_count(&self) -> u32 {
        u16_at(self.0, 0) as u32
    }
    fn lookup_count(&self) -> u32 {
        u16_at(self.0, 2) as u32
    }
    fn input(&self) -> &'a [u8] {
        struct_at(self.0, 4)
    }
    fn lookup_records(&self) -> &'a [u8] {
        let ic = self.input_count();
        struct_at(self.0, 4 + 2 * if ic != 0 { ic - 1 } else { 0 } as usize)
    }

    fn collect_glyphs(&self, c: &mut HbCollectGlyphsContext, collect: CollectFunc) {
        /* context_collect_glyphs_lookup */
        let ic = self.input_count();
        collect_array(
            c,
            CollectSet::Input,
            if ic != 0 { ic - 1 } else { 0 },
            self.input(),
            collect,
        );
        recurse_lookups(c, self.lookup_count(), self.lookup_records());
    }

    fn would_apply(&self, c: &HbWouldApplyContext, match_func: MatchFunc) -> bool {
        /* context_would_apply_lookup */
        would_match_input(c, self.input_count(), self.input(), match_func)
    }

    fn apply(&self, c: &mut HbOtApplyContext<'a, '_, '_>, match_func: MatchFunc<'a>) -> bool {
        context_apply_lookup(
            c,
            self.input_count(),
            self.input(),
            self.lookup_count(),
            self.lookup_records(),
            match_func,
        )
    }

    fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.check_struct(p, 4) && {
            let ic = c.u16(p) as u32;
            let lc = c.u16(p + 2) as u32;
            c.check_range(p + 4, 2 * if ic != 0 { ic - 1 } else { 0 } + 4 * lc)
        }
    }
}

/// `collect_array`
fn collect_array(
    c: &mut HbCollectGlyphsContext,
    which: CollectSet,
    count: u32,
    values: &[u8],
    collect: CollectFunc,
) {
    if let Some(glyphs) = c.set(which) {
        for i in 0..count as usize {
            collect.collect(glyphs, u16_at(values, i * 2) as u32);
        }
    }
}

/// `RuleSet<SmallTypes>`: `Array16OfOffset16To<Rule>`.
#[derive(Clone, Copy)]
struct RuleSet<'a>(&'a [u8]);

impl<'a> RuleSet<'a> {
    fn len(&self) -> u32 {
        u16_at(self.0, 0) as u32
    }
    fn rule(&self, i: u32) -> Rule<'a> {
        Rule(offset16_to(self.0, 2 + i as usize * 2))
    }

    fn collect_glyphs(&self, c: &mut HbCollectGlyphsContext, collect: CollectFunc) {
        for i in 0..self.len() {
            self.rule(i).collect_glyphs(c, collect);
        }
    }

    fn would_apply(&self, c: &HbWouldApplyContext, match_func: MatchFunc) -> bool {
        (0..self.len()).any(|i| self.rule(i).would_apply(c, match_func))
    }

    fn apply(&self, c: &mut HbOtApplyContext<'a, '_, '_>, match_func: MatchFunc<'a>) -> bool {
        let num_rules = self.len();
        if num_rules <= 4 {
            return self.apply_slow(c, match_func);
        }

        /* This version is optimized for speed by matching the first & second
         * components of the rule here, instead of calling into the matching code.
         *
         * Replicated from LigatureSet::apply(). */

        let idx = c.buffer.idx;
        c.iter_reset(Iter::Input, idx);
        c.iter_input.set_match_func(Some(MatchFunc::Always));
        c.iter_input.set_glyph_data(None);
        let mut unsafe_to = u32::MAX;
        let unsafe_to1 = 0;
        let mut unsafe_to2 = 0;
        let first: HbGlyphInfo;
        let mut second: Option<HbGlyphInfo> = None;
        let matched = c.iter_next(Iter::Input, None);
        if matched {
            let sidx = c.iter_input.idx;
            first = c.buffer.info[sidx as usize];
            unsafe_to = sidx + 1;
            if c.iter_may_skip(Iter::Input, &first) != MaySkip::No {
                /* Can't use the fast path if eg. the next char is a default-ignorable
                 * or other skippable. */
                return self.apply_slow(c, match_func);
            }
        } else {
            /* Failed to match a next glyph. Only try applying rules that have
             * no further input. */
            for i in 0..num_rules {
                let r = self.rule(i);
                if r.input_count() <= 1 && r.apply(c, match_func) {
                    return true;
                }
            }
            return false;
        }
        let matched = c.iter_next(Iter::Input, None);
        if matched {
            let sidx = c.iter_input.idx;
            let info = c.buffer.info[sidx as usize];
            if c.iter_may_skip(Iter::Input, &info) == MaySkip::No {
                second = Some(info);
                unsafe_to2 = sidx + 1;
            }
        }

        for i in 0..num_rules {
            let r = self.rule(i);
            let input = r.input();
            if r.input_count() <= 1 || match_func.matches(&first, u16_at(input, 0) as u32) {
                if second.is_none()
                    || r.input_count() <= 2
                    || match_func.matches(second.as_ref().unwrap(), u16_at(input, 2) as u32)
                {
                    if r.apply(c, match_func) {
                        if unsafe_to != u32::MAX {
                            let idx = c.buffer.idx;
                            c.buffer.unsafe_to_concat(idx, unsafe_to);
                        }
                        return true;
                    }
                } else {
                    unsafe_to = unsafe_to2;
                }
            } else if unsafe_to == u32::MAX {
                unsafe_to = unsafe_to1;
            }
        }
        if unsafe_to != u32::MAX {
            let idx = c.buffer.idx;
            c.buffer.unsafe_to_concat(idx, unsafe_to);
        }
        false
    }

    fn apply_slow(&self, c: &mut HbOtApplyContext<'a, '_, '_>, match_func: MatchFunc<'a>) -> bool {
        for i in 0..self.len() {
            if self.rule(i).apply(c, match_func) {
                return true;
            }
        }
        false
    }

    fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.sanitize_array16_of_offset16(p, p, Rule::sanitize)
    }
}

/// `Context` (formats 1 to 3) of a lookup subtable.
pub(crate) struct Context;

impl Context {
    /// `ContextFormat3`: coverageZ[glyphCount], then the lookup records.
    fn format3_lookup_records(d: &[u8]) -> &[u8] {
        let glyph_count = u16_at(d, 2) as usize;
        struct_at(d, 6 + glyph_count * 2)
    }

    /// `Context::dispatch` of `get_coverage`
    pub(crate) fn get_coverage(d: &[u8]) -> Coverage<'_> {
        match u16_at(d, 0) {
            1 | 2 => Coverage(offset16_to(d, 2)),
            3 => Coverage(offset16_to(d, 6)),
            _ => Coverage(NULL),
        }
    }

    /// `Context::dispatch` of `apply`
    pub(crate) fn apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
        match u16_at(d, 0) {
            1 => {
                /* ContextFormat1_4::apply */
                let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
                if index == NOT_COVERED {
                    return false;
                }

                let rule_set = RuleSet(Self::array_offset(d, 4, index));
                rule_set.apply(c, MatchFunc::Glyph)
            }
            2 => {
                /* ContextFormat2_5::_apply */
                let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
                if index == NOT_COVERED {
                    return false;
                }

                let class_def = ClassDef(offset16_to(d, 4));
                let index = class_def.get_class(c.buffer.cur(0).codepoint);
                let rule_set = RuleSet(Self::array_offset(d, 6, index));
                rule_set.apply(c, MatchFunc::Class(class_def))
            }
            3 => {
                /* ContextFormat3::apply */
                let index = Coverage(offset16_to(d, 6)).get_coverage(c.buffer.cur(0).codepoint);
                if index == NOT_COVERED {
                    return false;
                }

                let glyph_count = u16_at(d, 2) as u32;
                let lookup_count = u16_at(d, 4) as u32;
                context_apply_lookup(
                    c,
                    glyph_count,
                    struct_at(d, 8),
                    lookup_count,
                    Self::format3_lookup_records(d),
                    MatchFunc::Coverage(d),
                )
            }
            _ => false,
        }
    }

    /// The `index`th offset of the `Array16Of<OffsetTo<T>>` at `at`, from
    /// `d` (the Null object out of range).
    fn array_offset(d: &[u8], at: usize, index: u32) -> &[u8] {
        let len = u16_at(d, at) as u32;
        if index >= len {
            return NULL;
        }
        offset16_to(d, at + 2 + index as usize * 2)
    }

    /// `Context::dispatch` of `would_apply`
    pub(crate) fn would_apply(c: &HbWouldApplyContext, d: &[u8]) -> bool {
        match u16_at(d, 0) {
            1 => {
                let index = Coverage(offset16_to(d, 2)).get_coverage(c.glyphs[0]);
                RuleSet(Self::array_offset(d, 4, index)).would_apply(c, MatchFunc::Glyph)
            }
            2 => {
                let class_def = ClassDef(offset16_to(d, 4));
                let index = class_def.get_class(c.glyphs[0]);
                RuleSet(Self::array_offset(d, 6, index)).would_apply(c, MatchFunc::Class(class_def))
            }
            3 => {
                let glyph_count = u16_at(d, 2) as u32;
                would_match_input(c, glyph_count, struct_at(d, 8), MatchFunc::Coverage(d))
            }
            _ => false,
        }
    }

    /// `Context::dispatch` of `collect_glyphs`
    pub(crate) fn collect_glyphs(c: &mut HbCollectGlyphsContext, d: &[u8]) {
        match u16_at(d, 0) {
            1 => {
                c.collect_coverage(CollectSet::Input, Coverage(offset16_to(d, 2)));
                let len = u16_at(d, 4) as u32;
                for i in 0..len {
                    RuleSet(offset16_to(d, 6 + i as usize * 2))
                        .collect_glyphs(c, CollectFunc::Glyph);
                }
            }
            2 => {
                c.collect_coverage(CollectSet::Input, Coverage(offset16_to(d, 2)));
                let class_def = ClassDef(offset16_to(d, 4));
                let len = u16_at(d, 6) as u32;
                for i in 0..len {
                    RuleSet(offset16_to(d, 8 + i as usize * 2))
                        .collect_glyphs(c, CollectFunc::Class(class_def));
                }
            }
            3 => {
                c.collect_coverage(CollectSet::Input, Coverage(offset16_to(d, 6)));
                let glyph_count = u16_at(d, 2) as u32;
                let lookup_count = u16_at(d, 4) as u32;
                /* context_collect_glyphs_lookup */
                collect_array(
                    c,
                    CollectSet::Input,
                    if glyph_count != 0 { glyph_count - 1 } else { 0 },
                    struct_at(d, 8),
                    CollectFunc::Coverage(d),
                );
                recurse_lookups(c, lookup_count, Self::format3_lookup_records(d));
            }
            _ => {}
        }
    }

    /// `Context::dispatch` of `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        if !c.sanitize_u16(p) {
            return false;
        }
        match c.u16(p) {
            1 => {
                /* ContextFormat1_4::sanitize */
                c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                    && c.sanitize_array16_of_offset16(p + 4, p, RuleSet::sanitize)
            }
            2 => {
                /* ContextFormat2_5::sanitize */
                c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                    && c.sanitize_offset16(p + 4, p, ClassDef::sanitize)
                    && c.sanitize_array16_of_offset16(p + 6, p, RuleSet::sanitize)
            }
            3 => {
                /* ContextFormat3::sanitize */
                if !c.check_struct(p, 6) {
                    return false;
                }
                let count = c.u16(p + 2) as u32;
                if count == 0 {
                    return false; /* We want to access coverageZ[0] freely. */
                }
                if !c.check_array(p + 6, count, 2) {
                    return false;
                }
                for i in 0..count as usize {
                    if !c.sanitize_offset16(p + 6 + i * 2, p, Coverage::sanitize) {
                        return false;
                    }
                }
                let lookup_count = c.u16(p + 4) as u32;
                c.check_array(p + 6 + count as usize * 2, lookup_count, 4)
            }
            _ => true,
        }
    }
}

/* Chaining Contextual lookups */

/// `chain_context_apply_lookup`
#[allow(clippy::too_many_arguments)]
fn chain_context_apply_lookup<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    backtrack_count: u32,
    backtrack: &'a [u8],
    input_count: u32, /* Including the first glyph (not matched) */
    input: &'a [u8],  /* Array of input values--start with second glyph */
    lookahead_count: u32,
    lookahead: &'a [u8],
    lookup_count: u32,
    lookup_record_: &[u8],
    match_funcs: [MatchFunc<'a>; 3],
) -> bool {
    if input_count > HB_MAX_CONTEXT_LENGTH {
        return false;
    }
    let mut match_positions = vec![0u32; input_count.max(1).max(4) as usize];

    let mut start_index = c.buffer.out_len;
    let mut end_index = c.buffer.idx;
    let mut match_end = 0;
    let ret;
    'done: {
        if !(match_input(
            c,
            input_count,
            input,
            match_funcs[1],
            &mut match_end,
            &mut match_positions,
            None,
        ) && {
            end_index = match_end;
            true
        } && match_lookahead(
            c,
            lookahead_count,
            lookahead,
            match_funcs[2],
            match_end,
            &mut end_index,
        )) {
            let idx = c.buffer.idx;
            c.buffer.unsafe_to_concat(idx, end_index);
            ret = false;
            break 'done;
        }

        if !match_backtrack(
            c,
            backtrack_count,
            backtrack,
            match_funcs[0],
            &mut start_index,
        ) {
            c.buffer
                .unsafe_to_concat_from_outbuffer(start_index, end_index);
            ret = false;
            break 'done;
        }

        c.buffer
            .unsafe_to_break_from_outbuffer(start_index, end_index);
        apply_lookup(
            c,
            input_count,
            &mut match_positions,
            lookup_count,
            lookup_record_,
            match_end,
        );
        ret = true;
    }

    ret
}

/// `ChainRule<SmallTypes>`: backtrack (Array16Of), inputX
/// (HeadlessArray16Of), lookaheadX (Array16Of), lookupX (Array16Of
/// LookupRecord).
#[derive(Clone, Copy)]
struct ChainRule<'a>(&'a [u8]);

impl<'a> ChainRule<'a> {
    /// (backtrack, input, lookahead, lookup) positions and lengths
    fn parts(
        &self,
    ) -> (
        (u32, &'a [u8]),
        (u32, &'a [u8]),
        (u32, &'a [u8]),
        (u32, &'a [u8]),
    ) {
        let d = self.0;
        let backtrack_len = u16_at(d, 0) as usize;
        let input_at = 2 + backtrack_len * 2;
        let input_len_p1 = u16_at(d, input_at) as usize;
        let lookahead_at = input_at + 2 + input_len_p1.saturating_sub(1) * 2;
        let lookahead_len = u16_at(d, lookahead_at) as usize;
        let lookup_at = lookahead_at + 2 + lookahead_len * 2;
        let lookup_len = u16_at(d, lookup_at) as usize;
        (
            (backtrack_len as u32, struct_at(d, 2)),
            (input_len_p1 as u32, struct_at(d, input_at + 2)),
            (lookahead_len as u32, struct_at(d, lookahead_at + 2)),
            (lookup_len as u32, struct_at(d, lookup_at + 2)),
        )
    }

    fn collect_glyphs(&self, c: &mut HbCollectGlyphsContext, collect: [CollectFunc; 3]) {
        let (b, i, l, r) = self.parts();
        /* chain_context_collect_glyphs_lookup */
        collect_array(c, CollectSet::Before, b.0, b.1, collect[0]);
        collect_array(
            c,
            CollectSet::Input,
            if i.0 != 0 { i.0 - 1 } else { 0 },
            i.1,
            collect[1],
        );
        collect_array(c, CollectSet::After, l.0, l.1, collect[2]);
        recurse_lookups(c, r.0, r.1);
    }

    fn would_apply(&self, c: &HbWouldApplyContext, match_func: MatchFunc) -> bool {
        let (b, i, l, _) = self.parts();
        /* chain_context_would_apply_lookup */
        (if c.zero_context {
            b.0 == 0 && l.0 == 0
        } else {
            true
        }) && would_match_input(c, i.0, i.1, match_func)
    }

    fn apply(&self, c: &mut HbOtApplyContext<'a, '_, '_>, match_funcs: [MatchFunc<'a>; 3]) -> bool {
        let (b, i, l, r) = self.parts();
        chain_context_apply_lookup(c, b.0, b.1, i.0, i.1, l.0, l.1, r.0, r.1, match_funcs)
    }

    fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        /* Hyper-optimized sanitized because this is really hot. */
        if !c.sanitize_u16(p) {
            return false;
        }
        let input_at = p + 2 + c.u16(p) as usize * 2;
        if !c.sanitize_u16(input_at) {
            return false;
        }
        let input_len_p1 = c.u16(input_at) as usize;
        let lookahead_at = input_at + 2 + input_len_p1.saturating_sub(1) * 2;
        if !c.sanitize_u16(lookahead_at) {
            return false;
        }
        let lookup_at = lookahead_at + 2 + c.u16(lookahead_at) as usize * 2;
        c.sanitize_array16_shallow(lookup_at, 4)
    }
}

/// `ChainRuleSet<SmallTypes>`
#[derive(Clone, Copy)]
struct ChainRuleSet<'a>(&'a [u8]);

impl<'a> ChainRuleSet<'a> {
    fn len(&self) -> u32 {
        u16_at(self.0, 0) as u32
    }
    fn rule(&self, i: u32) -> ChainRule<'a> {
        ChainRule(offset16_to(self.0, 2 + i as usize * 2))
    }

    fn collect_glyphs(&self, c: &mut HbCollectGlyphsContext, collect: [CollectFunc; 3]) {
        for i in 0..self.len() {
            self.rule(i).collect_glyphs(c, collect);
        }
    }

    fn would_apply(&self, c: &HbWouldApplyContext, match_func: MatchFunc) -> bool {
        (0..self.len()).any(|i| self.rule(i).would_apply(c, match_func))
    }

    fn apply_slow(
        &self,
        c: &mut HbOtApplyContext<'a, '_, '_>,
        match_funcs: [MatchFunc<'a>; 3],
    ) -> bool {
        for i in 0..self.len() {
            if self.rule(i).apply(c, match_funcs) {
                return true;
            }
        }
        false
    }

    fn apply(&self, c: &mut HbOtApplyContext<'a, '_, '_>, match_funcs: [MatchFunc<'a>; 3]) -> bool {
        let num_rules = self.len();
        if num_rules <= 4 {
            return self.apply_slow(c, match_funcs);
        }

        /* This version is optimized for speed by matching the first & second
         * components of the rule here, instead of calling into the matching code.
         *
         * Replicated from LigatureSet::apply(). */

        let idx = c.buffer.idx;
        c.iter_reset(Iter::Input, idx);
        c.iter_input.set_match_func(Some(MatchFunc::Always));
        c.iter_input.set_glyph_data(None);
        let mut unsafe_to = u32::MAX;
        let mut unsafe_to1 = 0;
        let mut unsafe_to2 = 0;
        let first: HbGlyphInfo;
        let mut second: Option<HbGlyphInfo> = None;
        let matched = c.iter_next(Iter::Input, None);
        if matched {
            let sidx = c.iter_input.idx;
            first = c.buffer.info[sidx as usize];
            unsafe_to1 = sidx + 1;
            if c.iter_may_skip(Iter::Input, &first) != MaySkip::No {
                /* Can't use the fast path if eg. the next char is a default-ignorable
                 * or other skippable. */
                return self.apply_slow(c, match_funcs);
            }
        } else {
            /* Failed to match a next glyph. Only try applying rules that have
             * no further input and lookahead. */
            for i in 0..num_rules {
                let r = self.rule(i);
                let (_, input, lookahead, _) = r.parts();
                if input.0 <= 1 && lookahead.0 == 0 && r.apply(c, match_funcs) {
                    return true;
                }
            }
            return false;
        }
        let matched = c.iter_next(Iter::Input, None);
        if matched {
            let sidx = c.iter_input.idx;
            let info = c.buffer.info[sidx as usize];
            if c.iter_may_skip(Iter::Input, &info) == MaySkip::No {
                second = Some(info);
                unsafe_to2 = sidx + 1;
            }
        }

        let match_input_f = match_funcs[1];
        let match_lookahead_f = match_funcs[2];
        for i in 0..num_rules {
            let r = self.rule(i);
            let (_, input, lookahead, _) = r.parts();

            let len_p1 = input.0.max(1);
            let first_ok = if len_p1 > 1 {
                match_input_f.matches(&first, u16_at(input.1, 0) as u32)
            } else {
                lookahead.0 == 0 || match_lookahead_f.matches(&first, u16_at(lookahead.1, 0) as u32)
            };
            if first_ok {
                let second_ok = match &second {
                    None => true,
                    Some(second) => {
                        if len_p1 > 2 {
                            match_input_f.matches(second, u16_at(input.1, 2) as u32)
                        } else {
                            lookahead.0 <= 2 - len_p1
                                || match_lookahead_f.matches(
                                    second,
                                    u16_at(lookahead.1, (2 - len_p1) as usize * 2) as u32,
                                )
                        }
                    }
                };
                if second_ok {
                    if r.apply(c, match_funcs) {
                        if unsafe_to != u32::MAX {
                            let idx = c.buffer.idx;
                            c.buffer.unsafe_to_concat(idx, unsafe_to);
                        }
                        return true;
                    }
                } else {
                    unsafe_to = unsafe_to2;
                }
            } else if unsafe_to == u32::MAX {
                unsafe_to = unsafe_to1;
            }
        }
        if unsafe_to != u32::MAX {
            let idx = c.buffer.idx;
            c.buffer.unsafe_to_concat(idx, unsafe_to);
        }
        false
    }

    fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.sanitize_array16_of_offset16(p, p, ChainRule::sanitize)
    }
}

/// `ChainContext` (formats 1 to 3) of a lookup subtable.
pub(crate) struct ChainContext;

impl ChainContext {
    /// `ChainContextFormat3`: (backtrack, input, lookahead, lookup)
    /// arrays.
    fn format3_parts(d: &[u8]) -> ((u32, &[u8]), (u32, &[u8]), (u32, &[u8]), (u32, &[u8])) {
        let backtrack_len = u16_at(d, 2) as usize;
        let input_at = 4 + backtrack_len * 2;
        let input_len = u16_at(d, input_at) as usize;
        let lookahead_at = input_at + 2 + input_len * 2;
        let lookahead_len = u16_at(d, lookahead_at) as usize;
        let lookup_at = lookahead_at + 2 + lookahead_len * 2;
        let lookup_len = u16_at(d, lookup_at) as usize;
        (
            (backtrack_len as u32, struct_at(d, 4)),
            (input_len as u32, struct_at(d, input_at + 2)),
            (lookahead_len as u32, struct_at(d, lookahead_at + 2)),
            (lookup_len as u32, struct_at(d, lookup_at + 2)),
        )
    }

    /// The input array's first coverage (`input[0]`, Null if empty).
    fn format3_coverage(d: &[u8]) -> Coverage<'_> {
        let (_, input, _, _) = Self::format3_parts(d);
        if input.0 == 0 {
            return Coverage(NULL);
        }
        let off = u16_at(input.1, 0) as usize;
        Coverage(if off == 0 { NULL } else { struct_at(d, off) })
    }

    /// `ChainContext::dispatch` of `get_coverage`
    pub(crate) fn get_coverage(d: &[u8]) -> Coverage<'_> {
        match u16_at(d, 0) {
            1 | 2 => Coverage(offset16_to(d, 2)),
            3 => Self::format3_coverage(d),
            _ => Coverage(NULL),
        }
    }

    /// `ChainContext::dispatch` of `apply`
    pub(crate) fn apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
        match u16_at(d, 0) {
            1 => {
                /* ChainContextFormat1_4::apply */
                let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
                if index == NOT_COVERED {
                    return false;
                }

                let rule_set = ChainRuleSet(Context::array_offset(d, 4, index));
                rule_set.apply(c, [MatchFunc::Glyph, MatchFunc::Glyph, MatchFunc::Glyph])
            }
            2 => {
                /* ChainContextFormat2_5::_apply */
                let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
                if index == NOT_COVERED {
                    return false;
                }

                let backtrack_class_def = ClassDef(offset16_to(d, 4));
                let input_class_def = ClassDef(offset16_to(d, 6));
                let lookahead_class_def = ClassDef(offset16_to(d, 8));

                let index = input_class_def.get_class(c.buffer.cur(0).codepoint);
                let rule_set = ChainRuleSet(Context::array_offset(d, 10, index));
                rule_set.apply(
                    c,
                    [
                        MatchFunc::Class(backtrack_class_def),
                        MatchFunc::Class(input_class_def),
                        MatchFunc::Class(lookahead_class_def),
                    ],
                )
            }
            3 => {
                /* ChainContextFormat3::apply */
                let index = Self::format3_coverage(d).get_coverage(c.buffer.cur(0).codepoint);
                if index == NOT_COVERED {
                    return false;
                }

                let (b, i, l, r) = Self::format3_parts(d);
                chain_context_apply_lookup(
                    c,
                    b.0,
                    b.1,
                    i.0,
                    struct_at(i.1, 2),
                    l.0,
                    l.1,
                    r.0,
                    r.1,
                    [
                        MatchFunc::Coverage(d),
                        MatchFunc::Coverage(d),
                        MatchFunc::Coverage(d),
                    ],
                )
            }
            _ => false,
        }
    }

    /// `ChainContext::dispatch` of `would_apply`
    pub(crate) fn would_apply(c: &HbWouldApplyContext, d: &[u8]) -> bool {
        match u16_at(d, 0) {
            1 => {
                let index = Coverage(offset16_to(d, 2)).get_coverage(c.glyphs[0]);
                ChainRuleSet(Context::array_offset(d, 4, index)).would_apply(c, MatchFunc::Glyph)
            }
            2 => {
                let input_class_def = ClassDef(offset16_to(d, 6));
                let index = input_class_def.get_class(c.glyphs[0]);
                ChainRuleSet(Context::array_offset(d, 10, index))
                    .would_apply(c, MatchFunc::Class(input_class_def))
            }
            3 => {
                let (b, i, l, _) = Self::format3_parts(d);
                /* chain_context_would_apply_lookup */
                (if c.zero_context {
                    b.0 == 0 && l.0 == 0
                } else {
                    true
                }) && would_match_input(c, i.0, struct_at(i.1, 2), MatchFunc::Coverage(d))
            }
            _ => false,
        }
    }

    /// `ChainContext::dispatch` of `collect_glyphs`
    pub(crate) fn collect_glyphs(c: &mut HbCollectGlyphsContext, d: &[u8]) {
        match u16_at(d, 0) {
            1 => {
                c.collect_coverage(CollectSet::Input, Coverage(offset16_to(d, 2)));
                let len = u16_at(d, 4) as u32;
                for i in 0..len {
                    ChainRuleSet(offset16_to(d, 6 + i as usize * 2)).collect_glyphs(
                        c,
                        [CollectFunc::Glyph, CollectFunc::Glyph, CollectFunc::Glyph],
                    );
                }
            }
            2 => {
                c.collect_coverage(CollectSet::Input, Coverage(offset16_to(d, 2)));
                let backtrack_class_def = ClassDef(offset16_to(d, 4));
                let input_class_def = ClassDef(offset16_to(d, 6));
                let lookahead_class_def = ClassDef(offset16_to(d, 8));
                let len = u16_at(d, 10) as u32;
                for i in 0..len {
                    ChainRuleSet(offset16_to(d, 12 + i as usize * 2)).collect_glyphs(
                        c,
                        [
                            CollectFunc::Class(backtrack_class_def),
                            CollectFunc::Class(input_class_def),
                            CollectFunc::Class(lookahead_class_def),
                        ],
                    );
                }
            }
            3 => {
                c.collect_coverage(CollectSet::Input, Self::format3_coverage(d));
                let (b, i, l, r) = Self::format3_parts(d);
                /* chain_context_collect_glyphs_lookup */
                collect_array(c, CollectSet::Before, b.0, b.1, CollectFunc::Coverage(d));
                collect_array(
                    c,
                    CollectSet::Input,
                    if i.0 != 0 { i.0 - 1 } else { 0 },
                    struct_at(i.1, 2),
                    CollectFunc::Coverage(d),
                );
                collect_array(c, CollectSet::After, l.0, l.1, CollectFunc::Coverage(d));
                recurse_lookups(c, r.0, r.1);
            }
            _ => {}
        }
    }

    /// `ChainContext::dispatch` of `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        if !c.sanitize_u16(p) {
            return false;
        }
        match c.u16(p) {
            1 => {
                /* ChainContextFormat1_4::sanitize */
                c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                    && c.sanitize_array16_of_offset16(p + 4, p, ChainRuleSet::sanitize)
            }
            2 => {
                /* ChainContextFormat2_5::sanitize */
                c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                    && c.sanitize_offset16(p + 4, p, ClassDef::sanitize)
                    && c.sanitize_offset16(p + 6, p, ClassDef::sanitize)
                    && c.sanitize_offset16(p + 8, p, ClassDef::sanitize)
                    && c.sanitize_array16_of_offset16(p + 10, p, ChainRuleSet::sanitize)
            }
            3 => {
                /* ChainContextFormat3::sanitize */
                if !c.sanitize_array16_of_offset16(p + 2, p, Coverage::sanitize) {
                    return false;
                }
                let input_at = p + 4 + c.u16(p + 2) as usize * 2;
                if !c.sanitize_array16_of_offset16(input_at, p, Coverage::sanitize) {
                    return false;
                }
                if c.u16(input_at) == 0 {
                    return false; /* To be consistent with Context. */
                }
                let lookahead_at = input_at + 2 + c.u16(input_at) as usize * 2;
                if !c.sanitize_array16_of_offset16(lookahead_at, p, Coverage::sanitize) {
                    return false;
                }
                let lookup_at = lookahead_at + 2 + c.u16(lookahead_at) as usize * 2;
                c.sanitize_array16_shallow(lookup_at, 4)
            }
            _ => true,
        }
    }
}

/* Extension */

/// `ExtensionFormat1`: (format, extensionLookupType, extensionOffset).
pub(crate) struct Extension;

impl Extension {
    /// `get_type`
    pub(crate) fn get_type(d: &[u8]) -> u32 {
        match u16_at(d, 0) {
            1 => u16_at(d, 2) as u32,
            _ => 0,
        }
    }

    /// `get_subtable`: the extension subtable (Null for a null offset or
    /// another format).
    pub(crate) fn get_subtable(d: &[u8]) -> &[u8] {
        match u16_at(d, 0) {
            1 => offset32_to(d, 4),
            _ => NULL,
        }
    }

    /// `Extension::dispatch` of `sanitize`, with the table's subtable
    /// sanitize and its Extension type.
    pub(crate) fn sanitize(
        c: &mut HbSanitizeContext,
        p: usize,
        extension_type: u32,
        subtable_sanitize: fn(&mut HbSanitizeContext, usize, u32) -> bool,
    ) -> bool {
        /* may_dispatch: u.format.sanitize */
        if !c.sanitize_u16(p) {
            return false;
        }
        match c.u16(p) {
            1 => {
                /* ExtensionFormat1::dispatch: may_dispatch (this, this):
                 * ExtensionFormat1::sanitize */
                if !(c.check_struct(p, 8) && c.u16(p + 2) as u32 != extension_type) {
                    return false;
                }
                /* get_subtable<SubTable> ().dispatch (c, get_type ()): the
                 * offset itself is not sanitized */
                let off = c.u32(p + 4) as usize;
                let q = if off == 0 { NULL_POS } else { p + off };
                subtable_sanitize(c, q, c.u16(p + 2) as u32)
            }
            _ => true,
        }
    }
}

/* GSUB/GPOS Common */

/// `GSUBGPOS`
#[derive(Clone, Copy)]
pub(crate) struct GsubGpos<'a>(pub(crate) &'a [u8]);

impl<'a> GsubGpos<'a> {
    #[inline]
    fn major(&self) -> u16 {
        u16_at(self.0, 0)
    }
    #[inline]
    fn version(&self) -> u32 {
        u32_at(self.0, 0)
    }

    /// `get_script_list`
    pub(crate) fn get_script_list(&self) -> RecordArray<'a> {
        match self.major() {
            1 => record_list(offset16_to(self.0, 4)),
            _ => record_list(NULL),
        }
    }
    /// `get_feature_list`
    pub(crate) fn get_feature_list(&self) -> RecordArray<'a> {
        match self.major() {
            1 => record_list(offset16_to(self.0, 6)),
            _ => record_list(NULL),
        }
    }
    /// The lookup list.
    fn lookup_list(&self) -> &'a [u8] {
        match self.major() {
            1 => offset16_to(self.0, 8),
            _ => NULL,
        }
    }
    /// `get_lookup_count`
    pub(crate) fn get_lookup_count(&self) -> u32 {
        u16_at(self.lookup_list(), 0) as u32
    }
    /// `get_lookup`
    pub(crate) fn get_lookup(&self, i: u32) -> Lookup<'a> {
        let list = self.lookup_list();
        if i >= u16_at(list, 0) as u32 {
            return Lookup(NULL);
        }
        Lookup(offset16_to(list, 2 + i as usize * 2))
    }
    /// `get_feature_variations`
    pub(crate) fn get_feature_variations(&self) -> FeatureVariations<'a> {
        match self.major() {
            1 if self.version() >= 0x00010001 => FeatureVariations(offset32_to(self.0, 10)),
            _ => FeatureVariations(NULL),
        }
    }
    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        self.version() != 0
    }

    /// `feature_variation_collect_lookups` (without a feature
    /// substitutes map)
    pub(crate) fn feature_variation_collect_lookups(
        &self,
        feature_indexes: &HbSet,
        lookup_indexes: &mut HbSet,
    ) {
        self.get_feature_variations()
            .collect_lookups(feature_indexes, lookup_indexes);
    }

    /// `get_script_count`
    pub(crate) fn get_script_count(&self) -> u32 {
        self.get_script_list().len()
    }
    /// `get_script_tag`
    pub(crate) fn get_script_tag(&self, i: u32) -> HbTag {
        self.get_script_list().get_tag(i)
    }
    /// `get_script`
    pub(crate) fn get_script(&self, i: u32) -> Script<'a> {
        Script(self.get_script_list().get(i))
    }
    /// `find_script_index`
    pub(crate) fn find_script_index(&self, tag: HbTag, index: &mut u32) -> bool {
        self.get_script_list().find_index(tag, index)
    }

    /// `get_feature_count`
    pub(crate) fn get_feature_count(&self) -> u32 {
        self.get_feature_list().len()
    }
    /// `get_feature_tag`
    pub(crate) fn get_feature_tag(&self, i: u32) -> HbTag {
        if i == NOT_FOUND_INDEX {
            HB_TAG_NONE
        } else {
            self.get_feature_list().get_tag(i)
        }
    }
    /// `get_feature`
    pub(crate) fn get_feature(&self, i: u32) -> Feature<'a> {
        Feature(self.get_feature_list().get(i))
    }
    /// `find_feature_index`
    pub(crate) fn find_feature_index(&self, tag: HbTag, index: &mut u32) -> bool {
        self.get_feature_list().find_index(tag, index)
    }

    /// `find_variations_index`
    pub(crate) fn find_variations_index(&self, coords: &[i32], index: &mut u32) -> bool {
        self.get_feature_variations().find_index(coords, index)
    }

    /// `get_feature_variation`
    pub(crate) fn get_feature_variation(
        &self,
        feature_index: u32,
        variations_index: u32,
    ) -> Feature<'a> {
        if FEATURE_VARIATIONS_NOT_FOUND_INDEX != variations_index && self.version() >= 0x00010001 {
            if let Some(feature) = self
                .get_feature_variations()
                .find_substitute(variations_index, feature_index)
            {
                return feature;
            }
        }
        self.get_feature(feature_index)
    }

    /// `GSUBGPOS::sanitize<TLookup>`
    pub(crate) fn sanitize(
        c: &mut HbSanitizeContext,
        extension_type: u32,
        subtable_sanitize: fn(&mut HbSanitizeContext, usize, u32) -> bool,
    ) -> bool {
        let p = 0;
        if !c.check_struct(p, 4) {
            return false;
        }
        match c.u16(p) {
            1 => {
                /* GSUBGPOSVersion1_2::sanitize */
                if !(c.sanitize_offset16(p + 4, p, |c, q| {
                    /* ScriptList */
                    RecordArray::sanitize(c, q, q, |c, r, _| Script::sanitize(c, r))
                }) && c.sanitize_offset16(p + 6, p, |c, q| {
                    /* FeatureList */
                    RecordArray::sanitize(c, q, q, Feature::sanitize)
                }) && c.sanitize_offset16(p + 8, p, |c, q| {
                    /* List16OfOffsetTo<TLookup> */
                    c.sanitize_array16_of_offset16(q, q, |c, r| {
                        Lookup::sanitize(c, r, extension_type, subtable_sanitize)
                    })
                })) {
                    return false;
                }

                if !(c.u32(p) < 0x00010001
                    || c.sanitize_offset32(p + 10, p, FeatureVariations::sanitize))
                {
                    return false;
                }

                true
            }
            _ => true,
        }
    }
}

/// `GSUBGPOS::accelerator_t`
#[derive(Debug)]
pub(crate) struct GsubGposAccel {
    pub(crate) kind: GsubGposKind,
    pub(crate) table: Vec<u8>,
    pub(crate) lookup_count: u32,
}

impl GsubGposAccel {
    pub(crate) fn new(face: &HbFace, kind: GsubGposKind) -> GsubGposAccel {
        let (tag, extension_type, subtable_sanitize): (
            HbTag,
            u32,
            fn(&mut HbSanitizeContext, usize, u32) -> bool,
        ) = match kind {
            GsubGposKind::Gsub => (
                HB_OT_TAG_GSUB,
                7,
                hb_ot_layout_gsub::subst_lookup_subtable_sanitize,
            ),
            GsubGposKind::Gpos => (
                HB_OT_TAG_GPOS,
                9,
                hb_ot_layout_gpos::pos_lookup_subtable_sanitize,
            ),
        };
        let table = hb_sanitize_blob_with(
            face.reference_table(tag),
            Some(face.get_num_glyphs()),
            |c| c.lazy_some_gpos = true,
            |c| GsubGpos::sanitize(c, extension_type, subtable_sanitize),
        );
        /* (is_blocklisted is false for both tables) */
        let lookup_count = GsubGpos(&table).get_lookup_count();
        GsubGposAccel {
            kind,
            table,
            lookup_count,
        }
    }

    /// An accelerator of the empty table.
    pub(crate) fn empty(kind: GsubGposKind) -> GsubGposAccel {
        GsubGposAccel {
            kind,
            table: Vec::new(),
            lookup_count: 0,
        }
    }

    /// `table`
    #[inline]
    pub(crate) fn table(&self) -> GsubGpos<'_> {
        GsubGpos(&self.table)
    }
}
