// Rust translation of src/hb-ot-shape.hh and src/hb-ot-shape.cc from
// HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2009,2010  Red Hat, Inc.
// Copyright © 2010,2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! The OpenType shaper ("ot"): the shape plan and the shaping steps.
//!
//! Translation notes: Apple Advanced Typography (`morx`, `kerx`, `trak`)
//! is not translated; its tables are treated as absent (as for fonts
//! without them, which is every font that has OpenType layout tables in
//! practice).

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_face::*;
use super::hb_font::HbFont;
use super::hb_ot_layout::*;
use super::hb_ot_map::*;
use super::hb_ot_shape_fallback::*;
use super::hb_ot_shape_normalize::*;
use super::hb_ot_shaper::*;
use super::hb_unicode::*;

/// `hb_ot_shape_plan_key_t`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HbOtShapePlanKey {
    pub(crate) variations_index: [u32; 2],
}

impl HbOtShapePlanKey {
    /// `init`
    pub(crate) fn init(face: &HbFace, coords: &[i32]) -> HbOtShapePlanKey {
        let mut variations_index = [0; 2];
        for (table_index, vi) in variations_index.iter_mut().enumerate() {
            hb_ot_layout_table_find_feature_variations(face, TABLE_TAGS[table_index], coords, vi);
        }
        HbOtShapePlanKey { variations_index }
    }
}

/// `hb_ot_shape_plan_t`
#[derive(Debug)]
pub(crate) struct HbOtShapePlan {
    pub(crate) props: HbSegmentProperties,
    pub(crate) shaper: &'static HbOtShaper,
    pub(crate) map: HbOtMap,
    pub(crate) data: Option<ShaperData>,
    pub(crate) frac_mask: HbMask,
    pub(crate) numr_mask: HbMask,
    pub(crate) dnom_mask: HbMask,
    pub(crate) rtlm_mask: HbMask,
    pub(crate) kern_mask: HbMask,
    pub(crate) trak_mask: HbMask,

    pub(crate) requested_kerning: bool,
    pub(crate) requested_tracking: bool,
    pub(crate) has_frac: bool,
    pub(crate) has_vert: bool,
    pub(crate) has_gpos_mark: bool,
    pub(crate) zero_marks: bool,
    pub(crate) fallback_glyph_classes: bool,
    pub(crate) fallback_mark_positioning: bool,
    pub(crate) adjust_mark_positioning_when_zeroing: bool,

    pub(crate) apply_gpos: bool,
    pub(crate) apply_kern: bool,
    pub(crate) apply_fallback_kern: bool,
    pub(crate) apply_kerx: bool,
    pub(crate) apply_morx: bool,
    pub(crate) apply_trak: bool,
}

impl HbOtShapePlan {
    /// `init0`
    pub(crate) fn init0(
        face: &HbFace,
        props: &HbSegmentProperties,
        user_features: &[HbFeature],
        key: &HbOtShapePlanKey,
    ) -> Option<HbOtShapePlan> {
        let mut planner = HbOtShapePlanner::new(face, props);

        hb_ot_shape_collect_features(&mut planner, user_features);

        let mut plan = HbOtShapePlan {
            props: *props,
            shaper: &_hb_ot_shaper_default,
            map: HbOtMap::default(),
            data: None,
            frac_mask: 0,
            numr_mask: 0,
            dnom_mask: 0,
            rtlm_mask: 0,
            kern_mask: 0,
            trak_mask: 0,
            requested_kerning: false,
            requested_tracking: false,
            has_frac: false,
            has_vert: false,
            has_gpos_mark: false,
            zero_marks: false,
            fallback_glyph_classes: false,
            fallback_mark_positioning: false,
            adjust_mark_positioning_when_zeroing: false,
            apply_gpos: false,
            apply_kern: false,
            apply_fallback_kern: false,
            apply_kerx: false,
            apply_morx: false,
            apply_trak: false,
        };
        planner.compile(&mut plan, key);

        if let Some(data_create) = plan.shaper.data_create {
            plan.data = Some(data_create(&plan)?);
        }

        Some(plan)
    }

    /// `substitute`
    pub(crate) fn substitute(&self, font: &mut HbFont, buffer: &mut HbBuffer) {
        self.map.substitute(self, font, buffer);
    }

    /// `position`
    pub(crate) fn position(&self, font: &mut HbFont, buffer: &mut HbBuffer) {
        if self.apply_gpos {
            self.map.position(self, font, buffer);
        }
        /* (apply_kerx: AAT is not translated) */

        if self.apply_kern {
            hb_ot_layout_kern(self.requested_kerning, self.kern_mask, font, buffer);
        } else if self.apply_fallback_kern {
            _hb_ot_shape_fallback_kern(self, font, buffer);
        }

        /* (apply_trak: AAT is not translated) */
    }
}

/// `hb_ot_shape_planner_t`
pub(crate) struct HbOtShapePlanner<'f> {
    /* In the order that they are filled in. */
    pub(crate) face: &'f HbFace,
    pub(crate) props: HbSegmentProperties,
    pub(crate) map: HbOtMapBuilder<'f>,
    pub(crate) apply_morx: bool,
    pub(crate) script_zero_marks: bool,
    pub(crate) script_fallback_mark_positioning: bool,
    pub(crate) shaper: &'static HbOtShaper,
}

/// `_hb_apply_morx`: `hb_aat_layout_has_substitution (face)` is false
/// (AAT is not translated).
fn _hb_apply_morx(_face: &HbFace, _props: &HbSegmentProperties) -> bool {
    false
}

