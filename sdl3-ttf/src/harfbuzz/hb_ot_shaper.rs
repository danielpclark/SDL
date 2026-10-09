// Rust translation of src/hb-ot-shaper.hh and src/hb-ot-shaper-default.cc
// from HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2010,2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod

//! The OpenType shapers: their interface, the choice of the shaper of a
//! script, and the default shapers.

use super::hb_buffer::HbBuffer;
use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::HB_OT_TAG_DEFAULT_SCRIPT;
use super::hb_ot_shape::{HbOtShapePlan, HbOtShapePlanner};
use super::hb_ot_shape_normalize::*;

pub(crate) const HB_OT_SHAPE_MAX_COMBINING_MARKS: u32 = 32;

/// `hb_ot_shape_zero_width_marks_type_t`
pub(crate) type HbOtShapeZeroWidthMarksType = u32;
pub(crate) const HB_OT_SHAPE_ZERO_WIDTH_MARKS_NONE: HbOtShapeZeroWidthMarksType = 0;
pub(crate) const HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_EARLY: HbOtShapeZeroWidthMarksType = 1;
pub(crate) const HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_LATE: HbOtShapeZeroWidthMarksType = 2;

/// The data a shaper's `data_create` makes (C's `plan->data`).
#[derive(Debug)]
pub(crate) enum ShaperData {}

/// `decompose ()`: called during shape()'s normalization.
pub(crate) type DecomposeFunc = fn(
    c: &HbOtShapeNormalizeContext,
    font: &mut HbFont,
    ab: HbCodepoint,
    a: &mut HbCodepoint,
    b: &mut HbCodepoint,
) -> bool;
/// `compose ()`: called during shape()'s normalization.
pub(crate) type ComposeFunc =
    fn(c: &HbOtShapeNormalizeContext, a: HbCodepoint, b: HbCodepoint, ab: &mut HbCodepoint) -> bool;

/// `hb_ot_shaper_t`
#[derive(Debug)]
pub(crate) struct HbOtShaper {
    /* collect_features()
     * Called during shape_plan().
     * Shapers should use plan->map to add their features and callbacks.
     * May be NULL.
     */
    pub(crate) collect_features: Option<fn(&mut HbOtShapePlanner)>,

    /* override_features()
     * Called during shape_plan().
     * Shapers should use plan->map to override features and add callbacks after
     * common features are added.
     * May be NULL.
     */
    pub(crate) override_features: Option<fn(&mut HbOtShapePlanner)>,

    /* data_create()
     * Called at the end of shape_plan().
     * Whatever shapers return will be accessible through plan->data later.
     * If nullptr is returned, means a plan failure.
     */
    pub(crate) data_create: Option<fn(&HbOtShapePlan) -> Option<ShaperData>>,

    /* preprocess_text()
     * Called during shape().
     * Shapers can use to modify text before shaping starts.
     * May be NULL.
     */
    pub(crate) preprocess_text: Option<fn(&HbOtShapePlan, &mut HbBuffer, &mut HbFont)>,

    /* postprocess_glyphs()
     * Called during shape().
     * Shapers can use to modify glyphs after shaping ends.
     * May be NULL.
     */
    pub(crate) postprocess_glyphs: Option<fn(&HbOtShapePlan, &mut HbBuffer, &mut HbFont)>,

    /* decompose()
     * Called during shape()'s normalization.
     * May be NULL.
     */
    pub(crate) decompose: Option<DecomposeFunc>,

    /* compose()
     * Called during shape()'s normalization.
     * May be NULL.
     */
    pub(crate) compose: Option<ComposeFunc>,

    /* setup_masks()
     * Called during shape().
     * Shapers should use map to get feature masks and set on buffer.
     * Shapers may NOT modify characters.
     * May be NULL.
     */
    pub(crate) setup_masks: Option<fn(&HbOtShapePlan, &mut HbBuffer, &mut HbFont)>,

    /* reorder_marks()
     * Called during shape().
     * Shapers can use to modify ordering of combining marks.
     * May be NULL.
     */
    pub(crate) reorder_marks: Option<fn(&HbOtShapePlan, &mut HbBuffer, u32, u32)>,

    /* gpos_tag()
     * If not HB_TAG_NONE, then must match found GPOS script tag for
     * GPOS to be applied.  Otherwise, fallback positioning will be used.
     */
    pub(crate) gpos_tag: HbTag,

    pub(crate) normalization_preference: HbOtShapeNormalizationMode,

    pub(crate) zero_width_marks: HbOtShapeZeroWidthMarksType,

    pub(crate) fallback_position: bool,
}

/// `_hb_ot_shaper_default`
pub(crate) static _hb_ot_shaper_default: HbOtShaper = HbOtShaper {
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
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_DEFAULT,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_LATE,
    fallback_position: true,
};

