// Rust translation of src/hb-ot-shaper-indic.hh, src/hb-ot-shaper-indic.cc
// and the syllable finder of src/hb-ot-shaper-indic-machine.rl (whose
// generated tables are in hb_ot_shaper_indic_machine.rs) from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The Indic shaper (Devanagari, Bengali, Gurmukhi, Gujarati, Oriya,
//! Tamil, Telugu, Kannada, Malayalam, with their old-spec tags).
//!
//! Translation notes: `uniscribe_bug_compatible` (C reads it from the
//! `HB_OPTIONS` environment variable) is always off, as by default.

use std::sync::atomic::{AtomicU32, Ordering};

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_face::HbFace;
use super::hb_font::HbFont;
use super::hb_ot_layout::*;
use super::hb_ot_map::*;
use super::hb_ot_shape::{HbOtShapePlan, HbOtShapePlanner};
use super::hb_ot_shape_normalize::*;
use super::hb_ot_shaper::*;
use super::hb_ot_shaper_indic_machine::*;
use super::hb_ot_shaper_indic_table::hb_indic_get_categories;
use super::hb_ot_shaper_syllabic::*;
use super::hb_ot_shaper_vowel_constraints::_hb_preprocess_text_vowel_constraints;
use super::hb_ragel::ragel_exec;
use super::hb_unicode::*;

/* Visual positions in a syllable from left to right. */
/// `ot_position_t`
pub(crate) const POS_START: u8 = 0;
pub(crate) const POS_RA_TO_BECOME_REPH: u8 = 1;
pub(crate) const POS_PRE_M: u8 = 2;
pub(crate) const POS_PRE_C: u8 = 3;
pub(crate) const POS_BASE_C: u8 = 4;
pub(crate) const POS_AFTER_MAIN: u8 = 5;
pub(crate) const POS_ABOVE_C: u8 = 6;
pub(crate) const POS_BEFORE_SUB: u8 = 7;
pub(crate) const POS_BELOW_C: u8 = 8;
pub(crate) const POS_AFTER_SUB: u8 = 9;
pub(crate) const POS_BEFORE_POST: u8 = 10;
pub(crate) const POS_POST_C: u8 = 11;
pub(crate) const POS_AFTER_POST: u8 = 12;
pub(crate) const POS_SMVD: u8 = 13;
pub(crate) const POS_END: u8 = 14;

/* buffer var allocations */
/// `indic_category()`: `ot_shaper_var_u8_category()`
#[inline]
pub(crate) fn indic_category(info: &HbGlyphInfo) -> u8 {
    info.complex_var_u8_category()
}
#[inline]
pub(crate) fn set_indic_category(info: &mut HbGlyphInfo, v: u8) {
    info.set_complex_var_u8_category(v);
}
/// `indic_position()`: `ot_shaper_var_u8_auxiliary()`
#[inline]
pub(crate) fn indic_position(info: &HbGlyphInfo) -> u8 {
    info.complex_var_u8_auxiliary()
}
#[inline]
pub(crate) fn set_indic_position(info: &mut HbGlyphInfo, v: u8) {
    info.set_complex_var_u8_auxiliary(v);
}

/// `indic_syllable_type_t`
const INDIC_CONSONANT_SYLLABLE: u8 = 0;
const INDIC_VOWEL_SYLLABLE: u8 = 1;
const INDIC_STANDALONE_CLUSTER: u8 = 2;
const INDIC_SYMBOL_CLUSTER: u8 = 3;
const INDIC_BROKEN_CLUSTER: u8 = 4;
const INDIC_NON_INDIC_CLUSTER: u8 = 5;

/* The categories (I_Cat) */
const I_X: u8 = indic_syllable_machine_ex_X;
const I_C: u8 = indic_syllable_machine_ex_C;
const I_V: u8 = indic_syllable_machine_ex_V;
const I_N: u8 = indic_syllable_machine_ex_N;
const I_H: u8 = indic_syllable_machine_ex_H;
const I_ZWNJ: u8 = indic_syllable_machine_ex_ZWNJ;
const I_ZWJ: u8 = indic_syllable_machine_ex_ZWJ;
const I_M: u8 = indic_syllable_machine_ex_M;
const I_SM: u8 = indic_syllable_machine_ex_SM;
const I_PLACEHOLDER: u8 = indic_syllable_machine_ex_PLACEHOLDER;
const I_DOTTEDCIRCLE: u8 = indic_syllable_machine_ex_DOTTEDCIRCLE;
const I_RS: u8 = indic_syllable_machine_ex_RS;
const I_MPST: u8 = indic_syllable_machine_ex_MPst;
const I_REPHA: u8 = indic_syllable_machine_ex_Repha;
const I_RA: u8 = indic_syllable_machine_ex_Ra;
const I_CM: u8 = indic_syllable_machine_ex_CM;
const I_CS: u8 = indic_syllable_machine_ex_CS;

/// `find_syllables_indic` (hb-ot-shaper-indic-machine.rl)
fn find_syllables_indic(buffer: &mut HbBuffer) {
    let len = buffer.len as usize;
    let mut syllable_serial: u8 = 1;
    let mut found_broken = false;
    let info = &mut buffer.info;

    /* (the categories, read before the actions change the syllables) */
    let cats: Vec<u8> = info[..len].iter().map(indic_category).collect();

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
        &INDIC_TABLES,
        len,
        |p| cats[p] as u16,
        |a, st| match a {
            2 => st.te = st.p + 1,
            11 => {
                st.te = st.p + 1;
                found_syllable(st.ts, st.te, INDIC_NON_INDIC_CLUSTER, info);
            }
            13 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, INDIC_CONSONANT_SYLLABLE, info);
            }
            14 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, INDIC_VOWEL_SYLLABLE, info);
            }
            17 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, INDIC_STANDALONE_CLUSTER, info);
            }
            19 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, INDIC_SYMBOL_CLUSTER, info);
            }
            15 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, INDIC_BROKEN_CLUSTER, info);
                found_broken = true;
            }
            16 => {
                st.te = st.p;
                st.p -= 1;
                found_syllable(st.ts, st.te, INDIC_NON_INDIC_CLUSTER, info);
            }
            1 => {
                st.p = st.te - 1;
                found_syllable(st.ts, st.te, INDIC_CONSONANT_SYLLABLE, info);
            }
            3 => {
                st.p = st.te - 1;
                found_syllable(st.ts, st.te, INDIC_VOWEL_SYLLABLE, info);
            }
            7 => {
                st.p = st.te - 1;
                found_syllable(st.ts, st.te, INDIC_STANDALONE_CLUSTER, info);
            }
            8 => {
                st.p = st.te - 1;
                found_syllable(st.ts, st.te, INDIC_SYMBOL_CLUSTER, info);
            }
            4 => {
                st.p = st.te - 1;
                found_syllable(st.ts, st.te, INDIC_BROKEN_CLUSTER, info);
                found_broken = true;
            }
            6 => match st.act {
                1 => {
                    st.p = st.te - 1;
                    found_syllable(st.ts, st.te, INDIC_CONSONANT_SYLLABLE, info);
                }
                5 => {
                    st.p = st.te - 1;
                    found_syllable(st.ts, st.te, INDIC_BROKEN_CLUSTER, info);
                    found_broken = true;
                }
                6 => {
                    st.p = st.te - 1;
                    found_syllable(st.ts, st.te, INDIC_NON_INDIC_CLUSTER, info);
                }
                _ => {}
            },
            18 => {
                st.te = st.p + 1;
                st.act = 1;
            }
            5 => {
                st.te = st.p + 1;
                st.act = 5;
            }
            12 => {
                st.te = st.p + 1;
                st.act = 6;
            }
            _ => {}
        },
    );

    if found_broken {
        buffer.scratch_flags |= HB_BUFFER_SCRATCH_FLAG_HAS_BROKEN_SYLLABLE;
    }
}

/*
 * Indic shaper.
 */

/// `set_indic_properties`
#[inline]
fn set_indic_properties(info: &mut HbGlyphInfo) {
    let u = info.codepoint;
    let type_ = hb_indic_get_categories(u);
    set_indic_category(info, (type_ & 0xFF) as u8);
    set_indic_position(info, (type_ >> 8) as u8);
}

