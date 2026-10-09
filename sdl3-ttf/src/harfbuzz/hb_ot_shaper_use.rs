// Rust translation of src/hb-ot-shaper-use.cc, the syllable finder of
// src/hb-ot-shaper-use-machine.rl (whose generated tables are in
// hb_ot_shaper_use_machine.rs) and the generated
// src/hb-ot-shaper-arabic-joining-list.hh from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it).
// Copyright © 2015  Mozilla Foundation.
// Copyright © 2015  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Mozilla Author(s): Jonathan Kew
// Google Author(s): Behdad Esfahbod

//! The Universal Shaping Engine.
//!
//! Translation notes: the syllable machine runs over the glyphs that the
//! C code's filtered iterator yields (not CGJ, and not a ZWNJ followed by
//! a mark), its positions mapped back to buffer indices as the iterator's
//! enumeration does.

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::*;
use super::hb_ot_map::*;
use super::hb_ot_shape::{HbOtShapePlan, HbOtShapePlanner};
use super::hb_ot_shape_normalize::*;
use super::hb_ot_shaper::*;
use super::hb_ot_shaper_arabic::{
    data_create_arabic_plan, setup_masks_arabic_plan, ArabicShapePlan,
};
use super::hb_ot_shaper_syllabic::*;
use super::hb_ot_shaper_use_machine::*;
use super::hb_ot_shaper_use_table::hb_use_get_category;
use super::hb_ot_shaper_vowel_constraints::_hb_preprocess_text_vowel_constraints;
use super::hb_ragel::ragel_exec;
use super::hb_unicode::*;

/* buffer var allocations */
/// `use_category()`: `ot_shaper_var_u8_category()`
#[inline]
fn use_category(info: &HbGlyphInfo) -> u8 {
    info.complex_var_u8_category()
}
#[inline]
fn set_use_category(info: &mut HbGlyphInfo, v: u8) {
    info.set_complex_var_u8_category(v);
}

/* USE(Cat) */
const USE_B: u8 = use_syllable_machine_ex_B;
const USE_CGJ: u8 = use_syllable_machine_ex_CGJ;
const USE_FABV: u8 = use_syllable_machine_ex_FAbv;
const USE_FBLW: u8 = use_syllable_machine_ex_FBlw;
const USE_FMABV: u8 = use_syllable_machine_ex_FMAbv;
const USE_FMBLW: u8 = use_syllable_machine_ex_FMBlw;
const USE_FMPST: u8 = use_syllable_machine_ex_FMPst;
const USE_FPST: u8 = use_syllable_machine_ex_FPst;
const USE_H: u8 = use_syllable_machine_ex_H;
const USE_HVM: u8 = use_syllable_machine_ex_HVM;
const USE_IS: u8 = use_syllable_machine_ex_IS;
const USE_MABV: u8 = use_syllable_machine_ex_MAbv;
const USE_MBLW: u8 = use_syllable_machine_ex_MBlw;
const USE_MPRE: u8 = use_syllable_machine_ex_MPre;
const USE_MPST: u8 = use_syllable_machine_ex_MPst;
const USE_R: u8 = use_syllable_machine_ex_R;
const USE_VABV: u8 = use_syllable_machine_ex_VAbv;
const USE_VBLW: u8 = use_syllable_machine_ex_VBlw;
const USE_VMABV: u8 = use_syllable_machine_ex_VMAbv;
const USE_VMBLW: u8 = use_syllable_machine_ex_VMBlw;
const USE_VMPRE: u8 = use_syllable_machine_ex_VMPre;
const USE_VMPST: u8 = use_syllable_machine_ex_VMPst;
const USE_VPRE: u8 = use_syllable_machine_ex_VPre;
const USE_VPST: u8 = use_syllable_machine_ex_VPst;
const USE_ZWNJ: u8 = use_syllable_machine_ex_ZWNJ;