impl<'f> HbOtShapePlanner<'f> {
    /// `hb_ot_shape_planner_t (face, props)`
    pub(crate) fn new(face: &'f HbFace, props: &HbSegmentProperties) -> HbOtShapePlanner<'f> {
        let mut planner = HbOtShapePlanner {
            face,
            props: *props,
            map: HbOtMapBuilder::new(face, props),
            apply_morx: _hb_apply_morx(face, props),
            script_zero_marks: false,
            script_fallback_mark_positioning: false,
            shaper: &_hb_ot_shaper_default,
        };
        planner.shaper = hb_ot_shaper_categorize(&planner);

        planner.script_zero_marks =
            planner.shaper.zero_width_marks != HB_OT_SHAPE_ZERO_WIDTH_MARKS_NONE;
        planner.script_fallback_mark_positioning = planner.shaper.fallback_position;

        /* https://github.com/harfbuzz/harfbuzz/issues/1528 */
        if planner.apply_morx && !std::ptr::eq(planner.shaper, &_hb_ot_shaper_default) {
            planner.shaper = &_hb_ot_shaper_dumber;
        }
        planner
    }

    /// `compile`
    pub(crate) fn compile(&mut self, plan: &mut HbOtShapePlan, key: &HbOtShapePlanKey) {
        plan.props = self.props;
        plan.shaper = self.shaper;
        self.map.compile(&mut plan.map, key);

        plan.frac_mask = plan.map.get_1_mask(hb_tag(b'f', b'r', b'a', b'c'));
        plan.numr_mask = plan.map.get_1_mask(hb_tag(b'n', b'u', b'm', b'r'));
        plan.dnom_mask = plan.map.get_1_mask(hb_tag(b'd', b'n', b'o', b'm'));
        plan.has_frac = plan.frac_mask != 0 || (plan.numr_mask != 0 && plan.dnom_mask != 0);

        plan.rtlm_mask = plan.map.get_1_mask(hb_tag(b'r', b't', b'l', b'm'));
        plan.has_vert = plan.map.get_1_mask(hb_tag(b'v', b'e', b'r', b't')) != 0;

        let kern_tag = if hb_direction_is_horizontal(self.props.direction) {
            hb_tag(b'k', b'e', b'r', b'n')
        } else {
            hb_tag(b'v', b'k', b'r', b'n')
        };
        plan.kern_mask = plan.map.get_mask(kern_tag).0;
        plan.requested_kerning = plan.kern_mask != 0;
        plan.trak_mask = plan.map.get_mask(hb_tag(b't', b'r', b'a', b'k')).0;
        plan.requested_tracking = plan.trak_mask != 0;

        let has_gpos_kern =
            plan.map.get_feature_index(1, kern_tag) != HB_OT_LAYOUT_NO_FEATURE_INDEX;
        let disable_gpos =
            plan.shaper.gpos_tag != 0 && plan.shaper.gpos_tag != plan.map.chosen_script[1];

        /*
         * Decide who provides glyph classes. GDEF or Unicode.
         */

        if !hb_ot_layout_has_glyph_classes(self.face) {
            plan.fallback_glyph_classes = true;
        }

        /*
         * Decide who does substitutions. GSUB, morx, or fallback.
         */

        plan.apply_morx = self.apply_morx;

        /*
         * Decide who does positioning. GPOS, kerx, kern, or fallback.
         */

        /* (hb_aat_layout_has_positioning: AAT is not translated) */
        let has_kerx = false;
        let has_gsub = !self.apply_morx && hb_ot_layout_has_substitution(self.face);
        let has_gpos = !disable_gpos && hb_ot_layout_has_positioning(self.face);
        /* Prefer GPOS over kerx if GSUB is present;
         * https://github.com/harfbuzz/harfbuzz/issues/3008 */
        if has_kerx && !(has_gsub && has_gpos) {
            plan.apply_kerx = true;
        } else if has_gpos {
            plan.apply_gpos = true;
        }

        if !plan.apply_kerx && (!has_gpos_kern || !plan.apply_gpos) {
            if has_kerx {
                plan.apply_kerx = true;
            } else if hb_ot_layout_has_kerning(self.face) {
                plan.apply_kern = true;
            }
        }

        plan.apply_fallback_kern = !(plan.apply_gpos || plan.apply_kerx || plan.apply_kern);

        plan.zero_marks = self.script_zero_marks
            && !plan.apply_kerx
            && (!plan.apply_kern || !hb_ot_layout_has_machine_kerning(self.face));
        plan.has_gpos_mark = plan.map.get_1_mask(hb_tag(b'm', b'a', b'r', b'k')) != 0;

        plan.adjust_mark_positioning_when_zeroing = !plan.apply_gpos
            && !plan.apply_kerx
            && (!plan.apply_kern || !hb_ot_layout_has_cross_kerning(self.face));

        plan.fallback_mark_positioning =
            plan.adjust_mark_positioning_when_zeroing && self.script_fallback_mark_positioning;

        /* If we're using morx shaping, we cancel mark position adjustment because
        Apple Color Emoji assumes this will NOT be done when forming emoji sequences;
        https://github.com/harfbuzz/harfbuzz/issues/2967. */
        if plan.apply_morx {
            plan.adjust_mark_positioning_when_zeroing = false;
        }

        /* Currently we always apply trak. */
        /* (hb_aat_layout_has_tracking: AAT is not translated) */
        plan.apply_trak = false;
    }
}

const COMMON_FEATURES: [HbOtMapFeature; 7] = [
    HbOtMapFeature {
        tag: hb_tag(b'a', b'b', b'v', b'm'),
        flags: F_GLOBAL,
    },
    HbOtMapFeature {
        tag: hb_tag(b'b', b'l', b'w', b'm'),
        flags: F_GLOBAL,
    },
    HbOtMapFeature {
        tag: hb_tag(b'c', b'c', b'm', b'p'),
        flags: F_GLOBAL,
    },
    HbOtMapFeature {
        tag: hb_tag(b'l', b'o', b'c', b'l'),
        flags: F_GLOBAL,
    },
    HbOtMapFeature {
        tag: hb_tag(b'm', b'a', b'r', b'k'),
        flags: F_GLOBAL_MANUAL_JOINERS,
    },
    HbOtMapFeature {
        tag: hb_tag(b'm', b'k', b'm', b'k'),
        flags: F_GLOBAL_MANUAL_JOINERS,
    },
    HbOtMapFeature {
        tag: hb_tag(b'r', b'l', b'i', b'g'),
        flags: F_GLOBAL,
    },
];

const HORIZONTAL_FEATURES: [HbOtMapFeature; 7] = [
    HbOtMapFeature {
        tag: hb_tag(b'c', b'a', b'l', b't'),
        flags: F_GLOBAL,
    },
    HbOtMapFeature {
        tag: hb_tag(b'c', b'l', b'i', b'g'),
        flags: F_GLOBAL,
    },
    HbOtMapFeature {
        tag: hb_tag(b'c', b'u', b'r', b's'),
        flags: F_GLOBAL,
    },
    HbOtMapFeature {
        tag: hb_tag(b'd', b'i', b's', b't'),
        flags: F_GLOBAL,
    },
    HbOtMapFeature {
        tag: hb_tag(b'k', b'e', b'r', b'n'),
        flags: F_GLOBAL_HAS_FALLBACK,
    },
    HbOtMapFeature {
        tag: hb_tag(b'l', b'i', b'g', b'a'),
        flags: F_GLOBAL,
    },
    HbOtMapFeature {
        tag: hb_tag(b'r', b'c', b'l', b't'),
        flags: F_GLOBAL,
    },
];

/// `hb_ot_shape_collect_features`
fn hb_ot_shape_collect_features(planner: &mut HbOtShapePlanner, user_features: &[HbFeature]) {
    let map = &mut planner.map;

    map.is_simple = true;

    map.enable_feature(hb_tag(b'r', b'v', b'r', b'n'), F_NONE, 1);
    map.add_gsub_pause(None);

    match planner.props.direction {
        HB_DIRECTION_LTR => {
            map.enable_feature(hb_tag(b'l', b't', b'r', b'a'), F_NONE, 1);
            map.enable_feature(hb_tag(b'l', b't', b'r', b'm'), F_NONE, 1);
        }
        HB_DIRECTION_RTL => {
            map.enable_feature(hb_tag(b'r', b't', b'l', b'a'), F_NONE, 1);
            map.add_feature(hb_tag(b'r', b't', b'l', b'm'), F_NONE, 1);
        }
        _ => {}
    }

    /* Automatic fractions. */
    map.add_feature(hb_tag(b'f', b'r', b'a', b'c'), F_NONE, 1);
    map.add_feature(hb_tag(b'n', b'u', b'm', b'r'), F_NONE, 1);
    map.add_feature(hb_tag(b'd', b'n', b'o', b'm'), F_NONE, 1);

    /* Random! */
    map.enable_feature(
        hb_tag(b'r', b'a', b'n', b'd'),
        F_RANDOM,
        HB_OT_MAP_MAX_VALUE,
    );

    /* Tracking.  We enable dummy feature here just to allow disabling
     * AAT 'trak' table using features.
     * https://github.com/harfbuzz/harfbuzz/issues/1303 */
    map.enable_feature(hb_tag(b't', b'r', b'a', b'k'), F_HAS_FALLBACK, 1);

    map.enable_feature(hb_tag(b'H', b'a', b'r', b'f'), F_NONE, 1); /* Considered required. */
    map.enable_feature(hb_tag(b'H', b'A', b'R', b'F'), F_NONE, 1); /* Considered discretionary. */

    if let Some(collect_features) = planner.shaper.collect_features {
        planner.map.is_simple = false;
        collect_features(planner);
    }

    let map = &mut planner.map;
    map.enable_feature(hb_tag(b'B', b'u', b'z', b'z'), F_NONE, 1); /* Considered required. */
    map.enable_feature(hb_tag(b'B', b'U', b'Z', b'Z'), F_NONE, 1); /* Considered discretionary. */

    for f in &COMMON_FEATURES {
        map.add_map_feature(f);
    }

    if hb_direction_is_horizontal(planner.props.direction) {
        for f in &HORIZONTAL_FEATURES {
            map.add_map_feature(f);
        }
    } else {
        /* We only apply `vert` feature. See:
         * https://github.com/harfbuzz/harfbuzz/commit/d71c0df2d17f4590d5611239577a6cb532c26528
         * https://lists.freedesktop.org/archives/harfbuzz/2013-August/003490.html */

        /* We really want to find a 'vert' feature if there's any in the font, no
         * matter which script/langsys it is listed (or not) under.
         * See various bugs referenced from:
         * https://github.com/harfbuzz/harfbuzz/issues/63 */
        map.enable_feature(hb_tag(b'v', b'e', b'r', b't'), F_GLOBAL_SEARCH, 1);
    }

    if !user_features.is_empty() {
        map.is_simple = false;
    }

    for feature in user_features {
        map.add_feature(
            feature.tag,
            if feature.start == HB_FEATURE_GLOBAL_START && feature.end == HB_FEATURE_GLOBAL_END {
                F_GLOBAL
            } else {
                F_NONE
            },
            feature.value,
        );
    }

    if let Some(override_features) = planner.shaper.override_features {
        override_features(planner);
    }
}

/*
 * shaper
 */

/// `hb_ot_shape_context_t`
struct HbOtShapeContext<'p, 'b, 'f, 'a> {
    plan: &'p HbOtShapePlan,
    font: &'f mut HbFont<'a>,
    buffer: &'b mut HbBuffer,
    user_features: &'p [HbFeature],