/// `is_one_of`
#[inline]
fn is_one_of(info: &HbGlyphInfo, flags: u32) -> bool {
    /* If it ligated, all bets are off. */
    if _hb_glyph_info_ligated(info) {
        return false;
    }
    flag_unsafe(indic_category(info) as u32) & flags != 0
}

/* Note:
 *
 * We treat Vowels and placeholders as if they were consonants.  This is safe because Vowels
 * cannot happen in a consonant syllable.  The plus side however is, we can call the
 * consonant syllable logic from the vowel syllable function and get it all right!
 *
 * Keep in sync with consonant_categories in the generator. */
const CONSONANT_FLAGS_INDIC: u32 = flag(I_C as u32)
    | flag(I_CS as u32)
    | flag(I_RA as u32)
    | flag(I_CM as u32)
    | flag(I_V as u32)
    | flag(I_PLACEHOLDER as u32)
    | flag(I_DOTTEDCIRCLE as u32);

/// `is_consonant`
#[inline]
fn is_consonant(info: &HbGlyphInfo) -> bool {
    is_one_of(info, CONSONANT_FLAGS_INDIC)
}

const JOINER_FLAGS: u32 = flag(I_ZWJ as u32) | flag(I_ZWNJ as u32);

/// `is_joiner`
#[inline]
fn is_joiner(info: &HbGlyphInfo) -> bool {
    is_one_of(info, JOINER_FLAGS)
}

/// `is_halant`
#[inline]
fn is_halant(info: &HbGlyphInfo) -> bool {
    is_one_of(info, flag(I_H as u32))
}

/// `hb_indic_would_substitute_feature_t`: the lookups of a feature's
/// stage.
#[derive(Debug, Default)]
pub(crate) struct HbIndicWouldSubstituteFeature {
    lookups: Vec<u16>,
    zero_context: bool,
}

impl HbIndicWouldSubstituteFeature {
    /// `init`
    pub(crate) fn init(map: &HbOtMap, feature_tag: HbTag, zero_context: bool) -> Self {
        let lookups = map.get_stage_lookups(
            0, /*GSUB*/
            map.get_feature_stage(0 /*GSUB*/, feature_tag),
        );
        HbIndicWouldSubstituteFeature {
            lookups: lookups.iter().map(|l| l.index).collect(),
            zero_context,
        }
    }

    /// `would_substitute`
    pub(crate) fn would_substitute(&self, glyphs: &[HbCodepoint], face: &HbFace) -> bool {
        for &lookup in &self.lookups {
            if hb_ot_layout_lookup_would_substitute(face, lookup as u32, glyphs, self.zero_context)
            {
                return true;
            }
        }
        false
    }
}

/*
 * Indic configurations.  Note that we do not want to keep every single script-specific
 * behavior in these tables necessarily.  This should mainly be used for per-script
 * properties that are cheaper keeping here, than in the code.  Ie. if, say, one and
 * only one script has an exception, that one script can be if'ed directly in the code,
 * instead of adding a new flag in these structs.
 */

/// `reph_position_t`
const REPH_POS_AFTER_MAIN: u8 = POS_AFTER_MAIN;
const REPH_POS_BEFORE_SUB: u8 = POS_BEFORE_SUB;
const REPH_POS_AFTER_SUB: u8 = POS_AFTER_SUB;
const REPH_POS_BEFORE_POST: u8 = POS_BEFORE_POST;
const REPH_POS_AFTER_POST: u8 = POS_AFTER_POST;

/// `reph_mode_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RephMode {
    Implicit, /* Reph formed out of initial Ra,H sequence. */
    Explicit, /* Reph formed out of initial Ra,H,ZWJ sequence. */
    LogRepha, /* Encoded Repha character, needs reordering. */
}

/// `blwf_mode_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlwfMode {
    PreAndPost, /* Below-forms feature applied to pre-base and post-base. */
    PostOnly,   /* Below-forms feature applied to post-base only. */
}

/// `indic_config_t`
#[derive(Debug)]
struct IndicConfig {
    script: HbScript,
    has_old_spec: bool,
    virama: HbCodepoint,
    reph_pos: u8,
    reph_mode: RephMode,
    blwf_mode: BlwfMode,
}

const fn cfg(
    script: HbScript,
    has_old_spec: bool,
    virama: HbCodepoint,
    reph_pos: u8,
    reph_mode: RephMode,
    blwf_mode: BlwfMode,
) -> IndicConfig {
    IndicConfig {
        script,
        has_old_spec,
        virama,
        reph_pos,
        reph_mode,
        blwf_mode,
    }
}

#[rustfmt::skip]
static INDIC_CONFIGS: [IndicConfig; 10] = [
    /* Default.  Should be first. */
    cfg(HB_SCRIPT_INVALID,    false,      0, REPH_POS_BEFORE_POST, RephMode::Implicit, BlwfMode::PreAndPost),
    cfg(HB_SCRIPT_DEVANAGARI, true,  0x094D, REPH_POS_BEFORE_POST, RephMode::Implicit, BlwfMode::PreAndPost),
    cfg(HB_SCRIPT_BENGALI,    true,  0x09CD, REPH_POS_AFTER_SUB,   RephMode::Implicit, BlwfMode::PreAndPost),
    cfg(HB_SCRIPT_GURMUKHI,   true,  0x0A4D, REPH_POS_BEFORE_SUB,  RephMode::Implicit, BlwfMode::PreAndPost),
    cfg(HB_SCRIPT_GUJARATI,   true,  0x0ACD, REPH_POS_BEFORE_POST, RephMode::Implicit, BlwfMode::PreAndPost),
    cfg(HB_SCRIPT_ORIYA,      true,  0x0B4D, REPH_POS_AFTER_MAIN,  RephMode::Implicit, BlwfMode::PreAndPost),
    cfg(HB_SCRIPT_TAMIL,      true,  0x0BCD, REPH_POS_AFTER_POST,  RephMode::Implicit, BlwfMode::PreAndPost),
    cfg(HB_SCRIPT_TELUGU,     true,  0x0C4D, REPH_POS_AFTER_POST,  RephMode::Explicit, BlwfMode::PostOnly),
    cfg(HB_SCRIPT_KANNADA,    true,  0x0CCD, REPH_POS_AFTER_POST,  RephMode::Implicit, BlwfMode::PostOnly),
    cfg(HB_SCRIPT_MALAYALAM,  true,  0x0D4D, REPH_POS_AFTER_MAIN,  RephMode::LogRepha, BlwfMode::PreAndPost),
];

