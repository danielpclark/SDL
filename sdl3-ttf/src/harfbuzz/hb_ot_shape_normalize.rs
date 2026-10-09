// Rust translation of src/hb-ot-shape-normalize.hh and
// src/hb-ot-shape-normalize.cc from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! Normalization.
//!
//! Translation notes: the normalize context holds the plan and the
//! shaper's functions; the buffer and the font are passed alongside (C
//! keeps pointers to them in the context too).

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::*;
use super::hb_ot_shape::HbOtShapePlan;
use super::hb_ot_shaper::*;
use super::hb_unicode::*;

/* buffer var allocations, used during the normalization process */

/// `glyph_index()`: `var1.u32`
#[inline]
pub(crate) fn glyph_index(info: &HbGlyphInfo) -> HbCodepoint {
    info.var1
}
#[inline]
pub(crate) fn set_glyph_index(info: &mut HbGlyphInfo, g: HbCodepoint) {
    info.var1 = g;
}

/// `hb_ot_shape_normalization_mode_t`
pub(crate) type HbOtShapeNormalizationMode = u32;
pub(crate) const HB_OT_SHAPE_NORMALIZATION_MODE_NONE: HbOtShapeNormalizationMode = 0;
pub(crate) const HB_OT_SHAPE_NORMALIZATION_MODE_DECOMPOSED: HbOtShapeNormalizationMode = 1;
pub(crate) const HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS: HbOtShapeNormalizationMode = 2; /* Never composes base-to-base */
pub(crate) const HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS_NO_SHORT_CIRCUIT:
    HbOtShapeNormalizationMode = 3; /* Always fully decomposes and then recompose back */
pub(crate) const HB_OT_SHAPE_NORMALIZATION_MODE_AUTO: HbOtShapeNormalizationMode = 4; /* See hb-ot-shape-normalize.cc for logic. */
pub(crate) const HB_OT_SHAPE_NORMALIZATION_MODE_DEFAULT: HbOtShapeNormalizationMode =
    HB_OT_SHAPE_NORMALIZATION_MODE_AUTO;

/// `hb_ot_shape_normalize_context_t`
pub(crate) struct HbOtShapeNormalizeContext<'p> {
    pub(crate) plan: &'p HbOtShapePlan,
    pub(crate) unicode: HbUnicodeFuncs,
    pub(crate) not_found: HbCodepoint,
    pub(crate) decompose: DecomposeFunc,
    pub(crate) compose: ComposeFunc,
}

/*
 * HIGHLEVEL DESIGN:
 *
 * This file exports one main function: _hb_ot_shape_normalize().
 *
 * This function closely reflects the Unicode Normalization Algorithm,
 * yet it's different.
 *
 * Each shaper specifies whether it prefers decomposed (NFD) or composed (NFC).
 * The logic however tries to use whatever the font can support.
 *
 * In general what happens is that: each grapheme is decomposed in a chain
 * of 1:2 decompositions, marks reordered, and then recomposed if desired,
 * so far it's like Unicode Normalization.  However, the decomposition and
 * recomposition only happens if the font supports the resulting characters.
 *
 * The goals are:
 *
 *   - Try to render all canonically equivalent strings similarly.  To really
 *     achieve this we have to always do the full decomposition and then
 *     selectively recompose from there.  It's kinda too expensive though, so
 *     we skip some cases.  For example, if composed is desired, we simply
 *     don't touch 1-character clusters that are supported by the font, even
 *     though their NFC may be different.
 *
 *   - When a font has a precomposed character for a sequence but the 'ccmp'
 *     feature in the font is not adequate, use the precomposed character
 *     which typically has better mark positioning.
 *
 *   - When a font does not support a combining mark, but supports it precomposed
 *     with previous base, use that.  This needs the itemizer to have this
 *     knowledge too.  We need to provide assistance to the itemizer.
 *
 *   - When a font does not support a character but supports its canonical
 *     decomposition, well, use the decomposition.
 *
 *   - The shapers can customize the compose and decompose functions to
 *     offload some of their requirements to the normalizer.  For example, the
 *     Indic shaper may want to disallow recomposing of two matras.
 */

/// `decompose_unicode`
fn decompose_unicode(
    c: &HbOtShapeNormalizeContext,
    _font: &mut HbFont,
    ab: HbCodepoint,
    a: &mut HbCodepoint,
    b: &mut HbCodepoint,
) -> bool {
    c.unicode.decompose(ab, a, b)
}

/// `compose_unicode`
fn compose_unicode(
    c: &HbOtShapeNormalizeContext,
    a: HbCodepoint,
    b: HbCodepoint,
    ab: &mut HbCodepoint,
) -> bool {
    c.unicode.compose(a, b, ab)
}