    /* Transient stuff */
    target_direction: HbDirection,
}

/* Main shaper */

/* Prepare */

/// `_hb_codepoint_is_regional_indicator`
#[inline]
fn _hb_codepoint_is_regional_indicator(u: HbCodepoint) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&u)
}

/// `hb_set_unicode_props`
fn hb_set_unicode_props(buffer: &mut HbBuffer) {
    /* Implement enough of Unicode Graphemes here that shaping
     * in reverse-direction wouldn't break graphemes.  Namely,
     * we mark all marks and ZWJ and ZWJ,Extended_Pictographic
     * sequences as continuations.  The foreach_grapheme()
     * macro uses this bit.
     *
     * https://www.unicode.org/reports/tr29/#Regex_Definitions
     */
    let count = buffer.len as usize;
    let mut scratch = buffer.scratch_flags;
    let info = &mut buffer.info;
    let mut i = 0;
    while i < count {
        _hb_glyph_info_set_unicode_props(&mut info[i], &mut scratch);

        let gen_cat = _hb_glyph_info_get_general_category(&info[i]);

        if flag_unsafe(gen_cat)
            & (flag(HB_UNICODE_GENERAL_CATEGORY_LOWERCASE_LETTER)
                | flag(HB_UNICODE_GENERAL_CATEGORY_UPPERCASE_LETTER)
                | flag(HB_UNICODE_GENERAL_CATEGORY_TITLECASE_LETTER)
                | flag(HB_UNICODE_GENERAL_CATEGORY_OTHER_LETTER)
                | flag(HB_UNICODE_GENERAL_CATEGORY_SPACE_SEPARATOR))
            != 0
        {
            i += 1;
            continue;
        }

        /* Marks are already set as continuation by the above line.
         * Handle Emoji_Modifier and ZWJ-continuation. */
        if gen_cat == HB_UNICODE_GENERAL_CATEGORY_MODIFIER_SYMBOL
            && (0x1F3FB..=0x1F3FF).contains(&info[i].codepoint)
        {
            _hb_glyph_info_set_continuation(&mut info[i]);
        }
        /* Regional_Indicators are hairy as hell...
         * https://github.com/harfbuzz/harfbuzz/issues/2265 */
        else if i != 0 && _hb_codepoint_is_regional_indicator(info[i].codepoint) {
            if _hb_codepoint_is_regional_indicator(info[i - 1].codepoint)
                && !_hb_glyph_info_is_continuation(&info[i - 1])
            {
                _hb_glyph_info_set_continuation(&mut info[i]);
            }
        } else if _hb_glyph_info_is_zwj(&info[i]) {
            _hb_glyph_info_set_continuation(&mut info[i]);
            if i + 1 < count && _hb_unicode_is_emoji_Extended_Pictographic(info[i + 1].codepoint) {
                i += 1;
                _hb_glyph_info_set_unicode_props(&mut info[i], &mut scratch);
                _hb_glyph_info_set_continuation(&mut info[i]);
            }
        }
        /* Or part of the Other_Grapheme_Extend that is not marks.
         * As of Unicode 15 that is just:
         *
         * 200C          ; Other_Grapheme_Extend # Cf       ZERO WIDTH NON-JOINER
         * FF9E..FF9F    ; Other_Grapheme_Extend # Lm   [2] HALFWIDTH KATAKANA VOICED SOUND MARK..HALFWIDTH KATAKANA SEMI-VOICED SOUND MARK
         * E0020..E007F  ; Other_Grapheme_Extend # Cf  [96] TAG SPACE..CANCEL TAG
         *
         * ZWNJ is special, we don't want to merge it as there's no need, and keeping
         * it separate results in more granular clusters.
         * Tags are used for Emoji sub-region flag sequences:
         * https://github.com/harfbuzz/harfbuzz/issues/1556
         * Katakana ones were requested:
         * https://github.com/harfbuzz/harfbuzz/issues/3844
         */
        else if (0xFF9E..=0xFF9F).contains(&info[i].codepoint)
            || (0xE0020..=0xE007F).contains(&info[i].codepoint)
        {
            _hb_glyph_info_set_continuation(&mut info[i]);
        }
        i += 1;
    }
    buffer.scratch_flags = scratch;
}

