// Rust translation of src/hb-ot-shaper-arabic.hh, src/hb-ot-shaper-arabic.cc
// and src/hb-ot-shaper-arabic-fallback.hh from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it).
// Copyright © 2010,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The Arabic shaper (Arabic, Syriac, Mongolian, N'Ko, ...: the joining
//! scripts), with the Arabic fallback shaping for fonts without GSUB
//! forms.
//!
//! Translation notes: the fallback shaping's synthesized lookups are
//! built as GSUB lookup bytes laid out as HarfBuzz's serializer lays them
//! out, choosing the same subtable and coverage formats, and applied by
//! the GSUB translation. The Windows-1256 fallback (for Windows builds
//! only in C) is not translated.

use std::sync::OnceLock;

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::*;
use super::hb_ot_layout_common::{Lookup, LOOKUP_FLAG_IGNORE_MARKS};
use super::hb_ot_layout_gsubgpos::{GsubGposAccel, GsubGposKind, HbOtApplyContext};
use super::hb_ot_map::*;
use super::hb_ot_shape::{HbOtShapePlan, HbOtShapePlanner};
use super::hb_ot_shape_normalize::HB_OT_SHAPE_NORMALIZATION_MODE_DEFAULT;
use super::hb_ot_shaper::*;
use super::hb_ot_shaper_arabic_table::*;
use super::hb_unicode::*;

/* buffer var allocations */
/// `arabic_shaping_action()`: `ot_shaper_var_u8_auxiliary()`
#[inline]
fn arabic_shaping_action(info: &HbGlyphInfo) -> u8 {
    info.complex_var_u8_auxiliary()
}
#[inline]
fn set_arabic_shaping_action(info: &mut HbGlyphInfo, v: u8) {
    info.set_complex_var_u8_auxiliary(v);
}

const HB_BUFFER_SCRATCH_FLAG_ARABIC_HAS_STCH: u32 = HB_BUFFER_SCRATCH_FLAG_SHAPER0;

/* See:
 * https://github.com/harfbuzz/harfbuzz/commit/6e6f82b6f3dde0fc6c3c7d991d9ec6cfff57823d#commitcomment-14248516 */
/// `HB_ARABIC_GENERAL_CATEGORY_IS_WORD`
fn hb_arabic_general_category_is_word(gen_cat: u32) -> bool {
    flag_unsafe(gen_cat)
        & (flag(HB_UNICODE_GENERAL_CATEGORY_UNASSIGNED)
            | flag(HB_UNICODE_GENERAL_CATEGORY_PRIVATE_USE)
            /*| flag (HB_UNICODE_GENERAL_CATEGORY_LOWERCASE_LETTER)*/
            | flag(HB_UNICODE_GENERAL_CATEGORY_MODIFIER_LETTER)
            | flag(HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER)
            /*| flag (HB_UNICODE_GENERAL_CATEGORY_TITLECASE_LETTER)*/
            /*| flag (HB_UNICODE_GENERAL_CATEGORY_UPPERCASE_LETTER)*/
            | flag(HB_UNICODE_GENERAL_CATEGORY_SPACING_MARK)
            | flag(HB_UNICODE_GENERAL_CATEGORY_ENCLOSING_MARK)
            | flag(HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK)
            | flag(HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER)
            | flag(HB_UNICODE_GENERAL_CATEGORY_LETTER_NUMBER)
            | flag(HB_UNICODE_GENERAL_CATEGORY_OTHER_NUMBER)
            | flag(HB_UNICODE_GENERAL_CATEGORY_CURRENCY_SYMBOL)
            | flag(HB_UNICODE_GENERAL_CATEGORY_MODIFIER_SYMBOL)
            | flag(HB_UNICODE_GENERAL_CATEGORY_MATH_SYMBOL)
            | flag(HB_UNICODE_GENERAL_CATEGORY_OTHER_SYMBOL))
        != 0
}

/*
 * Joining types:
 */

/*
 * Bits used in the joining tables
 */
/// `hb_arabic_joining_type_t`
pub(crate) const JOINING_TYPE_U: u8 = 0;
pub(crate) const JOINING_TYPE_L: u8 = 1;
pub(crate) const JOINING_TYPE_R: u8 = 2;
pub(crate) const JOINING_TYPE_D: u8 = 3;
pub(crate) const JOINING_TYPE_C: u8 = JOINING_TYPE_D;
pub(crate) const JOINING_GROUP_ALAPH: u8 = 4;
pub(crate) const JOINING_GROUP_DALATH_RISH: u8 = 5;
pub(crate) const NUM_STATE_MACHINE_COLS: usize = 6;

pub(crate) const JOINING_TYPE_T: u8 = 7;
pub(crate) const JOINING_TYPE_X: u8 = 8; /* means: use general-category to choose between U or T. */

/// `hb_ot_shaper_arabic_table.rs`'s ligature sets.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LigatureSet<const NCOMP: usize, const NLIG: usize> {
    pub(crate) first: u16,
    pub(crate) ligatures: [LigaturePair<NCOMP>; NLIG],
}

/// A ligature of a ligature set.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LigaturePair<const NCOMP: usize> {
    pub(crate) components: [u16; NCOMP],
    pub(crate) ligature: u16,
}

/// `get_joining_type`
fn get_joining_type(u: HbCodepoint, gen_cat: u32) -> u8 {
    let j_type = joining_type(u);
    if j_type != JOINING_TYPE_X {
        return j_type;
    }

    if flag_unsafe(gen_cat)
        & (flag(HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK)
            | flag(HB_UNICODE_GENERAL_CATEGORY_ENCLOSING_MARK)
            | flag(HB_UNICODE_GENERAL_CATEGORY_FORMAT))
        != 0
    {
        JOINING_TYPE_T
    } else {
        JOINING_TYPE_U
    }
}

