// Rust translation of OT/Layout/GSUB/*.hh (src/hb-ot-layout-gsub-table.hh)
// from HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz pins it), without
// the subsetter and the glyph closure.
// Copyright © 2007,2008,2009,2010  Red Hat, Inc.
// Copyright © 2010,2012,2013  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod, Garret Rieger

//! GSUB -- Glyph Substitution
//! <https://docs.microsoft.com/en-us/typography/opentype/spec/gsub>

use super::hb_common::*;
use super::hb_open_type::*;
use super::hb_ot_layout::*;
use super::hb_ot_layout_common::*;
use super::hb_ot_layout_gsubgpos::*;
use super::hb_ot_map::HB_OT_MAP_MAX_VALUE;
use super::hb_sanitize::HbSanitizeContext;

/* SubstLookupSubTable::Type */
pub(crate) const SUBST_SINGLE: u32 = 1;
pub(crate) const SUBST_MULTIPLE: u32 = 2;
pub(crate) const SUBST_ALTERNATE: u32 = 3;
pub(crate) const SUBST_LIGATURE: u32 = 4;
pub(crate) const SUBST_CONTEXT: u32 = 5;
pub(crate) const SUBST_CHAIN_CONTEXT: u32 = 6;
pub(crate) const SUBST_EXTENSION: u32 = 7;
pub(crate) const SUBST_REVERSE_CHAIN_SINGLE: u32 = 8;

/// `Coverage::get_population` of the coverage at `p` (sanitized), or of
/// the Null Coverage (`NOT_COVERED`) for a null offset.
fn coverage_population_at(c: &HbSanitizeContext, p: usize) -> u32 {
    match c.u16(p) {
        1 => c.u16(p + 2) as u32,
        2 => {
            let count = c.u16(p + 2) as usize;
            let mut ret: u64 = 0;
            for i in 0..count {
                let q = p + 4 + i * 6;
                let first = c.u16(q) as u64;
                let last = c.u16(q + 2) as u64;
                if last >= first {
                    ret += last - first + 1;
                }
            }
            if ret > u32::MAX as u64 {
                u32::MAX
            } else {
                ret as u32
            }
        }
        _ => NOT_COVERED,
    }
}

/* SingleSubst */

/// `SingleSubst::dispatch` of `sanitize`
fn single_subst_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        1 => {
            /* SingleSubstFormat1_3::sanitize */
            c.check_struct(p, 6)
                && c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                /* The coverage  table may use a range to represent a set
                 * of glyphs, which means a small number of bytes can
                 * generate a large glyph set. Manually modify the
                 * sanitizer max ops to take this into account.
                 *
                 * Note: This check *must* be right after coverage sanitize. */
                && {
                    let off = c.u16(p + 2) as usize;
                    let pop = if off == 0 { NOT_COVERED } else { coverage_population_at(c, p + off) };
                    c.check_ops(pop >> 1)
                }
        }
        /* SingleSubstFormat2_4::sanitize */
        2 => {
            c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                && c.sanitize_array16_shallow(p + 4, 2)
        }
        _ => true,
    }
}

/// `SingleSubst::dispatch` of `apply`
fn single_subst_apply(c: &mut HbOtApplyContext, d: &[u8]) -> bool {
    match u16_at(d, 0) {
        1 => {
            /* SingleSubstFormat1_3::apply */
            let mut glyph_id = c.buffer.cur(0).codepoint;
            let index = Coverage(offset16_to(d, 2)).get_coverage(glyph_id);
            if index == NOT_COVERED {
                return false;
            }

            let delta = u16_at(d, 4) as u32;
            let mask = 0xFFFF; /* get_mask () */
            glyph_id = glyph_id.wrapping_add(delta) & mask;

            c.replace_glyph(glyph_id);

            true
        }
        2 => {
            /* SingleSubstFormat2_4::apply */
            let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
            if index == NOT_COVERED {
                return false;
            }

            let (len, arr) = array16(struct_at(d, 4));
            if index >= len {
                return false;
            }

            c.replace_glyph(u16_at(arr, index as usize * 2) as u32);

            true
        }
        _ => false,
    }
}