const INDIC_FEATURES: [HbOtMapFeature; 17] = [
    /*
     * Basic features.
     * These features are applied in order, one at a time, after initial_reordering,
     * constrained to the syllable.
     */
    HbOtMapFeature {
        tag: hb_tag(b'n', b'u', b'k', b't'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'a', b'k', b'h', b'n'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'r', b'p', b'h', b'f'),
        flags: F_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'r', b'k', b'r', b'f'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
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
        tag: hb_tag(b'h', b'a', b'l', b'f'),
        flags: F_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'p', b's', b't', b'f'),
        flags: F_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'v', b'a', b't', b'u'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'c', b'j', b'c', b't'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    /*
     * Other features.
     * These features are applied all at once, after final_reordering, constrained
     * to the syllable.
     * Default Bengali font in Windows for example has intermixed
     * lookups for init,pres,abvs,blws features.
     */
    HbOtMapFeature {
        tag: hb_tag(b'i', b'n', b'i', b't'),
        flags: F_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'p', b'r', b'e', b's'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'a', b'b', b'v', b's'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'b', b'l', b'w', b's'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'p', b's', b't', b's'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
    HbOtMapFeature {
        tag: hb_tag(b'h', b'a', b'l', b'n'),
        flags: F_GLOBAL_MANUAL_JOINERS | F_PER_SYLLABLE,
    },
];

/*
 * Must be in the same order as the indic_features array.
 */
const INDIC_RPHF: usize = 2;
const INDIC_PREF: usize = 4;
const INDIC_BLWF: usize = 5;
const INDIC_ABVF: usize = 6;
const INDIC_HALF: usize = 7;
const INDIC_PSTF: usize = 8;
const INDIC_INIT: usize = 11;
const INDIC_NUM_FEATURES: usize = 17;
const INDIC_BASIC_FEATURES: usize = INDIC_INIT; /* Don't forget to update this! */

/// `collect_features_indic`
fn collect_features_indic(plan: &mut HbOtShapePlanner) {
    let map = &mut plan.map;

    /* Do this before any lookups have been applied. */
    map.add_gsub_pause(Some(setup_syllables_indic));

    map.enable_feature(hb_tag(b'l', b'o', b'c', b'l'), F_PER_SYLLABLE, 1);
    /* The Indic specs do not require ccmp, but we apply it here since if
     * there is a use of it, it's typically at the beginning. */
    map.enable_feature(hb_tag(b'c', b'c', b'm', b'p'), F_PER_SYLLABLE, 1);

    let mut i = 0;
    map.add_gsub_pause(Some(initial_reordering_indic));

    while i < INDIC_BASIC_FEATURES {
        map.add_map_feature(&INDIC_FEATURES[i]);
        map.add_gsub_pause(None);
        i += 1;
    }

    map.add_gsub_pause(Some(final_reordering_indic));

    while i < INDIC_NUM_FEATURES {
        map.add_map_feature(&INDIC_FEATURES[i]);
        i += 1;
    }
}

/// `override_features_indic`
fn override_features_indic(plan: &mut HbOtShapePlanner) {
    plan.map.disable_feature(hb_tag(b'l', b'i', b'g', b'a'));
    plan.map.add_gsub_pause(Some(hb_syllabic_clear_var)); // Don't need syllables anymore, use stop to free buffer var
}

/// `indic_shape_plan_t`
#[derive(Debug)]
pub(crate) struct IndicShapePlan {
    config: &'static IndicConfig,
    is_old_spec: bool,
    uniscribe_bug_compatible: bool,
    /// (-1 as `u32::MAX`: not loaded yet)
    virama_glyph: AtomicU32,

    rphf: HbIndicWouldSubstituteFeature,
    pref: HbIndicWouldSubstituteFeature,
    blwf: HbIndicWouldSubstituteFeature,
    pstf: HbIndicWouldSubstituteFeature,
    vatu: HbIndicWouldSubstituteFeature,

    mask_array: [HbMask; INDIC_NUM_FEATURES],
}

impl IndicShapePlan {
    /// `load_virama_glyph`
    fn load_virama_glyph(&self, font: &mut HbFont, pglyph: &mut HbCodepoint) -> bool {
        let mut glyph = self.virama_glyph.load(Ordering::Relaxed);
        if glyph == u32::MAX {
            if self.config.virama == 0 || !font.get_nominal_glyph(self.config.virama, &mut glyph, 0)
            {
                glyph = 0;
            }
            /* Technically speaking, the spec says we should apply 'locl' to virama too.
             * Maybe one day... */

            /* Our get_nominal_glyph() function needs a font, so we can't get the virama glyph
             * during shape planning...  Instead, overwrite it here. */
            self.virama_glyph.store(glyph, Ordering::Relaxed);
        }

        *pglyph = glyph;
        glyph != 0
    }
}

/// `data_create_indic`
fn data_create_indic(plan: &HbOtShapePlan) -> Option<ShaperData> {
    let mut config = &INDIC_CONFIGS[0];
    for c in &INDIC_CONFIGS[1..] {
        if plan.props.script == c.script {
            config = c;
            break;
        }
    }

    let is_old_spec =
        config.has_old_spec && ((plan.map.chosen_script[0] & 0x000000FF) != b'2' as u32);

    /* Use zero-context would_substitute() matching for new-spec of the main
     * Indic scripts, and scripts with one spec only, but not for old-specs.
     * The new-spec for all dual-spec scripts says zero-context matching happens.
     *
     * However, testing with Malayalam shows that old and new spec both allow
     * context.  Testing with Bengali new-spec however shows that it doesn't.
     * So, the heuristic here is the way it is.  It should *only* be changed,
     * as we discover more cases of what Windows does.  DON'T TOUCH OTHERWISE.
     */
    let zero_context = !is_old_spec && plan.props.script != HB_SCRIPT_MALAYALAM;

    let mut mask_array = [0; INDIC_NUM_FEATURES];
    for (i, m) in mask_array.iter_mut().enumerate() {
        *m = if INDIC_FEATURES[i].flags & F_GLOBAL != 0 {
            0
        } else {
            plan.map.get_1_mask(INDIC_FEATURES[i].tag)
        };
    }

    Some(ShaperData::Indic(Box::new(IndicShapePlan {
        config,
        is_old_spec,
        uniscribe_bug_compatible: false,
        virama_glyph: AtomicU32::new(u32::MAX),
        rphf: HbIndicWouldSubstituteFeature::init(
            &plan.map,
            hb_tag(b'r', b'p', b'h', b'f'),
            zero_context,
        ),
        pref: HbIndicWouldSubstituteFeature::init(
            &plan.map,
            hb_tag(b'p', b'r', b'e', b'f'),
            zero_context,
        ),
        blwf: HbIndicWouldSubstituteFeature::init(
            &plan.map,
            hb_tag(b'b', b'l', b'w', b'f'),
            zero_context,
        ),
        pstf: HbIndicWouldSubstituteFeature::init(
            &plan.map,
            hb_tag(b'p', b's', b't', b'f'),
            zero_context,
        ),
        vatu: HbIndicWouldSubstituteFeature::init(
            &plan.map,
            hb_tag(b'v', b'a', b't', b'u'),
            zero_context,
        ),
        mask_array,
    })))
}

/// The plan's Indic data.
fn indic_plan(plan: &HbOtShapePlan) -> &IndicShapePlan {
    match &plan.data {
        Some(ShaperData::Indic(p)) => p,
        _ => unreachable!("the Indic shaper's plan has Indic data"),
    }
}

/// `consonant_position_from_face`
fn consonant_position_from_face(
    indic_plan: &IndicShapePlan,
    consonant: HbCodepoint,
    virama: HbCodepoint,
    face: &HbFace,
) -> u8 {
    /* For old-spec, the order of glyphs is Consonant,Virama,
     * whereas for new-spec, it's Virama,Consonant.  However,
     * some broken fonts (like Free Sans) simply copied lookups
     * from old-spec to new-spec without modification.
     * And oddly enough, Uniscribe seems to respect those lookups.
     * Eg. in the sequence U+0924,U+094D,U+0930, Uniscribe finds
     * base at 0.  The font however, only has lookups matching
     * 930,94D in 'blwf', not the expected 94D,930 (with new-spec
     * table).  As such, we simply match both sequences.  Seems
     * to work.
     *
     * Vatu is done as well, for:
     * https://github.com/harfbuzz/harfbuzz/issues/1587
     */
    let glyphs = [virama, consonant, virama];
    if indic_plan.blwf.would_substitute(&glyphs[0..2], face)
        || indic_plan.blwf.would_substitute(&glyphs[1..3], face)
        || indic_plan.vatu.would_substitute(&glyphs[0..2], face)
        || indic_plan.vatu.would_substitute(&glyphs[1..3], face)
    {
        return POS_BELOW_C;
    }
    if indic_plan.pstf.would_substitute(&glyphs[0..2], face)
        || indic_plan.pstf.would_substitute(&glyphs[1..3], face)
    {
        return POS_POST_C;
    }
    if indic_plan.pref.would_substitute(&glyphs[0..2], face)
        || indic_plan.pref.would_substitute(&glyphs[1..3], face)
    {
        return POS_POST_C;
    }
    POS_BASE_C
}

/// `setup_masks_indic`
fn setup_masks_indic(_plan: &HbOtShapePlan, buffer: &mut HbBuffer, _font: &mut HbFont) {
    buffer.allocate_var(VAR_COMPLEX_CATEGORY.0, VAR_COMPLEX_CATEGORY.1);
    buffer.allocate_var(VAR_COMPLEX_AUXILIARY.0, VAR_COMPLEX_AUXILIARY.1);

    /* We cannot setup masks here.  We save information about characters
     * and setup masks later on in a pause-callback. */

    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        set_indic_properties(info);
    }
}

/// `setup_syllables_indic`
fn setup_syllables_indic(_plan: &HbOtShapePlan, _font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    buffer.allocate_var(VAR_SYLLABLE.0, VAR_SYLLABLE.1);
    find_syllables_indic(buffer);
    for (start, end) in foreach_syllable(buffer) {
        buffer.unsafe_to_break(start, end);
    }
    false
}

/// `compare_indic_order`
fn compare_indic_order(pa: &HbGlyphInfo, pb: &HbGlyphInfo) -> std::cmp::Ordering {
    let a = indic_position(pa) as i32;
    let b = indic_position(pb) as i32;

    (a - b).cmp(&0)
}

