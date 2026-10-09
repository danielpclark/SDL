// Rust translation of src/hb-ot-layout.hh, src/hb-ot-layout.cc and the
// public constants of src/hb-ot-layout.h from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it).
// Copyright © 1998-2004  David Turner and Werner Lemberg
// Copyright © 2006  Behdad Esfahbod
// Copyright © 2007,2008,2009  Red Hat, Inc.
// Copyright © 2012,2013  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! OpenType layout: the buffer variables of shaping, and the GSUB/GPOS
//! script, language, feature and lookup selection and application.
//!
//! Translation notes: the GSUB/GPOS lookup accelerators' digests and
//! caches only make HarfBuzz skip work that would not change anything
//! (every subtable starts by checking its coverage of the current glyph),
//! so they are not translated. The BASE table, the name IDs and size
//! parameters of features, closures and the other APIs SDL_ttf's shaping
//! does not reach are not translated either.

use std::collections::HashMap;

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_face::*;
use super::hb_font::HbFont;
use super::hb_ot_layout_common::*;
use super::hb_ot_layout_gpos;
use super::hb_ot_layout_gsub;
use super::hb_ot_layout_gsubgpos::*;
use super::hb_set::HbSet;
use super::hb_unicode::*;

/* Public constants of hb-ot-layout.h */

/// `HB_OT_TAG_BASE`
pub const HB_OT_TAG_BASE: HbTag = hb_tag(b'B', b'A', b'S', b'E');
/// `HB_OT_TAG_JSTF`
pub const HB_OT_TAG_JSTF: HbTag = hb_tag(b'J', b'S', b'T', b'F');

/// `HB_OT_TAG_DEFAULT_SCRIPT`: OpenType script tag, `DFLT`, for features
/// that are not script-specific.
pub const HB_OT_TAG_DEFAULT_SCRIPT: HbTag = hb_tag(b'D', b'F', b'L', b'T');
/// `HB_OT_TAG_DEFAULT_LANGUAGE`: OpenType language tag, `dflt`. Not a
/// valid language tag, but some fonts mistakenly use it.
pub const HB_OT_TAG_DEFAULT_LANGUAGE: HbTag = hb_tag(b'd', b'f', b'l', b't');

/// `hb_ot_layout_glyph_class_t`: the GDEF classes defined for glyphs.
pub type HbOtLayoutGlyphClass = u32;
/// Glyphs not matching the other classifications
pub const HB_OT_LAYOUT_GLYPH_CLASS_UNCLASSIFIED: HbOtLayoutGlyphClass = 0;
/// Spacing, single characters, capable of accepting marks
pub const HB_OT_LAYOUT_GLYPH_CLASS_BASE_GLYPH: HbOtLayoutGlyphClass = 1;
/// Glyphs that represent ligation of multiple characters
pub const HB_OT_LAYOUT_GLYPH_CLASS_LIGATURE: HbOtLayoutGlyphClass = 2;
/// Non-spacing, combining glyphs that represent marks
pub const HB_OT_LAYOUT_GLYPH_CLASS_MARK: HbOtLayoutGlyphClass = 3;
/// Spacing glyphs that represent part of a single character
pub const HB_OT_LAYOUT_GLYPH_CLASS_COMPONENT: HbOtLayoutGlyphClass = 4;

/// `HB_OT_LAYOUT_NO_SCRIPT_INDEX`: special value for script index
/// indicating unsupported script.
pub const HB_OT_LAYOUT_NO_SCRIPT_INDEX: u32 = 0xFFFF;
/// `HB_OT_LAYOUT_NO_FEATURE_INDEX`: special value for feature index
/// indicating unsupported feature.
pub const HB_OT_LAYOUT_NO_FEATURE_INDEX: u32 = 0xFFFF;
/// `HB_OT_LAYOUT_DEFAULT_LANGUAGE_INDEX`: special value for language index
/// indicating default or unsupported language.
pub const HB_OT_LAYOUT_DEFAULT_LANGUAGE_INDEX: u32 = 0xFFFF;
/// `HB_OT_LAYOUT_NO_VARIATIONS_INDEX`: special value for variations index
/// indicating unsupported variation.
pub const HB_OT_LAYOUT_NO_VARIATIONS_INDEX: u32 = 0xFFFFFFFF;

/* hb-limits.hh */
const HB_MAX_SCRIPTS: u32 = 500;
const HB_MAX_LANGSYS: u32 = 2000;
const HB_MAX_FEATURE_INDICES: u32 = 1500;

/*
 * kern
 */

/// `hb_ot_layout_has_kerning`
pub(crate) fn hb_ot_layout_has_kerning(face: &HbFace) -> bool {
    face.kern().has_data()
}

/// `hb_ot_layout_has_machine_kerning`
pub(crate) fn hb_ot_layout_has_machine_kerning(face: &HbFace) -> bool {
    face.kern().has_state_machine()
}

/// `hb_ot_layout_has_cross_kerning`
pub(crate) fn hb_ot_layout_has_cross_kerning(face: &HbFace) -> bool {
    face.kern().has_cross_stream()
}

/// `hb_ot_layout_kern`, with the plan's `requested_kerning` and
/// `kern_mask`.
pub(crate) fn hb_ot_layout_kern(
    requested_kerning: bool,
    kern_mask: HbMask,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
) {
    let face = font.p.face.clone();
    let kern = face.kern();
    /* AAT::hb_aat_apply_context_t c (plan, font, buffer, blob); */
    kern.apply(&face, font, buffer, requested_kerning, kern_mask);
}

/*
 * GDEF
 */

/// `hb_ot_layout_glyph_props_flags_t`
pub(crate) const HB_OT_LAYOUT_GLYPH_PROPS_BASE_GLYPH: u32 = 0x02;
pub(crate) const HB_OT_LAYOUT_GLYPH_PROPS_LIGATURE: u32 = 0x04;
pub(crate) const HB_OT_LAYOUT_GLYPH_PROPS_MARK: u32 = 0x08;
/* The following are used internally; not derived from GDEF. */
pub(crate) const HB_OT_LAYOUT_GLYPH_PROPS_SUBSTITUTED: u32 = 0x10;
pub(crate) const HB_OT_LAYOUT_GLYPH_PROPS_LIGATED: u32 = 0x20;
pub(crate) const HB_OT_LAYOUT_GLYPH_PROPS_MULTIPLIED: u32 = 0x40;
pub(crate) const HB_OT_LAYOUT_GLYPH_PROPS_PRESERVE: u32 = HB_OT_LAYOUT_GLYPH_PROPS_SUBSTITUTED
    | HB_OT_LAYOUT_GLYPH_PROPS_LIGATED
    | HB_OT_LAYOUT_GLYPH_PROPS_MULTIPLIED;