/// `FEATURE_IS_SYRIAC`
fn feature_is_syriac(tag: HbTag) -> bool {
    (b'2'..=b'3').contains(&(tag as u8))
}

const ARABIC_FEATURES: [HbTag; 8] = [
    hb_tag(b'i', b's', b'o', b'l'),
    hb_tag(b'f', b'i', b'n', b'a'),
    hb_tag(b'f', b'i', b'n', b'2'),
    hb_tag(b'f', b'i', b'n', b'3'),
    hb_tag(b'm', b'e', b'd', b'i'),
    hb_tag(b'm', b'e', b'd', b'2'),
    hb_tag(b'i', b'n', b'i', b't'),
    HB_TAG_NONE,
];

/* Same order as the feature array */
/// `arabic_action_t`
const ISOL: u8 = 0;
const FINA: u8 = 1;
const FIN2: u8 = 2;
const FIN3: u8 = 3;
const MEDI: u8 = 4;
const MED2: u8 = 5;
const INIT: u8 = 6;
const NONE: u8 = 7;
const ARABIC_NUM_FEATURES: usize = NONE as usize;
/* We abuse the same byte for other things... */
const STCH_FIXED: u8 = 8;
const STCH_REPEATING: u8 = 9;

/// `arabic_state_table_entry`
#[derive(Clone, Copy)]
struct ArabicStateTableEntry {
    prev_action: u8,
    curr_action: u8,
    next_state: u16,
}

const fn e(prev_action: u8, curr_action: u8, next_state: u16) -> ArabicStateTableEntry {
    ArabicStateTableEntry {
        prev_action,
        curr_action,
        next_state,
    }
}

#[rustfmt::skip]
static ARABIC_STATE_TABLE: [[ArabicStateTableEntry; NUM_STATE_MACHINE_COLS]; 7] = [
    /*   jt_U,          jt_L,          jt_R,          jt_D,          jg_ALAPH,      jg_DALATH_RISH */

    /* State 0: prev was U, not willing to join. */
    [ e(NONE,NONE,0), e(NONE,ISOL,2), e(NONE,ISOL,1), e(NONE,ISOL,2), e(NONE,ISOL,1), e(NONE,ISOL,6), ],

    /* State 1: prev was R or ISOL/ALAPH, not willing to join. */
    [ e(NONE,NONE,0), e(NONE,ISOL,2), e(NONE,ISOL,1), e(NONE,ISOL,2), e(NONE,FIN2,5), e(NONE,ISOL,6), ],

    /* State 2: prev was D/L in ISOL form, willing to join. */
    [ e(NONE,NONE,0), e(NONE,ISOL,2), e(INIT,FINA,1), e(INIT,FINA,3), e(INIT,FINA,4), e(INIT,FINA,6), ],

    /* State 3: prev was D in FINA form, willing to join. */
    [ e(NONE,NONE,0), e(NONE,ISOL,2), e(MEDI,FINA,1), e(MEDI,FINA,3), e(MEDI,FINA,4), e(MEDI,FINA,6), ],

    /* State 4: prev was FINA ALAPH, not willing to join. */
    [ e(NONE,NONE,0), e(NONE,ISOL,2), e(MED2,ISOL,1), e(MED2,ISOL,2), e(MED2,FIN2,5), e(MED2,ISOL,6), ],

    /* State 5: prev was FIN2/FIN3 ALAPH, not willing to join. */
    [ e(NONE,NONE,0), e(NONE,ISOL,2), e(ISOL,ISOL,1), e(ISOL,ISOL,2), e(ISOL,FIN2,5), e(ISOL,ISOL,6), ],

    /* State 6: prev was DALATH/RISH, not willing to join. */
    [ e(NONE,NONE,0), e(NONE,ISOL,2), e(NONE,ISOL,1), e(NONE,ISOL,2), e(NONE,FIN3,5), e(NONE,ISOL,6), ],
];

/// `deallocate_buffer_var`
fn deallocate_buffer_var(_plan: &HbOtShapePlan, _font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    buffer.deallocate_var(VAR_COMPLEX_AUXILIARY.0, VAR_COMPLEX_AUXILIARY.1);
    false
}