/// `hb_insert_dotted_circle`
fn hb_insert_dotted_circle(buffer: &mut HbBuffer, font: &mut HbFont) {
    if buffer.flags & HB_BUFFER_FLAG_DO_NOT_INSERT_DOTTED_CIRCLE != 0 {
        return;
    }

    if (buffer.flags & HB_BUFFER_FLAG_BOT) == 0
        || buffer.context_len[0] != 0
        || !_hb_glyph_info_is_unicode_mark(&buffer.info[0])
    {
        return;
    }

    if !font.has_glyph(0x25CC) {
        return;
    }

    let mut dottedcircle = HbGlyphInfo {
        codepoint: 0x25CC,
        ..Default::default()
    };
    let mut scratch = buffer.scratch_flags;
    _hb_glyph_info_set_unicode_props(&mut dottedcircle, &mut scratch);
    buffer.scratch_flags = scratch;

    buffer.clear_output();

    buffer.idx = 0;
    let mut info = dottedcircle;
    info.cluster = buffer.cur(0).cluster;
    info.mask = buffer.cur(0).mask;
    let _ = buffer.output_info(info);

    let _ = buffer.sync();
}

/// `hb_form_clusters`
fn hb_form_clusters(buffer: &mut HbBuffer) {
    if buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_HAS_NON_ASCII == 0 {
        return;
    }

    if buffer.cluster_level == HB_BUFFER_CLUSTER_LEVEL_MONOTONE_GRAPHEMES {
        for (start, end) in foreach_grapheme(buffer) {
            buffer.merge_clusters(start, end);
        }
    } else {
        for (start, end) in foreach_grapheme(buffer) {
            buffer.unsafe_to_break(start, end);
        }
    }
}

