// Rust translation of src/hb-ot-shaper-myanmar.cc and the syllable finder
// of src/hb-ot-shaper-myanmar-machine.rl (whose generated tables are in
// hb_ot_shaper_myanmar_machine.rs) from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2011,2012,2013  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The Myanmar shaper, and the Myanmar Zawgyi one (which does no
//! processing).

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::*;
use super::hb_ot_map::*;
use super::hb_ot_shape::{HbOtShapePlan, HbOtShapePlanner};
use super::hb_ot_shape_normalize::*;
use super::hb_ot_shaper::*;
use super::hb_ot_shaper_indic::*;
use super::hb_ot_shaper_indic_table::hb_indic_get_categories;
use super::hb_ot_shaper_myanmar_machine::*;
use super::hb_ot_shaper_syllabic::*;
use super::hb_ragel::ragel_exec;
use super::hb_unicode::*;

/* buffer var allocations */
/// `myanmar_category()`: `ot_shaper_var_u8_category()`
#[inline]
fn myanmar_category(info: &HbGlyphInfo) -> u8 {
    info.complex_var_u8_category()
}
/// `myanmar_position()`: `ot_shaper_var_u8_auxiliary()`
#[inline]
fn myanmar_position(info: &HbGlyphInfo) -> u8 {
    info.complex_var_u8_auxiliary()
}
#[inline]
fn set_myanmar_position(info: &mut HbGlyphInfo, v: u8) {
    info.set_complex_var_u8_auxiliary(v);
}

/* M_Cat */
const M_A: u8 = myanmar_syllable_machine_ex_A;
const M_AS: u8 = myanmar_syllable_machine_ex_As;
const M_C: u8 = myanmar_syllable_machine_ex_C;
const M_CS: u8 = myanmar_syllable_machine_ex_CS;
const M_DOTTEDCIRCLE: u8 = myanmar_syllable_machine_ex_DOTTEDCIRCLE;
const M_GB: u8 = myanmar_syllable_machine_ex_GB;
const M_H: u8 = myanmar_syllable_machine_ex_H;
const M_IV: u8 = myanmar_syllable_machine_ex_IV;
const M_MR: u8 = myanmar_syllable_machine_ex_MR;
const M_RA: u8 = myanmar_syllable_machine_ex_Ra;
const M_VBLW: u8 = myanmar_syllable_machine_ex_VBlw;
const M_VPRE: u8 = myanmar_syllable_machine_ex_VPre;
const M_VS: u8 = myanmar_syllable_machine_ex_VS;

/// `myanmar_syllable_type_t`
const MYANMAR_CONSONANT_SYLLABLE: u8 = 0;
const MYANMAR_BROKEN_CLUSTER: u8 = 1;
const MYANMAR_NON_MYANMAR_CLUSTER: u8 = 2;

/// `find_syllables_myanmar` (hb-ot-shaper-myanmar-machine.rl)
fn find_syllables_myanmar(buffer: &mut HbBuffer) {
    let len = buffer.len as usize;
    let mut syllable_serial: u8 = 1;
    let mut found_broken = false;
    let info = &mut buffer.info;

    /* (the categories, read before the actions change the syllables) */
    let cats: Vec<u8> = info[..len].iter().map(myanmar_category).collect();

    let mut found_syllable = |ts: i64, te: i64, syllable_type: u8, info: &mut [HbGlyphInfo]| {
        for i in ts..te {
            info[i as usize].set_syllable((syllable_serial << 4) | syllable_type);
        }
        syllable_serial += 1;
        if syllable_serial == 16 {
            syllable_serial = 1;
        }
    };

    ragel_exec(
        &MYANMAR_TABLES,
        len,
        |p| cats[p] as u16,
        |a, st| match a {
            6 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, MYANMAR_CONSONANT_SYLLABLE, info);
            }
            4 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, MYANMAR_NON_MYANMAR_CLUSTER, info);
            }
            8 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, MYANMAR_BROKEN_CLUSTER, info);
                found_broken = true;
            }
            3 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, MYANMAR_NON_MYANMAR_CLUSTER, info);
            }
            5 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, MYANMAR_CONSONANT_SYLLABLE, info);
            }
            7 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, MYANMAR_BROKEN_CLUSTER, info);
                found_broken = true;
            }
            9 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, MYANMAR_NON_MYANMAR_CLUSTER, info);
            }
            _ => {}
        },
    );

    if found_broken {
        buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_BROKEN_SYLLABLE;
    }
}