/// `collect_features_arabic`
pub(crate) fn collect_features_arabic(plan: &mut HbOtShapePlanner) {
    let script = plan.props.script;
    let map = &mut plan.map;

    /* We apply features according to the Arabic spec, with pauses
     * in between most.
     *
     * The pause between init/medi/... and rlig is required.  See eg:
     * https://bugzilla.mozilla.org/show_bug.cgi?id=644184
     *
     * The pauses between init/medi/... themselves are not necessarily
     * needed as only one of those features is applied to any character.
     * The only difference it makes is when fonts have contextual
     * substitutions.  We now follow the order of the spec, which makes
     * for better experience if that's what Uniscribe is doing.
     *
     * At least for Arabic, looks like Uniscribe has a pause between
     * rlig and calt.  Otherwise the IranNastaliq's ALLAH ligature won't
     * work.  However, testing shows that rlig and calt are applied
     * together for Mongolian in Uniscribe.  As such, we only add a
     * pause for Arabic, not other scripts.
     */

    map.enable_feature(hb_tag(b's', b't', b'c', b'h'), F_NONE, 1);
    map.add_gsub_pause(Some(record_stch));

    map.enable_feature(hb_tag(b'c', b'c', b'm', b'p'), F_MANUAL_ZWJ, 1);
    map.enable_feature(hb_tag(b'l', b'o', b'c', b'l'), F_MANUAL_ZWJ, 1);

    map.add_gsub_pause(None);

    for &tag in &ARABIC_FEATURES[..ARABIC_NUM_FEATURES] {
        let has_fallback = script == HB_SCRIPT_ARABIC && !feature_is_syriac(tag);
        map.add_feature(
            tag,
            F_MANUAL_ZWJ | if has_fallback { F_HAS_FALLBACK } else { F_NONE },
            1,
        );
        map.add_gsub_pause(None);
    }
    map.add_gsub_pause(Some(deallocate_buffer_var));

    /* Normally, Unicode says a ZWNJ means "don't ligate".  In Arabic script
     * however, it says a ZWJ should also mean "don't ligate".  So we run
     * the main ligating features as MANUAL_ZWJ. */

    map.enable_feature(
        hb_tag(b'r', b'l', b'i', b'g'),
        F_MANUAL_ZWJ | F_HAS_FALLBACK,
        1,
    );

    if script == HB_SCRIPT_ARABIC {
        map.add_gsub_pause(Some(arabic_fallback_shape));
    }

    /* No pause after rclt.  See 98460779bae19e4d64d29461ff154b3527bf8420. */
    map.enable_feature(hb_tag(b'c', b'a', b'l', b't'), F_MANUAL_ZWJ, 1);
    /* https://github.com/harfbuzz/harfbuzz/issues/1573 */
    if !map.has_feature(hb_tag(b'r', b'c', b'l', b't')) {
        map.add_gsub_pause(None);
        map.enable_feature(hb_tag(b'r', b'c', b'l', b't'), F_MANUAL_ZWJ, 1);
    }

    map.enable_feature(hb_tag(b'l', b'i', b'g', b'a'), F_MANUAL_ZWJ, 1);
    map.enable_feature(hb_tag(b'c', b'l', b'i', b'g'), F_MANUAL_ZWJ, 1);

    /* The spec includes 'cswh'.  Earlier versions of Windows
     * used to enable this by default, but testing suggests
     * that Windows 8 and later do not enable it by default,
     * and spec now says 'Off by default'.
     * We disabled this in ae23c24c32.
     * Note that IranNastaliq uses this feature extensively
     * to fixup broken glyph sequences.  Oh well...
     * Test case: U+0643,U+0640,U+0631. */
    //map->enable_feature (HB_TAG('c','s','w','h'), F_MANUAL_ZWJ);
    map.enable_feature(hb_tag(b'm', b's', b'e', b't'), F_MANUAL_ZWJ, 1);
}

/// `arabic_shape_plan_t`
#[derive(Debug)]
pub(crate) struct ArabicShapePlan {
    /* The "+ 1" in the next array is to accommodate for the "NONE" command,
     * which is not an OpenType feature, but this simplifies the code by not
     * having to do a "if (... < NONE) ..." and just rely on the fact that
     * mask_array[NONE] == 0. */
    pub(crate) mask_array: [HbMask; ARABIC_NUM_FEATURES + 1],

    fallback_plan: OnceLock<ArabicFallbackPlan>,

    pub(crate) do_fallback: bool,
    pub(crate) has_stch: bool,
}

/// `data_create_arabic`
pub(crate) fn data_create_arabic_plan(plan: &HbOtShapePlan) -> ArabicShapePlan {
    let mut arabic_plan = ArabicShapePlan {
        mask_array: [0; ARABIC_NUM_FEATURES + 1],
        fallback_plan: OnceLock::new(),
        do_fallback: plan.props.script == HB_SCRIPT_ARABIC,
        has_stch: plan.map.get_1_mask(hb_tag(b's', b't', b'c', b'h')) != 0,
    };

    for i in 0..ARABIC_NUM_FEATURES {
        arabic_plan.mask_array[i] = plan.map.get_1_mask(ARABIC_FEATURES[i]);
        arabic_plan.do_fallback = arabic_plan.do_fallback
            && (feature_is_syriac(ARABIC_FEATURES[i])
                || plan.map.needs_fallback(ARABIC_FEATURES[i]));
    }

    arabic_plan
}

fn data_create_arabic(plan: &HbOtShapePlan) -> Option<ShaperData> {
    Some(ShaperData::Arabic(Box::new(data_create_arabic_plan(plan))))
}

/// The plan's Arabic data.
fn arabic_plan(plan: &HbOtShapePlan) -> Option<&ArabicShapePlan> {
    match &plan.data {
        Some(ShaperData::Arabic(p)) => Some(p),
        _ => None,
    }
}

/// `arabic_joining`
fn arabic_joining(buffer: &mut HbBuffer) {
    let count = buffer.len as usize;
    let mut prev = usize::MAX;
    let mut state = 0usize;

    /* Check pre-context */
    for i in 0..buffer.context_len[0] as usize {
        let c = buffer.context[0][i];
        let this_type = get_joining_type(c, buffer.unicode.general_category(c));

        if this_type == JOINING_TYPE_T {
            continue;
        }

        let entry = &ARABIC_STATE_TABLE[state][this_type as usize];
        state = entry.next_state as usize;
        break;
    }

    for i in 0..count {
        let this_type = get_joining_type(
            buffer.info[i].codepoint,
            _hb_glyph_info_get_general_category(&buffer.info[i]),
        );

        if this_type == JOINING_TYPE_T {
            set_arabic_shaping_action(&mut buffer.info[i], NONE);
            continue;
        }

        let entry = ARABIC_STATE_TABLE[state][this_type as usize];

        if entry.prev_action != NONE && prev != usize::MAX {
            set_arabic_shaping_action(&mut buffer.info[prev], entry.prev_action);
            buffer.safe_to_insert_tatweel(prev as u32, i as u32 + 1);
        } else if prev == usize::MAX {
            if this_type >= JOINING_TYPE_R {
                buffer.unsafe_to_concat_from_outbuffer(0, i as u32 + 1);
            }
        } else if this_type >= JOINING_TYPE_R || (2..=5).contains(&state)
        /* States that have a possible prev_action. */
        {
            buffer.unsafe_to_concat(prev as u32, i as u32 + 1);
        }

        set_arabic_shaping_action(&mut buffer.info[i], entry.curr_action);

        prev = i;
        state = entry.next_state as usize;
    }

    for i in 0..buffer.context_len[1] as usize {
        let c = buffer.context[1][i];
        let this_type = get_joining_type(c, buffer.unicode.general_category(c));

        if this_type == JOINING_TYPE_T {
            continue;
        }

        let entry = ARABIC_STATE_TABLE[state][this_type as usize];
        if entry.prev_action != NONE && prev != usize::MAX {
            set_arabic_shaping_action(&mut buffer.info[prev], entry.prev_action);
            let len = buffer.len;
            buffer.safe_to_insert_tatweel(prev as u32, len);
        } else if (2..=5).contains(&state)
        /* States that have a possible prev_action. */
        {
            let len = buffer.len;
            buffer.unsafe_to_concat(prev as u32, len);
        }
        break;
    }
}