/// `SingleSubst::dispatch` of `collect_glyphs`
fn single_subst_collect_glyphs(c: &mut HbCollectGlyphsContext, d: &[u8]) {
    match u16_at(d, 0) {
        1 => {
            let coverage = Coverage(offset16_to(d, 2));
            if !c.collect_coverage(CollectSet::Input, coverage) {
                return;
            }
            let delta = u16_at(d, 4) as u32;
            let mask = 0xFFFF;
            for g in coverage.iter() {
                c.add(CollectSet::Output, g.wrapping_add(delta) & mask);
            }
        }
        2 => {
            let coverage = Coverage(offset16_to(d, 2));
            if !c.collect_coverage(CollectSet::Input, coverage) {
                return;
            }
            /* hb_zip (this+coverage, substitute) | hb_map (hb_second) */
            let (len, arr) = array16(struct_at(d, 4));
            let n = coverage.iter().len().min(len as usize);
            for i in 0..n {
                c.add(CollectSet::Output, u16_at(arr, i * 2) as u32);
            }
        }
        _ => {}
    }
}

/* MultipleSubst */

/// `Sequence::apply`
fn sequence_apply(c: &mut HbOtApplyContext, d: &[u8]) -> bool {
    let (count, arr) = array16(d);

    /* Special-case to make it in-place and not consider this
     * as a "multiplied" substitution. */
    if count == 1 {
        c.replace_glyph(u16_at(arr, 0) as u32);
        return true;
    }
    /* Spec disallows this, but Uniscribe allows it.
     * https://github.com/harfbuzz/harfbuzz/issues/253 */
    else if count == 0 {
        c.buffer.delete_glyph();
        return true;
    }

    let klass = if _hb_glyph_info_is_ligature(c.buffer.cur(0)) {
        HB_OT_LAYOUT_GLYPH_PROPS_BASE_GLYPH
    } else {
        0
    };
    let lig_id = _hb_glyph_info_get_lig_id(c.buffer.cur(0));

    for i in 0..count {
        /* If is attached to a ligature, don't disturb that.
         * https://github.com/harfbuzz/harfbuzz/issues/3069 */
        if lig_id == 0 {
            _hb_glyph_info_set_lig_props_for_component(c.buffer.cur_mut(0), i);
        }
        c.output_glyph_for_component(u16_at(arr, i as usize * 2) as u32, klass);
    }
    c.buffer.skip_glyph();

    true
}

/// `MultipleSubst::dispatch` of `sanitize`
fn multiple_subst_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        /* MultipleSubstFormat1_2::sanitize */
        1 => {
            c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                && c.sanitize_array16_of_offset16(p + 4, p, |c, q| c.sanitize_array16_shallow(q, 2))
        }
        _ => true,
    }
}

/// The `index`th offset (from `d`) of the `Array16Of<Offset16To<T>>` at
/// `at` (the Null object out of range).
fn array_offset(d: &[u8], at: usize, index: u32) -> &[u8] {
    let len = u16_at(d, at) as u32;
    if index >= len {
        return NULL;
    }
    offset16_to(d, at + 2 + index as usize * 2)
}

/// `MultipleSubst::dispatch` of `apply`
fn multiple_subst_apply(c: &mut HbOtApplyContext, d: &[u8]) -> bool {
    match u16_at(d, 0) {
        1 => {
            let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
            if index == NOT_COVERED {
                return false;
            }

            sequence_apply(c, array_offset(d, 4, index))
        }
        _ => false,
    }
}