/// `hb_ensure_native_direction`
fn hb_ensure_native_direction(buffer: &mut HbBuffer) {
    let direction = buffer.props.direction;
    let mut horiz_dir = hb_script_get_horizontal_direction(buffer.props.script);

    /* Numeric runs in natively-RTL scripts are actually native-LTR, so we reset
     * the horiz_dir if the run contains at least one decimal-number char, and no
     * letter chars (ideally we should be checking for chars with strong
     * directionality but hb-unicode currently lacks bidi categories).
     *
     * This allows digit sequences in Arabic etc to be shaped in "native"
     * direction, so that features like ligatures will work as intended.
     *
     * https://github.com/harfbuzz/harfbuzz/issues/501
     *
     * Similar thing about Regional_Indicators; They are bidi=L, but Script=Common.
     * If they are present in a run of natively-RTL text, they get assigned a script
     * with natively RTL direction, which would result in wrong shaping if we
     * assign such native RTL direction to them then. Detect that as well.
     *
     * https://github.com/harfbuzz/harfbuzz/issues/3314
     */
    if horiz_dir == HB_DIRECTION_RTL && direction == HB_DIRECTION_LTR {
        let mut found_number = false;
        let mut found_letter = false;
        let mut found_ri = false;
        let count = buffer.len as usize;
        for info in &buffer.info[..count] {
            let gc = _hb_glyph_info_get_general_category(info);
            if gc == HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER {
                found_number = true;
            } else if hb_unicode_general_category_is_letter(gc) {
                found_letter = true;
                break;
            } else if _hb_codepoint_is_regional_indicator(info.codepoint) {
                found_ri = true;
            }
        }
        if (found_number || found_ri) && !found_letter {
            horiz_dir = HB_DIRECTION_LTR;
        }
    }

    /* TODO vertical:
     * The only BTT vertical script is Ogham, but it's not clear to me whether OpenType
     * Ogham fonts are supposed to be implemented BTT or not.  Need to research that
     * first. */
    if (hb_direction_is_horizontal(direction)
        && direction != horiz_dir
        && horiz_dir != HB_DIRECTION_INVALID)
        || (hb_direction_is_vertical(direction) && direction != HB_DIRECTION_TTB)
    {
        _hb_ot_layout_reverse_graphemes(buffer);
        buffer.props.direction = hb_direction_reverse(buffer.props.direction);
    }
}

/*
 * Substitute
 */

/// `hb_vert_char_for`
fn hb_vert_char_for(u: HbCodepoint) -> HbCodepoint {
    match u >> 8 {
        0x20 => match u {
            0x2013 => return 0xfe32, // EN DASH
            0x2014 => return 0xfe31, // EM DASH
            0x2025 => return 0xfe30, // TWO DOT LEADER
            0x2026 => return 0xfe19, // HORIZONTAL ELLIPSIS
            _ => {}
        },
        0x30 => match u {
            0x3001 => return 0xfe11, // IDEOGRAPHIC COMMA
            0x3002 => return 0xfe12, // IDEOGRAPHIC FULL STOP
            0x3008 => return 0xfe3f, // LEFT ANGLE BRACKET
            0x3009 => return 0xfe40, // RIGHT ANGLE BRACKET
            0x300a => return 0xfe3d, // LEFT DOUBLE ANGLE BRACKET
            0x300b => return 0xfe3e, // RIGHT DOUBLE ANGLE BRACKET
            0x300c => return 0xfe41, // LEFT CORNER BRACKET
            0x300d => return 0xfe42, // RIGHT CORNER BRACKET
            0x300e => return 0xfe43, // LEFT WHITE CORNER BRACKET
            0x300f => return 0xfe44, // RIGHT WHITE CORNER BRACKET
            0x3010 => return 0xfe3b, // LEFT BLACK LENTICULAR BRACKET
            0x3011 => return 0xfe3c, // RIGHT BLACK LENTICULAR BRACKET
            0x3014 => return 0xfe39, // LEFT TORTOISE SHELL BRACKET
            0x3015 => return 0xfe3a, // RIGHT TORTOISE SHELL BRACKET
            0x3016 => return 0xfe17, // LEFT WHITE LENTICULAR BRACKET
            0x3017 => return 0xfe18, // RIGHT WHITE LENTICULAR BRACKET
            _ => {}
        },
        0xfe => {
            if u == 0xfe4f {
                return 0xfe34; // WAVY LOW LINE
            }
        }
        0xff => match u {
            0xff01 => return 0xfe15, // FULLWIDTH EXCLAMATION MARK
            0xff08 => return 0xfe35, // FULLWIDTH LEFT PARENTHESIS
            0xff09 => return 0xfe36, // FULLWIDTH RIGHT PARENTHESIS
            0xff0c => return 0xfe10, // FULLWIDTH COMMA
            0xff1a => return 0xfe13, // FULLWIDTH COLON
            0xff1b => return 0xfe14, // FULLWIDTH SEMICOLON
            0xff1f => return 0xfe16, // FULLWIDTH QUESTION MARK
            0xff3b => return 0xfe47, // FULLWIDTH LEFT SQUARE BRACKET
            0xff3d => return 0xfe48, // FULLWIDTH RIGHT SQUARE BRACKET
            0xff3f => return 0xfe33, // FULLWIDTH LOW LINE
            0xff5b => return 0xfe37, // FULLWIDTH LEFT CURLY BRACKET
            0xff5d => return 0xfe38, // FULLWIDTH RIGHT CURLY BRACKET
            _ => {}
        },
        _ => {}
    }

    u
}

/// `hb_ot_rotate_chars`
fn hb_ot_rotate_chars(c: &mut HbOtShapeContext) {
    let count = c.buffer.len as usize;

    if hb_direction_is_backward(c.target_direction) {
        let unicode = c.buffer.unicode;
        let rtlm_mask = c.plan.rtlm_mask;

        for i in 0..count {
            let codepoint = unicode.mirroring(c.buffer.info[i].codepoint);
            if codepoint != c.buffer.info[i].codepoint && c.font.has_glyph(codepoint) {
                c.buffer.info[i].codepoint = codepoint;
            } else {
                c.buffer.info[i].mask |= rtlm_mask;
            }
        }
    }

    if hb_direction_is_vertical(c.target_direction) && !c.plan.has_vert {
        for i in 0..count {
            let codepoint = hb_vert_char_for(c.buffer.info[i].codepoint);
            if codepoint != c.buffer.info[i].codepoint && c.font.has_glyph(codepoint) {
                c.buffer.info[i].codepoint = codepoint;
            }
        }
    }
}