/// `mongolian_variation_selectors`
fn mongolian_variation_selectors(buffer: &mut HbBuffer) {
    /* Copy arabic_shaping_action() from base to Mongolian variation selectors. */
    let count = buffer.len as usize;
    for i in 1..count {
        let u = buffer.info[i].codepoint;
        if (0x180B..=0x180D).contains(&u) || u == 0x180F {
            let a = arabic_shaping_action(&buffer.info[i - 1]);
            set_arabic_shaping_action(&mut buffer.info[i], a);
        }
    }
}

/// `setup_masks_arabic_plan`
pub(crate) fn setup_masks_arabic_plan(
    arabic_plan: &ArabicShapePlan,
    buffer: &mut HbBuffer,
    script: HbScript,
) {
    buffer.allocate_var(VAR_COMPLEX_AUXILIARY.0, VAR_COMPLEX_AUXILIARY.1);

    arabic_joining(buffer);
    if script == HB_SCRIPT_MONGOLIAN {
        mongolian_variation_selectors(buffer);
    }

    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        info.mask |= arabic_plan.mask_array[arabic_shaping_action(info) as usize];
    }
}

/// `setup_masks_arabic`
fn setup_masks_arabic(plan: &HbOtShapePlan, buffer: &mut HbBuffer, _font: &mut HbFont) {
    if let Some(arabic_plan) = arabic_plan(plan) {
        setup_masks_arabic_plan(arabic_plan, buffer, plan.props.script);
    }
}

/// `arabic_fallback_shape`
fn arabic_fallback_shape(plan: &HbOtShapePlan, font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    let Some(arabic_plan) = arabic_plan(plan) else {
        return false;
    };
    if !arabic_plan.do_fallback {
        return false;
    }

    /* This sucks.  We need a font to build the fallback plan... */
    let fallback_plan = arabic_plan
        .fallback_plan
        .get_or_init(|| arabic_fallback_plan_create(plan, font));

    arabic_fallback_plan_shape(fallback_plan, font, buffer);
    true
}

/*
 * Stretch feature: "stch".
 * See example here:
 * https://docs.microsoft.com/en-us/typography/script-development/syriac
 * We implement this in a generic way, such that the Arabic subtending
 * marks can use it as well.
 */

/// `record_stch`
fn record_stch(plan: &HbOtShapePlan, _font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    let Some(arabic_plan) = arabic_plan(plan) else {
        return false;
    };
    if !arabic_plan.has_stch {
        return false;
    }

    /* 'stch' feature was just applied.  Look for anything that multiplied,
     * and record it for stch treatment later.  Note that rtlm, frac, etc
     * are applied before stch, but we assume that they didn't result in
     * anything multiplying into 5 pieces, so it's safe-ish... */

    let count = buffer.len as usize;
    for i in 0..count {
        if _hb_glyph_info_multiplied(&buffer.info[i]) {
            let comp = _hb_glyph_info_get_lig_comp(&buffer.info[i]);
            set_arabic_shaping_action(
                &mut buffer.info[i],
                if comp % 2 != 0 {
                    STCH_REPEATING
                } else {
                    STCH_FIXED
                },
            );
            buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_ARABIC_HAS_STCH;
        }
    }

    false
}

#[inline]
fn is_stch(info: &HbGlyphInfo) -> bool {
    (STCH_FIXED..=STCH_REPEATING).contains(&arabic_shaping_action(info))
}