/// `use_syllable_type_t`
const USE_VIRAMA_TERMINATED_CLUSTER: u8 = 0;
const USE_SAKOT_TERMINATED_CLUSTER: u8 = 1;
const USE_STANDARD_CLUSTER: u8 = 2;
const USE_NUMBER_JOINER_TERMINATED_CLUSTER: u8 = 3;
const USE_NUMERAL_CLUSTER: u8 = 4;
const USE_SYMBOL_CLUSTER: u8 = 5;
const USE_HIEROGLYPH_CLUSTER: u8 = 6;
const USE_BROKEN_CLUSTER: u8 = 7;
const USE_NON_CLUSTER: u8 = 8;

/// `not_ccs_default_ignorable`
#[inline]
fn not_ccs_default_ignorable(i: &HbGlyphInfo) -> bool {
    use_category(i) != USE_CGJ
}

/// `find_syllables_use` (hb-ot-shaper-use-machine.rl)
fn find_syllables_use(buffer: &mut HbBuffer) {
    let len = buffer.len as usize;
    let info = &mut buffer.info;

    /* The filtered iterator: the buffer indices of the glyphs it yields. */
    let mut index: Vec<u32> = Vec::new();
    for p in 0..len {
        if !not_ccs_default_ignorable(&info[p]) {
            continue;
        }
        let mut keep = true;
        if use_category(&info[p]) == USE_ZWNJ {
            for i in p + 1..len {
                if not_ccs_default_ignorable(&info[i]) {
                    keep = !_hb_glyph_info_is_unicode_mark(&info[i]);
                    break;
                }
            }
        }
        if keep {
            index.push(p as u32);
        }
    }
    let cats: Vec<u8> = index
        .iter()
        .map(|&i| use_category(&info[i as usize]))
        .collect();
    /* (*it).second.first: the buffer index at a machine position; at the
     * end, the buffer's length */
    let buffer_index = |k: i64| -> u32 {
        if (k as usize) < index.len() {
            index[k as usize]
        } else {
            len as u32
        }
    };

    let mut syllable_serial: u8 = 1;
    let mut found_broken = false;
    let mut found_syllable = |ts: i64, te: i64, syllable_type: u8, info: &mut [HbGlyphInfo]| {
        for i in buffer_index(ts)..buffer_index(te) {
            info[i as usize].set_syllable((syllable_serial << 4) | syllable_type);
        }
        syllable_serial += 1;
        if syllable_serial == 16 {
            syllable_serial = 1;
        }
    };

    ragel_exec(
        &USE_TABLES,
        cats.len(),
        |p| cats[p] as u16,
        |a, st| match a {
            6 => st.te = st.p + 1,
            14 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, USE_VIRAMA_TERMINATED_CLUSTER, info);
            }
            12 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, USE_SAKOT_TERMINATED_CLUSTER, info);
            }
            10 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, USE_STANDARD_CLUSTER, info);
            }
            18 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, USE_NUMBER_JOINER_TERMINATED_CLUSTER, info);
            }
            16 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, USE_NUMERAL_CLUSTER, info);
            }
            8 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, USE_SYMBOL_CLUSTER, info);
            }
            22 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, USE_HIEROGLYPH_CLUSTER, info);
            }
            5 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, USE_BROKEN_CLUSTER, info);
                found_broken = true;
            }
            4 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, USE_NON_CLUSTER, info);
            }
            13 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, USE_VIRAMA_TERMINATED_CLUSTER, info);
            }
            11 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, USE_SAKOT_TERMINATED_CLUSTER, info);
            }
            9 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, USE_STANDARD_CLUSTER, info);
            }
            17 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, USE_NUMBER_JOINER_TERMINATED_CLUSTER, info);
            }
            15 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, USE_NUMERAL_CLUSTER, info);
            }
            7 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, USE_SYMBOL_CLUSTER, info);
            }
            21 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, USE_HIEROGLYPH_CLUSTER, info);
            }
            19 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, USE_BROKEN_CLUSTER, info);
                found_broken = true;
            }
            20 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, USE_NON_CLUSTER, info);
            }
            1 => {
                st.p = st.te - 1;
                found_syllable(st.ts, st.te, USE_SYMBOL_CLUSTER, info);
            }
            _ => {}
        },
    );

    if found_broken {
        buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_BROKEN_SYLLABLE;
    }
}

/* hb-ot-shaper-arabic-joining-list.hh */