/// `set_glyph`
#[inline]
fn set_glyph(info: &mut HbGlyphInfo, font: &mut HbFont) {
    let mut g = 0;
    let _ = font.get_nominal_glyph(info.codepoint, &mut g, 0);
    set_glyph_index(info, g);
}

/// `output_char`
#[inline]
fn output_char(buffer: &mut HbBuffer, unichar: HbCodepoint, glyph: HbCodepoint) {
    /* This is very confusing indeed. */
    set_glyph_index(buffer.cur_mut(0), glyph);
    let _ = buffer.output_glyph(unichar);
    let mut scratch = buffer.scratch_flags;
    _hb_glyph_info_set_unicode_props(buffer.prev_mut(), &mut scratch);
    buffer.scratch_flags = scratch;
}

/// `next_char`
#[inline]
fn next_char(buffer: &mut HbBuffer, glyph: HbCodepoint) {
    set_glyph_index(buffer.cur_mut(0), glyph);
    let _ = buffer.next_glyph();
}

/// `skip_char`
#[inline]
fn skip_char(buffer: &mut HbBuffer) {
    buffer.skip_glyph();
}

/// `decompose`: returns 0 if didn't decompose, number of resulting
/// characters otherwise.
fn decompose(
    c: &HbOtShapeNormalizeContext,
    buffer: &mut HbBuffer,
    font: &mut HbFont,
    shortest: bool,
    ab: HbCodepoint,
) -> u32 {
    let mut a = 0;
    let mut b = 0;
    let mut a_glyph = 0;
    let mut b_glyph = 0;

    if !(c.decompose)(c, font, ab, &mut a, &mut b)
        || (b != 0 && !font.get_nominal_glyph(b, &mut b_glyph, 0))
    {
        return 0;
    }

    let has_a = font.get_nominal_glyph(a, &mut a_glyph, 0);
    if shortest && has_a {
        /* Output a and b */
        output_char(buffer, a, a_glyph);
        if b != 0 {
            output_char(buffer, b, b_glyph);
            return 2;
        }
        return 1;
    }

    let ret = decompose(c, buffer, font, shortest, a);
    if ret != 0 {
        if b != 0 {
            output_char(buffer, b, b_glyph);
            return ret + 1;
        }
        return ret;
    }

    if has_a {
        output_char(buffer, a, a_glyph);
        if b != 0 {
            output_char(buffer, b, b_glyph);
            return 2;
        }
        return 1;
    }

    0
}

/// `decompose_current_character`
fn decompose_current_character(
    c: &HbOtShapeNormalizeContext,
    buffer: &mut HbBuffer,
    font: &mut HbFont,
    shortest: bool,
) {
    let u = buffer.cur(0).codepoint;
    let mut glyph = 0;

    if shortest && font.get_nominal_glyph(u, &mut glyph, c.not_found) {
        next_char(buffer, glyph);
        return;
    }

    if decompose(c, buffer, font, shortest, u) != 0 {
        skip_char(buffer);
        return;
    }

    if !shortest && font.get_nominal_glyph(u, &mut glyph, c.not_found) {
        next_char(buffer, glyph);
        return;
    }

    if _hb_glyph_info_is_unicode_space(buffer.cur(0)) {
        let mut space_glyph = 0;
        let space_type = HbUnicodeFuncs::space_fallback_type(u);
        if space_type != NOT_SPACE
            && (font.get_nominal_glyph(0x0020, &mut space_glyph, 0) || {
                space_glyph = buffer.invisible;
                space_glyph != 0
            })
        {
            _hb_glyph_info_set_unicode_space_fallback_type(buffer.cur_mut(0), space_type as u32);
            next_char(buffer, space_glyph);
            buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_SPACE_FALLBACK;
            return;
        }
    }

    if u == 0x2011 {
        /* U+2011 is the only sensible character that is a no-break version of another character
         * and not a space.  The space ones are handled already.  Handle this lone one. */
        let mut other_glyph = 0;
        if font.get_nominal_glyph(0x2010, &mut other_glyph, 0) {
            next_char(buffer, other_glyph);
            return;
        }
    }

    next_char(buffer, glyph); /* glyph is initialized in earlier branches. */
}