/// `collect_glyphs` of the formats whose sets (sequences, alternate sets)
/// add their arrays to the output
fn collect_sets_output(c: &mut HbCollectGlyphsContext, d: &[u8]) {
    let coverage = Coverage(offset16_to(d, 2));
    if !c.collect_coverage(CollectSet::Input, coverage) {
        return;
    }
    /* hb_zip (this+coverage, sequence) | hb_map (hb_second) */
    let len = u16_at(d, 4) as usize;
    let n = coverage.iter().len().min(len);
    for i in 0..n {
        let set = offset16_to(d, 6 + i * 2);
        let (count, arr) = array16(set);
        for j in 0..count as usize {
            c.add(CollectSet::Output, u16_at(arr, j * 2) as u32);
        }
    }
}

/* AlternateSubst */

/// `AlternateSet::apply`
fn alternate_set_apply(c: &mut HbOtApplyContext, d: &[u8]) -> bool {
    let (count, arr) = array16(d);

    if count == 0 {
        return false;
    }

    let glyph_mask = c.buffer.cur(0).mask;
    let lookup_mask = c.lookup_mask;

    /* Note: This breaks badly if two features enabled this lookup together. */

    let shift = lookup_mask.trailing_zeros();
    let mut alt_index = (lookup_mask & glyph_mask) >> shift;

    /* If alt_index is MAX_VALUE, randomize feature if it is the rand feature. */
    if alt_index == HB_OT_MAP_MAX_VALUE && c.random {
        /* Maybe we can do better than unsafe-to-break all; but since we are
         * changing random state, it would be hard to track that.  Good 'nough. */
        let len = c.buffer.len;
        c.buffer.unsafe_to_break(0, len);
        alt_index = c.random_number() % count + 1;
    }

    if alt_index > count || alt_index == 0 {
        return false;
    }

    c.replace_glyph(u16_at(arr, (alt_index - 1) as usize * 2) as u32);

    true
}

/// `AlternateSubst::dispatch` of `apply`
fn alternate_subst_apply(c: &mut HbOtApplyContext, d: &[u8]) -> bool {
    match u16_at(d, 0) {
        1 => {
            let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
            if index == NOT_COVERED {
                return false;
            }

            alternate_set_apply(c, array_offset(d, 4, index))
        }
        _ => false,
    }
}

/* LigatureSubst */

/// `Ligature`: ligGlyph, component (HeadlessArray16Of).
#[derive(Clone, Copy)]
struct Ligature<'a>(&'a [u8]);

impl<'a> Ligature<'a> {
    fn lig_glyph(&self) -> HbCodepoint {
        u16_at(self.0, 0) as u32
    }
    /// `component.lenP1`
    fn len_p1(&self) -> u32 {
        u16_at(self.0, 2) as u32
    }
    /// `component.arrayZ`
    fn components(&self) -> &'a [u8] {
        struct_at(self.0, 4)
    }
    /// `component[i]` (HeadlessArrayOf: Null for 0 and out of range)
    fn component(&self, i: u32) -> HbCodepoint {
        if i >= self.len_p1() || i == 0 {
            return 0;
        }
        u16_at(self.components(), (i - 1) as usize * 2) as u32
    }

    fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.sanitize_u16(p) && c.sanitize_headless_array16_shallow(p + 2, 2)
    }

    fn collect_glyphs(&self, c: &mut HbCollectGlyphsContext) {
        let n = self.len_p1().saturating_sub(1);
        for i in 0..n as usize {
            c.add(CollectSet::Input, u16_at(self.components(), i * 2) as u32);
        }
        c.add(CollectSet::Output, self.lig_glyph());
    }

    fn would_apply(&self, c: &HbWouldApplyContext) -> bool {
        if c.len != self.len_p1() {
            return false;
        }

        for i in 1..c.len {
            if c.glyphs[i as usize] != self.component(i) {
                return false;
            }
        }

        true
    }

    fn apply(&self, c: &mut HbOtApplyContext<'a, '_, '_>) -> bool {
        let count = self.len_p1();

        if count == 0 {
            return false;
        }

        /* Special-case to make it in-place and not consider this
         * as a "ligated" substitution. */
        if count == 1 {
            c.replace_glyph(self.lig_glyph());
            return true;
        }

        let mut total_component_count = 0;

        if count > HB_MAX_CONTEXT_LENGTH {
            return false;
        }
        let mut match_positions = vec![0u32; count.max(4) as usize];

        let mut match_end = 0;

        if !match_input(
            c,
            count,
            self.components(),
            MatchFunc::Glyph,
            &mut match_end,
            &mut match_positions,
            Some(&mut total_component_count),
        ) {
            let idx = c.buffer.idx;
            c.buffer.unsafe_to_concat(idx, match_end);
            return false;
        }

        ligate_input(
            c,
            count,
            &match_positions,
            match_end,
            self.lig_glyph(),
            total_component_count,
        );

        true
    }
}