/// `update_consonant_positions_indic`
fn update_consonant_positions_indic(
    plan: &HbOtShapePlan,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
) {
    let indic_plan = indic_plan(plan);

    let mut virama = 0;
    if indic_plan.load_virama_glyph(font, &mut virama) {
        let face = font.p.face.clone();
        let count = buffer.len as usize;
        for i in 0..count {
            if indic_position(&buffer.info[i]) == POS_BASE_C {
                let consonant = buffer.info[i].codepoint;
                let p = consonant_position_from_face(indic_plan, consonant, virama, &face);
                set_indic_position(&mut buffer.info[i], p);
            }
        }
    }
}

/* Rules from:
 * https://docs.microsqoft.com/en-us/typography/script-development/devanagari */

/// `initial_reordering_consonant_syllable`
fn initial_reordering_consonant_syllable(
    plan: &HbOtShapePlan,
    face: &HbFace,
    buffer: &mut HbBuffer,
    start: u32,
    end: u32,
) {
    let indic_plan = indic_plan(plan);
    let (start_u, end_u) = (start as usize, end as usize);

    /* https://github.com/harfbuzz/harfbuzz/issues/435#issuecomment-335560167
     * // For compatibility with legacy usage in Kannada,
     * // Ra+h+ZWJ must behave like Ra+ZWJ+h...
     */
    if buffer.props.script == HB_SCRIPT_KANNADA
        && start + 3 <= end
        && is_one_of(&buffer.info[start_u], flag(I_RA as u32))
        && is_one_of(&buffer.info[start_u + 1], flag(I_H as u32))
        && is_one_of(&buffer.info[start_u + 2], flag(I_ZWJ as u32))
    {
        buffer.merge_clusters(start + 1, start + 3);
        buffer.info.swap(start_u + 1, start_u + 2);
    }

    /* 1. Find base consonant:
     *
     * The shaping engine finds the base consonant of the syllable, using the
     * following algorithm: starting from the end of the syllable, move backwards
     * until a consonant is found that does not have a below-base or post-base
     * form (post-base forms have to follow below-base forms), or that is not a
     * pre-base-reordering Ra, or arrive at the first consonant. The consonant
     * stopped at will be the base.
     *
     *   o If the syllable starts with Ra + Halant (in a script that has Reph)
     *     and has more than one consonant, Ra is excluded from candidates for
     *     base consonants.
     */

    let mut base = end;
    let mut has_reph = false;

    {
        /* -> If the syllable starts with Ra + Halant (in a script that has Reph)
         *    and has more than one consonant, Ra is excluded from candidates for
         *    base consonants. */
        let mut limit = start;
        let info = &buffer.info;
        if indic_plan.mask_array[INDIC_RPHF] != 0
            && start + 3 <= end
            && ((indic_plan.config.reph_mode == RephMode::Implicit
                && !is_joiner(&info[start_u + 2]))
                || (indic_plan.config.reph_mode == RephMode::Explicit
                    && indic_category(&info[start_u + 2]) == I_ZWJ))
        {
            /* See if it matches the 'rphf' feature. */
            let glyphs = [
                info[start_u].codepoint,
                info[start_u + 1].codepoint,
                if indic_plan.config.reph_mode == RephMode::Explicit {
                    info[start_u + 2].codepoint
                } else {
                    0
                },
            ];
            if indic_plan.rphf.would_substitute(&glyphs[0..2], face)
                || (indic_plan.config.reph_mode == RephMode::Explicit
                    && indic_plan.rphf.would_substitute(&glyphs, face))
            {
                limit += 2;
                while limit < end && is_joiner(&info[limit as usize]) {
                    limit += 1;
                }
                base = start;
                has_reph = true;
            }
        } else if indic_plan.config.reph_mode == RephMode::LogRepha
            && indic_category(&info[start_u]) == I_REPHA
        {
            limit += 1;
            while limit < end && is_joiner(&info[limit as usize]) {
                limit += 1;
            }
            base = start;
            has_reph = true;
        }

        {
            /* -> starting from the end of the syllable, move backwards */
            let mut i = end;
            let mut seen_below = false;
            loop {
                i -= 1;
                let iu = i as usize;
                /* -> until a consonant is found */
                if is_consonant(&info[iu]) {
                    /* -> that does not have a below-base or post-base form
                     * (post-base forms have to follow below-base forms), */
                    if indic_position(&info[iu]) != POS_BELOW_C
                        && (indic_position(&info[iu]) != POS_POST_C || seen_below)
                    {
                        base = i;
                        break;
                    }
                    if indic_position(&info[iu]) == POS_BELOW_C {
                        seen_below = true;
                    }

                    /* -> or that is not a pre-base-reordering Ra,
                     *
                     * IMPLEMENTATION NOTES:
                     *
                     * Our pre-base-reordering Ra's are marked POS_POST_C, so will be skipped
                     * by the logic above already.
                     */

                    /* -> or arrive at the first consonant. The consonant stopped at will
                     * be the base. */
                    base = i;
                } else {
                    /* A ZWJ after a Halant stops the base search, and requests an explicit
                     * half form.
                     * A ZWJ before a Halant, requests a subjoined form instead, and hence
                     * search continues.  This is particularly important for Bengali
                     * sequence Ra,H,Ya that should form Ya-Phalaa by subjoining Ya. */
                    if start < i
                        && indic_category(&info[iu]) == I_ZWJ
                        && indic_category(&info[iu - 1]) == I_H
                    {
                        break;
                    }
                }
                if i <= limit {
                    break;
                }
            }
        }

        /* -> If the syllable starts with Ra + Halant (in a script that has Reph)
         *    and has more than one consonant, Ra is excluded from candidates for
         *    base consonants.
         *
         *  Only do this for unforced Reph. (ie. not for Ra,H,ZWJ. */
        if has_reph && base == start && limit - base <= 2 {
            /* Have no other consonant, so Reph is not formed and Ra becomes base. */
            has_reph = false;
        }
    }

    /* 2. Decompose and reorder Matras:
     *
     * Each matra and any syllable modifier sign in the syllable are moved to the
     * appropriate position relative to the consonant(s) in the syllable. The
     * shaping engine decomposes two- or three-part matras into their constituent
     * parts before any repositioning. Matra characters are classified by which
     * consonant in a conjunct they have affinity for and are reordered to the
     * following positions:
     *
     *   o Before first half form in the syllable
     *   o After subjoined consonants
     *   o After post-form consonant
     *   o After main consonant (for above marks)
     *
     * IMPLEMENTATION NOTES:
     *
     * The normalize() routine has already decomposed matras for us, so we don't
     * need to worry about that.
     */

    /* 3.  Reorder marks to canonical order:
     *
     * Adjacent nukta and halant or nukta and vedic sign are always repositioned
     * if necessary, so that the nukta is first.
     *
     * IMPLEMENTATION NOTES:
     *
     * We don't need to do this: the normalize() routine already did this for us.
     */

    let info = &mut buffer.info;

    /* Reorder characters */

    for i in start_u..base as usize {
        let p = POS_PRE_C.min(indic_position(&info[i]));
        set_indic_position(&mut info[i], p);
    }

    if base < end {
        set_indic_position(&mut info[base as usize], POS_BASE_C);
    }

    /* Handle beginning Ra */
    if has_reph {
        set_indic_position(&mut info[start_u], POS_RA_TO_BECOME_REPH);
    }

    /* For old-style Indic script tags, move the first post-base Halant after
     * last consonant.
     *
     * Reports suggest that in some scripts Uniscribe does this only if there
     * is *not* a Halant after last consonant already.  We know that is the
     * case for Kannada, while it reorders unconditionally in other scripts,
     * eg. Malayalam, Bengali, and Devanagari.  We don't currently know about
     * other scripts, so we block Kannada.
     *
     * Kannada test case:
     * U+0C9A,U+0CCD,U+0C9A,U+0CCD
     * With some versions of Lohit Kannada.
     * https://bugs.freedesktop.org/show_bug.cgi?id=59118
     *
     * Malayalam test case:
     * U+0D38,U+0D4D,U+0D31,U+0D4D,U+0D31,U+0D4D
     * With lohit-ttf-20121122/Lohit-Malayalam.ttf
     *
     * Bengali test case:
     * U+0998,U+09CD,U+09AF,U+09CD
     * With Windows XP vrinda.ttf
     * https://github.com/harfbuzz/harfbuzz/issues/1073
     *
     * Devanagari test case:
     * U+091F,U+094D,U+0930,U+094D
     * With chandas.ttf
     * https://github.com/harfbuzz/harfbuzz/issues/1071
     */
    if indic_plan.is_old_spec {
        let disallow_double_halants = buffer.props.script == HB_SCRIPT_KANNADA;
        for i in base as usize + 1..end_u {
            if indic_category(&info[i]) == I_H {
                let mut j = end_u - 1;
                while j > i {
                    if is_consonant(&info[j])
                        || (disallow_double_halants && indic_category(&info[j]) == I_H)
                    {
                        break;
                    }
                    j -= 1;
                }
                if indic_category(&info[j]) != I_H && j > i {
                    /* Move Halant to after last consonant. */
                    let t = info[i];
                    info.copy_within(i + 1..j + 1, i);
                    info[j] = t;
                }
                break;
            }
        }
    }

    /* Attach misc marks to previous char to move with them. */
    {
        let mut last_pos = POS_START;
        for i in start_u..end_u {
            if flag_unsafe(indic_category(&info[i]) as u32)
                & (JOINER_FLAGS
                    | flag(I_N as u32)
                    | flag(I_RS as u32)
                    | flag(I_CM as u32)
                    | flag(I_H as u32))
                != 0
            {
                set_indic_position(&mut info[i], last_pos);
                if indic_category(&info[i]) == I_H && indic_position(&info[i]) == POS_PRE_M {
                    /*
                     * Uniscribe doesn't move the Halant with Left Matra.
                     * TEST: U+092B,U+093F,U+094D
                     * We follow.
                     */
                    let mut j = i;
                    while j > start_u {
                        if indic_position(&info[j - 1]) != POS_PRE_M {
                            let p = indic_position(&info[j - 1]);
                            set_indic_position(&mut info[i], p);
                            break;
                        }
                        j -= 1;
                    }
                }
            } else if indic_position(&info[i]) != POS_SMVD {
                if indic_category(&info[i]) == I_MPST
                    && i > start_u
                    && indic_category(&info[i - 1]) == I_SM
                {
                    let p = indic_position(&info[i]);
                    set_indic_position(&mut info[i - 1], p);
                }
                last_pos = indic_position(&info[i]);
            }
        }
    }
    /* For post-base consonants let them own anything before them
     * since the last consonant or matra. */
    {
        let mut last = base as usize;
        for i in base as usize + 1..end_u {
            if is_consonant(&info[i]) {
                for j in last + 1..i {
                    if indic_position(&info[j]) < POS_SMVD {
                        let p = indic_position(&info[i]);
                        set_indic_position(&mut info[j], p);
                    }
                }
                last = i;
            } else if flag_unsafe(indic_category(&info[i]) as u32)
                & (flag(I_M as u32) | flag(I_MPST as u32))
                != 0
            {
                last = i;
            }
        }
    }

    {
        /* Use syllable() for sort accounting temporarily. */
        let syllable = buffer.info[start_u].syllable();
        for i in start_u..end_u {
            buffer.info[i].set_syllable((i - start_u) as u8);
        }

        /* Sit tight, rock 'n roll! */
        buffer.info[start_u..end_u].sort_by(compare_indic_order);

        /* Find base again; also flip left-matra sequence. */
        let mut first_left_matra = end;
        let mut last_left_matra = end;
        base = end;
        for i in start..end {
            let p = indic_position(&buffer.info[i as usize]);
            if p == POS_BASE_C {
                base = i;
                break;
            } else if p == POS_PRE_M {
                if first_left_matra == end {
                    first_left_matra = i;
                }
                last_left_matra = i;
            }
        }
        /* https://github.com/harfbuzz/harfbuzz/issues/3863 */
        if first_left_matra < last_left_matra {
            /* No need to merge clusters, handled later. */
            buffer.reverse_range(first_left_matra, last_left_matra + 1);
            /* Reverse back nuktas, etc. */
            let mut i = first_left_matra;
            let mut j = i;
            while j <= last_left_matra {
                if flag_unsafe(indic_category(&buffer.info[j as usize]) as u32)
                    & (flag(I_M as u32) | flag(I_MPST as u32))
                    != 0
                {
                    buffer.reverse_range(i, j + 1);
                    i = j + 1;
                }
                j += 1;
            }
        }

        /* Things are out-of-control for post base positions, they may shuffle
         * around like crazy.  In old-spec mode, we move halants around, so in
         * that case merge all clusters after base.  Otherwise, check the sort
         * order and merge as needed.
         * For pre-base stuff, we handle cluster issues in final reordering.
         *
         * We could use buffer->sort() for this, if there was no special
         * reordering of pre-base stuff happening later...
         * We don't want to merge_clusters all of that, which buffer->sort()
         * would.  Here's a concrete example:
         *
         * Assume there's a pre-base consonant and explicit Halant before base,
         * followed by a prebase-reordering (left) Matra:
         *
         *   C,H,ZWNJ,B,M
         *
         * At this point in reordering we would have:
         *
         *   M,C,H,ZWNJ,B
         *
         * whereas in final reordering we will bring the Matra closer to Base:
         *
         *   C,H,ZWNJ,M,B
         *
         * That's why we don't want to merge-clusters anything before the Base
         * at this point.  But if something moved from after Base to before it,
         * we should merge clusters from base to them.  In final-reordering, we
         * only move things around before base, and merge-clusters up to base.
         * These two merge-clusters from the two sides of base will interlock
         * to merge things correctly.  See:
         * https://github.com/harfbuzz/harfbuzz/issues/2272
         */
        if indic_plan.is_old_spec || end - start > 127 {
            buffer.merge_clusters(base, end);
        } else {
            /* Note!  syllable() is a one-byte field. */
            for i in base..end {
                if buffer.info[i as usize].syllable() != 255 {
                    let mut min = i;
                    let mut max = i;
                    let mut j = start + buffer.info[i as usize].syllable() as u32;
                    while j != i {
                        min = min.min(j);
                        max = max.max(j);
                        let next = start + buffer.info[j as usize].syllable() as u32;
                        buffer.info[j as usize].set_syllable(255); /* So we don't process j later again. */
                        j = next;
                    }
                    buffer.merge_clusters(base.max(min), max + 1);
                }
            }
        }

        /* Put syllable back in. */
        for i in start_u..end_u {
            buffer.info[i].set_syllable(syllable);
        }
    }

    let info = &mut buffer.info;

    /* Setup masks now */

    {
        /* Reph */
        let mut i = start_u;
        while i < end_u && indic_position(&info[i]) == POS_RA_TO_BECOME_REPH {
            info[i].mask |= indic_plan.mask_array[INDIC_RPHF];
            i += 1;
        }

        /* Pre-base */
        let mut mask = indic_plan.mask_array[INDIC_HALF];
        if !indic_plan.is_old_spec && indic_plan.config.blwf_mode == BlwfMode::PreAndPost {
            mask |= indic_plan.mask_array[INDIC_BLWF];
        }
        for inf in &mut info[start_u..base as usize] {
            inf.mask |= mask;
        }
        /* Base */
        mask = 0;
        if base < end {
            info[base as usize].mask |= mask;
        }
        /* Post-base */
        mask = indic_plan.mask_array[INDIC_BLWF]
            | indic_plan.mask_array[INDIC_ABVF]
            | indic_plan.mask_array[INDIC_PSTF];
        for inf in &mut info[(base as usize + 1).min(end_u)..end_u] {
            inf.mask |= mask;
        }
    }

    if indic_plan.is_old_spec && buffer.props.script == HB_SCRIPT_DEVANAGARI {
        /* Old-spec eye-lash Ra needs special handling.  From the
         * spec:
         *
         * "The feature 'below-base form' is applied to consonants
         * having below-base forms and following the base consonant.
         * The exception is vattu, which may appear below half forms
         * as well as below the base glyph. The feature 'below-base
         * form' will be applied to all such occurrences of Ra as well."
         *
         * Test case: U+0924,U+094D,U+0930,U+094d,U+0915
         * with Sanskrit 2003 font.
         *
         * However, note that Ra,Halant,ZWJ is the correct way to
         * request eyelash form of Ra, so we wouldbn't inhibit it
         * in that sequence.
         *
         * Test case: U+0924,U+094D,U+0930,U+094d,U+200D,U+0915
         */
        let mut i = start;
        while i + 1 < base {
            let iu = i as usize;
            if indic_category(&info[iu]) == I_RA
                && indic_category(&info[iu + 1]) == I_H
                && (i + 2 == base || indic_category(&info[iu + 2]) != I_ZWJ)
            {
                info[iu].mask |= indic_plan.mask_array[INDIC_BLWF];
                info[iu + 1].mask |= indic_plan.mask_array[INDIC_BLWF];
            }
            i += 1;
        }
    }

    let pref_len = 2;
    if indic_plan.mask_array[INDIC_PREF] != 0 && base + pref_len < end {
        /* Find a Halant,Ra sequence and mark it for pre-base-reordering processing. */
        let mut i = base + 1;
        while i + pref_len - 1 < end {
            let glyphs = [info[i as usize].codepoint, info[i as usize + 1].codepoint];
            if indic_plan.pref.would_substitute(&glyphs, face) {
                for _ in 0..pref_len {
                    info[i as usize].mask |= indic_plan.mask_array[INDIC_PREF];
                    i += 1;
                }
                break;
            }
            i += 1;
        }
    }

    /* Apply ZWJ/ZWNJ effects */
    for i in start_u + 1..end_u {
        if is_joiner(&info[i]) {
            let non_joiner = indic_category(&info[i]) == I_ZWNJ;
            let mut j = i;

            loop {
                j -= 1;

                /* ZWJ/ZWNJ should disable CJCT.  They do that by simply
                 * being there, since we don't skip them for the CJCT
                 * feature (ie. F_MANUAL_ZWJ) */

                /* A ZWNJ disables HALF. */
                if non_joiner {
                    info[j].mask &= !indic_plan.mask_array[INDIC_HALF];
                }

                if !(j > start_u && !is_consonant(&info[j])) {
                    break;
                }
            }
        }
    }
}