/// `has_arabic_joining`
fn has_arabic_joining(script: HbScript) -> bool {
    /* List of scripts that have data in arabic-table. */
    matches!(
        script,
        HB_SCRIPT_ADLAM
            | HB_SCRIPT_ARABIC
            | HB_SCRIPT_CHORASMIAN
            | HB_SCRIPT_HANIFI_ROHINGYA
            | HB_SCRIPT_MANDAIC
            | HB_SCRIPT_MANICHAEAN
            | HB_SCRIPT_MONGOLIAN
            | HB_SCRIPT_NKO
            | HB_SCRIPT_OLD_UYGHUR
            | HB_SCRIPT_PHAGS_PA
            | HB_SCRIPT_PSALTER_PAHLAVI
            | HB_SCRIPT_SOGDIAN
            | HB_SCRIPT_SYRIAC
    )
}

/*
 * Universal Shaping Engine.
 * https://docs.microsoft.com/en-us/typography/script-development/use
 */

static USE_BASIC_FEATURES: [HbTag; 7] = [
    /*
     * Basic features.
     * These features are applied all at once, before reordering, constrained
     * to the syllable.
     */
    hb_tag(b'r', b'k', b'r', b'f'),
    hb_tag(b'a', b'b', b'v', b'f'),
    hb_tag(b'b', b'l', b'w', b'f'),
    hb_tag(b'h', b'a', b'l', b'f'),
    hb_tag(b'p', b's', b't', b'f'),
    hb_tag(b'v', b'a', b't', b'u'),
    hb_tag(b'c', b'j', b'c', b't'),
];
static USE_TOPOGRAPHICAL_FEATURES: [HbTag; 4] = [
    hb_tag(b'i', b's', b'o', b'l'),
    hb_tag(b'i', b'n', b'i', b't'),
    hb_tag(b'm', b'e', b'd', b'i'),
    hb_tag(b'f', b'i', b'n', b'a'),
];
/* Same order as use_topographical_features. */
/// `joining_form_t`
const JOINING_FORM_ISOL: usize = 0;
const JOINING_FORM_INIT: usize = 1;
const JOINING_FORM_MEDI: usize = 2;
const JOINING_FORM_FINA: usize = 3;
const _JOINING_FORM_NONE: usize = 4;
static USE_OTHER_FEATURES: [HbTag; 5] = [
    /*
     * Other features.
     * These features are applied all at once, after reordering and
     * clearing syllables.
     */
    hb_tag(b'a', b'b', b'v', b's'),
    hb_tag(b'b', b'l', b'w', b's'),
    hb_tag(b'h', b'a', b'l', b'n'),
    hb_tag(b'p', b'r', b'e', b's'),
    hb_tag(b'p', b's', b't', b's'),
];

/// `_hb_clear_substitution_flags` as a pause function
fn clear_substitution_flags(
    _plan: &HbOtShapePlan,
    _font: &mut HbFont,
    buffer: &mut HbBuffer,
) -> bool {
    _hb_clear_substitution_flags(buffer)
}

/// `collect_features_use`
fn collect_features_use(plan: &mut HbOtShapePlanner) {
    let map = &mut plan.map;

    /* Do this before any lookups have been applied. */
    map.add_gsub_pause(Some(setup_syllables_use));

    /* "Default glyph pre-processing group" */
    map.enable_feature(hb_tag(b'l', b'o', b'c', b'l'), F_PER_SYLLABLE, 1);
    map.enable_feature(hb_tag(b'c', b'c', b'm', b'p'), F_PER_SYLLABLE, 1);
    map.enable_feature(hb_tag(b'n', b'u', b'k', b't'), F_PER_SYLLABLE, 1);
    map.enable_feature(
        hb_tag(b'a', b'k', b'h', b'n'),
        F_MANUAL_ZWJ | F_PER_SYLLABLE,
        1,
    );

    /* "Reordering group" */
    map.add_gsub_pause(Some(clear_substitution_flags));
    map.add_feature(
        hb_tag(b'r', b'p', b'h', b'f'),
        F_MANUAL_ZWJ | F_PER_SYLLABLE,
        1,
    );
    map.add_gsub_pause(Some(record_rphf_use));
    map.add_gsub_pause(Some(clear_substitution_flags));
    map.enable_feature(
        hb_tag(b'p', b'r', b'e', b'f'),
        F_MANUAL_ZWJ | F_PER_SYLLABLE,
        1,
    );
    map.add_gsub_pause(Some(record_pref_use));

    /* "Orthographic unit shaping group" */
    for &feature in &USE_BASIC_FEATURES {
        map.enable_feature(feature, F_MANUAL_ZWJ | F_PER_SYLLABLE, 1);
    }

    map.add_gsub_pause(Some(reorder_use));
    map.add_gsub_pause(Some(hb_syllabic_clear_var)); // Don't need syllables anymore, use stop to free buffer var

    /* "Topographical features" */
    for &feature in &USE_TOPOGRAPHICAL_FEATURES {
        map.add_feature(feature, F_NONE, 1);
    }
    map.add_gsub_pause(None);

    /* "Standard typographic presentation" */
    for &feature in &USE_OTHER_FEATURES {
        map.enable_feature(feature, F_MANUAL_ZWJ, 1);
    }
}