/// `LigatureSet`: `Array16OfOffset16To<Ligature>`.
#[derive(Clone, Copy)]
struct LigatureSet<'a>(&'a [u8]);

impl<'a> LigatureSet<'a> {
    fn len(&self) -> u32 {
        u16_at(self.0, 0) as u32
    }
    fn ligature(&self, i: u32) -> Ligature<'a> {
        Ligature(offset16_to(self.0, 2 + i as usize * 2))
    }

    fn sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
        c.sanitize_array16_of_offset16(p, p, Ligature::sanitize)
    }

    fn collect_glyphs(&self, c: &mut HbCollectGlyphsContext) {
        for i in 0..self.len() {
            self.ligature(i).collect_glyphs(c);
        }
    }

    fn would_apply(&self, c: &HbWouldApplyContext) -> bool {
        (0..self.len()).any(|i| self.ligature(i).would_apply(c))
    }

    fn apply(&self, c: &mut HbOtApplyContext<'a, '_, '_>) -> bool {
        let num_ligs = self.len();

        if num_ligs <= 4 {
            return self.apply_slow(c);
        }

        /* This version is optimized for speed by matching the first component
         * of the ligature here, instead of calling into the ligation code.
         *
         * This is replicated in ChainRuleSet and RuleSet. */

        let idx = c.buffer.idx;
        c.iter_reset(Iter::Input, idx);
        c.iter_input.set_match_func(Some(MatchFunc::Always));
        c.iter_input.set_glyph_data(None);

        let mut unsafe_to = 0;
        let first;
        let matched = c.iter_next(Iter::Input, Some(&mut unsafe_to));
        if matched {
            let sidx = c.iter_input.idx;
            let info = c.buffer.info[sidx as usize];
            first = info.codepoint;
            unsafe_to = sidx + 1;
            if c.iter_may_skip(Iter::Input, &info) != MaySkip::No {
                /* Can't use the fast path if eg. the next char is a default-ignorable
                 * or other skippable. */
                return self.apply_slow(c);
            }
        } else {
            return self.apply_slow(c);
        }

        let mut unsafe_to_concat = false;

        for i in 0..num_ligs {
            let lig = self.ligature(i);
            if lig.len_p1() <= 1 || u16_at(lig.components(), 0) as u32 == first {
                if lig.apply(c) {
                    if unsafe_to_concat {
                        let idx = c.buffer.idx;
                        c.buffer.unsafe_to_concat(idx, unsafe_to);
                    }
                    return true;
                }
            } else if lig.len_p1() > 1 {
                unsafe_to_concat = true;
            }
        }
        if unsafe_to_concat {
            let idx = c.buffer.idx;
            c.buffer.unsafe_to_concat(idx, unsafe_to);
        }

        false
    }

    fn apply_slow(&self, c: &mut HbOtApplyContext<'a, '_, '_>) -> bool {
        for i in 0..self.len() {
            if self.ligature(i).apply(c) {
                return true;
            }
        }
        false
    }
}