/// `initial_reordering_standalone_cluster`
fn initial_reordering_standalone_cluster(
    plan: &HbOtShapePlan,
    face: &HbFace,
    buffer: &mut HbBuffer,
    start: u32,
    end: u32,
) {
    /* We treat placeholder/dotted-circle as if they are consonants, so we
     * should just chain.  Only if not in compatibility mode that is... */

    let indic_plan = indic_plan(plan);
    if indic_plan.uniscribe_bug_compatible {
        /* For dotted-circle, this is what Uniscribe does:
         * If dotted-circle is the last glyph, it just does nothing.
         * Ie. It doesn't form Reph. */
        if indic_category(&buffer.info[end as usize - 1]) == I_DOTTEDCIRCLE {
            return;
        }
    }

    initial_reordering_consonant_syllable(plan, face, buffer, start, end);
}

/// `initial_reordering_syllable_indic`
fn initial_reordering_syllable_indic(
    plan: &HbOtShapePlan,
    face: &HbFace,
    buffer: &mut HbBuffer,
    start: u32,
    end: u32,
) {
    let syllable_type = buffer.info[start as usize].syllable() & 0x0F;
    match syllable_type {
        /* We made the vowels look like consonants.  So let's call the consonant logic! */
        INDIC_VOWEL_SYLLABLE | INDIC_CONSONANT_SYLLABLE => {
            initial_reordering_consonant_syllable(plan, face, buffer, start, end);
        }

        /* We already inserted dotted-circles, so just call the standalone_cluster. */
        INDIC_BROKEN_CLUSTER | INDIC_STANDALONE_CLUSTER => {
            initial_reordering_standalone_cluster(plan, face, buffer, start, end);
        }

        INDIC_SYMBOL_CLUSTER | INDIC_NON_INDIC_CLUSTER => {}
        _ => {}
    }
}

