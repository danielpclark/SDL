// Rust translation of src/hb-ot-shaper-khmer.cc and the syllable finder of
// src/hb-ot-shaper-khmer-machine.rl (whose generated tables are in
// hb_ot_shaper_khmer_machine.rs) from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The Khmer shaper.
//!
//! Translation notes: `uniscribe_bug_compatible` (C reads it from the
//! `HB_OPTIONS` environment variable) is always off, as by default.

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::*;
use super::hb_ot_map::*;
use super::hb_ot_shape::{HbOtShapePlan, HbOtShapePlanner};
use super::hb_ot_shape_normalize::*;
use super::hb_ot_shaper::*;
use super::hb_ot_shaper_indic_table::hb_indic_get_categories;
use super::hb_ot_shaper_khmer_machine::*;
use super::hb_ot_shaper_syllabic::*;
use super::hb_ragel::ragel_exec;
use super::hb_unicode::*;

/* buffer var allocations */
/// `khmer_category()`: `ot_shaper_var_u8_category()`
#[inline]
fn khmer_category(info: &HbGlyphInfo) -> u8 {
    info.complex_var_u8_category()
}

/* K_Cat */
const K_H: u8 = khmer_syllable_machine_ex_H;
const K_RA: u8 = khmer_syllable_machine_ex_Ra;
const K_VPRE: u8 = khmer_syllable_machine_ex_VPre;
const K_DOTTEDCIRCLE: u8 = khmer_syllable_machine_ex_DOTTEDCIRCLE;

/// `khmer_syllable_type_t`
const KHMER_CONSONANT_SYLLABLE: u8 = 0;
const KHMER_BROKEN_CLUSTER: u8 = 1;
const KHMER_NON_KHMER_CLUSTER: u8 = 2;

/// `find_syllables_khmer` (hb-ot-shaper-khmer-machine.rl)
fn find_syllables_khmer(buffer: &mut HbBuffer) {
    let len = buffer.len as usize;
    let mut syllable_serial: u8 = 1;
    let mut found_broken = false;
    let info = &mut buffer.info;

    /* (the categories, read before the actions change the syllables) */
    let cats: Vec<u8> = info[..len].iter().map(khmer_category).collect();

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
        &KHMER_TABLES,
        len,
        |p| cats[p] as u16,
        |a, st| match a {
            2 => st.te = st.p + 1,
            8 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, KHMER_NON_KHMER_CLUSTER, info);
            }
            10 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, KHMER_CONSONANT_SYLLABLE, info);
            }
            11 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, KHMER_BROKEN_CLUSTER, info);
                found_broken = true;
            }
            12 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, KHMER_NON_KHMER_CLUSTER, info);
            }
            1 => {
                st.p = st.te - 1;
                found_syllable(st.ts, st.te, KHMER_CONSONANT_SYLLABLE, info);
            }
            3 => {
                st.p = st.te - 1;
                found_syllable(st.ts, st.te, KHMER_BROKEN_CLUSTER, info);
                found_broken = true;
            }
            5 => match st.act {
                2 => {
                    st.p = st.te - 1;
                    found_syllable(st.ts, st.te, KHMER_BROKEN_CLUSTER, info);
                    found_broken = true;
                }
                3 => {
                    st.p = st.te - 1;
                    found_syllable(st.ts, st.te, KHMER_NON_KHMER_CLUSTER, info);
                }
                _ => {}
            },
            4 => {
                st.te = st.p + 1;
                st.act = 2;
            }
            9 => {
                st.te = st.p + 1;
                st.act = 3;
            }
            _ => {}
        },
    );

    if found_broken {
        buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_BROKEN_SYLLABLE;
    }
}

/*
 * Khmer shaper.
 */