/// `LigatureSubst::dispatch` of `apply`
fn ligature_subst_apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
    match u16_at(d, 0) {
        1 => {
            let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
            if index == NOT_COVERED {
                return false;
            }

            LigatureSet(array_offset(d, 4, index)).apply(c)
        }
        _ => false,
    }
}

/* ReverseChainSingleSubst */

/// `ReverseChainSingleSubstFormat1`: (backtrack, lookahead, substitute)
/// arrays.
fn reverse_chain_parts(d: &[u8]) -> ((u32, &[u8]), (u32, &[u8]), (u32, &[u8])) {
    let backtrack_len = u16_at(d, 4) as usize;
    let lookahead_at = 6 + backtrack_len * 2;
    let lookahead_len = u16_at(d, lookahead_at) as usize;
    let substitute_at = lookahead_at + 2 + lookahead_len * 2;
    let substitute_len = u16_at(d, substitute_at) as usize;
    (
        (backtrack_len as u32, struct_at(d, 6)),
        (lookahead_len as u32, struct_at(d, lookahead_at + 2)),
        (substitute_len as u32, struct_at(d, substitute_at + 2)),
    )
}

/// `ReverseChainSingleSubst::dispatch` of `sanitize`
fn reverse_chain_single_subst_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        1 => {
            if !(c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                && c.sanitize_array16_of_offset16(p + 4, p, Coverage::sanitize))
            {
                return false;
            }
            let lookahead_at = p + 6 + c.u16(p + 4) as usize * 2;
            if !c.sanitize_array16_of_offset16(lookahead_at, p, Coverage::sanitize) {
                return false;
            }
            let substitute_at = lookahead_at + 2 + c.u16(lookahead_at) as usize * 2;
            c.sanitize_array16_shallow(substitute_at, 2)
        }
        _ => true,
    }
}

/// `ReverseChainSingleSubst::dispatch` of `apply`
fn reverse_chain_single_subst_apply<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, d: &'a [u8]) -> bool {
    match u16_at(d, 0) {
        1 => {
            let index = Coverage(offset16_to(d, 2)).get_coverage(c.buffer.cur(0).codepoint);
            if index == NOT_COVERED {
                return false;
            }

            if c.nesting_level_left != HB_MAX_NESTING_LEVEL {
                return false; /* No chaining to this type */
            }

            let (backtrack, lookahead, substitute) = reverse_chain_parts(d);
            if index >= substitute.0 {
                return false;
            }

            let mut start_index = 0;
            let mut end_index = 0;
            let idx = c.buffer.idx;
            if match_backtrack(
                c,
                backtrack.0,
                backtrack.1,
                MatchFunc::Coverage(d),
                &mut start_index,
            ) && match_lookahead(
                c,
                lookahead.0,
                lookahead.1,
                MatchFunc::Coverage(d),
                idx + 1,
                &mut end_index,
            ) {
                c.buffer
                    .unsafe_to_break_from_outbuffer(start_index, end_index);
                c.replace_glyph_inplace(u16_at(substitute.1, index as usize * 2) as u32);
                /* Note: We DON'T decrease buffer->idx.  The main loop does it
                 * for us.  This is useful for preventing surprises if someone
                 * calls us through a Context lookup. */
                true
            } else {
                c.buffer
                    .unsafe_to_concat_from_outbuffer(start_index, end_index);
                false
            }
        }
        _ => false,
    }
}

/* SubstLookupSubTable */