/*
 * Buffer var routines.
 */

/// `_hb_next_syllable`
#[inline]
pub(crate) fn _hb_next_syllable(buffer: &HbBuffer, mut start: u32) -> u32 {
    let info = &buffer.info;
    let count = buffer.len;

    let syllable = info[start as usize].syllable();
    loop {
        start += 1;
        if !(start < count && syllable == info[start as usize].syllable()) {
            break;
        }
    }

    start
}

/// `foreach_syllable`: the `(start, end)` ranges of the syllables (taken
/// up front; C computes each `end` after the loop body runs on the
/// previous syllable, which only differs if the body changes the
/// buffer's syllables, which no caller does).
pub(crate) fn foreach_syllable(buffer: &HbBuffer) -> Vec<(u32, u32)> {
    let count = buffer.len;
    let mut v = Vec::new();
    let mut start = 0;
    let mut end = if count != 0 {
        _hb_next_syllable(buffer, 0)
    } else {
        0
    };
    while start < count {
        v.push((start, end));
        start = end;
        end = _hb_next_syllable(buffer, start);
    }
    v
}

/* unicode_props */

/* Design:
 * unicode_props() is a two-byte number.  The low byte includes:
 * - General_Category: 5 bits.
 * - A bit each for:
 *   * Is it Default_Ignorable(); we have a modified Default_Ignorable().
 *   * Whether it's one of the four Mongolian Free Variation Selectors,
 *     CGJ, or other characters that are hidden but should not be ignored
 *     like most other Default_Ignorable()s do during matching.
 *   * Whether it's a grapheme continuation.
 *
 * The high-byte has different meanings, switched by the Gen-Cat:
 * - For Mn,Mc,Me: the modified Combining_Class.
 * - For Cf: whether it's ZWJ, ZWNJ, or something else.
 * - For Ws: index of which space character this is, if space fallback
 *   is needed, ie. we don't set this by default, only if asked to.
 */

/// `hb_unicode_props_flags_t`
pub(crate) const UPROPS_MASK_GEN_CAT: u32 = 0x001F;
pub(crate) const UPROPS_MASK_IGNORABLE: u32 = 0x0020;
pub(crate) const UPROPS_MASK_HIDDEN: u32 = 0x0040; /* MONGOLIAN FREE VARIATION SELECTOR 1..4, or TAG characters */
pub(crate) const UPROPS_MASK_CONTINUATION: u32 = 0x0080;

/* If GEN_CAT=FORMAT, top byte masks: */
pub(crate) const UPROPS_MASK_Cf_ZWJ: u32 = 0x0100;
pub(crate) const UPROPS_MASK_Cf_ZWNJ: u32 = 0x0200;

/// `_hb_glyph_info_set_unicode_props`
#[inline]
pub(crate) fn _hb_glyph_info_set_unicode_props(
    info: &mut HbGlyphInfo,
    scratch_flags: &mut HbBufferScratchFlags,
) {
    let unicode = HbUnicodeFuncs;
    let u = info.codepoint;
    let gen_cat = unicode.general_category(u);
    let mut props = gen_cat;

    if u >= 0x80 {
        *scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_NON_ASCII;

        if HbUnicodeFuncs::is_default_ignorable(u) {
            *scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_DEFAULT_IGNORABLES;
            props |= UPROPS_MASK_IGNORABLE;
            if u == 0x200C {
                props |= UPROPS_MASK_Cf_ZWNJ;
            } else if u == 0x200D {
                props |= UPROPS_MASK_Cf_ZWJ;
            }
            /* Mongolian Free Variation Selectors need to be remembered
             * because although we need to hide them like default-ignorables,
             * they need to non-ignorable during shaping.  This is similar to
             * what we do for joiners in Indic-like shapers, but since the
             * FVSes are GC=Mn, we have use a separate bit to remember them.
             * Fixes:
             * https://github.com/harfbuzz/harfbuzz/issues/234 */
            else if (0x180B..=0x180D).contains(&u) || u == 0x180F {
                props |= UPROPS_MASK_HIDDEN;
            }
            /* TAG characters need similar treatment. Fixes:
             * https://github.com/harfbuzz/harfbuzz/issues/463 */
            else if (0xE0020..=0xE007F).contains(&u) {
                props |= UPROPS_MASK_HIDDEN;
            }
            /* COMBINING GRAPHEME JOINER should not be skipped; at least some times.
             * https://github.com/harfbuzz/harfbuzz/issues/554 */
            else if u == 0x034F {
                *scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_CGJ;
                props |= UPROPS_MASK_HIDDEN;
            }
        }

        if hb_unicode_general_category_is_mark(gen_cat) {
            props |= UPROPS_MASK_CONTINUATION;
            props |= unicode.modified_combining_class(u) << 8;
        }
    }

    info.set_unicode_props(props);
}

/// `_hb_glyph_info_set_general_category`
#[inline]
pub(crate) fn _hb_glyph_info_set_general_category(
    info: &mut HbGlyphInfo,
    gen_cat: HbUnicodeGeneralCategory,
) {
    /* Clears top-byte. */
    info.set_unicode_props(gen_cat | (info.unicode_props() & (0xFF & !UPROPS_MASK_GEN_CAT)));
}

/// `_hb_glyph_info_get_general_category`
#[inline]
pub(crate) fn _hb_glyph_info_get_general_category(info: &HbGlyphInfo) -> HbUnicodeGeneralCategory {
    info.unicode_props() & UPROPS_MASK_GEN_CAT
}

/// `_hb_glyph_info_is_unicode_mark`
#[inline]
pub(crate) fn _hb_glyph_info_is_unicode_mark(info: &HbGlyphInfo) -> bool {
    hb_unicode_general_category_is_mark(info.unicode_props() & UPROPS_MASK_GEN_CAT)
}

/// `_hb_glyph_info_set_modified_combining_class`
#[inline]
pub(crate) fn _hb_glyph_info_set_modified_combining_class(
    info: &mut HbGlyphInfo,
    modified_class: u32,
) {
    if !_hb_glyph_info_is_unicode_mark(info) {
        return;
    }
    info.set_unicode_props((modified_class << 8) | (info.unicode_props() & 0xFF));
}