/*
 * Myanmar shaper.
 */

static MYANMAR_BASIC_FEATURES: [HbTag; 4] = [
    /*
     * Basic features.
     * These features are applied in order, one at a time, after reordering,
     * constrained to the syllable.
     */
    hb_tag(b'r', b'p', b'h', b'f'),
    hb_tag(b'p', b'r', b'e', b'f'),
    hb_tag(b'b', b'l', b'w', b'f'),
    hb_tag(b'p', b's', b't', b'f'),
];
static MYANMAR_OTHER_FEATURES: [HbTag; 4] = [
    /*
     * Other features.
     * These features are applied all at once, after clearing syllables.
     */
    hb_tag(b'p', b'r', b'e', b's'),
    hb_tag(b'a', b'b', b'v', b's'),
    hb_tag(b'b', b'l', b'w', b's'),
    hb_tag(b'p', b's', b't', b's'),
];

/// `set_myanmar_properties`
#[inline]
fn set_myanmar_properties(info: &mut HbGlyphInfo) {
    let u = info.codepoint;
    let type_ = hb_indic_get_categories(u);

    info.set_complex_var_u8_category((type_ & 0xFF) as u8);
}

/// `is_one_of_myanmar`
#[inline]
fn is_one_of_myanmar(info: &HbGlyphInfo, flags: u32) -> bool {
    /* If it ligated, all bets are off. */
    if _hb_glyph_info_ligated(info) {
        return false;
    }
    flag_unsafe(myanmar_category(info) as u32) & flags != 0
}

/* Note:
 *
 * We treat Vowels and placeholders as if they were consonants.  This is safe because Vowels
 * cannot happen in a consonant syllable.  The plus side however is, we can call the
 * consonant syllable logic from the vowel syllable function and get it all right!
 *
 * Keep in sync with consonant_categories in the generator. */
const CONSONANT_FLAGS_MYANMAR: u32 = flag(M_C as u32)
    | flag(M_CS as u32)
    | flag(M_RA as u32)
    | /* FLAG (M_Cat(CM)) | */ flag(M_IV as u32)
    | flag(M_GB as u32)
    | flag(M_DOTTEDCIRCLE as u32);

/// `is_consonant_myanmar`
#[inline]
fn is_consonant_myanmar(info: &HbGlyphInfo) -> bool {
    is_one_of_myanmar(info, CONSONANT_FLAGS_MYANMAR)
}

/// `collect_features_myanmar`
fn collect_features_myanmar(plan: &mut HbOtShapePlanner) {
    let map = &mut plan.map;

    /* Do this before any lookups have been applied. */
    map.add_gsub_pause(Some(setup_syllables_myanmar));

    map.enable_feature(hb_tag(b'l', b'o', b'c', b'l'), F_PER_SYLLABLE, 1);
    /* The Indic specs do not require ccmp, but we apply it here since if
     * there is a use of it, it's typically at the beginning. */
    map.enable_feature(hb_tag(b'c', b'c', b'm', b'p'), F_PER_SYLLABLE, 1);

    map.add_gsub_pause(Some(reorder_myanmar));

    for &feature in &MYANMAR_BASIC_FEATURES {
        map.enable_feature(feature, F_MANUAL_ZWJ | F_PER_SYLLABLE, 1);
        map.add_gsub_pause(None);
    }
    map.add_gsub_pause(Some(hb_syllabic_clear_var)); // Don't need syllables anymore, use stop to free buffer var

    for &feature in &MYANMAR_OTHER_FEATURES {
        map.enable_feature(feature, F_MANUAL_ZWJ, 1);
    }
}