const KHMER_FEATURES: [HbOtMapFeature; 9] = [
    /*
     * Basic features.
     * These features are applied all at once, before reordering, constrained
     * to the syllable.
     */
    HbOtMapFeature {
        tag: hb_tag(b'p', b'r', b'e', b'f'),
        flags: F_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'b', b'l', b'w', b'f'),
        flags: F_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'a', b'b', b'v', b'f'),
        flags: F_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'p', b's', b't', b'f'),
        flags: F_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'c', b'f', b'a', b'r'),
        flags: F_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    /*
     * Other features.
     * These features are applied all at once after clearing syllables.
     */
    HbOtMapFeature {
        tag: hb_tag(b'p', b'r', b'e', b's'),
        flags: F_GLOBAL_MANUAL_JOINERS,
    },
    HbOtMapFeature {
        tag: hb_tag(b'a', b'b', b'v', b's'),
        flags: F_GLOBAL_MANUAL_JOINERS,
    },
    HbOtMapFeature {
        tag: hb_tag(b'b', b'l', b'w', b's'),
        flags: F_GLOBAL_MANUAL_JOINERS,
    },
    HbOtMapFeature {
        tag: hb_tag(b'p', b's', b't', b's'),
        flags: F_GLOBAL_MANUAL_JOINERS,
    },
];

/*
 * Must be in the same order as the khmer_features array.
 */
const KHMER_PREF: usize = 0;
const KHMER_BLWF: usize = 1;
const KHMER_ABVF: usize = 2;
const KHMER_PSTF: usize = 3;
const KHMER_CFAR: usize = 4;
const _KHMER_PRES: usize = 5;
const KHMER_NUM_FEATURES: usize = 9;
const KHMER_BASIC_FEATURES: usize = _KHMER_PRES; /* Don't forget to update this! */

/// `set_khmer_properties`
#[inline]
fn set_khmer_properties(info: &mut HbGlyphInfo) {
    let u = info.codepoint;
    let type_ = hb_indic_get_categories(u);

    info.set_complex_var_u8_category((type_ & 0xFF) as u8);
}

/// `collect_features_khmer`
fn collect_features_khmer(plan: &mut HbOtShapePlanner) {
    let map = &mut plan.map;

    /* Do this before any lookups have been applied. */
    map.add_gsub_pause(Some(setup_syllables_khmer));
    map.add_gsub_pause(Some(reorder_khmer));

    /* Testing suggests that Uniscribe does NOT pause between basic
     * features.  Test with KhmerUI.ttf and the following three
     * sequences:
     *
     *   U+1789,U+17BC
     *   U+1789,U+17D2,U+1789
     *   U+1789,U+17D2,U+1789,U+17BC
     *
     * https://github.com/harfbuzz/harfbuzz/issues/974
     */
    map.enable_feature(hb_tag(b'l', b'o', b'c', b'l'), F_PER_SYLLABLE, 1);
    map.enable_feature(hb_tag(b'c', b'c', b'm', b'p'), F_PER_SYLLABLE, 1);

    let mut i = 0;
    while i < KHMER_BASIC_FEATURES {
        map.add_map_feature(&KHMER_FEATURES[i]);
        i += 1;
    }

    /* https://github.com/harfbuzz/harfbuzz/issues/3531 */
    map.add_gsub_pause(Some(hb_syllabic_clear_var)); // Don't need syllables anymore, use stop to free buffer var

    while i < KHMER_NUM_FEATURES {
        map.add_map_feature(&KHMER_FEATURES[i]);
        i += 1;
    }
}

/// `override_features_khmer`
fn override_features_khmer(plan: &mut HbOtShapePlanner) {
    let map = &mut plan.map;

    /* Khmer spec has 'clig' as part of required shaping features:
     * "Apply feature 'clig' to form ligatures that are desired for
     * typographical correctness.", hence in overrides... */
    map.enable_feature(hb_tag(b'c', b'l', b'i', b'g'), F_NONE, 1);

    /* Uniscribe does not apply 'kern' in Khmer. */
    /* (hb_options ().uniscribe_bug_compatible: always off) */

    map.disable_feature(hb_tag(b'l', b'i', b'g', b'a'));
}