/// `initial_reordering_indic`
fn initial_reordering_indic(
    plan: &HbOtShapePlan,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
) -> bool {
    let mut ret = false;

    update_consonant_positions_indic(plan, font, buffer);
    if hb_syllabic_insert_dotted_circles(
        font,
        buffer,
        INDIC_BROKEN_CLUSTER as u32,
        I_DOTTEDCIRCLE as u32,
        I_REPHA as i32,
        POS_END as i32,
    ) {
        ret = true;
    }

    let face = font.p.face.clone();
    for (start, end) in foreach_syllable(buffer) {
        initial_reordering_syllable_indic(plan, &face, buffer, start, end);
    }

    ret
}

/// `final_reordering_syllable_indic`
fn final_reordering_syllable_indic(
    plan: &HbOtShapePlan,
    buffer: &mut HbBuffer,
    start: u32,
    end: u32,
) {
    let indic_plan = indic_plan(plan);
    let (start_u, end_u) = (start as usize, end as usize);

    /* This function relies heavily on halant glyphs.  Lots of ligation
     * and possibly multiple substitutions happened prior to this
     * phase, and that might have messed up our properties.  Recover
     * from a particular case of that where we're fairly sure that a
     * class of I_Cat(H) is desired but has been lost. */
    /* We don't call load_virama_glyph(), since we know it's already
     * loaded. */
    let virama_glyph = indic_plan.virama_glyph.load(Ordering::Relaxed);
    if virama_glyph != 0 {
        for i in start_u..end_u {
            let info = &mut buffer.info[i];
            if info.codepoint == virama_glyph
                && _hb_glyph_info_ligated(info)
                && _hb_glyph_info_multiplied(info)
            {
                /* This will make sure that this glyph passes is_halant() test. */
                set_indic_category(info, I_H);
                _hb_glyph_info_clear_ligated_and_multiplied(info);
            }
        }
    }

    /* 4. Final reordering:
     *
     * After the localized forms and basic shaping forms GSUB features have been
     * applied (see below), the shaping engine performs some final glyph
     * reordering before applying all the remaining font features to the entire
     * syllable.
     */

    let mut try_pref = indic_plan.mask_array[INDIC_PREF] != 0;

    /* Find base again */
    let mut base = start;
    {
        let info = &mut buffer.info;
        while base < end {
            if indic_position(&info[base as usize]) >= POS_BASE_C {
                if try_pref && base + 1 < end {
                    for i in base + 1..end {
                        if (info[i as usize].mask & indic_plan.mask_array[INDIC_PREF]) != 0 {
                            if !(_hb_glyph_info_substituted(&info[i as usize])
                                && _hb_glyph_info_ligated_and_didnt_multiply(&info[i as usize]))
                            {
                                /* Ok, this was a 'pref' candidate but didn't form any.
                                 * Base is around here... */
                                base = i;
                                while base < end && is_halant(&info[base as usize]) {
                                    base += 1;
                                }
                                if base < end {
                                    set_indic_position(&mut info[base as usize], POS_BASE_C);
                                }

                                try_pref = false;
                            }
                            break;
                        }
                    }
                    if base == end {
                        break;
                    }
                }
                /* For Malayalam, skip over unformed below- (but NOT post-) forms. */
                if buffer.props.script == HB_SCRIPT_MALAYALAM {
                    let mut i = base + 1;
                    while i < end {
                        while i < end && is_joiner(&info[i as usize]) {
                            i += 1;
                        }
                        if i == end || !is_halant(&info[i as usize]) {
                            break;
                        }
                        i += 1; /* Skip halant. */
                        while i < end && is_joiner(&info[i as usize]) {
                            i += 1;
                        }
                        if i < end
                            && is_consonant(&info[i as usize])
                            && indic_position(&info[i as usize]) == POS_BELOW_C
                        {
                            base = i;
                            set_indic_position(&mut info[base as usize], POS_BASE_C);
                        }
                        i += 1;
                    }
                }

                if start < base && indic_position(&info[base as usize]) > POS_BASE_C {
                    base -= 1;
                }
                break;
            }
            base += 1;
        }
        if base == end && start < base && is_one_of(&info[base as usize - 1], flag(I_ZWJ as u32)) {
            base -= 1;
        }
        if base < end {
            while start < base
                && is_one_of(&info[base as usize], flag(I_N as u32) | flag(I_H as u32))
            {
                base -= 1;
            }
        }
    }

    /*   o Reorder matras:
     *
     *     If a pre-base matra character had been reordered before applying basic
     *     features, the glyph can be moved closer to the main consonant based on
     *     whether half-forms had been formed. Actual position for the matra is
     *     defined as “after last standalone halant glyph, after initial matra
     *     position and before the main consonant”. If ZWJ or ZWNJ follow this
     *     halant, position is moved after it.
     *
     * IMPLEMENTATION NOTES:
     *
     * It looks like the last sentence is wrong.  Testing, with Windows 7 Uniscribe
     * and Devanagari shows that the behavior is best described as:
     *
     * "If ZWJ follows this halant, matra is NOT repositioned after this halant.
     *  If ZWNJ follows this halant, position is moved after it."
     *
     * Test case, with Adobe Devanagari or Nirmala UI:
     *
     *   U+091F,U+094D,U+200C,U+092F,U+093F
     *   (Matra moves to the middle, after ZWNJ.)
     *
     *   U+091F,U+094D,U+200D,U+092F,U+093F
     *   (Matra does NOT move, stays to the left.)
     *
     * https://github.com/harfbuzz/harfbuzz/issues/1070
     */

    if start + 1 < end && start < base
    /* Otherwise there can't be any pre-base matra characters. */
    {
        /* If we lost track of base, alas, position before last thingy. */
        let mut new_pos = if base == end { base - 2 } else { base - 1 };

        /* Malayalam / Tamil do not have "half" forms or explicit virama forms.
         * The glyphs formed by 'half' are Chillus or ligated explicit viramas.
         * We want to position matra after them.
         */
        if buffer.props.script != HB_SCRIPT_MALAYALAM && buffer.props.script != HB_SCRIPT_TAMIL {
            let info = &buffer.info;
            'search: loop {
                while new_pos > start
                    && !is_one_of(
                        &info[new_pos as usize],
                        flag(I_M as u32) | flag(I_MPST as u32) | flag(I_H as u32),
                    )
                {
                    new_pos -= 1;
                }

                /* If we found no Halant we are done.
                 * Otherwise only proceed if the Halant does
                 * not belong to the Matra itself! */
                if is_halant(&info[new_pos as usize])
                    && indic_position(&info[new_pos as usize]) != POS_PRE_M
                {
                    if new_pos + 1 < end {
                        /* -> If ZWJ follows this halant, matra is NOT repositioned after this halant. */
                        if indic_category(&info[new_pos as usize + 1]) == I_ZWJ {
                            /* Keep searching. */
                            if new_pos > start {
                                new_pos -= 1;
                                continue 'search;
                            }
                        }

                        /* -> If ZWNJ follows this halant, position is moved after it.
                         *
                         * IMPLEMENTATION NOTES:
                         *
                         * This is taken care of by the state-machine. A Halant,ZWNJ is a terminating
                         * sequence for a consonant syllable; any pre-base matras occurring after it
                         * will belong to the subsequent syllable.
                         */
                    }
                } else {
                    new_pos = start; /* No move. */
                }
                break;
            }
        }

        if start < new_pos && indic_position(&buffer.info[new_pos as usize]) != POS_PRE_M {
            /* Now go see if there's actually any matras... */
            let mut i = new_pos;
            while i > start {
                if indic_position(&buffer.info[i as usize - 1]) == POS_PRE_M {
                    let old_pos = i - 1;
                    if old_pos < base && base <= new_pos {
                        /* Shouldn't actually happen. */
                        base -= 1;
                    }

                    let (o, n) = (old_pos as usize, new_pos as usize);
                    let tmp = buffer.info[o];
                    buffer.info.copy_within(o + 1..n + 1, o);
                    buffer.info[n] = tmp;

                    /* Note: this merge_clusters() is intentionally *after* the reordering.
                     * Indic matra reordering is special and tricky... */
                    buffer.merge_clusters(new_pos, end.min(base + 1));

                    new_pos -= 1;
                }
                i -= 1;
            }
        } else {
            for i in start..base {
                if indic_position(&buffer.info[i as usize]) == POS_PRE_M {
                    buffer.merge_clusters(i, end.min(base + 1));
                    break;
                }
            }
        }
    }

    /*   o Reorder reph:
     *
     *     Reph’s original position is always at the beginning of the syllable,
     *     (i.e. it is not reordered at the character reordering stage). However,
     *     it will be reordered according to the basic-forms shaping results.
     *     Possible positions for reph, depending on the script, are; after main,
     *     before post-base consonant forms, and after post-base consonant forms.
     */

    /* Two cases:
     *
     * - If repha is encoded as a sequence of characters (Ra,H or Ra,H,ZWJ), then
     *   we should only move it if the sequence ligated to the repha form.
     *
     * - If repha is encoded separately and in the logical position, we should only
     *   move it if it did NOT ligate.  If it ligated, it's probably the font trying
     *   to make it work without the reordering.
     */
    if start + 1 < end
        && indic_position(&buffer.info[start_u]) == POS_RA_TO_BECOME_REPH
        && ((indic_category(&buffer.info[start_u]) == I_REPHA)
            ^ _hb_glyph_info_ligated_and_didnt_multiply(&buffer.info[start_u]))
    {
        let reph_pos = indic_plan.config.reph_pos;
        let info = &buffer.info;

        /* Step 2 / 5: "after the first explicit halant glyph between the first
         * post-reph consonant and last main consonant" */
        let halant_pos = || -> Option<u32> {
            let mut new_reph_pos = start + 1;
            while new_reph_pos < base && !is_halant(&info[new_reph_pos as usize]) {
                new_reph_pos += 1;
            }

            if new_reph_pos < base && is_halant(&info[new_reph_pos as usize]) {
                /* ->If ZWJ or ZWNJ are following this halant, position is moved after it. */
                if new_reph_pos + 1 < base && is_joiner(&info[new_reph_pos as usize + 1]) {
                    new_reph_pos += 1;
                }
                return Some(new_reph_pos);
            }
            None
        };

        let new_reph_pos: u32 = 'reph_move: {
            /*       1. If reph should be positioned after post-base consonant forms,
             *          proceed to step 5.
             */
            if reph_pos != REPH_POS_AFTER_POST {
                /*       2. If the reph repositioning class is not after post-base: target
                 *          position is after the first explicit halant glyph between the
                 *          first post-reph consonant and last main consonant. If ZWJ or ZWNJ
                 *          are following this halant, position is moved after it. If such
                 *          position is found, this is the target position. Otherwise,
                 *          proceed to the next step.
                 *
                 *          Note: in old-implementation fonts, where classifications were
                 *          fixed in shaping engine, there was no case where reph position
                 *          will be found on this step.
                 */
                if let Some(p) = halant_pos() {
                    break 'reph_move p;
                }

                /*       3. If reph should be repositioned after the main consonant: find the
                 *          first consonant not ligated with main, or find the first
                 *          consonant that is not a potential pre-base-reordering Ra.
                 */
                if reph_pos == REPH_POS_AFTER_MAIN {
                    let mut new_reph_pos = base;
                    while new_reph_pos + 1 < end
                        && indic_position(&info[new_reph_pos as usize + 1]) <= POS_AFTER_MAIN
                    {
                        new_reph_pos += 1;
                    }
                    if new_reph_pos < end {
                        break 'reph_move new_reph_pos;
                    }
                }

                /*       4. If reph should be positioned before post-base consonant, find
                 *          first post-base classified consonant not ligated with main. If no
                 *          consonant is found, the target position should be before the
                 *          first matra, syllable modifier sign or vedic sign.
                 */
                /* This is our take on what step 4 is trying to say (and failing, BADLY). */
                if reph_pos == REPH_POS_AFTER_SUB {
                    let mut new_reph_pos = base;
                    while new_reph_pos + 1 < end
                        && flag_unsafe(indic_position(&info[new_reph_pos as usize + 1]) as u32)
                            & (flag(POS_POST_C as u32)
                                | flag(POS_AFTER_POST as u32)
                                | flag(POS_SMVD as u32))
                            == 0
                    {
                        new_reph_pos += 1;
                    }
                    if new_reph_pos < end {
                        break 'reph_move new_reph_pos;
                    }
                }
            }

            /*       5. If no consonant is found in steps 3 or 4, move reph to a position
             *          immediately before the first post-base matra, syllable modifier
             *          sign or vedic sign that has a reordering class after the intended
             *          reph position. For example, if the reordering position for reph
             *          is post-main, it will skip above-base matras that also have a
             *          post-main position.
             */
            /* reph_step_5: */
            {
                /* Copied from step 2. */
                if let Some(p) = halant_pos() {
                    break 'reph_move p;
                }
            }
            /* See https://github.com/harfbuzz/harfbuzz/issues/2298#issuecomment-615318654 */

            /*       6. Otherwise, reorder reph to the end of the syllable.
             */
            {
                let mut new_reph_pos = end - 1;
                while new_reph_pos > start
                    && indic_position(&info[new_reph_pos as usize]) == POS_SMVD
                {
                    new_reph_pos -= 1;
                }

                /*
                 * If the Reph is to be ending up after a Matra,Halant sequence,
                 * position it before that Halant so it can interact with the Matra.
                 * However, if it's a plain Consonant,Halant we shouldn't do that.
                 * Uniscribe doesn't do this.
                 * TEST: U+0930,U+094D,U+0915,U+094B,U+094D
                 */
                if !indic_plan.uniscribe_bug_compatible && is_halant(&info[new_reph_pos as usize]) {
                    /* (C re-reads the bound each iteration) */
                    let mut i = base + 1;
                    while i < new_reph_pos {
                        if flag_unsafe(indic_category(&info[i as usize]) as u32)
                            & (flag(I_M as u32) | flag(I_MPST as u32))
                            != 0
                        {
                            /* Ok, got it. */
                            new_reph_pos -= 1;
                        }
                        i += 1;
                    }
                }

                break 'reph_move new_reph_pos;
            }
        };

        /* reph_move: */
        {
            /* Move */
            buffer.merge_clusters(start, new_reph_pos + 1);
            let n = new_reph_pos as usize;
            let reph = buffer.info[start_u];
            buffer.info.copy_within(start_u + 1..n + 1, start_u);
            buffer.info[n] = reph;

            if start < base && base <= new_reph_pos {
                base -= 1;
            }
        }
    }

    /*   o Reorder pre-base-reordering consonants:
     *
     *     If a pre-base-reordering consonant is found, reorder it according to
     *     the following rules:
     */

    if try_pref && base + 1 < end
    /* Otherwise there can't be any pre-base-reordering Ra. */
    {
        for i in base + 1..end {
            if (buffer.info[i as usize].mask & indic_plan.mask_array[INDIC_PREF]) != 0 {
                /*       1. Only reorder a glyph produced by substitution during application
                 *          of the <pref> feature. (Note that a font may shape a Ra consonant with
                 *          the feature generally but block it in certain contexts.)
                 */
                /* Note: We just check that something got substituted.  We don't check that
                 * the <pref> feature actually did it...
                 *
                 * Reorder pref only if it ligated. */
                if _hb_glyph_info_ligated_and_didnt_multiply(&buffer.info[i as usize]) {
                    /*
                     *       2. Try to find a target position the same way as for pre-base matra.
                     *          If it is found, reorder pre-base consonant glyph.
                     *
                     *       3. If position is not found, reorder immediately before main
                     *          consonant.
                     */

                    let mut new_pos = base;
                    /* Malayalam / Tamil do not have "half" forms or explicit virama forms.
                     * The glyphs formed by 'half' are Chillus or ligated explicit viramas.
                     * We want to position matra after them.
                     */
                    if buffer.props.script != HB_SCRIPT_MALAYALAM
                        && buffer.props.script != HB_SCRIPT_TAMIL
                    {
                        while new_pos > start
                            && !is_one_of(
                                &buffer.info[new_pos as usize - 1],
                                flag(I_M as u32) | flag(I_MPST as u32) | flag(I_H as u32),
                            )
                        {
                            new_pos -= 1;
                        }
                    }

                    if new_pos > start && is_halant(&buffer.info[new_pos as usize - 1]) {
                        /* -> If ZWJ or ZWNJ follow this halant, position is moved after it. */
                        if new_pos < end && is_joiner(&buffer.info[new_pos as usize]) {
                            new_pos += 1;
                        }
                    }

                    {
                        let old_pos = i;

                        buffer.merge_clusters(new_pos, old_pos + 1);
                        let (o, n) = (old_pos as usize, new_pos as usize);
                        let tmp = buffer.info[o];
                        buffer.info.copy_within(n..o, n + 1);
                        buffer.info[n] = tmp;

                        if new_pos <= base && base < old_pos {
                            base += 1;
                        }
                    }
                }

                break;
            }
        }
    }

    /* Apply 'init' to the Left Matra if it's a word start. */
    if indic_position(&buffer.info[start_u]) == POS_PRE_M {
        if start == 0
            || flag_unsafe(_hb_glyph_info_get_general_category(
                &buffer.info[start_u - 1],
            )) & flag_range(
                HB_UNICODE_GENERAL_CATEGORY_FORMAT,
                HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK,
            ) == 0
        {
            buffer.info[start_u].mask |= indic_plan.mask_array[INDIC_INIT];
        } else {
            buffer.unsafe_to_break(start - 1, start + 1);
        }
    }

    /*
     * Finish off the clusters and go home!
     */
    if indic_plan.uniscribe_bug_compatible {
        match plan.props.script {
            HB_SCRIPT_TAMIL => {}

            _ => {
                /* Uniscribe merges the entire syllable into a single cluster... Except for Tamil.
                 * This means, half forms are submerged into the main consonant's cluster.
                 * This is unnecessary, and makes cursor positioning harder, but that's what
                 * Uniscribe does. */
                buffer.merge_clusters(start, end);
            }
        }
    }
}