/// `setup_masks_myanmar`
fn setup_masks_myanmar(_plan: &HbOtShapePlan, buffer: &mut HbBuffer, _font: &mut HbFont) {
    buffer.allocate_var(VAR_COMPLEX_CATEGORY.0, VAR_COMPLEX_CATEGORY.1);
    buffer.allocate_var(VAR_COMPLEX_AUXILIARY.0, VAR_COMPLEX_AUXILIARY.1);

    /* No masks, we just save information about characters. */

    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        set_myanmar_properties(info);
    }
}

/// `setup_syllables_myanmar`
fn setup_syllables_myanmar(
    _plan: &HbOtShapePlan,
    _font: &mut HbFont,
    buffer: &mut HbBuffer,
) -> bool {
    buffer.allocate_var(VAR_SYLLABLE.0, VAR_SYLLABLE.1);
    find_syllables_myanmar(buffer);
    for (start, end) in foreach_syllable(buffer) {
        buffer.unsafe_to_break(start, end);
    }
    false
}

/// `compare_myanmar_order`
fn compare_myanmar_order(pa: &HbGlyphInfo, pb: &HbGlyphInfo) -> i32 {
    let a = myanmar_position(pa) as i32;
    let b = myanmar_position(pb) as i32;

    a - b
}

/* Rules from:
 * https://docs.microsoft.com/en-us/typography/script-development/myanmar */

/// `initial_reordering_consonant_syllable`
fn initial_reordering_consonant_syllable(buffer: &mut HbBuffer, start: u32, end: u32) {
    let mut base = end;
    let mut has_reph = false;

    {
        let info = &buffer.info;
        let mut limit = start;
        if start + 3 <= end
            && myanmar_category(&info[start as usize]) == M_RA
            && myanmar_category(&info[(start + 1) as usize]) == M_AS
            && myanmar_category(&info[(start + 2) as usize]) == M_H
        {
            limit += 3;
            base = start;
            has_reph = true;
        }

        {
            if !has_reph {
                base = limit;
            }

            for i in limit..end {
                if is_consonant_myanmar(&info[i as usize]) {
                    base = i;
                    break;
                }
            }
        }
    }

    /* Reorder! */
    {
        let info = &mut buffer.info;
        let mut i = start;
        while i < start + if has_reph { 3 } else { 0 } {
            set_myanmar_position(&mut info[i as usize], POS_AFTER_MAIN);
            i += 1;
        }
        while i < base {
            set_myanmar_position(&mut info[i as usize], POS_PRE_C);
            i += 1;
        }
        if i < end {
            set_myanmar_position(&mut info[i as usize], POS_BASE_C);
            i += 1;
        }
        let mut pos = POS_AFTER_MAIN;
        /* The following loop may be ugly, but it implements all of
         * Myanmar reordering! */
        while i < end {
            let iu = i as usize;
            i += 1;
            if myanmar_category(&info[iu]) == M_MR
            /* Pre-base reordering */
            {
                set_myanmar_position(&mut info[iu], POS_PRE_C);
                continue;
            }
            if myanmar_category(&info[iu]) == M_VPRE
            /* Left matra */
            {
                set_myanmar_position(&mut info[iu], POS_PRE_M);
                continue;
            }
            if myanmar_category(&info[iu]) == M_VS {
                let p = myanmar_position(&info[iu - 1]);
                set_myanmar_position(&mut info[iu], p);
                continue;
            }

            if pos == POS_AFTER_MAIN && myanmar_category(&info[iu]) == M_VBLW {
                pos = POS_BELOW_C;
                set_myanmar_position(&mut info[iu], pos);
                continue;
            }

            if pos == POS_BELOW_C && myanmar_category(&info[iu]) == M_A {
                set_myanmar_position(&mut info[iu], POS_BEFORE_SUB);
                continue;
            }
            if pos == POS_BELOW_C && myanmar_category(&info[iu]) == M_VBLW {
                set_myanmar_position(&mut info[iu], pos);
                continue;
            }
            if pos == POS_BELOW_C && myanmar_category(&info[iu]) != M_A {
                pos = POS_AFTER_SUB;
                set_myanmar_position(&mut info[iu], pos);
                continue;
            }
            set_myanmar_position(&mut info[iu], pos);
        }
    }

    /* Sit tight, rock 'n roll! */
    buffer.sort(start, end, compare_myanmar_order);

    /* Flip left-matra sequence. */
    let mut first_left_matra = end;
    let mut last_left_matra = end;
    for i in start..end {
        if myanmar_position(&buffer.info[i as usize]) == POS_PRE_M {
            if first_left_matra == end {
                first_left_matra = i;
            }
            last_left_matra = i;
        }
    }
    /* https://github.com/harfbuzz/harfbuzz/issues/3863 */
    if first_left_matra < last_left_matra {
        /* No need to merge clusters, done already? */
        buffer.reverse_range(first_left_matra, last_left_matra + 1);
        /* Reverse back VS, etc. */
        let mut i = first_left_matra;
        for j in first_left_matra..=last_left_matra {
            if myanmar_category(&buffer.info[j as usize]) == M_VPRE {
                buffer.reverse_range(i, j + 1);
                i = j + 1;
            }
        }
    }
}