/// `khmer_shape_plan_t`
#[derive(Debug)]
pub(crate) struct KhmerShapePlan {
    mask_array: [HbMask; KHMER_NUM_FEATURES],
}

/// `data_create_khmer`
fn data_create_khmer(plan: &HbOtShapePlan) -> Option<ShaperData> {
    let mut khmer_plan = KhmerShapePlan {
        mask_array: [0; KHMER_NUM_FEATURES],
    };

    for i in 0..KHMER_NUM_FEATURES {
        khmer_plan.mask_array[i] = if (KHMER_FEATURES[i].flags & F_GLOBAL) != 0 {
            0
        } else {
            plan.map.get_1_mask(KHMER_FEATURES[i].tag)
        };
    }

    Some(ShaperData::Khmer(Box::new(khmer_plan)))
}

/// The plan's Khmer data.
fn khmer_plan(plan: &HbOtShapePlan) -> &KhmerShapePlan {
    match &plan.data {
        Some(ShaperData::Khmer(p)) => p,
        _ => unreachable!("the Khmer shaper's plan has Khmer data"),
    }
}

/// `setup_masks_khmer`
fn setup_masks_khmer(_plan: &HbOtShapePlan, buffer: &mut HbBuffer, _font: &mut HbFont) {
    buffer.allocate_var(VAR_COMPLEX_CATEGORY.0, VAR_COMPLEX_CATEGORY.1);

    /* We cannot setup masks here.  We save information about characters
     * and setup masks later on in a pause-callback. */

    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        set_khmer_properties(info);
    }
}

/// `setup_syllables_khmer`
fn setup_syllables_khmer(_plan: &HbOtShapePlan, _font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    buffer.allocate_var(VAR_SYLLABLE.0, VAR_SYLLABLE.1);
    find_syllables_khmer(buffer);
    for (start, end) in foreach_syllable(buffer) {
        buffer.unsafe_to_break(start, end);
    }
    false
}

/* Rules from:
 * https://docs.microsoft.com/en-us/typography/script-development/devanagari */

/// `reorder_consonant_syllable`
fn reorder_consonant_syllable(plan: &HbOtShapePlan, buffer: &mut HbBuffer, start: u32, end: u32) {
    let khmer_plan = khmer_plan(plan);

    /* Setup masks. */
    {
        /* Post-base */
        let mask = khmer_plan.mask_array[KHMER_BLWF]
            | khmer_plan.mask_array[KHMER_ABVF]
            | khmer_plan.mask_array[KHMER_PSTF];
        for i in start + 1..end {
            buffer.info[i as usize].mask |= mask;
        }
    }

    let mut num_coengs = 0;
    for i in start + 1..end {
        /* """
         * When a COENG + (Cons | IndV) combination are found (and subscript count
         * is less than two) the character combination is handled according to the
         * subscript type of the character following the COENG.
         *
         * ...
         *
         * Subscript Type 2 - The COENG + RO characters are reordered to immediately
         * before the base glyph. Then the COENG + RO characters are assigned to have
         * the 'pref' OpenType feature applied to them.
         * """
         */
        if khmer_category(&buffer.info[i as usize]) == K_H && num_coengs <= 2 && i + 1 < end {
            num_coengs += 1;

            if khmer_category(&buffer.info[(i + 1) as usize]) == K_RA {
                for j in 0..2 {
                    buffer.info[(i + j) as usize].mask |= khmer_plan.mask_array[KHMER_PREF];
                }

                /* Move the Coeng,Ro sequence to the start. */
                buffer.merge_clusters(start, i + 2);
                let info = &mut buffer.info;
                let t0 = info[i as usize];
                let t1 = info[(i + 1) as usize];
                info.copy_within(start as usize..i as usize, (start + 2) as usize);
                info[start as usize] = t0;
                info[(start + 1) as usize] = t1;

                /* Mark the subsequent stuff with 'cfar'.  Used in Khmer.
                 * Read the feature spec.
                 * This allows distinguishing the following cases with MS Khmer fonts:
                 * U+1784,U+17D2,U+179A,U+17D2,U+1782
                 * U+1784,U+17D2,U+1782,U+17D2,U+179A
                 */
                if khmer_plan.mask_array[KHMER_CFAR] != 0 {
                    for j in i + 2..end {
                        info[j as usize].mask |= khmer_plan.mask_array[KHMER_CFAR];
                    }
                }

                num_coengs = 2; /* Done. */
            }
        }
        /* Reorder left matra piece. */
        else if khmer_category(&buffer.info[i as usize]) == K_VPRE {
            /* Move to the start. */
            buffer.merge_clusters(start, i + 1);
            let info = &mut buffer.info;
            let t = info[i as usize];
            info.copy_within(start as usize..i as usize, (start + 1) as usize);
            info[start as usize] = t;
        }
    }
}