/// `handle_variation_selector_cluster`
fn handle_variation_selector_cluster(
    buffer: &mut HbBuffer,
    font: &mut HbFont,
    end: u32,
    _short_circuit: bool,
) {
    /* Currently if there's a variation-selector we give-up on normalization, it's just too hard. */
    while buffer.idx < end - 1 && buffer.successful {
        if HbUnicodeFuncs::is_variation_selector(buffer.cur(1).codepoint) {
            let mut g = 0;
            let (u, vs) = (buffer.cur(0).codepoint, buffer.cur(1).codepoint);
            if font.get_variation_glyph(u, vs, &mut g, 0) {
                set_glyph_index(buffer.cur_mut(0), g);
                let unicode = buffer.cur(0).codepoint;
                let _ = buffer.replace_glyphs(2, 1, &[unicode]);
            } else {
                /* (the failed lookup still sets glyph_index, to 0) */
                set_glyph_index(buffer.cur_mut(0), g);
                /* Just pass on the two characters separately, let GSUB do its magic. */
                set_glyph(buffer.cur_mut(0), font);
                let _ = buffer.next_glyph();
                set_glyph(buffer.cur_mut(0), font);
                let _ = buffer.next_glyph();
            }
            /* Skip any further variation selectors. */
            while buffer.idx < end
                && buffer.successful
                && HbUnicodeFuncs::is_variation_selector(buffer.cur(0).codepoint)
            {
                set_glyph(buffer.cur_mut(0), font);
                let _ = buffer.next_glyph();
            }
        } else {
            set_glyph(buffer.cur_mut(0), font);
            let _ = buffer.next_glyph();
        }
    }
    if buffer.idx < end {
        set_glyph(buffer.cur_mut(0), font);
        let _ = buffer.next_glyph();
    }
}

/// `decompose_multi_char_cluster`
fn decompose_multi_char_cluster(
    c: &HbOtShapeNormalizeContext,
    buffer: &mut HbBuffer,
    font: &mut HbFont,
    end: u32,
    short_circuit: bool,
) {
    let mut i = buffer.idx;
    while i < end && buffer.successful {
        if HbUnicodeFuncs::is_variation_selector(buffer.info[i as usize].codepoint) {
            handle_variation_selector_cluster(buffer, font, end, short_circuit);
            return;
        }
        i += 1;
    }

    while buffer.idx < end && buffer.successful {
        decompose_current_character(c, buffer, font, short_circuit);
    }
}

/// `compare_combining_class`
fn compare_combining_class(pa: &HbGlyphInfo, pb: &HbGlyphInfo) -> i32 {
    let a = _hb_glyph_info_get_modified_combining_class(pa);
    let b = _hb_glyph_info_get_modified_combining_class(pb);
    if a < b {
        -1
    } else if a == b {
        0
    } else {
        1
    }
}