/// `hb_ot_shape_setup_masks_fraction`
fn hb_ot_shape_setup_masks_fraction(c: &mut HbOtShapeContext) {
    if (c.buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_HAS_NON_ASCII) == 0 || !c.plan.has_frac {
        return;
    }

    let buffer = &mut *c.buffer;

    let pre_mask;
    let post_mask;
    if hb_direction_is_forward(buffer.props.direction) {
        pre_mask = c.plan.numr_mask | c.plan.frac_mask;
        post_mask = c.plan.frac_mask | c.plan.dnom_mask;
    } else {
        pre_mask = c.plan.frac_mask | c.plan.dnom_mask;
        post_mask = c.plan.numr_mask | c.plan.frac_mask;
    }

    let count = buffer.len;
    let mut i = 0;
    while i < count {
        if buffer.info[i as usize].codepoint == 0x2044
        /* FRACTION SLASH */
        {
            let mut start = i;
            let mut end = i + 1;
            while start != 0
                && _hb_glyph_info_get_general_category(&buffer.info[start as usize - 1])
                    == HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER
            {
                start -= 1;
            }
            while end < count
                && _hb_glyph_info_get_general_category(&buffer.info[end as usize])
                    == HB_UNICODE_GENERAL_CATEGORY_DECIMAL_NUMBER
            {
                end += 1;
            }
            if start == i || end == i + 1 {
                if start == i {
                    buffer.unsafe_to_concat(start, start + 1);
                }
                if end == i + 1 {
                    buffer.unsafe_to_concat(end - 1, end);
                }
                i += 1;
                continue;
            }

            buffer.unsafe_to_break(start, end);

            for j in start..i {
                buffer.info[j as usize].mask |= pre_mask;
            }
            buffer.info[i as usize].mask |= c.plan.frac_mask;
            for j in i + 1..end {
                buffer.info[j as usize].mask |= post_mask;
            }

            i = end - 1;
        }
        i += 1;
    }
}

/// `hb_ot_shape_initialize_masks`
fn hb_ot_shape_initialize_masks(c: &mut HbOtShapeContext) {
    let global_mask = c.plan.map.get_global_mask();
    c.buffer.reset_masks(global_mask);
}

/// `hb_ot_shape_setup_masks`
fn hb_ot_shape_setup_masks(c: &mut HbOtShapeContext) {
    hb_ot_shape_setup_masks_fraction(c);

    if let Some(setup_masks) = c.plan.shaper.setup_masks {
        setup_masks(c.plan, c.buffer, c.font);
    }

    for feature in c.user_features {
        if !(feature.start == HB_FEATURE_GLOBAL_START && feature.end == HB_FEATURE_GLOBAL_END) {
            let (mask, shift) = c.plan.map.get_mask(feature.tag);
            c.buffer
                .set_masks(feature.value << shift, mask, feature.start, feature.end);
        }
    }
}

/// `hb_ot_zero_width_default_ignorables`
fn hb_ot_zero_width_default_ignorables(buffer: &mut HbBuffer) {
    if (buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_HAS_DEFAULT_IGNORABLES) == 0
        || (buffer.flags & HB_BUFFER_FLAG_PRESERVE_DEFAULT_IGNORABLES) != 0
        || (buffer.flags & HB_BUFFER_FLAG_REMOVE_DEFAULT_IGNORABLES) != 0
    {
        return;
    }

    let count = buffer.len as usize;
    for i in 0..count {
        if _hb_glyph_info_is_default_ignorable(&buffer.info[i]) {
            let pos = &mut buffer.pos[i];
            pos.x_advance = 0;
            pos.y_advance = 0;
            pos.x_offset = 0;
            pos.y_offset = 0;
        }
    }
}

/// `hb_ot_hide_default_ignorables`
fn hb_ot_hide_default_ignorables(buffer: &mut HbBuffer, font: &mut HbFont) {
    if (buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_HAS_DEFAULT_IGNORABLES) == 0
        || (buffer.flags & HB_BUFFER_FLAG_PRESERVE_DEFAULT_IGNORABLES) != 0
    {
        return;
    }

    let count = buffer.len as usize;
    let mut invisible = buffer.invisible;
    if (buffer.flags & HB_BUFFER_FLAG_REMOVE_DEFAULT_IGNORABLES) == 0
        && (invisible != 0 || font.get_nominal_glyph(b' ' as u32, &mut invisible, 0))
    {
        /* Replace default-ignorables with a zero-advance invisible glyph. */
        for info in &mut buffer.info[..count] {
            if _hb_glyph_info_is_default_ignorable(info) {
                info.codepoint = invisible;
            }
        }
    } else {
        buffer.delete_glyphs_inplace(_hb_glyph_info_is_default_ignorable);
    }
}

/// `hb_ot_map_glyphs_fast`
#[inline]
fn hb_ot_map_glyphs_fast(buffer: &mut HbBuffer) {
    /* Normalization process sets up glyph_index(), we just copy it. */
    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        info.codepoint = glyph_index(info);
    }

    buffer.content_type = HB_BUFFER_CONTENT_TYPE_GLYPHS;
}

/// `hb_synthesize_glyph_classes`
fn hb_synthesize_glyph_classes(buffer: &mut HbBuffer) {
    let count = buffer.len as usize;
    for info in &mut buffer.info[..count] {
        /* Never mark default-ignorables as marks.
         * They won't get in the way of lookups anyway,
         * but having them as mark will cause them to be skipped
         * over if the lookup-flag says so, but at least for the
         * Mongolian variation selectors, looks like Uniscribe
         * marks them as non-mark.  Some Mongolian fonts without
         * GDEF rely on this.  Another notable character that
         * this applies to is COMBINING GRAPHEME JOINER. */
        let klass = if _hb_glyph_info_get_general_category(info)
            != HB_UNICODE_GENERAL_CATEGORY_NON_SPACING_MARK
            || _hb_glyph_info_is_default_ignorable(info)
        {
            HB_OT_LAYOUT_GLYPH_PROPS_BASE_GLYPH
        } else {
            HB_OT_LAYOUT_GLYPH_PROPS_MARK
        };
        _hb_glyph_info_set_glyph_props(info, klass);
    }
}