/// `final_reordering_indic`
fn final_reordering_indic(plan: &HbOtShapePlan, _font: &mut HbFont, buffer: &mut HbBuffer) -> bool {
    let count = buffer.len;
    if count == 0 {
        return false;
    }

    for (start, end) in foreach_syllable(buffer) {
        final_reordering_syllable_indic(plan, buffer, start, end);
    }

    buffer.deallocate_var(VAR_COMPLEX_CATEGORY.0, VAR_COMPLEX_CATEGORY.1);
    buffer.deallocate_var(VAR_COMPLEX_AUXILIARY.0, VAR_COMPLEX_AUXILIARY.1);

    false
}

/// `preprocess_text_indic`
fn preprocess_text_indic(plan: &HbOtShapePlan, buffer: &mut HbBuffer, font: &mut HbFont) {
    let indic_plan = indic_plan(plan);
    if !indic_plan.uniscribe_bug_compatible {
        _hb_preprocess_text_vowel_constraints(plan, buffer, font);
    }
}

/// `decompose_indic`
fn decompose_indic(
    c: &HbOtShapeNormalizeContext,
    _font: &mut HbFont,
    ab: HbCodepoint,
    a: &mut HbCodepoint,
    b: &mut HbCodepoint,
) -> bool {
    match ab {
        /* Don't decompose these. */
        0x0931 => return false, /* DEVANAGARI LETTER RRA */
        // https://github.com/harfbuzz/harfbuzz/issues/779
        0x09DC => return false, /* BENGALI LETTER RRA */
        0x09DD => return false, /* BENGALI LETTER RHA */
        0x0B94 => return false, /* TAMIL LETTER AU */
        _ => {}
    }

    c.unicode.decompose(ab, a, b)
}