/// `_hb_glyph_info_get_modified_combining_class`
#[inline]
pub(crate) fn _hb_glyph_info_get_modified_combining_class(info: &HbGlyphInfo) -> u32 {
    if _hb_glyph_info_is_unicode_mark(info) {
        info.unicode_props() >> 8
    } else {
        0
    }
}

/// `info_cc`
#[inline]
pub(crate) fn info_cc(info: &HbGlyphInfo) -> u32 {
    _hb_glyph_info_get_modified_combining_class(info)
}

/// `_hb_glyph_info_is_unicode_space`
#[inline]
pub(crate) fn _hb_glyph_info_is_unicode_space(info: &HbGlyphInfo) -> bool {
    _hb_glyph_info_get_general_category(info) == HB_UNICODE_GENERAL_CATEGORY_SPACE_SEPARATOR
}

/// `_hb_glyph_info_set_unicode_space_fallback_type`
#[inline]
pub(crate) fn _hb_glyph_info_set_unicode_space_fallback_type(info: &mut HbGlyphInfo, s: u32) {
    if !_hb_glyph_info_is_unicode_space(info) {
        return;
    }
    info.set_unicode_props((s << 8) | (info.unicode_props() & 0xFF));
}

/// `_hb_glyph_info_get_unicode_space_fallback_type`
#[inline]
pub(crate) fn _hb_glyph_info_get_unicode_space_fallback_type(info: &HbGlyphInfo) -> u32 {
    if _hb_glyph_info_is_unicode_space(info) {
        info.unicode_props() >> 8
    } else {
        NOT_SPACE as u32
    }
}

/// `_hb_glyph_info_is_default_ignorable`
#[inline]
pub(crate) fn _hb_glyph_info_is_default_ignorable(info: &HbGlyphInfo) -> bool {
    (info.unicode_props() & UPROPS_MASK_IGNORABLE) != 0 && !_hb_glyph_info_substituted(info)
}

/// `_hb_glyph_info_is_default_ignorable_and_not_hidden`
#[inline]
pub(crate) fn _hb_glyph_info_is_default_ignorable_and_not_hidden(info: &HbGlyphInfo) -> bool {
    (info.unicode_props() & (UPROPS_MASK_IGNORABLE | UPROPS_MASK_HIDDEN)) == UPROPS_MASK_IGNORABLE
        && !_hb_glyph_info_substituted(info)
}

/// `_hb_glyph_info_unhide`
#[inline]
pub(crate) fn _hb_glyph_info_unhide(info: &mut HbGlyphInfo) {
    info.set_unicode_props(info.unicode_props() & !UPROPS_MASK_HIDDEN);
}

/// `_hb_glyph_info_set_continuation`
#[inline]
pub(crate) fn _hb_glyph_info_set_continuation(info: &mut HbGlyphInfo) {
    info.set_unicode_props(info.unicode_props() | UPROPS_MASK_CONTINUATION);
}

/// `_hb_glyph_info_reset_continuation`
#[inline]
pub(crate) fn _hb_glyph_info_reset_continuation(info: &mut HbGlyphInfo) {
    info.set_unicode_props(info.unicode_props() & !UPROPS_MASK_CONTINUATION);
}

/// `_hb_glyph_info_is_continuation`
#[inline]
pub(crate) fn _hb_glyph_info_is_continuation(info: &HbGlyphInfo) -> bool {
    info.unicode_props() & UPROPS_MASK_CONTINUATION != 0
}

/// `_hb_grapheme_group_func`
#[inline]
pub(crate) fn _hb_grapheme_group_func(_a: &HbGlyphInfo, b: &HbGlyphInfo) -> bool {
    _hb_glyph_info_is_continuation(b)
}

/// `foreach_grapheme`
pub(crate) fn foreach_grapheme(buffer: &HbBuffer) -> Vec<(u32, u32)> {
    foreach_group(buffer, _hb_grapheme_group_func)
}

/// `_hb_ot_layout_reverse_graphemes`
#[inline]
pub(crate) fn _hb_ot_layout_reverse_graphemes(buffer: &mut HbBuffer) {
    let merge = buffer.cluster_level == HB_BUFFER_CLUSTER_LEVEL_MONOTONE_CHARACTERS;
    buffer.reverse_groups(_hb_grapheme_group_func, merge);
}

/// `_hb_glyph_info_is_unicode_format`
#[inline]
pub(crate) fn _hb_glyph_info_is_unicode_format(info: &HbGlyphInfo) -> bool {
    _hb_glyph_info_get_general_category(info) == HB_UNICODE_GENERAL_CATEGORY_FORMAT
}

/// `_hb_glyph_info_is_zwnj`
#[inline]
pub(crate) fn _hb_glyph_info_is_zwnj(info: &HbGlyphInfo) -> bool {
    _hb_glyph_info_is_unicode_format(info) && (info.unicode_props() & UPROPS_MASK_Cf_ZWNJ) != 0
}

/// `_hb_glyph_info_is_zwj`
#[inline]
pub(crate) fn _hb_glyph_info_is_zwj(info: &HbGlyphInfo) -> bool {
    _hb_glyph_info_is_unicode_format(info) && (info.unicode_props() & UPROPS_MASK_Cf_ZWJ) != 0
}

/// `_hb_glyph_info_is_joiner`
#[inline]
pub(crate) fn _hb_glyph_info_is_joiner(info: &HbGlyphInfo) -> bool {
    _hb_glyph_info_is_unicode_format(info)
        && (info.unicode_props() & (UPROPS_MASK_Cf_ZWNJ | UPROPS_MASK_Cf_ZWJ)) != 0
}

/// `_hb_glyph_info_flip_joiners`
#[inline]
pub(crate) fn _hb_glyph_info_flip_joiners(info: &mut HbGlyphInfo) {
    if !_hb_glyph_info_is_unicode_format(info) {
        return;
    }
    info.set_unicode_props(info.unicode_props() ^ (UPROPS_MASK_Cf_ZWNJ | UPROPS_MASK_Cf_ZWJ));
}

/* lig_props: aka lig_id / lig_comp
 *
 * When a ligature is formed:
 *
 *   - The ligature glyph and any marks in between all the same newly allocated
 *     lig_id,
 *   - The ligature glyph will get lig_num_comps set to the number of components
 *   - The marks get lig_comp > 0, reflecting which component of the ligature
 *     they were applied to.
 *   - This is used in GPOS to attach marks to the right component of a ligature
 *     in MarkLigPos,
 *   - Note that when marks are ligated together, much of the above is skipped
 *     and the current lig_id reused.
 *
 * When a multiple-substitution is done:
 *
 *   - All resulting glyphs will have lig_id = 0,
 *   - The resulting glyphs will have lig_comp = 0, 1, 2, ... respectively.
 *   - This is used in GPOS to attach marks to the first component of a
 *     multiple substitution in MarkBasePos.
 *
 * The numbers are also used in GPOS to do mark-to-mark positioning only
 * to marks that belong to the same component of the same ligature.
 */