/// `hb_ot_substitute_default`
fn hb_ot_substitute_default(c: &mut HbOtShapeContext) {
    hb_ot_rotate_chars(c);

    c.buffer.allocate_var(VAR_GLYPH_INDEX.0, VAR_GLYPH_INDEX.1);

    _hb_ot_shape_normalize(c.plan, c.buffer, c.font);

    hb_ot_shape_setup_masks(c);

    /* This is unfortunate to go here, but necessary... */
    if c.plan.fallback_mark_positioning {
        _hb_ot_shape_fallback_mark_position_recategorize_marks(c.buffer);
    }

    hb_ot_map_glyphs_fast(c.buffer);

    c.buffer
        .deallocate_var(VAR_GLYPH_INDEX.0, VAR_GLYPH_INDEX.1);
}

/// `hb_ot_substitute_plan`
fn hb_ot_substitute_plan(c: &mut HbOtShapeContext) {
    let face = c.font.p.face.clone();
    hb_ot_layout_substitute_start(&face, c.buffer);

    if c.plan.fallback_glyph_classes {
        hb_synthesize_glyph_classes(c.buffer);
    }

    /* (apply_morx: AAT is not translated) */
    c.plan.substitute(c.font, c.buffer);
}

/// `hb_ot_substitute_pre`
fn hb_ot_substitute_pre(c: &mut HbOtShapeContext) {
    hb_ot_substitute_default(c);

    _hb_buffer_allocate_gsubgpos_vars(c.buffer);

    hb_ot_substitute_plan(c);
}

/// `hb_ot_substitute_post`
fn hb_ot_substitute_post(c: &mut HbOtShapeContext) {
    hb_ot_hide_default_ignorables(c.buffer, c.font);

    if let Some(postprocess_glyphs) = c.plan.shaper.postprocess_glyphs {
        postprocess_glyphs(c.plan, c.buffer, c.font);
    }
}

/*
 * Position
 */

/// `adjust_mark_offsets`
#[inline]
fn adjust_mark_offsets(pos: &mut HbGlyphPosition) {
    pos.x_offset = pos.x_offset.wrapping_sub(pos.x_advance);
    pos.y_offset = pos.y_offset.wrapping_sub(pos.y_advance);
}

/// `zero_mark_width`
#[inline]
fn zero_mark_width(pos: &mut HbGlyphPosition) {
    pos.x_advance = 0;
    pos.y_advance = 0;
}

/// `zero_mark_widths_by_gdef`
fn zero_mark_widths_by_gdef(buffer: &mut HbBuffer, adjust_offsets: bool) {
    let count = buffer.len as usize;
    for i in 0..count {
        if _hb_glyph_info_is_mark(&buffer.info[i]) {
            if adjust_offsets {
                adjust_mark_offsets(&mut buffer.pos[i]);
            }
            zero_mark_width(&mut buffer.pos[i]);
        }
    }
}

/// `hb_ot_position_default`
fn hb_ot_position_default(c: &mut HbOtShapeContext) {
    let direction = c.buffer.props.direction;
    let count = c.buffer.len as usize;

    let glyphs: Vec<HbCodepoint> = c.buffer.info[..count].iter().map(|i| i.codepoint).collect();
    let mut advances = vec![0; count];
    if hb_direction_is_horizontal(direction) {
        c.font.get_glyph_h_advances(&glyphs, &mut advances);
        for (p, a) in c.buffer.pos[..count].iter_mut().zip(&advances) {
            p.x_advance = *a;
        }
        /* The nil glyph_h_origin() func returns 0, so no need to apply it. */
        if c.font.has_glyph_h_origin_func() {
            for i in 0..count {
                let (mut x, mut y) = (c.buffer.pos[i].x_offset, c.buffer.pos[i].y_offset);
                c.font.subtract_glyph_h_origin(glyphs[i], &mut x, &mut y);
                c.buffer.pos[i].x_offset = x;
                c.buffer.pos[i].y_offset = y;
            }
        }
    } else {
        c.font.get_glyph_v_advances(&glyphs, &mut advances);
        for (p, a) in c.buffer.pos[..count].iter_mut().zip(&advances) {
            p.y_advance = *a;
        }
        for i in 0..count {
            let (mut x, mut y) = (c.buffer.pos[i].x_offset, c.buffer.pos[i].y_offset);
            c.font.subtract_glyph_v_origin(glyphs[i], &mut x, &mut y);
            c.buffer.pos[i].x_offset = x;
            c.buffer.pos[i].y_offset = y;
        }
    }
    if c.buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_HAS_SPACE_FALLBACK != 0 {
        _hb_ot_shape_fallback_spaces(c.font, c.buffer);
    }
}