/// `_hb_ot_shape_normalize`
pub(crate) fn _hb_ot_shape_normalize(
    plan: &HbOtShapePlan,
    buffer: &mut HbBuffer,
    font: &mut HbFont,
) {
    if buffer.len == 0 {
        return;
    }

    let mut mode = plan.shaper.normalization_preference;
    if mode == HB_OT_SHAPE_NORMALIZATION_MODE_AUTO {
        if plan.has_gpos_mark {
            // https://github.com/harfbuzz/harfbuzz/issues/653#issuecomment-423905920
            //mode = HB_OT_SHAPE_NORMALIZATION_MODE_DECOMPOSED;
            mode = HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS;
        } else {
            mode = HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS;
        }
    }

    let c = HbOtShapeNormalizeContext {
        plan,
        unicode: buffer.unicode,
        not_found: buffer.not_found,
        decompose: plan.shaper.decompose.unwrap_or(decompose_unicode),
        compose: plan.shaper.compose.unwrap_or(compose_unicode),
    };

    let always_short_circuit = mode == HB_OT_SHAPE_NORMALIZATION_MODE_NONE;
    let might_short_circuit = always_short_circuit
        || (mode != HB_OT_SHAPE_NORMALIZATION_MODE_DECOMPOSED
            && mode != HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS_NO_SHORT_CIRCUIT);
    let mut count;

    /* We do a fairly straightforward yet custom normalization process in three
     * separate rounds: decompose, reorder, recompose (if desired).  Currently
     * this makes two buffer swaps.  We can make it faster by moving the last
     * two rounds into the inner loop for the first round, but it's more readable
     * this way. */

    /* First round, decompose */

    let mut all_simple = true;
    {
        buffer.clear_output();
        count = buffer.len;
        buffer.idx = 0;
        loop {
            let mut end = buffer.idx + 1;
            while end < count {
                if _hb_glyph_info_is_unicode_mark(&buffer.info[end as usize]) {
                    break;
                }
                end += 1;
            }

            if end < count {
                end -= 1; /* Leave one base for the marks to cluster with. */
            }

            /* From idx to end are simple clusters. */
            if might_short_circuit {
                let start = buffer.idx as usize;
                let n = (end - buffer.idx) as usize;
                let unicodes: Vec<HbCodepoint> = buffer.info[start..start + n]
                    .iter()
                    .map(|i| i.codepoint)
                    .collect();
                let mut glyphs: Vec<HbCodepoint> = buffer.info[start..start + n]
                    .iter()
                    .map(glyph_index)
                    .collect();
                let done = font.get_nominal_glyphs(&unicodes, &mut glyphs);
                for (k, g) in glyphs.iter().enumerate() {
                    set_glyph_index(&mut buffer.info[start + k], *g);
                }
                if !buffer.next_glyphs(done) {
                    break;
                }
            }
            while buffer.idx < end && buffer.successful {
                decompose_current_character(&c, buffer, font, might_short_circuit);
            }

            if buffer.idx == count || !buffer.successful {
                break;
            }

            all_simple = false;

            /* Find all the marks now. */
            end = buffer.idx + 1;
            while end < count {
                if !_hb_glyph_info_is_unicode_mark(&buffer.info[end as usize]) {
                    break;
                }
                end += 1;
            }

            /* idx to end is one non-simple cluster. */
            decompose_multi_char_cluster(&c, buffer, font, end, always_short_circuit);

            if !(buffer.idx < count && buffer.successful) {
                break;
            }
        }
        let _ = buffer.sync();
    }

    /* Second round, reorder (inplace) */

    if !all_simple {
        count = buffer.len;
        let mut i = 0;
        while i < count {
            if _hb_glyph_info_get_modified_combining_class(&buffer.info[i as usize]) == 0 {
                i += 1;
                continue;
            }

            let mut end = i + 1;
            while end < count {
                if _hb_glyph_info_get_modified_combining_class(&buffer.info[end as usize]) == 0 {
                    break;
                }
                end += 1;
            }

            /* We are going to do a O(n^2).  Only do this if the sequence is short. */
            if end - i > HB_OT_SHAPE_MAX_COMBINING_MARKS {
                i = end + 1;
                continue;
            }

            buffer.sort(i, end, compare_combining_class);

            if let Some(reorder_marks) = plan.shaper.reorder_marks {
                reorder_marks(plan, buffer, i, end);
            }

            i = end + 1;
        }
    }
    if buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_HAS_CGJ != 0 {
        /* For all CGJ, check if it prevented any reordering at all.
         * If it did NOT, then make it skippable.
         * https://github.com/harfbuzz/harfbuzz/issues/554
         */
        let count = buffer.len as usize;
        let info = &mut buffer.info;
        let mut i = 1;
        while i + 1 < count {
            if info[i].codepoint == 0x034F/*CGJ*/
                && (info_cc(&info[i + 1]) == 0 || info_cc(&info[i - 1]) <= info_cc(&info[i + 1]))
            {
                _hb_glyph_info_unhide(&mut info[i]);
            }
            i += 1;
        }
    }

    /* Third round, recompose */

    if !all_simple
        && buffer.successful
        && (mode == HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS
            || mode == HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS_NO_SHORT_CIRCUIT)
    {
        /* As noted in the comment earlier, we don't try to combine
         * ccc=0 chars with their previous Starter. */

        buffer.clear_output();
        count = buffer.len;
        let mut starter = 0;
        let _ = buffer.next_glyph();
        while buffer.idx < count
        /* No need for: && buffer->successful */
        {
            let mut composed = 0;
            let mut glyph = 0;
            if
            /* We don't try to compose a non-mark character with it's preceding starter.
             * This is both an optimization to avoid trying to compose every two neighboring
             * glyphs in most scripts AND a desired feature for Hangul.  Apparently Hangul
             * fonts are not designed to mix-and-match pre-composed syllables and Jamo. */
            _hb_glyph_info_is_unicode_mark(buffer.cur(0)) {
                if
                /* If there's anything between the starter and this char, they should have CCC
                 * smaller than this character's. */
                (starter == buffer.out_len - 1 || info_cc(buffer.prev()) < info_cc(buffer.cur(0))) &&
                    /* And compose. */
                    (c.compose)(&c, buffer.out_info(starter).codepoint, buffer.cur(0).codepoint, &mut composed) &&
                    /* And the font has glyph for the composite. */
                    font.get_nominal_glyph(composed, &mut glyph, 0)
                {
                    /* Composes. */
                    if !buffer.next_glyph() {
                        break; /* Copy to out-buffer. */
                    }
                    let out_len = buffer.out_len;
                    buffer.merge_out_clusters(starter, out_len);
                    buffer.out_len -= 1; /* Remove the second composable. */
                    /* Modify starter and carry on. */
                    buffer.out_info_mut(starter).codepoint = composed;
                    set_glyph_index(buffer.out_info_mut(starter), glyph);
                    let mut scratch = buffer.scratch_flags;
                    _hb_glyph_info_set_unicode_props(buffer.out_info_mut(starter), &mut scratch);
                    buffer.scratch_flags = scratch;

                    continue;
                }
            }

            /* Blocked, or doesn't compose. */
            if !buffer.next_glyph() {
                break;
            }

            if info_cc(buffer.prev()) == 0 {
                starter = buffer.out_len - 1;
            }
        }
        let _ = buffer.sync();
    }
}