/// `reorder_syllable_myanmar`
fn reorder_syllable_myanmar(buffer: &mut HbBuffer, start: u32, end: u32) {
    let syllable_type = buffer.info[start as usize].syllable() & 0x0F;
    match syllable_type {
        /* We already inserted dotted-circles, so just call the consonant_syllable. */
        MYANMAR_BROKEN_CLUSTER | MYANMAR_CONSONANT_SYLLABLE => {
            initial_reordering_consonant_syllable(buffer, start, end);
        }
        MYANMAR_NON_MYANMAR_CLUSTER => {}
        _ => {}
    }
}

/// `reorder_myanmar`
fn reorder_myanmar(_plan: &HbOtShapePlan, font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    let mut ret = false;
    /* (buffer->message (font, "start reordering myanmar"): no message
     * callback, so it is true) */
    if hb_syllabic_insert_dotted_circles(
        font,
        buffer,
        MYANMAR_BROKEN_CLUSTER as u32,
        M_DOTTEDCIRCLE as u32,
        -1,
        -1,
    ) {
        ret = true;
    }

    for (start, end) in foreach_syllable(buffer) {
        reorder_syllable_myanmar(buffer, start, end);
    }

    buffer.deallocate_var(VAR_COMPLEX_CATEGORY.0, VAR_COMPLEX_CATEGORY.1);
    buffer.deallocate_var(VAR_COMPLEX_AUXILIARY.0, VAR_COMPLEX_AUXILIARY.1);

    ret
}

/// `_hb_ot_shaper_myanmar`
pub(crate) static _hb_ot_shaper_myanmar: HbOtShaper = HbOtShaper {
    collect_features: Some(collect_features_myanmar),
    override_features: None,
    data_create: None,
    preprocess_text: None,
    postprocess_glyphs: None,
    decompose: None,
    compose: None,
    setup_masks: Some(setup_masks_myanmar),
    reorder_marks: None,
    gpos_tag: HB_TAG_NONE,
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS_NO_SHORT_CIRCUIT,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_EARLY,
    fallback_position: false,
};

/* Ugly Zawgyi encoding.
 * Disable all auto processing.
 * https://github.com/harfbuzz/harfbuzz/issues/1162 */
/// `_hb_ot_shaper_myanmar_zawgyi`
pub(crate) static _hb_ot_shaper_myanmar_zawgyi: HbOtShaper = HbOtShaper {
    collect_features: None,
    override_features: None,
    data_create: None,
    preprocess_text: None,
    postprocess_glyphs: None,
    decompose: None,
    compose: None,
    setup_masks: None,
    reorder_marks: None,
    gpos_tag: HB_TAG_NONE,
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_NONE,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_NONE,
    fallback_position: false,
};