/// `_hb_glyph_info_clear_lig_props`
#[inline]
pub(crate) fn _hb_glyph_info_clear_lig_props(info: &mut HbGlyphInfo) {
    info.set_lig_props(0);
}

const IS_LIG_BASE: u32 = 0x10;

/// `_hb_glyph_info_set_lig_props_for_ligature`
#[inline]
pub(crate) fn _hb_glyph_info_set_lig_props_for_ligature(
    info: &mut HbGlyphInfo,
    lig_id: u32,
    lig_num_comps: u32,
) {
    info.set_lig_props((lig_id << 5) | IS_LIG_BASE | (lig_num_comps & 0x0F));
}

/// `_hb_glyph_info_set_lig_props_for_mark`
#[inline]
pub(crate) fn _hb_glyph_info_set_lig_props_for_mark(
    info: &mut HbGlyphInfo,
    lig_id: u32,
    lig_comp: u32,
) {
    info.set_lig_props((lig_id << 5) | (lig_comp & 0x0F));
}

/// `_hb_glyph_info_set_lig_props_for_component`
#[inline]
pub(crate) fn _hb_glyph_info_set_lig_props_for_component(info: &mut HbGlyphInfo, comp: u32) {
    _hb_glyph_info_set_lig_props_for_mark(info, 0, comp);
}

/// `_hb_glyph_info_get_lig_id`
#[inline]
pub(crate) fn _hb_glyph_info_get_lig_id(info: &HbGlyphInfo) -> u32 {
    info.lig_props() >> 5
}

/// `_hb_glyph_info_ligated_internal`
#[inline]
pub(crate) fn _hb_glyph_info_ligated_internal(info: &HbGlyphInfo) -> bool {
    info.lig_props() & IS_LIG_BASE != 0
}

/// `_hb_glyph_info_get_lig_comp`
#[inline]
pub(crate) fn _hb_glyph_info_get_lig_comp(info: &HbGlyphInfo) -> u32 {
    if _hb_glyph_info_ligated_internal(info) {
        0
    } else {
        info.lig_props() & 0x0F
    }
}

/// `_hb_glyph_info_get_lig_num_comps`
#[inline]
pub(crate) fn _hb_glyph_info_get_lig_num_comps(info: &HbGlyphInfo) -> u32 {
    if (info.glyph_props() & HB_OT_LAYOUT_GLYPH_PROPS_LIGATURE) != 0
        && _hb_glyph_info_ligated_internal(info)
    {
        info.lig_props() & 0x0F
    } else {
        1
    }
}

/// `_hb_allocate_lig_id`
#[inline]
pub(crate) fn _hb_allocate_lig_id(buffer: &mut HbBuffer) -> u8 {
    let mut lig_id = buffer.next_serial() & 0x07;
    if lig_id == 0 {
        lig_id = _hb_allocate_lig_id(buffer); /* in case of overflow */
    }
    lig_id
}

/* glyph_props: */

/// `_hb_glyph_info_set_glyph_props`
#[inline]
pub(crate) fn _hb_glyph_info_set_glyph_props(info: &mut HbGlyphInfo, props: u32) {
    info.set_glyph_props(props);
}

/// `_hb_glyph_info_get_glyph_props`
#[inline]
pub(crate) fn _hb_glyph_info_get_glyph_props(info: &HbGlyphInfo) -> u32 {
    info.glyph_props()
}

/// `_hb_glyph_info_is_base_glyph`
#[inline]
pub(crate) fn _hb_glyph_info_is_base_glyph(info: &HbGlyphInfo) -> bool {
    info.glyph_props() & HB_OT_LAYOUT_GLYPH_PROPS_BASE_GLYPH != 0
}

/// `_hb_glyph_info_is_ligature`
#[inline]
pub(crate) fn _hb_glyph_info_is_ligature(info: &HbGlyphInfo) -> bool {
    info.glyph_props() & HB_OT_LAYOUT_GLYPH_PROPS_LIGATURE != 0
}

/// `_hb_glyph_info_is_mark`
#[inline]
pub(crate) fn _hb_glyph_info_is_mark(info: &HbGlyphInfo) -> bool {
    info.glyph_props() & HB_OT_LAYOUT_GLYPH_PROPS_MARK != 0
}

/// `_hb_glyph_info_substituted`
#[inline]
pub(crate) fn _hb_glyph_info_substituted(info: &HbGlyphInfo) -> bool {
    info.glyph_props() & HB_OT_LAYOUT_GLYPH_PROPS_SUBSTITUTED != 0
}

/// `_hb_glyph_info_ligated`
#[inline]
pub(crate) fn _hb_glyph_info_ligated(info: &HbGlyphInfo) -> bool {
    info.glyph_props() & HB_OT_LAYOUT_GLYPH_PROPS_LIGATED != 0
}

/// `_hb_glyph_info_multiplied`
#[inline]
pub(crate) fn _hb_glyph_info_multiplied(info: &HbGlyphInfo) -> bool {
    info.glyph_props() & HB_OT_LAYOUT_GLYPH_PROPS_MULTIPLIED != 0
}

/// `_hb_glyph_info_ligated_and_didnt_multiply`
#[inline]
pub(crate) fn _hb_glyph_info_ligated_and_didnt_multiply(info: &HbGlyphInfo) -> bool {
    _hb_glyph_info_ligated(info) && !_hb_glyph_info_multiplied(info)
}

/// `_hb_glyph_info_clear_ligated_and_multiplied`
#[inline]
pub(crate) fn _hb_glyph_info_clear_ligated_and_multiplied(info: &mut HbGlyphInfo) {
    info.set_glyph_props(
        info.glyph_props()
            & !(HB_OT_LAYOUT_GLYPH_PROPS_LIGATED | HB_OT_LAYOUT_GLYPH_PROPS_MULTIPLIED),
    );
}

/// `_hb_glyph_info_clear_substituted`
#[inline]
pub(crate) fn _hb_glyph_info_clear_substituted(info: &mut HbGlyphInfo) {
    info.set_glyph_props(info.glyph_props() & !HB_OT_LAYOUT_GLYPH_PROPS_SUBSTITUTED);
}