/// `use_shape_plan_t`
#[derive(Debug)]
pub(crate) struct UseShapePlan {
    rphf_mask: HbMask,

    arabic_plan: Option<Box<ArabicShapePlan>>,
}

/// `data_create_use`
fn data_create_use(plan: &HbOtShapePlan) -> Option<ShaperData> {
    let mut use_plan = UseShapePlan {
        rphf_mask: plan.map.get_1_mask(hb_tag(b'r', b'p', b'h', b'f')),
        arabic_plan: None,
    };

    if has_arabic_joining(plan.props.script) {
        use_plan.arabic_plan = Some(Box::new(data_create_arabic_plan(plan)));
    }

    Some(ShaperData::Use(Box::new(use_plan)))
}

/// The plan's USE data.
fn use_plan(plan: &HbOtShapePlan) -> &UseShapePlan {
    match &plan.data {
        Some(ShaperData::Use(p)) => p,
        _ => unreachable!("the USE shaper's plan has USE data"),
    }
}

/// `setup_masks_use`
fn setup_masks_use(plan: &HbOtShapePlan, buffer: &mut HbBuffer, _font: &mut HbFont) {
    let use_plan = use_plan(plan);

    /* Do this before allocating use_category(). */
    if let Some(arabic_plan) = &use_plan.arabic_plan {
        setup_masks_arabic_plan(arabic_plan, buffer, plan.props.script);
    }

    buffer.allocate_var(VAR_COMPLEX_CATEGORY.0, VAR_COMPLEX_CATEGORY.1);

    /* We cannot setup masks here.  We save information about characters
     * and setup masks later on in a pause-callback. */

    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        let cat = hb_use_get_category(info.codepoint);
        set_use_category(info, cat);
    }
}

/// `setup_rphf_mask`
fn setup_rphf_mask(plan: &HbOtShapePlan, buffer: &mut HbBuffer) {
    let use_plan = use_plan(plan);

    let mask = use_plan.rphf_mask;
    if mask == 0 {
        return;
    }

    for (start, end) in foreach_syllable(buffer) {
        let info = &mut buffer.info;
        let limit = if use_category(&info[start as usize]) == USE_R {
            1
        } else {
            3u32.min(end - start)
        };
        for i in start..start + limit {
            info[i as usize].mask |= mask;
        }
    }
}