/// `apply_stch`
fn apply_stch(_plan: &HbOtShapePlan, buffer: &mut HbBuffer, font: &mut HbFont) {
    if buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_ARABIC_HAS_STCH == 0 {
        return;
    }

    let rtl = buffer.props.direction == HB_DIRECTION_RTL;

    if !rtl {
        buffer.reverse();
    }

    /* We do a two pass implementation:
     * First pass calculates the exact number of extra glyphs we need,
     * We then enlarge buffer to have that much room,
     * Second pass applies the stretch, copying things to the end of buffer.
     */

    let sign: i32 = if font.p.x_scale < 0 { -1 } else { 1 };
    let mut extra_glyphs_needed: u32 = 0; // Set during MEASURE, used during CUT
    const MEASURE: u32 = 0;
    const CUT: u32 = 1;

    for step in MEASURE..=CUT {
        let count = buffer.len;
        let new_len = count + extra_glyphs_needed; // write head during CUT
        let mut j = new_len as usize;
        let mut i = count as usize;
        while i != 0 {
            if !is_stch(&buffer.info[i - 1]) {
                if step == CUT {
                    j -= 1;
                    buffer.info[j] = buffer.info[i - 1];
                    buffer.pos[j] = buffer.pos[i - 1];
                }
                i -= 1;
                continue;
            }

            /* Yay, justification! */

            let mut w_total: HbPosition = 0; // Total to be filled
            let mut w_fixed: HbPosition = 0; // Sum of fixed tiles
            let mut w_repeating: HbPosition = 0; // Sum of repeating tiles
            let mut n_fixed: i32 = 0;
            let mut n_repeating: i32 = 0;

            let end = i;
            while i != 0 && is_stch(&buffer.info[i - 1]) {
                i -= 1;
                let width = font.get_glyph_h_advance(buffer.info[i].codepoint);
                if arabic_shaping_action(&buffer.info[i]) == STCH_FIXED {
                    w_fixed = w_fixed.wrapping_add(width);
                    n_fixed += 1;
                } else {
                    w_repeating = w_repeating.wrapping_add(width);
                    n_repeating += 1;
                }
            }
            let start = i;
            let mut context = i;
            while context != 0
                && !is_stch(&buffer.info[context - 1])
                && (_hb_glyph_info_is_default_ignorable(&buffer.info[context - 1])
                    || hb_arabic_general_category_is_word(_hb_glyph_info_get_general_category(
                        &buffer.info[context - 1],
                    )))
            {
                context -= 1;
                w_total = w_total.wrapping_add(buffer.pos[context].x_advance);
            }
            i += 1; // Don't touch i again.
            let _ = n_fixed;

            /* Number of additional times to repeat each repeating tile. */
            let mut n_copies: i32 = 0;

            let mut w_remaining = w_total.wrapping_sub(w_fixed);
            if sign * w_remaining > sign * w_repeating && sign * w_repeating > 0 {
                n_copies = (sign * w_remaining) / (sign * w_repeating) - 1;
            }

            /* See if we can improve the fit by adding an extra repeat and squeezing them together a bit. */
            let mut extra_repeat_overlap: HbPosition = 0;
            let shortfall = sign * w_remaining - sign * w_repeating * (n_copies + 1);
            if shortfall > 0 && n_repeating > 0 {
                n_copies += 1;
                let excess = (n_copies + 1) * sign * w_repeating - sign * w_remaining;
                if excess > 0 {
                    extra_repeat_overlap = excess / (n_copies * n_repeating);
                    w_remaining = 0;
                }
            }

            if step == MEASURE {
                extra_glyphs_needed =
                    extra_glyphs_needed.wrapping_add((n_copies * n_repeating) as u32);
            } else {
                buffer.unsafe_to_break(context as u32, end as u32);
                let mut x_offset: HbPosition = w_remaining / 2;
                let mut k = end;
                while k > start {
                    let width = font.get_glyph_h_advance(buffer.info[k - 1].codepoint);

                    let mut repeat: u32 = 1;
                    if arabic_shaping_action(&buffer.info[k - 1]) == STCH_REPEATING {
                        repeat = repeat.wrapping_add(n_copies as u32);
                    }

                    buffer.pos[k - 1].x_advance = 0;
                    for n in 0..repeat {
                        if rtl {
                            x_offset -= width;
                            if n > 0 {
                                x_offset += extra_repeat_overlap;
                            }
                        }
                        buffer.pos[k - 1].x_offset = x_offset;
                        /* Append copy. */
                        j -= 1;
                        buffer.info[j] = buffer.info[k - 1];
                        buffer.pos[j] = buffer.pos[k - 1];

                        if !rtl {
                            x_offset += width;
                            if n > 0 {
                                x_offset -= extra_repeat_overlap;
                            }
                        }
                    }
                    k -= 1;
                }
            }
            /* (the loop's i-- of C, after the i++ above) */
            i -= 1;
        }

        if step == MEASURE {
            if !buffer.ensure(count + extra_glyphs_needed) {
                break;
            }
        } else {
            debug_assert!(j == 0);
            buffer.len = new_len;
        }
    }

    if !rtl {
        buffer.reverse();
    }
}

/// `postprocess_glyphs_arabic`
fn postprocess_glyphs_arabic(plan: &HbOtShapePlan, buffer: &mut HbBuffer, font: &mut HbFont) {
    apply_stch(plan, buffer, font);
}

/* https://www.unicode.org/reports/tr53/ */

const MODIFIER_COMBINING_MARKS: [HbCodepoint; 14] = [
    0x0654, /* ARABIC HAMZA ABOVE */
    0x0655, /* ARABIC HAMZA BELOW */
    0x0658, /* ARABIC MARK NOON GHUNNA */
    0x06DC, /* ARABIC SMALL HIGH SEEN */
    0x06E3, /* ARABIC SMALL LOW SEEN */
    0x06E7, /* ARABIC SMALL HIGH YEH */
    0x06E8, /* ARABIC SMALL HIGH NOON */
    0x08CA, /* ARABIC SMALL HIGH FARSI YEH */
    0x08CB, /* ARABIC SMALL HIGH YEH BARREE WITH TWO DOTS BELOW */
    0x08CD, /* ARABIC SMALL HIGH ZAH */
    0x08CE, /* ARABIC LARGE ROUND DOT ABOVE */
    0x08CF, /* ARABIC LARGE ROUND DOT BELOW */
    0x08D3, /* ARABIC SMALL LOW WAW */
    0x08F3, /* ARABIC SMALL HIGH WAW */
];

/// `info_is_mcm`
#[inline]
fn info_is_mcm(info: &HbGlyphInfo) -> bool {
    MODIFIER_COMBINING_MARKS.contains(&info.codepoint)
}