/// `_hb_clear_substitution_flags` (a pause function)
pub(crate) fn _hb_clear_substitution_flags(buffer: &mut HbBuffer) -> bool {
    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        _hb_glyph_info_clear_substituted(info);
    }
    false
}

/* Allocation / deallocation: the byte ranges of `var1` (0..4) and `var2`
 * (4..8) the variables use. */

/// `glyph_props` (`var1.u16[0]`)
pub(crate) const VAR_GLYPH_PROPS: (u32, u32) = (0, 2);
/// `lig_props` (`var1.u8[2]`)
pub(crate) const VAR_LIG_PROPS: (u32, u32) = (2, 1);
/// `syllable` (`var1.u8[3]`)
pub(crate) const VAR_SYLLABLE: (u32, u32) = (3, 1);
/// `unicode_props` (`var2.u16[0]`)
pub(crate) const VAR_UNICODE_PROPS: (u32, u32) = (4, 2);
/// `complex_var_u8_category` (`var2.u8[2]`)
pub(crate) const VAR_COMPLEX_CATEGORY: (u32, u32) = (6, 1);
/// `complex_var_u8_auxiliary` (`var2.u8[3]`)
pub(crate) const VAR_COMPLEX_AUXILIARY: (u32, u32) = (7, 1);
/// `glyph_index` (`var1.u32`)
pub(crate) const VAR_GLYPH_INDEX: (u32, u32) = (0, 4);
/// `normalizer_glyph_index` (`var1.u32`)
pub(crate) const VAR_NORMALIZER_GLYPH_INDEX: (u32, u32) = (0, 4);

/// `_hb_buffer_allocate_unicode_vars`
#[inline]
pub(crate) fn _hb_buffer_allocate_unicode_vars(buffer: &mut HbBuffer) {
    buffer.allocate_var(VAR_UNICODE_PROPS.0, VAR_UNICODE_PROPS.1);
}

/// `_hb_buffer_deallocate_unicode_vars`
#[inline]
pub(crate) fn _hb_buffer_deallocate_unicode_vars(buffer: &mut HbBuffer) {
    buffer.deallocate_var(VAR_UNICODE_PROPS.0, VAR_UNICODE_PROPS.1);
}

/// `_hb_buffer_allocate_gsubgpos_vars`
#[inline]
pub(crate) fn _hb_buffer_allocate_gsubgpos_vars(buffer: &mut HbBuffer) {
    buffer.allocate_var(VAR_GLYPH_PROPS.0, VAR_GLYPH_PROPS.1);
    buffer.allocate_var(VAR_LIG_PROPS.0, VAR_LIG_PROPS.1);
}

/// `_hb_buffer_deallocate_gsubgpos_vars`
#[inline]
pub(crate) fn _hb_buffer_deallocate_gsubgpos_vars(buffer: &mut HbBuffer) {
    buffer.deallocate_var(VAR_LIG_PROPS.0, VAR_LIG_PROPS.1);
    buffer.deallocate_var(VAR_GLYPH_PROPS.0, VAR_GLYPH_PROPS.1);
}

/*
 * hb-ot-layout.cc
 */

/// `_hb_ot_layout_set_glyph_props`
fn _hb_ot_layout_set_glyph_props(face: &HbFace, buffer: &mut HbBuffer) {
    let gdef = face.gdef();

    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        _hb_glyph_info_set_glyph_props(info, gdef.get_glyph_props(info.codepoint));
        _hb_glyph_info_clear_lig_props(info);
    }
}

/* Public API */

/// `hb_ot_layout_has_glyph_classes`
pub(crate) fn hb_ot_layout_has_glyph_classes(face: &HbFace) -> bool {
    face.gdef().table().has_glyph_classes()
}

/// `hb_ot_layout_get_glyph_class`
pub(crate) fn hb_ot_layout_get_glyph_class(
    face: &HbFace,
    glyph: HbCodepoint,
) -> HbOtLayoutGlyphClass {
    face.gdef().table().get_glyph_class(glyph)
}

/*
 * GSUB/GPOS
 */

/// `get_gsubgpos_table`
fn get_gsubgpos_table(face: &HbFace, table_tag: HbTag) -> GsubGpos<'_> {
    match table_tag {
        HB_OT_TAG_GSUB => face.gsub().table(),
        HB_OT_TAG_GPOS => face.gpos().table(),
        _ => GsubGpos(&[]),
    }
}

/// `hb_ot_layout_table_get_script_tags`
pub(crate) fn hb_ot_layout_table_get_script_tags(
    face: &HbFace,
    table_tag: HbTag,
    start_offset: u32,
    max_count: u32,
    script_tags: &mut Vec<HbTag>,
) -> u32 {
    let g = get_gsubgpos_table(face, table_tag);
    g.get_script_list()
        .get_tags(start_offset, max_count, script_tags)
}

const HB_OT_TAG_LATIN_SCRIPT: HbTag = hb_tag(b'l', b'a', b't', b'n');

/// `hb_ot_layout_table_find_script`
pub(crate) fn hb_ot_layout_table_find_script(
    face: &HbFace,
    table_tag: HbTag,
    script_tag: HbTag,
    script_index: &mut u32,
) -> bool {
    let g = get_gsubgpos_table(face, table_tag);

    if g.find_script_index(script_tag, script_index) {
        return true;
    }

    /* try finding 'DFLT' */
    if g.find_script_index(HB_OT_TAG_DEFAULT_SCRIPT, script_index) {
        return false;
    }

    /* try with 'dflt'; MS site has had typos and many fonts use it now :(.
     * including many versions of DejaVu Sans Mono! */
    if g.find_script_index(HB_OT_TAG_DEFAULT_LANGUAGE, script_index) {
        return false;
    }

    /* try with 'latn'; some old fonts put their features there even though
    they're really trying to support Thai, for example :( */
    if g.find_script_index(HB_OT_TAG_LATIN_SCRIPT, script_index) {
        return false;
    }

    *script_index = HB_OT_LAYOUT_NO_SCRIPT_INDEX;
    false
}