/// `SubstLookupSubTable::dispatch` of `sanitize` (with the lookup type)
pub(crate) fn subst_lookup_subtable_sanitize(
    c: &mut HbSanitizeContext,
    p: usize,
    lookup_type: u32,
) -> bool {
    match lookup_type {
        SUBST_SINGLE => single_subst_sanitize(c, p),
        SUBST_MULTIPLE => multiple_subst_sanitize(c, p),
        SUBST_ALTERNATE => {
            if !c.sanitize_u16(p) {
                return false;
            }
            match c.u16(p) {
                /* AlternateSubstFormat1_2::sanitize */
                1 => {
                    c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                        && c.sanitize_array16_of_offset16(p + 4, p, |c, q| {
                            c.sanitize_array16_shallow(q, 2)
                        })
                }
                _ => true,
            }
        }
        SUBST_LIGATURE => {
            if !c.sanitize_u16(p) {
                return false;
            }
            match c.u16(p) {
                /* LigatureSubstFormat1_2::sanitize */
                1 => {
                    c.sanitize_offset16(p + 2, p, Coverage::sanitize)
                        && c.sanitize_array16_of_offset16(p + 4, p, LigatureSet::sanitize)
                }
                _ => true,
            }
        }
        SUBST_CONTEXT => Context::sanitize(c, p),
        SUBST_CHAIN_CONTEXT => ChainContext::sanitize(c, p),
        SUBST_EXTENSION => {
            Extension::sanitize(c, p, SUBST_EXTENSION, subst_lookup_subtable_sanitize)
        }
        SUBST_REVERSE_CHAIN_SINGLE => reverse_chain_single_subst_sanitize(c, p),
        _ => true,
    }
}

/// `SubstLookupSubTable::dispatch` of `apply` (with the lookup type)
pub(crate) fn subst_lookup_subtable_apply<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    st: &'a [u8],
    lookup_type: u32,
) -> bool {
    match lookup_type {
        SUBST_SINGLE => single_subst_apply(c, st),
        SUBST_MULTIPLE => multiple_subst_apply(c, st),
        SUBST_ALTERNATE => alternate_subst_apply(c, st),
        SUBST_LIGATURE => ligature_subst_apply(c, st),
        SUBST_CONTEXT => Context::apply(c, st),
        SUBST_CHAIN_CONTEXT => ChainContext::apply(c, st),
        SUBST_EXTENSION => {
            let t = Extension::get_type(st);
            if t == SUBST_EXTENSION {
                return false;
            }
            subst_lookup_subtable_apply(c, Extension::get_subtable(st), t)
        }
        SUBST_REVERSE_CHAIN_SINGLE => reverse_chain_single_subst_apply(c, st),
        _ => false,
    }
}

/// `SubstLookupSubTable::dispatch` of `would_apply`
fn subst_lookup_subtable_would_apply(c: &HbWouldApplyContext, st: &[u8], lookup_type: u32) -> bool {
    match lookup_type {
        SUBST_SINGLE | SUBST_MULTIPLE | SUBST_ALTERNATE | SUBST_REVERSE_CHAIN_SINGLE => {
            let known = match lookup_type {
                SUBST_SINGLE => matches!(u16_at(st, 0), 1 | 2),
                _ => u16_at(st, 0) == 1,
            };
            known
                && c.len == 1
                && Coverage(offset16_to(st, 2)).get_coverage(c.glyphs[0]) != NOT_COVERED
        }
        SUBST_LIGATURE => {
            if u16_at(st, 0) != 1 {
                return false;
            }
            let index = Coverage(offset16_to(st, 2)).get_coverage(c.glyphs[0]);
            if index == NOT_COVERED {
                return false;
            }
            LigatureSet(array_offset(st, 4, index)).would_apply(c)
        }
        SUBST_CONTEXT => Context::would_apply(c, st),
        SUBST_CHAIN_CONTEXT => ChainContext::would_apply(c, st),
        SUBST_EXTENSION => {
            let t = Extension::get_type(st);
            if t == SUBST_EXTENSION {
                return false;
            }
            subst_lookup_subtable_would_apply(c, Extension::get_subtable(st), t)
        }
        _ => false,
    }
}