/// `reorder_marks_arabic`
fn reorder_marks_arabic(_plan: &HbOtShapePlan, buffer: &mut HbBuffer, mut start: u32, end: u32) {
    let mut i = start;
    let mut cc = 220;
    while cc <= 230 {
        while i < end && info_cc(&buffer.info[i as usize]) < cc {
            i += 1;
        }

        if i == end {
            break;
        }

        if info_cc(&buffer.info[i as usize]) > cc {
            cc += 10;
            continue;
        }

        let mut j = i;
        while j < end
            && info_cc(&buffer.info[j as usize]) == cc
            && info_is_mcm(&buffer.info[j as usize])
        {
            j += 1;
        }

        if i == j {
            cc += 10;
            continue;
        }

        /* Shift it! */
        debug_assert!(j - i <= HB_OT_SHAPE_MAX_COMBINING_MARKS);
        buffer.merge_clusters(start, j);
        let (s, i_, j_) = (start as usize, i as usize, j as usize);
        let temp: Vec<HbGlyphInfo> = buffer.info[i_..j_].to_vec();
        buffer.info.copy_within(s..i_, s + j_ - i_);
        buffer.info[s..s + (j_ - i_)].copy_from_slice(&temp);

        /* Renumber CC such that the reordered sequence is still sorted.
         * 22 and 26 are chosen because they are smaller than all Arabic categories,
         * and are folded back to 220/230 respectively during fallback mark positioning.
         *
         * We do this because the CGJ-handling logic in the normalizer relies on
         * mark sequences having an increasing order even after this reordering.
         * https://github.com/harfbuzz/harfbuzz/issues/554
         * This, however, does break some obscure sequences, where the normalizer
         * might compose a sequence that it should not.  For example, in the seequence
         * ALEF, HAMZAH, MADDAH, we should NOT try to compose ALEF+MADDAH, but with this
         * renumbering, we will.
         */
        let new_start = start + j - i;
        let new_cc = if cc == 220 {
            HB_MODIFIED_COMBINING_CLASS_CCC22
        } else {
            HB_MODIFIED_COMBINING_CLASS_CCC26
        };
        while start < new_start {
            _hb_glyph_info_set_modified_combining_class(&mut buffer.info[start as usize], new_cc);
            start += 1;
        }

        i = j;
        cc += 10;
    }
}

/// `_hb_ot_shaper_arabic`
pub(crate) static _hb_ot_shaper_arabic: HbOtShaper = HbOtShaper {
    collect_features: Some(collect_features_arabic),
    override_features: None,
    data_create: Some(data_create_arabic),
    preprocess_text: None,
    postprocess_glyphs: Some(postprocess_glyphs_arabic),
    decompose: None,
    compose: None,
    setup_masks: Some(setup_masks_arabic),
    reorder_marks: Some(reorder_marks_arabic),
    gpos_tag: HB_TAG_NONE,
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_DEFAULT,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_LATE,
    fallback_position: true,
};

/*
 * hb-ot-shaper-arabic-fallback.hh
 */

/* Features ordered the same as the entries in shaping_table rows,
 * followed by rlig.  Don't change.
 *
 * We currently support one subtable per lookup, and one lookup
 * per feature.  But we allow duplicate features, so we use that!
 */
const ARABIC_FALLBACK_FEATURES: [HbTag; 7] = [
    hb_tag(b'i', b'n', b'i', b't'),
    hb_tag(b'm', b'e', b'd', b'i'),
    hb_tag(b'f', b'i', b'n', b'a'),
    hb_tag(b'i', b's', b'o', b'l'),
    hb_tag(b'r', b'l', b'i', b'g'),
    hb_tag(b'r', b'l', b'i', b'g'),
    hb_tag(b'r', b'l', b'i', b'g'),
];

/// Big-endian writing of the serialized lookups.
fn push16(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&(x as u16).to_be_bytes());
}

/// `Coverage::serialize`: the coverage of the sorted `glyphs`, in the
/// format HarfBuzz's serializer picks.
fn serialize_coverage(glyphs: &[u32]) -> Vec<u8> {
    let count = glyphs.len();
    let mut num_ranges = 0;
    let mut last: u32 = u32::MAX - 1; /* (hb_codepoint_t) -2 */
    let mut unsorted = false;
    for &g in glyphs {
        if last != u32::MAX - 1 && g < last {
            unsorted = true;
        }
        if last.wrapping_add(1) != g {
            num_ranges += 1;
        }
        last = g;
    }
    let format = if !unsorted && count <= num_ranges * 3 {
        1
    } else {
        2
    };

    let mut v = Vec::new();
    push16(&mut v, format);
    if format == 1 {
        /* CoverageFormat1::serialize */
        push16(&mut v, count as u32);
        for &g in glyphs {
            push16(&mut v, g);
        }
    } else {
        /* CoverageFormat2::serialize */
        let mut ranges: Vec<(u32, u32, u32)> = Vec::new();
        let mut last: u32 = u32::MAX - 1;
        let mut unsorted = false;
        for (count, &g) in glyphs.iter().enumerate() {
            if last.wrapping_add(1) != g {
                if last != u32::MAX - 1 && last.wrapping_add(1) > g {
                    unsorted = true;
                }
                ranges.push((g, g, count as u32));
            }
            if let Some(r) = ranges.last_mut() {
                r.1 = g;
            }
            last = g;
        }
        if unsorted {
            /* rangeRecord.as_array ().qsort (RangeRecord<Types>::cmp_range) */
            super::hb_algs::hb_qsort(&mut ranges, |a, b| {
                if a.0 < b.0 {
                    -1
                } else if a.0 > b.0 {
                    1
                } else {
                    0
                }
            });
        }
        push16(&mut v, ranges.len() as u32);
        for (first, last, value) in ranges {
            push16(&mut v, first);
            push16(&mut v, last);
            push16(&mut v, value);
        }
    }
    v
}

/// A one-subtable lookup of `lookup_type` with `lookup_flag`.
fn serialize_lookup(lookup_type: u32, lookup_flag: u32, subtable: Vec<u8>) -> Vec<u8> {
    let mut v = Vec::new();
    push16(&mut v, lookup_type);
    push16(&mut v, lookup_flag);
    push16(&mut v, 1);
    push16(&mut v, 8);
    v.extend(subtable);
    v
}