/// `hb_ot_layout_table_select_script`
pub(crate) fn hb_ot_layout_table_select_script(
    face: &HbFace,
    table_tag: HbTag,
    script_tags: &[HbTag],
    script_index: &mut u32,
    chosen_script: &mut HbTag,
) -> bool {
    let g = get_gsubgpos_table(face, table_tag);

    for &t in script_tags {
        if g.find_script_index(t, script_index) {
            *chosen_script = t;
            return true;
        }
    }

    /* try finding 'DFLT' */
    if g.find_script_index(HB_OT_TAG_DEFAULT_SCRIPT, script_index) {
        *chosen_script = HB_OT_TAG_DEFAULT_SCRIPT;
        return false;
    }

    /* try with 'dflt'; MS site has had typos and many fonts use it now :( */
    if g.find_script_index(HB_OT_TAG_DEFAULT_LANGUAGE, script_index) {
        *chosen_script = HB_OT_TAG_DEFAULT_LANGUAGE;
        return false;
    }

    /* try with 'latn'; some old fonts put their features there even though
    they're really trying to support Thai, for example :( */
    if g.find_script_index(HB_OT_TAG_LATIN_SCRIPT, script_index) {
        *chosen_script = HB_OT_TAG_LATIN_SCRIPT;
        return false;
    }

    *script_index = HB_OT_LAYOUT_NO_SCRIPT_INDEX;
    *chosen_script = HB_TAG_NONE;
    false
}

/// `hb_ot_layout_table_find_feature`
pub(crate) fn hb_ot_layout_table_find_feature(
    face: &HbFace,
    table_tag: HbTag,
    feature_tag: HbTag,
    feature_index: &mut u32,
) -> bool {
    let g = get_gsubgpos_table(face, table_tag);

    let num_features = g.get_feature_count();
    for i in 0..num_features {
        if feature_tag == g.get_feature_tag(i) {
            *feature_index = i;
            return true;
        }
    }

    *feature_index = HB_OT_LAYOUT_NO_FEATURE_INDEX;
    false
}

/// `hb_ot_layout_script_select_language2`
pub(crate) fn hb_ot_layout_script_select_language2(
    face: &HbFace,
    table_tag: HbTag,
    script_index: u32,
    language_tags: &[HbTag],
    language_index: &mut u32,
    chosen_language: &mut HbTag,
) -> bool {
    let s = get_gsubgpos_table(face, table_tag).get_script(script_index);

    for &t in language_tags {
        if s.find_lang_sys_index(t, language_index) {
            *chosen_language = t;
            return true;
        }
    }

    /* try finding 'dflt' */
    if s.find_lang_sys_index(HB_OT_TAG_DEFAULT_LANGUAGE, language_index) {
        *chosen_language = HB_OT_TAG_DEFAULT_LANGUAGE;
        return false;
    }

    *language_index = HB_OT_LAYOUT_DEFAULT_LANGUAGE_INDEX;
    *chosen_language = HB_TAG_NONE;
    false
}

/// `hb_ot_layout_language_get_required_feature`
pub(crate) fn hb_ot_layout_language_get_required_feature(
    face: &HbFace,
    table_tag: HbTag,
    script_index: u32,
    language_index: u32,
    feature_index: &mut u32,
    feature_tag: &mut HbTag,
) -> bool {
    let g = get_gsubgpos_table(face, table_tag);
    let l = g.get_script(script_index).get_lang_sys(language_index);

    let index = l.get_required_feature_index();
    *feature_index = index;
    *feature_tag = g.get_feature_tag(index);

    l.has_required_feature()
}

/// `hb_ot_layout_language_find_feature`
pub(crate) fn hb_ot_layout_language_find_feature(
    face: &HbFace,
    table_tag: HbTag,
    script_index: u32,
    language_index: u32,
    feature_tag: HbTag,
    feature_index: &mut u32,
) -> bool {
    let g = get_gsubgpos_table(face, table_tag);
    let l = g.get_script(script_index).get_lang_sys(language_index);

    let num_features = l.get_feature_count();
    for i in 0..num_features {
        let f_index = l.get_feature_index(i);

        if feature_tag == g.get_feature_tag(f_index) {
            *feature_index = f_index;
            return true;
        }
    }

    *feature_index = HB_OT_LAYOUT_NO_FEATURE_INDEX;
    false
}

/// `hb_ot_layout_table_get_lookup_count`
pub(crate) fn hb_ot_layout_table_get_lookup_count(face: &HbFace, table_tag: HbTag) -> u32 {
    get_gsubgpos_table(face, table_tag).get_lookup_count()
}

/// `hb_collect_features_context_t`
struct HbCollectFeaturesContext<'g> {
    g: GsubGpos<'g>,
    feature_indices: &'g mut HbSet,
    feature_indices_filter: HbSet,
    has_feature_filter: bool,

    visited_script: HbSet,
    visited_langsys: HbSet,
    script_count: u32,
    langsys_count: u32,
    feature_index_count: u32,
}

impl<'g> HbCollectFeaturesContext<'g> {
    fn new(g: GsubGpos<'g>, feature_indices: &'g mut HbSet, features: Option<&[HbTag]>) -> Self {
        let mut c = HbCollectFeaturesContext {
            g,
            feature_indices,
            feature_indices_filter: HbSet::new(),
            has_feature_filter: false,
            visited_script: HbSet::new(),
            visited_langsys: HbSet::new(),
            script_count: 0,
            langsys_count: 0,
            feature_index_count: 0,
        };
        c.compute_feature_filter(features);
        c
    }

    fn compute_feature_filter(&mut self, features: Option<&[HbTag]>) {
        let Some(features) = features else {
            self.has_feature_filter = false;
            return;
        };

        self.has_feature_filter = true;
        let mut features_set = HbSet::new();
        for &f in features {
            features_set.add(f);
        }

        for i in 0..self.g.get_feature_count() {
            let tag = self.g.get_feature_tag(i);
            if features_set.has(tag) {
                self.feature_indices_filter.add(i);
            }
        }
    }

    /// The offset of an object of the table from its start (C's
    /// `(uintptr_t) &p - (uintptr_t) &g`).
    fn delta(&self, p: &[u8]) -> HbCodepoint {
        (p.as_ptr() as usize).wrapping_sub(self.g.0.as_ptr() as usize) as HbCodepoint
    }

    fn visited_script(&mut self, s: Script) -> bool {
        /* We might have Null() object here.  Don't want to involve
         * that in the memoize.  So, detect empty objects and return. */
        if !s.has_default_lang_sys() && s.get_lang_sys_count() == 0 {
            return true;
        }

        let n = self.script_count;
        self.script_count += 1;
        if n > HB_MAX_SCRIPTS {
            return true;
        }

        let delta = self.delta(s.0);
        Self::visited(delta, &mut self.visited_script)
    }