/// `hb_ot_position_plan`
fn hb_ot_position_plan(c: &mut HbOtShapeContext) {
    let count = c.buffer.len as usize;

    /* If the font has no GPOS and direction is forward, then when
     * zeroing mark widths, we shift the mark with it, such that the
     * mark is positioned hanging over the previous glyph.  When
     * direction is backward we don't shift and it will end up
     * hanging over the next glyph after the final reordering.
     *
     * Note: If fallback positioning happens, we don't care about
     * this as it will be overridden.
     */
    let adjust_offsets_when_zeroing = c.plan.adjust_mark_positioning_when_zeroing
        && hb_direction_is_forward(c.buffer.props.direction);

    /* We change glyph origin to what GPOS expects (horizontal), apply GPOS, change it back. */

    /* The nil glyph_h_origin() func returns 0, so no need to apply it. */
    if c.font.has_glyph_h_origin_func() {
        for i in 0..count {
            let g = c.buffer.info[i].codepoint;
            let (mut x, mut y) = (c.buffer.pos[i].x_offset, c.buffer.pos[i].y_offset);
            c.font.add_glyph_h_origin(g, &mut x, &mut y);
            c.buffer.pos[i].x_offset = x;
            c.buffer.pos[i].y_offset = y;
        }
    }

    hb_ot_layout_position_start(c.buffer);

    if c.plan.zero_marks
        && c.plan.shaper.zero_width_marks == HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_EARLY
    {
        zero_mark_widths_by_gdef(c.buffer, adjust_offsets_when_zeroing);
    }

    c.plan.position(c.font, c.buffer);

    if c.plan.zero_marks
        && c.plan.shaper.zero_width_marks == HB_OT_SHAPE_ZERO_WIDTH_MARKS_BY_GDEF_LATE
    {
        zero_mark_widths_by_gdef(c.buffer, adjust_offsets_when_zeroing);
    }

    /* Finish off.  Has to follow a certain order. */
    hb_ot_layout_position_finish_advances(c.buffer);
    hb_ot_zero_width_default_ignorables(c.buffer);
    hb_ot_layout_position_finish_offsets(c.font, c.buffer);

    /* The nil glyph_h_origin() func returns 0, so no need to apply it. */
    if c.font.has_glyph_h_origin_func() {
        for i in 0..count {
            let g = c.buffer.info[i].codepoint;
            let (mut x, mut y) = (c.buffer.pos[i].x_offset, c.buffer.pos[i].y_offset);
            c.font.subtract_glyph_h_origin(g, &mut x, &mut y);
            c.buffer.pos[i].x_offset = x;
            c.buffer.pos[i].y_offset = y;
        }
    }

    if c.plan.fallback_mark_positioning {
        _hb_ot_shape_fallback_mark_position(c.plan, c.font, c.buffer, adjust_offsets_when_zeroing);
    }
}

/// `hb_ot_position`
fn hb_ot_position(c: &mut HbOtShapeContext) {
    c.buffer.clear_positions();

    hb_ot_position_default(c);

    hb_ot_position_plan(c);

    if hb_direction_is_backward(c.buffer.props.direction) {
        c.buffer.hb_reverse();
    }

    _hb_buffer_deallocate_gsubgpos_vars(c.buffer);
}

/// `hb_propagate_flags`
fn hb_propagate_flags(buffer: &mut HbBuffer) {
    /* Propagate cluster-level glyph flags to be the same on all cluster glyphs.
     * Simplifies using them. */

    if buffer.scratch_flags & HB_BUFFER_SCRATCH_FLAG_HAS_GLYPH_FLAGS == 0 {
        return;
    }

    /* If we are producing SAFE_TO_INSERT_TATWEEL, then do two things:
     *
     * - If the places that the Arabic shaper marked as SAFE_TO_INSERT_TATWEEL,
     *   are UNSAFE_TO_BREAK, then clear the SAFE_TO_INSERT_TATWEEL,
     * - Any place that is SAFE_TO_INSERT_TATWEEL, is also now UNSAFE_TO_BREAK.
     *
     * We couldn't make this interaction earlier. It has to be done here.
     */
    let flip_tatweel = buffer.flags & HB_BUFFER_FLAG_PRODUCE_SAFE_TO_INSERT_TATWEEL != 0;

    let clear_concat = (buffer.flags & HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT) == 0;

    for (start, end) in foreach_cluster(buffer) {
        let mut mask = 0;
        for i in start..end {
            mask |= buffer.info[i as usize].mask & HB_GLYPH_FLAG_DEFINED;
        }

        if flip_tatweel {
            if mask & HB_GLYPH_FLAG_UNSAFE_TO_BREAK != 0 {
                mask &= !HB_GLYPH_FLAG_SAFE_TO_INSERT_TATWEEL;
            }
            if mask & HB_GLYPH_FLAG_SAFE_TO_INSERT_TATWEEL != 0 {
                mask |= HB_GLYPH_FLAG_UNSAFE_TO_BREAK | HB_GLYPH_FLAG_UNSAFE_TO_CONCAT;
            }
        }

        if clear_concat {
            mask &= !HB_GLYPH_FLAG_UNSAFE_TO_CONCAT;
        }

        for i in start..end {
            buffer.info[i as usize].mask = mask;
        }
    }
}

/* Pull it all together! */

/// `hb_ot_shape_internal`
fn hb_ot_shape_internal(c: &mut HbOtShapeContext) {
    /* Save the original direction, we use it later. */
    c.target_direction = c.buffer.props.direction;

    _hb_buffer_allocate_unicode_vars(c.buffer);

    hb_ot_shape_initialize_masks(c);
    hb_set_unicode_props(c.buffer);
    hb_insert_dotted_circle(c.buffer, c.font);

    hb_form_clusters(c.buffer);

    hb_ensure_native_direction(c.buffer);

    if let Some(preprocess_text) = c.plan.shaper.preprocess_text {
        preprocess_text(c.plan, c.buffer, c.font);
    }

    hb_ot_substitute_pre(c);
    hb_ot_position(c);
    hb_ot_substitute_post(c);

    hb_propagate_flags(c.buffer);

    _hb_buffer_deallocate_unicode_vars(c.buffer);

    c.buffer.props.direction = c.target_direction;

    c.buffer.leave();
}

/// `_hb_ot_shape`
pub(crate) fn _hb_ot_shape(
    plan: &HbOtShapePlan,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
    features: &[HbFeature],
) -> bool {
    let mut c = HbOtShapeContext {
        plan,
        font,
        buffer,
        user_features: features,
        target_direction: HB_DIRECTION_INVALID,
    };
    hb_ot_shape_internal(&mut c);

    true
}