/// `reorder_syllable_khmer`
fn reorder_syllable_khmer(plan: &HbOtShapePlan, buffer: &mut HbBuffer, start: u32, end: u32) {
    let syllable_type = buffer.info[start as usize].syllable() & 0x0F;
    match syllable_type {
        /* We already inserted dotted-circles, so just call the consonant_syllable. */
        KHMER_BROKEN_CLUSTER | KHMER_CONSONANT_SYLLABLE => {
            reorder_consonant_syllable(plan, buffer, start, end);
        }
        KHMER_NON_KHMER_CLUSTER => {}
        _ => {}
    }
}

/// `reorder_khmer`
fn reorder_khmer(plan: &HbOtShapePlan, font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    let mut ret = false;
    /* (buffer->message (font, "start reordering khmer"): no message
     * callback, so it is true) */
    if hb_syllabic_insert_dotted_circles(
        font,
        buffer,
        KHMER_BROKEN_CLUSTER as u32,
        K_DOTTEDCIRCLE as u32,
        -1,
        -1,
    ) {
        ret = true;
    }

    for (start, end) in foreach_syllable(buffer) {
        reorder_syllable_khmer(plan, buffer, start, end);
    }
    buffer.deallocate_var(VAR_COMPLEX_CATEGORY.0, VAR_COMPLEX_CATEGORY.1);

    ret
}

/// `decompose_khmer`
fn decompose_khmer(
    c: &HbOtShapeNormalizeContext,
    _font: &mut HbFont,
    ab: HbCodepoint,
    a: &mut HbCodepoint,
    b: &mut HbCodepoint,
) -> bool {
    match ab {
        /*
         * Decompose split matras that don't have Unicode decompositions.
         */

        /* Khmer */
        0x17BE | 0x17BF | 0x17C0 | 0x17C4 | 0x17C5 => {
            *a = 0x17C1;
            *b = ab;
            return true;
        }
        _ => {}
    }

    c.unicode.decompose(ab, a, b)
}

/// `compose_khmer`
fn compose_khmer(
    c: &HbOtShapeNormalizeContext,
    a: HbCodepoint,
    b: HbCodepoint,
    ab: &mut HbCodepoint,
) -> bool {
    /* Avoid recomposing split matras. */
    if hb_unicode_general_category_is_mark(c.unicode.general_category(a)) {
        return false;
    }

    c.unicode.compose(a, b, ab)
}

/// `_hb_ot_shaper_khmer`
pub(crate) static _hb_ot_shaper_khmer: HbOtShaper = HbOtShaper {
    collect_features: Some(collect_features_khmer),
    override_features: Some(override_features_khmer),
    data_create: Some(data_create_khmer),
    preprocess_text: None,
    postprocess_glyphs: None,
    decompose: Some(decompose_khmer),
    compose: Some(compose_khmer),
    setup_masks: Some(setup_masks_khmer),
    reorder_marks: None,
    gpos_tag: HB_TAG_NONE,
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS_NO_SHORT_CIRCUIT,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_NONE,
    fallback_position: false,
};