    fn visited_langsys(&mut self, l: LangSys) -> bool {
        /* We might have Null() object here.  Don't want to involve
         * that in the memoize.  So, detect empty objects and return. */
        if !l.has_required_feature() && l.get_feature_count() == 0 {
            return true;
        }

        let n = self.langsys_count;
        self.langsys_count += 1;
        if n > HB_MAX_LANGSYS {
            return true;
        }

        let delta = self.delta(l.0);
        Self::visited(delta, &mut self.visited_langsys)
    }

    fn visited_feature_indices(&mut self, count: u32) -> bool {
        self.feature_index_count = self.feature_index_count.wrapping_add(count);
        self.feature_index_count > HB_MAX_FEATURE_INDICES
    }

    fn visited(delta: HbCodepoint, visited_set: &mut HbSet) -> bool {
        if visited_set.has(delta) {
            return true;
        }

        visited_set.add(delta);
        false
    }
}

/// `langsys_collect_features`
fn langsys_collect_features(c: &mut HbCollectFeaturesContext, l: LangSys) {
    if c.visited_langsys(l) {
        return;
    }

    if !c.has_feature_filter {
        /* All features. */
        if l.has_required_feature() && !c.visited_feature_indices(1) {
            c.feature_indices.add(l.get_required_feature_index());
        }

        // TODO(garretrieger): filter out indices >= feature count?
        if !c.visited_feature_indices(l.feature_index().len()) {
            l.add_feature_indexes_to(c.feature_indices);
        }
    } else {
        if c.feature_indices_filter.is_empty() {
            return;
        }
        let num_features = l.get_feature_count();
        for i in 0..num_features {
            let feature_index = l.get_feature_index(i);
            if !c.feature_indices_filter.has(feature_index) {
                continue;
            }

            c.feature_indices.add(feature_index);
            c.feature_indices_filter.del(feature_index);
        }
    }
}

/// `script_collect_features`
fn script_collect_features(
    c: &mut HbCollectFeaturesContext,
    s: Script,
    languages: Option<&[HbTag]>,
) {
    if c.visited_script(s) {
        return;
    }

    match languages {
        None => {
            /* All languages. */
            if s.has_default_lang_sys() {
                langsys_collect_features(c, s.get_default_lang_sys());
            }

            let count = s.get_lang_sys_count();
            for language_index in 0..count {
                langsys_collect_features(c, s.get_lang_sys(language_index));
            }
        }
        Some(languages) => {
            for &l in languages {
                let mut language_index = 0;
                if s.find_lang_sys_index(l, &mut language_index) {
                    langsys_collect_features(c, s.get_lang_sys(language_index));
                }
            }
        }
    }
}

/// `hb_ot_layout_collect_features`: the indices of the features of
/// `table_tag` for the given scripts, languages and features (`None`:
/// all; the C arrays' terminating zeros are left out).
pub(crate) fn hb_ot_layout_collect_features(
    face: &HbFace,
    table_tag: HbTag,
    scripts: Option<&[HbTag]>,
    languages: Option<&[HbTag]>,
    features: Option<&[HbTag]>,
    feature_indexes: &mut HbSet,
) {
    let g = get_gsubgpos_table(face, table_tag);
    let mut c = HbCollectFeaturesContext::new(g, feature_indexes, features);
    match scripts {
        None => {
            /* All scripts. */
            let count = c.g.get_script_count();
            for script_index in 0..count {
                let s = c.g.get_script(script_index);
                script_collect_features(&mut c, s, languages);
            }
        }
        Some(scripts) => {
            for &t in scripts {
                let mut script_index = 0;
                if c.g.find_script_index(t, &mut script_index) {
                    let s = c.g.get_script(script_index);
                    script_collect_features(&mut c, s, languages);
                }
            }
        }
    }
}

/// `hb_ot_layout_collect_features_map`
pub(crate) fn hb_ot_layout_collect_features_map(
    face: &HbFace,
    table_tag: HbTag,
    script_index: u32,
    language_index: u32,
    feature_map: &mut HashMap<HbTag, u32>,
) {
    let g = get_gsubgpos_table(face, table_tag);
    let l = g.get_script(script_index).get_lang_sys(language_index);

    let count = l.get_feature_count();

    /* Loop in reverse, such that earlier entries win. That emulates
     * a linear search, which seems to be what other implementations do.
     * We found that with arialuni_t.ttf, the "ur" language system has
     * duplicate features, and the earlier ones work but not later ones.
     */
    for i in (1..=count).rev() {
        let mut feature_index = Vec::new();
        let feature_count = l.get_feature_indexes(i - 1, 1, &mut feature_index);
        if feature_count == 0 || feature_index.is_empty() {
            break;
        }
        let feature_tag = g.get_feature_tag(feature_index[0]);
        feature_map.insert(feature_tag, feature_index[0]);
    }
}

/// `hb_ot_layout_collect_lookups`
pub(crate) fn hb_ot_layout_collect_lookups(
    face: &HbFace,
    table_tag: HbTag,
    scripts: Option<&[HbTag]>,
    languages: Option<&[HbTag]>,
    features: Option<&[HbTag]>,
    lookup_indexes: &mut HbSet,
) {
    let g = get_gsubgpos_table(face, table_tag);

    let mut feature_indexes = HbSet::new();
    hb_ot_layout_collect_features(
        face,
        table_tag,
        scripts,
        languages,
        features,
        &mut feature_indexes,
    );

    for feature_index in feature_indexes.iter() {
        g.get_feature(feature_index)
            .add_lookup_indexes_to(lookup_indexes);
    }

    g.feature_variation_collect_lookups(&feature_indexes, lookup_indexes);
}

/// `hb_ot_layout_lookup_collect_glyphs`
pub(crate) fn hb_ot_layout_lookup_collect_glyphs(
    face: &HbFace,
    table_tag: HbTag,
    lookup_index: u32,
    glyphs_before: Option<&mut HbSet>,
    glyphs_input: Option<&mut HbSet>,
    glyphs_after: Option<&mut HbSet>,
    glyphs_output: Option<&mut HbSet>,
) {
    let mut c = HbCollectGlyphsContext {
        face,
        before: glyphs_before.as_ref().map(|_| HbSet::new()),
        input: glyphs_input.as_ref().map(|_| HbSet::new()),
        after: glyphs_after.as_ref().map(|_| HbSet::new()),
        output: glyphs_output.as_ref().map(|_| HbSet::new()),
        recurse_gsub: false,
        recursed_lookups: HbSet::new(),
        nesting_level_left: HB_MAX_NESTING_LEVEL,
    };

    match table_tag {
        HB_OT_TAG_GSUB => {
            let l = face.gsub().table().get_lookup(lookup_index);
            hb_ot_layout_gsub::subst_lookup_collect_glyphs(&mut c, l);
        }
        HB_OT_TAG_GPOS => {
            let l = face.gpos().table().get_lookup(lookup_index);
            hb_ot_layout_gpos::pos_lookup_collect_glyphs(&mut c, l);
        }
        _ => {}
    }

    /* (the sets are filled in place in C) */
    for (dst, src) in [
        (glyphs_before, c.before),
        (glyphs_input, c.input),
        (glyphs_after, c.after),
        (glyphs_output, c.output),
    ] {
        if let (Some(dst), Some(src)) = (dst, src) {
            dst.union(&src);
        }
    }
}