/// `SubstLookup::serialize_single`
fn serialize_single(lookup_flag: u32, glyphs: &[u32], substitutes: &[u32]) -> Vec<u8> {
    /* SingleSubst::serialize: format 1 if all deltas are the same */
    let mut format = 2;
    let mut delta = 0;
    if !glyphs.is_empty() {
        format = 1;
        let mask = 0xFFFF;
        let get_delta = |k: usize| substitutes[k].wrapping_sub(glyphs[k]) & mask;
        delta = get_delta(0);
        if !(1..glyphs.len()).all(|k| get_delta(k) == delta) {
            format = 2;
        }
    }
    let coverage = serialize_coverage(glyphs);
    let mut st = Vec::new();
    if format == 1 {
        push16(&mut st, 1);
        push16(&mut st, 6); /* coverage */
        push16(&mut st, delta);
    } else {
        push16(&mut st, 2);
        push16(&mut st, 6 + 2 * substitutes.len() as u32); /* coverage */
        push16(&mut st, substitutes.len() as u32);
        for &s in substitutes {
            push16(&mut st, s);
        }
    }
    st.extend(coverage);
    serialize_lookup(1, lookup_flag, st)
}

/// `SubstLookup::serialize_ligature`
fn serialize_ligature(
    lookup_flag: u32,
    first_glyphs: &[u32],
    ligature_per_first_glyph_count_list: &[u32],
    ligatures_list: &[u32],
    component_count_list: &[u32],
    component_list: &[u32],
) -> Vec<u8> {
    /* LigatureSubstFormat1::serialize */
    let n = first_glyphs.len();
    let mut sets: Vec<Vec<u8>> = Vec::new();
    let mut lig_i = 0usize;
    let mut comp_i = 0usize;
    for &ligature_count in &ligature_per_first_glyph_count_list[..n] {
        /* LigatureSet::serialize */
        let ligature_count = (ligature_count as usize).min(ligatures_list.len() - lig_i);
        let mut ligs: Vec<Vec<u8>> = Vec::new();
        for k in 0..ligature_count {
            let component_count = (component_count_list[lig_i + k] as i32 - 1).max(0) as usize;
            let avail = component_list.len() - comp_i;
            let comps = &component_list[comp_i..comp_i + component_count.min(avail)];
            /* Ligature::serialize */
            let mut l = Vec::new();
            push16(&mut l, ligatures_list[lig_i + k]);
            push16(&mut l, comps.len() as u32 + 1);
            for &c in comps {
                push16(&mut l, c);
            }
            ligs.push(l);
            comp_i += component_count.min(avail);
        }
        lig_i += ligature_count;
        let mut set = Vec::new();
        push16(&mut set, ligs.len() as u32);
        let mut off = 2 + 2 * ligs.len();
        for l in &ligs {
            push16(&mut set, off as u32);
            off += l.len();
        }
        for l in ligs {
            set.extend(l);
        }
        sets.push(set);
    }
    let mut st = Vec::new();
    push16(&mut st, 1);
    let header = 6 + 2 * n;
    let sets_len: usize = sets.iter().map(|s| s.len()).sum();
    push16(&mut st, (header + sets_len) as u32); /* coverage */
    push16(&mut st, n as u32);
    let mut off = header;
    for s in &sets {
        push16(&mut st, off as u32);
        off += s.len();
    }
    for s in sets {
        st.extend(s);
    }
    st.extend(serialize_coverage(first_glyphs));
    serialize_lookup(4, lookup_flag, st)
}

/// `hb_font_get_glyph (font, u, 0, &glyph)`
fn font_get_glyph(font: &mut HbFont, u: HbCodepoint, glyph: &mut HbCodepoint) -> bool {
    font.get_nominal_glyph(u, glyph, 0)
}

/// `arabic_fallback_synthesize_lookup_single`
fn arabic_fallback_synthesize_lookup_single(
    font: &mut HbFont,
    feature_index: usize,
) -> Option<Vec<u8>> {
    let mut glyphs: Vec<u32> = Vec::new();
    let mut substitutes: Vec<u32> = Vec::new();

    /* Populate arrays */
    for u in SHAPING_TABLE_FIRST..SHAPING_TABLE_LAST + 1 {
        let s = shaping_table[(u - SHAPING_TABLE_FIRST) as usize][feature_index] as u32;
        let mut u_glyph = 0;
        let mut s_glyph = 0;

        if s == 0
            || !font_get_glyph(font, u, &mut u_glyph)
            || !font_get_glyph(font, s, &mut s_glyph)
            || u_glyph == s_glyph
            || u_glyph > 0xFFFF
            || s_glyph > 0xFFFF
        {
            continue;
        }

        glyphs.push(u_glyph);
        substitutes.push(s_glyph);
    }

    if glyphs.is_empty() {
        return None;
    }

    /* Bubble-sort or something equally good!
     * May not be good-enough for presidential candidate interviews, but good-enough for us... */
    /* (hb_stable_sort) */
    let mut pairs: Vec<(u32, u32)> = glyphs
        .iter()
        .copied()
        .zip(substitutes.iter().copied())
        .collect();
    pairs.sort_by_key(|p| p.0);
    let glyphs: Vec<u32> = pairs.iter().map(|p| p.0).collect();
    let substitutes: Vec<u32> = pairs.iter().map(|p| p.1).collect();

    Some(serialize_single(
        LOOKUP_FLAG_IGNORE_MARKS,
        &glyphs,
        &substitutes,
    ))
}