/* Same as default but no mark advance zeroing / fallback positioning.
 * Dumbest shaper ever, basically. */
/// `_hb_ot_shaper_dumber`
pub(crate) static _hb_ot_shaper_dumber: HbOtShaper = HbOtShaper {
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
    normalization_preference: HB_OT_SHAPE_NORMALIZATION_MODE_DEFAULT,
    zero_width_marks: HB_OT_SHAPE_ZERO_WIDTH_MARKS_NONE,
    fallback_position: false,
};

/// `HB_SCRIPT_MYANMAR_ZAWGYI`
pub(crate) const HB_SCRIPT_MYANMAR_ZAWGYI: HbScript = hb_tag(b'Q', b'a', b'a', b'g');

/// `hb_ot_shaper_categorize`
pub(crate) fn hb_ot_shaper_categorize(planner: &HbOtShapePlanner) -> &'static HbOtShaper {
    let chosen_script = planner.map.chosen_script[0];
    match planner.props.script {
        /* Unicode-1.1 additions */
        HB_SCRIPT_ARABIC |
        /* Unicode-3.0 additions */
        HB_SCRIPT_SYRIAC => {
            /* For Arabic script, use the Arabic shaper even if no OT script tag was found.
             * This is because we do fallback shaping for Arabic script (and not others).
             * But note that Arabic shaping is applicable only to horizontal layout; for
             * vertical text, just use the generic shaper instead. */
            if (chosen_script != HB_OT_TAG_DEFAULT_SCRIPT || planner.props.script == HB_SCRIPT_ARABIC)
                && hb_direction_is_horizontal(planner.props.direction)
            {
                &_hb_ot_shaper_default
            } else {
                &_hb_ot_shaper_default
            }
        }

        /* Unicode-1.1 additions */
        HB_SCRIPT_THAI | HB_SCRIPT_LAO => &_hb_ot_shaper_default,

        /* Unicode-1.1 additions */
        HB_SCRIPT_HANGUL => &_hb_ot_shaper_default,

        /* Unicode-1.1 additions */
        HB_SCRIPT_HEBREW => &_hb_ot_shaper_default,

        /* Unicode-1.1 additions */
        HB_SCRIPT_BENGALI
        | HB_SCRIPT_DEVANAGARI
        | HB_SCRIPT_GUJARATI
        | HB_SCRIPT_GURMUKHI
        | HB_SCRIPT_KANNADA
        | HB_SCRIPT_MALAYALAM
        | HB_SCRIPT_ORIYA
        | HB_SCRIPT_TAMIL
        | HB_SCRIPT_TELUGU => {
            /* If the designer designed the font for the 'DFLT' script,
             * (or we ended up arbitrarily pick 'latn'), use the default shaper.
             * Otherwise, use the specific shaper.
             *
             * If it's indy3 tag, send to USE. */
            if chosen_script == hb_tag(b'D', b'F', b'L', b'T') || chosen_script == hb_tag(b'l', b'a', b't', b'n') {
                &_hb_ot_shaper_default
            } else if (chosen_script & 0x000000FF) == b'3' as u32 {
                &_hb_ot_shaper_default
            } else {
                &_hb_ot_shaper_default
            }
        }

        HB_SCRIPT_KHMER => &_hb_ot_shaper_default,

        HB_SCRIPT_MYANMAR => {
            /* If the designer designed the font for the 'DFLT' script,
             * (or we ended up arbitrarily pick 'latn'), use the default shaper.
             * Otherwise, use the specific shaper.
             *
             * If designer designed for 'mymr' tag, also send to default
             * shaper.  That's tag used from before Myanmar shaping spec
             * was developed.  The shaping spec uses 'mym2' tag. */
            if chosen_script == hb_tag(b'D', b'F', b'L', b'T')
                || chosen_script == hb_tag(b'l', b'a', b't', b'n')
                || chosen_script == hb_tag(b'm', b'y', b'm', b'r')
            {
                &_hb_ot_shaper_default
            } else {
                &_hb_ot_shaper_default
            }
        }

        /* https://github.com/harfbuzz/harfbuzz/issues/1162 */
        HB_SCRIPT_MYANMAR_ZAWGYI => &_hb_ot_shaper_default,

        /* Unicode-2.0 additions */
        HB_SCRIPT_TIBETAN
        /* Unicode-3.0 additions */
        | HB_SCRIPT_MONGOLIAN
        | HB_SCRIPT_SINHALA
        /* Unicode-3.2 additions */
        | HB_SCRIPT_BUHID
        | HB_SCRIPT_HANUNOO
        | HB_SCRIPT_TAGALOG
        | HB_SCRIPT_TAGBANWA
        /* Unicode-4.0 additions */
        | HB_SCRIPT_LIMBU
        | HB_SCRIPT_TAI_LE
        /* Unicode-4.1 additions */
        | HB_SCRIPT_BUGINESE
        | HB_SCRIPT_KHAROSHTHI
        | HB_SCRIPT_SYLOTI_NAGRI
        | HB_SCRIPT_TIFINAGH
        /* Unicode-5.0 additions */
        | HB_SCRIPT_BALINESE
        | HB_SCRIPT_NKO
        | HB_SCRIPT_PHAGS_PA
        /* Unicode-5.1 additions */
        | HB_SCRIPT_CHAM
        | HB_SCRIPT_KAYAH_LI
        | HB_SCRIPT_LEPCHA
        | HB_SCRIPT_REJANG
        | HB_SCRIPT_SAURASHTRA
        | HB_SCRIPT_SUNDANESE
        /* Unicode-5.2 additions */
        | HB_SCRIPT_EGYPTIAN_HIEROGLYPHS
        | HB_SCRIPT_JAVANESE
        | HB_SCRIPT_KAITHI
        | HB_SCRIPT_MEETEI_MAYEK
        | HB_SCRIPT_TAI_THAM
        | HB_SCRIPT_TAI_VIET
        /* Unicode-6.0 additions */
        | HB_SCRIPT_BATAK
        | HB_SCRIPT_BRAHMI
        | HB_SCRIPT_MANDAIC
        /* Unicode-6.1 additions */
        | HB_SCRIPT_CHAKMA
        | HB_SCRIPT_MIAO
        | HB_SCRIPT_SHARADA
        | HB_SCRIPT_TAKRI
        /* Unicode-7.0 additions */
        | HB_SCRIPT_DUPLOYAN
        | HB_SCRIPT_GRANTHA
        | HB_SCRIPT_KHOJKI
        | HB_SCRIPT_KHUDAWADI
        | HB_SCRIPT_MAHAJANI
        | HB_SCRIPT_MANICHAEAN
        | HB_SCRIPT_MODI
        | HB_SCRIPT_PAHAWH_HMONG
        | HB_SCRIPT_PSALTER_PAHLAVI
        | HB_SCRIPT_SIDDHAM
        | HB_SCRIPT_TIRHUTA
        /* Unicode-8.0 additions */
        | HB_SCRIPT_AHOM
        | HB_SCRIPT_MULTANI
        /* Unicode-9.0 additions */
        | HB_SCRIPT_ADLAM
        | HB_SCRIPT_BHAIKSUKI
        | HB_SCRIPT_MARCHEN
        | HB_SCRIPT_NEWA
        /* Unicode-10.0 additions */
        | HB_SCRIPT_MASARAM_GONDI
        | HB_SCRIPT_SOYOMBO
        | HB_SCRIPT_ZANABAZAR_SQUARE
        /* Unicode-11.0 additions */
        | HB_SCRIPT_DOGRA
        | HB_SCRIPT_GUNJALA_GONDI
        | HB_SCRIPT_HANIFI_ROHINGYA
        | HB_SCRIPT_MAKASAR
        | HB_SCRIPT_MEDEFAIDRIN
        | HB_SCRIPT_OLD_SOGDIAN
        | HB_SCRIPT_SOGDIAN
        /* Unicode-12.0 additions */
        | HB_SCRIPT_ELYMAIC
        | HB_SCRIPT_NANDINAGARI
        | HB_SCRIPT_NYIAKENG_PUACHUE_HMONG
        | HB_SCRIPT_WANCHO
        /* Unicode-13.0 additions */
        | HB_SCRIPT_CHORASMIAN
        | HB_SCRIPT_DIVES_AKURU
        | HB_SCRIPT_KHITAN_SMALL_SCRIPT
        | HB_SCRIPT_YEZIDI
        /* Unicode-14.0 additions */
        | HB_SCRIPT_CYPRO_MINOAN
        | HB_SCRIPT_OLD_UYGHUR
        | HB_SCRIPT_TANGSA
        | HB_SCRIPT_TOTO
        | HB_SCRIPT_VITHKUQI
        /* Unicode-15.0 additions */
        | HB_SCRIPT_KAWI
        | HB_SCRIPT_NAG_MUNDARI => {
            /* If the designer designed the font for the 'DFLT' script,
             * (or we ended up arbitrarily pick 'latn'), use the default shaper.
             * Otherwise, use the specific shaper.
             * Note that for some simple scripts, there may not be *any*
             * GSUB/GPOS needed, so there may be no scripts found! */
            if chosen_script == hb_tag(b'D', b'F', b'L', b'T') || chosen_script == hb_tag(b'l', b'a', b't', b'n') {
                &_hb_ot_shaper_default
            } else {
                &_hb_ot_shaper_default
            }
        }

        _ => &_hb_ot_shaper_default,
    }
}