/// `SubstLookupSubTable::dispatch` of `collect_glyphs`
fn subst_lookup_subtable_collect_glyphs(
    c: &mut HbCollectGlyphsContext,
    st: &[u8],
    lookup_type: u32,
) {
    match lookup_type {
        SUBST_SINGLE => single_subst_collect_glyphs(c, st),
        SUBST_MULTIPLE | SUBST_ALTERNATE => {
            if u16_at(st, 0) == 1 {
                collect_sets_output(c, st);
            }
        }
        SUBST_LIGATURE => {
            if u16_at(st, 0) == 1 {
                let coverage = Coverage(offset16_to(st, 2));
                if !c.collect_coverage(CollectSet::Input, coverage) {
                    return;
                }
                let len = u16_at(st, 4) as usize;
                let n = coverage.iter().len().min(len);
                for i in 0..n {
                    LigatureSet(offset16_to(st, 6 + i * 2)).collect_glyphs(c);
                }
            }
        }
        SUBST_CONTEXT => Context::collect_glyphs(c, st),
        SUBST_CHAIN_CONTEXT => ChainContext::collect_glyphs(c, st),
        SUBST_EXTENSION => {
            let t = Extension::get_type(st);
            if t != SUBST_EXTENSION {
                subst_lookup_subtable_collect_glyphs(c, Extension::get_subtable(st), t);
            }
        }
        SUBST_REVERSE_CHAIN_SINGLE => {
            if u16_at(st, 0) != 1 {
                return;
            }
            if !c.collect_coverage(CollectSet::Input, Coverage(offset16_to(st, 2))) {
                return;
            }
            let (backtrack, lookahead, substitute) = reverse_chain_parts(st);
            for i in 0..backtrack.0 as usize {
                let off = u16_at(backtrack.1, i * 2) as usize;
                let cov = Coverage(if off == 0 { NULL } else { struct_at(st, off) });
                if !c.collect_coverage(CollectSet::Before, cov) {
                    return;
                }
            }
            for i in 0..lookahead.0 as usize {
                let off = u16_at(lookahead.1, i * 2) as usize;
                let cov = Coverage(if off == 0 { NULL } else { struct_at(st, off) });
                if !c.collect_coverage(CollectSet::After, cov) {
                    return;
                }
            }
            for i in 0..substitute.0 as usize {
                c.add(CollectSet::Output, u16_at(substitute.1, i * 2) as u32);
            }
        }
        _ => {}
    }
}

/* SubstLookup */

/// `SubstLookup::lookup_type_is_reverse`
#[inline]
pub(crate) fn lookup_type_is_reverse(lookup_type: u32) -> bool {
    lookup_type == SUBST_REVERSE_CHAIN_SINGLE
}

/// `SubstLookup::is_reverse`
pub(crate) fn subst_lookup_is_reverse(l: Lookup) -> bool {
    let t = l.get_type();
    if t == SUBST_EXTENSION {
        /* get_subtable (0).u.extension.is_reverse () */
        return lookup_type_is_reverse(Extension::get_type(l.get_subtable(0)));
    }
    lookup_type_is_reverse(t)
}

/// `SubstLookup::collect_glyphs`
pub(crate) fn subst_lookup_collect_glyphs(c: &mut HbCollectGlyphsContext, l: Lookup) {
    c.recurse_gsub = true;
    /* dispatch (c): every subtable */
    let t = l.get_type();
    for i in 0..l.get_subtable_count() {
        subst_lookup_subtable_collect_glyphs(c, l.get_subtable(i), t);
    }
}

/// `SubstLookup::would_apply`
pub(crate) fn subst_lookup_would_apply(c: &HbWouldApplyContext, l: Lookup) -> bool {
    if c.len == 0 {
        return false;
    }
    /* (the accelerator's digest only prefilters the coverage check of the
     * first glyph) */
    let t = l.get_type();
    (0..l.get_subtable_count()).any(|i| subst_lookup_subtable_would_apply(c, l.get_subtable(i), t))
}