/// `arabic_fallback_synthesize_lookup_ligature`
fn arabic_fallback_synthesize_lookup_ligature<const NCOMP: usize, const NLIG: usize>(
    font: &mut HbFont,
    lig_table: &[LigatureSet<NCOMP, NLIG>],
    lookup_flags: u32,
) -> Option<Vec<u8>> {
    let mut first_glyphs: Vec<u32> = Vec::new();
    let mut first_glyphs_indirection: Vec<usize> = Vec::new();
    let mut ligature_per_first_glyph_count_list: Vec<u32> = Vec::new();

    /* We know that all our ligatures have the same number of components. */
    let mut ligature_list: Vec<u32> = Vec::new();
    let mut component_count_list: Vec<u32> = Vec::new();
    let mut component_list: Vec<u32> = Vec::new();

    /* Populate arrays */

    /* Sort out the first-glyphs */
    for (first_glyph_idx, set) in lig_table.iter().enumerate() {
        let first_u = set.first as u32;
        let mut first_glyph = 0;
        if !font_get_glyph(font, first_u, &mut first_glyph) {
            continue;
        }
        first_glyphs.push(first_glyph);
        ligature_per_first_glyph_count_list.push(0);
        first_glyphs_indirection.push(first_glyph_idx);
    }
    /* (hb_stable_sort) */
    let mut pairs: Vec<(u32, usize)> = first_glyphs
        .iter()
        .copied()
        .zip(first_glyphs_indirection.iter().copied())
        .collect();
    pairs.sort_by_key(|p| p.0);
    let first_glyphs: Vec<u32> = pairs.iter().map(|p| p.0).collect();
    let first_glyphs_indirection: Vec<usize> = pairs.iter().map(|p| p.1).collect();

    /* Now that the first-glyphs are sorted, walk again, populate ligatures. */
    for i in 0..first_glyphs.len() {
        let first_glyph_idx = first_glyphs_indirection[i];

        for ligature_idx in 0..NLIG {
            let lig = &lig_table[first_glyph_idx].ligatures[ligature_idx];
            let ligature_u = lig.ligature as u32;
            let mut ligature_glyph = 0;
            if !font_get_glyph(font, ligature_u, &mut ligature_glyph) {
                continue;
            }

            let component_count = NCOMP;
            let mut matched = true;
            for j in 0..component_count {
                let component_u = lig.components[j] as u32;
                let mut component_glyph = 0;
                if component_u == 0 || !font.get_nominal_glyph(component_u, &mut component_glyph, 0)
                {
                    matched = false;
                    break;
                }
                component_list.push(component_glyph);
            }
            if !matched {
                continue;
            }

            component_count_list.push(1 + component_count as u32);
            ligature_list.push(ligature_glyph);

            ligature_per_first_glyph_count_list[i] += 1;
        }
    }

    if ligature_list.is_empty() {
        return None;
    }

    Some(serialize_ligature(
        lookup_flags,
        &first_glyphs,
        &ligature_per_first_glyph_count_list,
        &ligature_list,
        &component_count_list,
        &component_list,
    ))
}

/// `arabic_fallback_synthesize_lookup`
fn arabic_fallback_synthesize_lookup(font: &mut HbFont, feature_index: usize) -> Option<Vec<u8>> {
    if feature_index < 4 {
        arabic_fallback_synthesize_lookup_single(font, feature_index)
    } else {
        match feature_index {
            4 => arabic_fallback_synthesize_lookup_ligature(
                font,
                &ligature_3_table,
                LOOKUP_FLAG_IGNORE_MARKS,
            ),
            5 => arabic_fallback_synthesize_lookup_ligature(
                font,
                &ligature_table,
                LOOKUP_FLAG_IGNORE_MARKS,
            ),
            6 => arabic_fallback_synthesize_lookup_ligature(font, &ligature_mark_table, 0),
            _ => None,
        }
    }
}

/// `arabic_fallback_plan_t`: the masks and the synthesized lookups.
#[derive(Debug)]
struct ArabicFallbackPlan {
    lookups: Vec<(HbMask, Vec<u8>)>,
}

/// `arabic_fallback_plan_init_unicode`
fn arabic_fallback_plan_init_unicode(
    fallback_plan: &mut ArabicFallbackPlan,
    plan: &HbOtShapePlan,
    font: &mut HbFont,
) -> bool {
    for (i, &tag) in ARABIC_FALLBACK_FEATURES.iter().enumerate() {
        let mask = plan.map.get_1_mask(tag);
        if mask != 0 {
            if let Some(lookup) = arabic_fallback_synthesize_lookup(font, i) {
                fallback_plan.lookups.push((mask, lookup));
            }
        }
    }

    !fallback_plan.lookups.is_empty()
}

/// `arabic_fallback_plan_create`
fn arabic_fallback_plan_create(plan: &HbOtShapePlan, font: &mut HbFont) -> ArabicFallbackPlan {
    let mut fallback_plan = ArabicFallbackPlan {
        lookups: Vec::new(),
    };

    /* Try synthesizing GSUB table using Unicode Arabic Presentation Forms,
     * in case the font has cmap entries for the presentation-forms characters. */
    if arabic_fallback_plan_init_unicode(&mut fallback_plan, plan, font) {
        return fallback_plan;
    }

    /* (the Windows-1256 fallback of Windows builds is not translated) */

    fallback_plan
}

/// `arabic_fallback_plan_shape`
fn arabic_fallback_plan_shape(
    fallback_plan: &ArabicFallbackPlan,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
) {
    let face = font.p.face.clone();
    let empty = GsubGposAccel::empty(GsubGposKind::Gsub);
    let mut c = HbOtApplyContext::new(0, font, &face, buffer, &empty);
    for (mask, lookup) in &fallback_plan.lookups {
        c.set_lookup_mask(*mask, true);
        hb_ot_layout_substitute_lookup(&mut c, Lookup(lookup));
    }
}