/// `compose_indic`
fn compose_indic(
    c: &HbOtShapeNormalizeContext,
    a: HbCodepoint,
    b: HbCodepoint,
    ab: &mut HbCodepoint,
) -> bool {
    /* Avoid recomposing split matras. */
    if hb_unicode_general_category_is_mark(c.unicode.general_category(a)) {
        return false;
    }

    /* Composition-exclusion exceptions that we want to recompose. */
    if a == 0x09AF && b == 0x09BC {
        *ab = 0x09DF;
        return true;
    }

    c.unicode.compose(a, b, ab)
}

/// `_hb_ot_shaper_indic`
pub(crate) static _hb_ot_shaper_indic: HbOtShaper = HbOtShaper {
    collect_features: Some(collect_features_indic),
    override_features: Some(override_features_indic),
    data_create: Some(data_create_indic),
    preprocess_text: Some(preprocess_text_indic),
    postprocess_glyphs: None,
    decompose: Some(decompose_indic),
    compose: Some(compose_indic),
    setup_masks: Some(setup_masks_indic),
    reorder_marks: None,
    gpos_tag: HB_TAG_NONE,
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_COMPOSED_DIACRITICS_NO_SHORT_CIRCUIT,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_NONE,
    fallback_position: false,
};

#[allow(dead_code)]
const _UNUSED: [u8; 4] = [I_X, REPH_POS_BEFORE_SUB, REPH_POS_BEFORE_POST, I_SM];