/// `setup_topographical_masks`
fn setup_topographical_masks(plan: &HbOtShapePlan, buffer: &mut HbBuffer) {
    let use_plan = use_plan(plan);
    if use_plan.arabic_plan.is_some() {
        return;
    }

    let mut masks: [HbMask; 4] = [0; 4];
    let mut all_masks = 0;
    for i in 0..4 {
        masks[i] = plan.map.get_1_mask(USE_TOPOGRAPHICAL_FEATURES[i]);
        if masks[i] == plan.map.get_global_mask() {
            masks[i] = 0;
        }
        all_masks |= masks[i];
    }
    if all_masks == 0 {
        return;
    }
    let other_masks = !all_masks;

    let mut last_start = 0;
    let mut last_form = _JOINING_FORM_NONE;
    for (start, end) in foreach_syllable(buffer) {
        let info = &mut buffer.info;
        let syllable_type = info[start as usize].syllable() & 0x0F;
        match syllable_type {
            USE_HIEROGLYPH_CLUSTER | USE_NON_CLUSTER => {
                /* These don't join.  Nothing to do. */
                last_form = _JOINING_FORM_NONE;
            }

            USE_VIRAMA_TERMINATED_CLUSTER
            | USE_SAKOT_TERMINATED_CLUSTER
            | USE_STANDARD_CLUSTER
            | USE_NUMBER_JOINER_TERMINATED_CLUSTER
            | USE_NUMERAL_CLUSTER
            | USE_SYMBOL_CLUSTER
            | USE_BROKEN_CLUSTER => {
                let join = last_form == JOINING_FORM_FINA || last_form == JOINING_FORM_ISOL;

                if join {
                    /* Fixup previous syllable's form. */
                    last_form = if last_form == JOINING_FORM_FINA {
                        JOINING_FORM_MEDI
                    } else {
                        JOINING_FORM_INIT
                    };
                    for i in last_start..start {
                        info[i as usize].mask =
                            (info[i as usize].mask & other_masks) | masks[last_form];
                    }
                }

                /* Form for this syllable. */
                last_form = if join {
                    JOINING_FORM_FINA
                } else {
                    JOINING_FORM_ISOL
                };
                for i in start..end {
                    info[i as usize].mask =
                        (info[i as usize].mask & other_masks) | masks[last_form];
                }
            }
            _ => {}
        }

        last_start = start;
    }
}

/// `setup_syllables_use`
fn setup_syllables_use(plan: &HbOtShapePlan, _font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    buffer.allocate_var(VAR_SYLLABLE.0, VAR_SYLLABLE.1);
    find_syllables_use(buffer);
    for (start, end) in foreach_syllable(buffer) {
        buffer.unsafe_to_break(start, end);
    }
    setup_rphf_mask(plan, buffer);
    setup_topographical_masks(plan, buffer);
    false
}

/// `record_rphf_use`
fn record_rphf_use(plan: &HbOtShapePlan, _font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    let use_plan = use_plan(plan);

    let mask = use_plan.rphf_mask;
    if mask == 0 {
        return false;
    }

    for (start, end) in foreach_syllable(buffer) {
        let info = &mut buffer.info;
        /* Mark a substituted repha as USE(R). */
        let mut i = start;
        while i < end && (info[i as usize].mask & mask) != 0 {
            if _hb_glyph_info_substituted(&info[i as usize]) {
                set_use_category(&mut info[i as usize], USE_R);
                break;
            }
            i += 1;
        }
    }
    false
}

/// `record_pref_use`
fn record_pref_use(_plan: &HbOtShapePlan, _font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    for (start, end) in foreach_syllable(buffer) {
        let info = &mut buffer.info;
        /* Mark a substituted pref as VPre, as they behave the same way. */
        for i in start..end {
            if _hb_glyph_info_substituted(&info[i as usize]) {
                set_use_category(&mut info[i as usize], USE_VPRE);
                break;
            }
        }
    }
    false
}

/// `is_halant_use`
#[inline]
fn is_halant_use(info: &HbGlyphInfo) -> bool {
    (use_category(info) == USE_H || use_category(info) == USE_HVM || use_category(info) == USE_IS)
        && !_hb_glyph_info_ligated(info)
}

const POST_BASE_FLAGS64: u64 = flag64(USE_FABV as u32)
    | flag64(USE_FBLW as u32)
    | flag64(USE_FPST as u32)
    | flag64(USE_FMABV as u32)
    | flag64(USE_FMBLW as u32)
    | flag64(USE_FMPST as u32)
    | flag64(USE_MABV as u32)
    | flag64(USE_MBLW as u32)
    | flag64(USE_MPST as u32)
    | flag64(USE_MPRE as u32)
    | flag64(USE_VABV as u32)
    | flag64(USE_VBLW as u32)
    | flag64(USE_VPST as u32)
    | flag64(USE_VPRE as u32)
    | flag64(USE_VMABV as u32)
    | flag64(USE_VMBLW as u32)
    | flag64(USE_VMPST as u32)
    | flag64(USE_VMPRE as u32);