/* Variations support */

/// `hb_ot_layout_table_find_feature_variations`
pub(crate) fn hb_ot_layout_table_find_feature_variations(
    face: &HbFace,
    table_tag: HbTag,
    coords: &[i32],
    variations_index: &mut u32,
) -> bool {
    let g = get_gsubgpos_table(face, table_tag);
    g.find_variations_index(coords, variations_index)
}

/// `hb_ot_layout_feature_with_variations_get_lookups`
pub(crate) fn hb_ot_layout_feature_with_variations_get_lookups(
    face: &HbFace,
    table_tag: HbTag,
    feature_index: u32,
    variations_index: u32,
    start_offset: u32,
    max_count: u32,
    lookup_indexes: &mut Vec<u32>,
) -> u32 {
    let g = get_gsubgpos_table(face, table_tag);

    let f = g.get_feature_variation(feature_index, variations_index);

    f.get_lookup_indexes(start_offset, max_count, lookup_indexes)
}

/*
 * OT::GSUB
 */

/// `hb_ot_layout_has_substitution`
pub(crate) fn hb_ot_layout_has_substitution(face: &HbFace) -> bool {
    face.gsub().table().has_data()
}

/// `hb_ot_layout_lookup_would_substitute`
pub(crate) fn hb_ot_layout_lookup_would_substitute(
    face: &HbFace,
    lookup_index: u32,
    glyphs: &[HbCodepoint],
    zero_context: bool,
) -> bool {
    let gsub = face.gsub();
    if lookup_index >= gsub.lookup_count {
        return false;
    }
    let c = HbWouldApplyContext {
        glyphs,
        len: glyphs.len() as u32,
        zero_context,
    };

    let l = gsub.table().get_lookup(lookup_index);
    hb_ot_layout_gsub::subst_lookup_would_apply(&c, l)
}

/// `hb_ot_layout_substitute_start`
pub(crate) fn hb_ot_layout_substitute_start(face: &HbFace, buffer: &mut HbBuffer) {
    _hb_ot_layout_set_glyph_props(face, buffer);
}

/*
 * GPOS
 */

/// `hb_ot_layout_has_positioning`
pub(crate) fn hb_ot_layout_has_positioning(face: &HbFace) -> bool {
    face.gpos().table().has_data()
}

/// `hb_ot_layout_position_start`
pub(crate) fn hb_ot_layout_position_start(buffer: &mut HbBuffer) {
    hb_ot_layout_gpos::gpos_position_start(buffer);
}

/// `hb_ot_layout_position_finish_advances`
pub(crate) fn hb_ot_layout_position_finish_advances(buffer: &mut HbBuffer) {
    hb_ot_layout_gpos::gpos_position_finish_advances(buffer);
}

/// `hb_ot_layout_position_finish_offsets`
pub(crate) fn hb_ot_layout_position_finish_offsets(font: &HbFont, buffer: &mut HbBuffer) {
    hb_ot_layout_gpos::gpos_position_finish_offsets(font.p.slant, font.p.slant_xy, buffer);
}

/*
 * Parts of different types are implemented here such that they have direct
 * access to GSUB/GPOS lookups.
 */

/// `apply_forward`
#[inline]
fn apply_forward<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, lookup: Lookup<'a>) -> bool {
    let mut ret = false;
    while c.buffer.idx < c.buffer.len && c.buffer.successful {
        let mut applied = false;
        let cur = *c.buffer.cur(0);
        if (cur.mask & c.lookup_mask) != 0 && c.check_glyph_property(&cur, c.lookup_props) {
            applied = c.apply_lookup_subtables(lookup);
        }

        if applied {
            ret = true;
        } else {
            let _ = c.buffer.next_glyph();
        }
    }
    ret
}

/// `apply_backward`
#[inline]
fn apply_backward<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, lookup: Lookup<'a>) -> bool {
    let mut ret = false;
    loop {
        let cur = *c.buffer.cur(0);
        if (cur.mask & c.lookup_mask) != 0 && c.check_glyph_property(&cur, c.lookup_props) {
            ret |= c.apply_lookup_subtables(lookup);
        }

        /* The reverse lookup doesn't "advance" cursor (for good reason). */
        c.buffer.idx = c.buffer.idx.wrapping_sub(1);
        if (c.buffer.idx as i32) < 0 {
            break;
        }
    }
    ret
}

/// `apply_string<Proxy>` (GSUB: `table_index` 0, not in place; GPOS: 1,
/// always in place)
pub(crate) fn apply_string<'a>(c: &mut HbOtApplyContext<'a, '_, '_>, lookup: Lookup<'a>) -> bool {
    let always_inplace = c.table_index == 1;

    if c.buffer.len == 0 || c.lookup_mask == 0 {
        return false;
    }

    let mut ret = false;

    c.set_lookup_props(lookup.get_props());

    let is_reverse = !always_inplace && hb_ot_layout_gsub::subst_lookup_is_reverse(lookup);
    if !is_reverse {
        /* in/out forward substitution/positioning */
        if !always_inplace {
            c.buffer.clear_output();
        }

        c.buffer.idx = 0;
        ret = apply_forward(c, lookup);

        if !always_inplace {
            let _ = c.buffer.sync();
        }
    } else {
        /* in-place backward substitution/positioning */
        c.buffer.idx = c.buffer.len - 1;
        ret = apply_backward(c, lookup);
    }

    ret
}

/// `hb_ot_layout_substitute_lookup`
pub(crate) fn hb_ot_layout_substitute_lookup<'a>(
    c: &mut HbOtApplyContext<'a, '_, '_>,
    lookup: Lookup<'a>,
) {
    apply_string(c, lookup);
}