/// `reorder_syllable_use`
fn reorder_syllable_use(buffer: &mut HbBuffer, start: u32, end: u32) {
    let syllable_type = buffer.info[start as usize].syllable() & 0x0F;
    /* Only a few syllable types need reordering. */
    if (flag_unsafe(syllable_type as u32)
        & (flag(USE_VIRAMA_TERMINATED_CLUSTER as u32)
            | flag(USE_SAKOT_TERMINATED_CLUSTER as u32)
            | flag(USE_STANDARD_CLUSTER as u32)
            | flag(USE_SYMBOL_CLUSTER as u32)
            | flag(USE_BROKEN_CLUSTER as u32)))
        == 0
    {
        return;
    }

    /* Move things forward. */
    if use_category(&buffer.info[start as usize]) == USE_R && end - start > 1 {
        /* Got a repha.  Reorder it towards the end, but before the first post-base
         * glyph. */
        let mut i = start + 1;
        while i < end {
            let is_post_base_glyph = (flag64_unsafe(use_category(&buffer.info[i as usize]) as u32)
                & POST_BASE_FLAGS64)
                != 0
                || is_halant_use(&buffer.info[i as usize]);
            if is_post_base_glyph || i == end - 1 {
                /* If we hit a post-base glyph, move before it; otherwise move to the
                 * end. Shift things in between backward. */

                if is_post_base_glyph {
                    i -= 1;
                }

                buffer.merge_clusters(start, i + 1);
                let info = &mut buffer.info;
                let t = info[start as usize];
                info.copy_within((start + 1) as usize..(i + 1) as usize, start as usize);
                info[i as usize] = t;

                break;
            }
            i += 1;
        }
    }

    /* Move things back. */
    let mut j = start;
    for i in start..end {
        let flag = flag_unsafe(use_category(&buffer.info[i as usize]) as u32);
        if is_halant_use(&buffer.info[i as usize]) {
            /* If we hit a halant, move after it; otherwise move to the beginning, and
             * shift things in between forward. */
            j = i + 1;
        } else if (flag & (super::hb_unicode::flag(USE_VPRE as u32) | super::hb_unicode::flag(USE_VMPRE as u32))) != 0
            /* Only move the first component of a MultipleSubst. */
            && 0 == _hb_glyph_info_get_lig_comp(&buffer.info[i as usize])
            && j < i
        {
            buffer.merge_clusters(j, i + 1);
            let info = &mut buffer.info;
            let t = info[i as usize];
            info.copy_within(j as usize..i as usize, (j + 1) as usize);
            info[j as usize] = t;
        }
    }
}

/// `reorder_use`
fn reorder_use(_plan: &HbOtShapePlan, font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    let mut ret = false;
    /* (buffer->message (font, "start reordering USE"): no message
     * callback, so it is true) */
    if hb_syllabic_insert_dotted_circles(
        font,
        buffer,
        USE_BROKEN_CLUSTER as u32,
        USE_B as u32,
        USE_R as i32,
        -1,
    ) {
        ret = true;
    }

    for (start, end) in foreach_syllable(buffer) {
        reorder_syllable_use(buffer, start, end);
    }

    buffer.deallocate_var(VAR_COMPLEX_CATEGORY.0, VAR_COMPLEX_CATEGORY.1);

    ret
}

/// `preprocess_text_use`
fn preprocess_text_use(plan: &HbOtShapePlan, buffer: &mut HbBuffer, font: &mut HbFont) {
    _hb_preprocess_text_vowel_constraints(plan, buffer, font);
}

/// `compose_use`
fn compose_use(
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

/// `_hb_ot_shaper_use`
pub(crate) static _hb_ot_shaper_use: HbOtShaper = HbOtShaper {
    collect_features: Some(collect_features_use),
    override_features: None,
    data_create: Some(data_create_use),
    preprocess_text: Some(preprocess_text_use),
    postprocess_glyphs: None,
    decompose: None,
    compose: Some(compose_use),
    setup_masks: Some(setup_masks_use),
    reorder_marks: None,
    gpos_tag: HB_TAG_NONE,
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS_NO_SHORT_CIRCUIT,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_EARLY,
    fallback_position: false,
};
