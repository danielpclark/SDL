// Rust translation of src/hb-ot-map.hh and src/hb-ot-map.cc (and of
// `hb_ot_map_t::apply`, `substitute` and `position` from
// src/hb-ot-layout.cc) from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2009,2010  Red Hat, Inc.
// Copyright © 2010,2011,2013  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! The map of features to masks and lookups.

use std::collections::HashMap;

use super::hb_algs::hb_qsort;
use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_face::*;
use super::hb_font::HbFont;
use super::hb_open_type::hb_bsearch_impl;
use super::hb_ot_layout::*;
use super::hb_ot_layout_gsubgpos::HbOtApplyContext;
use super::hb_ot_shape::{HbOtShapePlan, HbOtShapePlanKey};
use super::hb_ot_tag::*;

pub(crate) const HB_OT_MAP_MAX_BITS: u32 = 8;
pub(crate) const HB_OT_MAP_MAX_VALUE: u32 = (1 << HB_OT_MAP_MAX_BITS) - 1;

/// `table_tags`
pub(crate) const TABLE_TAGS: [HbTag; 2] = [HB_OT_TAG_GSUB, HB_OT_TAG_GPOS];

/// `hb_ot_map_t::feature_map_t`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FeatureMap {
    pub(crate) tag: HbTag,      /* should be first for our bsearch to work */
    pub(crate) index: [u32; 2], /* GSUB/GPOS */
    pub(crate) stage: [u32; 2], /* GSUB/GPOS */
    pub(crate) shift: u32,
    pub(crate) mask: HbMask,
    pub(crate) _1_mask: HbMask, /* mask for value=1, for quick access */
    pub(crate) needs_fallback: bool,
    pub(crate) auto_zwnj: bool,
    pub(crate) auto_zwj: bool,
    pub(crate) random: bool,
    pub(crate) per_syllable: bool,
}

/// `hb_ot_map_t::lookup_map_t`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LookupMap {
    pub(crate) index: u16,
    pub(crate) auto_zwnj: bool,
    pub(crate) auto_zwj: bool,
    pub(crate) random: bool,
    pub(crate) per_syllable: bool,
    pub(crate) mask: HbMask,
    pub(crate) feature_tag: HbTag,
}

impl LookupMap {
    /// `lookup_map_t::cmp`
    fn cmp(a: &LookupMap, b: &LookupMap) -> i32 {
        if a.index < b.index {
            -1
        } else if a.index > b.index {
            1
        } else {
            0
        }
    }
}

/// `pause_func_t`: pause functions return true if new glyph indices might
/// have been added to the buffer.  This is used to update buffer digest.
pub(crate) type PauseFunc = fn(&HbOtShapePlan, &mut HbFont, &mut HbBuffer) -> bool;

/// `hb_ot_map_t::stage_map_t`
#[derive(Debug, Clone, Copy)]
pub(crate) struct StageMap {
    pub(crate) last_lookup: u32, /* Cumulative */
    pub(crate) pause_func: Option<PauseFunc>,
}

/// `hb_ot_map_t`
#[derive(Debug, Clone, Default)]
pub(crate) struct HbOtMap {
    pub(crate) chosen_script: [HbTag; 2],
    pub(crate) found_script: [bool; 2],

    global_mask: HbMask,

    features: Vec<FeatureMap>,
    lookups: [Vec<LookupMap>; 2], /* GSUB/GPOS */
    stages: [Vec<StageMap>; 2],   /* GSUB/GPOS */
}

impl HbOtMap {
    /// `features.bsearch (feature_tag)`
    fn find_feature(&self, feature_tag: HbTag) -> Option<&FeatureMap> {
        let r = hb_bsearch_impl(self.features.len(), |mid| {
            let tag = self.features[mid].tag;
            if feature_tag < tag {
                -1
            } else if feature_tag > tag {
                1
            } else {
                0
            }
        });
        r.ok().map(|i| &self.features[i as usize])
    }

    /// `get_global_mask`
    pub(crate) fn get_global_mask(&self) -> HbMask {
        self.global_mask
    }

    /// `get_mask`: the mask and the shift of a feature.
    pub(crate) fn get_mask(&self, feature_tag: HbTag) -> (HbMask, u32) {
        match self.find_feature(feature_tag) {
            Some(map) => (map.mask, map.shift),
            None => (0, 0),
        }
    }

    /// `needs_fallback`
    pub(crate) fn needs_fallback(&self, feature_tag: HbTag) -> bool {
        self.find_feature(feature_tag)
            .is_some_and(|m| m.needs_fallback)
    }

    /// `get_1_mask`
    pub(crate) fn get_1_mask(&self, feature_tag: HbTag) -> HbMask {
        self.find_feature(feature_tag).map_or(0, |m| m._1_mask)
    }

    /// `get_feature_index`
    pub(crate) fn get_feature_index(&self, table_index: usize, feature_tag: HbTag) -> u32 {
        self.find_feature(feature_tag)
            .map_or(HB_OT_LAYOUT_NO_FEATURE_INDEX, |m| m.index[table_index])
    }

    /// `get_feature_stage`
    pub(crate) fn get_feature_stage(&self, table_index: usize, feature_tag: HbTag) -> u32 {
        self.find_feature(feature_tag)
            .map_or(u32::MAX, |m| m.stage[table_index])
    }

    /// `get_stage_lookups`
    pub(crate) fn get_stage_lookups(&self, table_index: usize, stage: u32) -> &[LookupMap] {
        let stages = &self.stages[table_index];
        if stage as usize > stages.len() {
            return &[];
        }
        let start = if stage != 0 {
            stages[stage as usize - 1].last_lookup as usize
        } else {
            0
        };
        let end = if (stage as usize) < stages.len() {
            stages[stage as usize].last_lookup as usize
        } else {
            self.lookups[table_index].len()
        };
        &self.lookups[table_index][start..end]
    }

    /// `collect_lookups`
    pub(crate) fn collect_lookups(
        &self,
        table_index: usize,
        lookups_out: &mut super::hb_set::HbSet,
    ) {
        for l in &self.lookups[table_index] {
            lookups_out.add(l.index as u32);
        }
    }

    /// `hb_ot_map_t::apply<Proxy>` (of hb-ot-layout.cc)
    fn apply(
        &self,
        table_index: usize,
        plan: &HbOtShapePlan,
        font: &mut HbFont,
        buffer: &mut HbBuffer,
    ) {
        let face = font.p.face.clone();
        let accel = if table_index == 0 {
            face.gsub()
        } else {
            face.gpos()
        };
        let mut i = 0;

        let mut c = HbOtApplyContext::new(table_index as u32, font, &face, buffer, accel);

        for stage in &self.stages[table_index] {
            while i < stage.last_lookup as usize {
                let lookup = &self.lookups[table_index][i];
                i += 1;

                let lookup_index = lookup.index as u32;

                /* (proxy.accel.get_accel (lookup_index) is null past the
                 * lookup count; its digest only skips lookups that would
                 * not apply) */
                if lookup_index >= accel.lookup_count {
                    continue;
                }

                c.set_lookup_index(lookup_index);
                c.set_lookup_mask(lookup.mask, false);
                c.set_auto_zwj(lookup.auto_zwj, false);
                c.set_auto_zwnj(lookup.auto_zwnj, false);
                c.set_random(lookup.random);
                c.set_per_syllable(lookup.per_syllable, false);
                /* apply_string's set_lookup_props initializes the iterators. */

                apply_string(&mut c, accel.table().get_lookup(lookup_index));
            }

            if let Some(pause_func) = stage.pause_func {
                /* (the buffer digest is not translated) */
                let _ = pause_func(plan, c.font, c.buffer);
            }
        }
    }

    /// `substitute`
    pub(crate) fn substitute(
        &self,
        plan: &HbOtShapePlan,
        font: &mut HbFont,
        buffer: &mut HbBuffer,
    ) {
        self.apply(0, plan, font, buffer);
    }

    /// `position`
    pub(crate) fn position(&self, plan: &HbOtShapePlan, font: &mut HbFont, buffer: &mut HbBuffer) {
        self.apply(1, plan, font, buffer);
    }
}

/// `hb_ot_map_feature_flags_t`
pub(crate) type HbOtMapFeatureFlags = u32;
pub(crate) const F_NONE: HbOtMapFeatureFlags = 0x0000;
pub(crate) const F_GLOBAL: HbOtMapFeatureFlags = 0x0001; /* Feature applies to all characters; results in no mask allocated for it. */
pub(crate) const F_HAS_FALLBACK: HbOtMapFeatureFlags = 0x0002; /* Has fallback implementation, so include mask bit even if feature not found. */
pub(crate) const F_MANUAL_ZWNJ: HbOtMapFeatureFlags = 0x0004; /* Don't skip over ZWNJ when matching **context**. */
pub(crate) const F_MANUAL_ZWJ: HbOtMapFeatureFlags = 0x0008; /* Don't skip over ZWJ when matching **input**. */
pub(crate) const F_MANUAL_JOINERS: HbOtMapFeatureFlags = F_MANUAL_ZWNJ | F_MANUAL_ZWJ;
pub(crate) const F_GLOBAL_MANUAL_JOINERS: HbOtMapFeatureFlags = F_GLOBAL | F_MANUAL_JOINERS;
pub(crate) const F_GLOBAL_HAS_FALLBACK: HbOtMapFeatureFlags = F_GLOBAL | F_HAS_FALLBACK;
pub(crate) const F_GLOBAL_SEARCH: HbOtMapFeatureFlags = 0x0010; /* If feature not found in LangSys, look for it in global feature list and pick one. */
pub(crate) const F_RANDOM: HbOtMapFeatureFlags = 0x0020; /* Randomly select a glyph from an AlternateSubstFormat1 subtable. */
pub(crate) const F_PER_SYLLABLE: HbOtMapFeatureFlags = 0x0040; /* Contain lookup application to within syllable. */

/// `hb_ot_map_feature_t`
#[derive(Debug, Clone, Copy)]
pub(crate) struct HbOtMapFeature {
    pub(crate) tag: HbTag,
    pub(crate) flags: HbOtMapFeatureFlags,
}

/// `hb_ot_map_builder_t::feature_info_t`
#[derive(Debug, Clone, Copy)]
struct FeatureInfo {
    tag: HbTag,
    seq: u32, /* sequence#, used for stable sorting only */
    max_value: u32,
    flags: HbOtMapFeatureFlags,
    default_value: u32, /* for non-global features, what should the unset glyphs take */
    stage: [u32; 2],    /* GSUB/GPOS */
}

impl FeatureInfo {
    /// `feature_info_t::cmp`
    fn cmp(a: &FeatureInfo, b: &FeatureInfo) -> i32 {
        if a.tag != b.tag {
            if a.tag < b.tag {
                -1
            } else {
                1
            }
        } else if a.seq < b.seq {
            -1
        } else if a.seq > b.seq {
            1
        } else {
            0
        }
    }
}

/// `hb_ot_map_builder_t::stage_info_t`
#[derive(Debug, Clone, Copy)]
struct StageInfo {
    index: u32,
    pause_func: Option<PauseFunc>,
}

/// `hb_bit_storage`
fn hb_bit_storage(v: u32) -> u32 {
    32 - v.leading_zeros()
}

/// `hb_ot_map_builder_t`
pub(crate) struct HbOtMapBuilder<'f> {
    pub(crate) face: &'f HbFace,
    pub(crate) props: HbSegmentProperties,
    pub(crate) is_simple: bool,
    pub(crate) chosen_script: [HbTag; 2],
    pub(crate) found_script: [bool; 2],
    pub(crate) script_index: [u32; 2],
    pub(crate) language_index: [u32; 2],

    current_stage: [u32; 2], /* GSUB/GPOS */
    feature_infos: Vec<FeatureInfo>,
    stages: [Vec<StageInfo>; 2], /* GSUB/GPOS */
}

impl<'f> HbOtMapBuilder<'f> {
    /// `hb_ot_map_builder_t (face, props)`
    pub(crate) fn new(face: &'f HbFace, props: &HbSegmentProperties) -> HbOtMapBuilder<'f> {
        let mut b = HbOtMapBuilder {
            face,
            props: *props,
            is_simple: false,
            chosen_script: [0; 2],
            found_script: [false; 2],
            script_index: [0; 2],
            language_index: [0; 2],
            current_stage: [0; 2],
            feature_infos: Vec::new(),
            stages: [Vec::new(), Vec::new()],
        };

        /* Fetch script/language indices for GSUB/GPOS.  We need these later to skip
         * features not available in either table and not waste precious bits for them. */

        let mut script_tags = [0; HB_OT_MAX_TAGS_PER_SCRIPT];
        let mut language_tags = [0; HB_OT_MAX_TAGS_PER_LANGUAGE];

        let (script_count, language_count) = hb_ot_tags_from_script_and_language(
            b.props.script,
            b.props.language,
            Some(&mut script_tags),
            Some(&mut language_tags),
        );

        for table_index in 0..2 {
            let table_tag = TABLE_TAGS[table_index];
            b.found_script[table_index] = hb_ot_layout_table_select_script(
                face,
                table_tag,
                &script_tags[..script_count],
                &mut b.script_index[table_index],
                &mut b.chosen_script[table_index],
            );
            let mut chosen_language = 0;
            hb_ot_layout_script_select_language2(
                face,
                table_tag,
                b.script_index[table_index],
                &language_tags[..language_count],
                &mut b.language_index[table_index],
                &mut chosen_language,
            );
        }

        b
    }

    /// `add_feature`
    pub(crate) fn add_feature(&mut self, tag: HbTag, flags: HbOtMapFeatureFlags, value: u32) {
        if tag == 0 {
            return;
        }
        let seq = self.feature_infos.len() as u32 + 1;
        self.feature_infos.push(FeatureInfo {
            tag,
            seq,
            max_value: value,
            flags,
            default_value: if flags & F_GLOBAL != 0 { value } else { 0 },
            stage: self.current_stage,
        });
    }

    /// `add_feature (const hb_ot_map_feature_t &feat)`
    pub(crate) fn add_map_feature(&mut self, feat: &HbOtMapFeature) {
        self.add_feature(feat.tag, feat.flags, 1);
    }

    /// `enable_feature`
    pub(crate) fn enable_feature(&mut self, tag: HbTag, flags: HbOtMapFeatureFlags, value: u32) {
        self.add_feature(tag, F_GLOBAL | flags, value);
    }

    /// `disable_feature`
    pub(crate) fn disable_feature(&mut self, tag: HbTag) {
        self.add_feature(tag, F_GLOBAL, 0);
    }

    /// `add_gsub_pause`
    pub(crate) fn add_gsub_pause(&mut self, pause_func: Option<PauseFunc>) {
        self.add_pause(0, pause_func);
    }

    /// `add_gpos_pause`
    pub(crate) fn add_gpos_pause(&mut self, pause_func: Option<PauseFunc>) {
        self.add_pause(1, pause_func);
    }

    /// `has_feature`
    pub(crate) fn has_feature(&self, tag: HbTag) -> bool {
        for table_index in 0..2 {
            let mut feature_index = 0;
            if hb_ot_layout_language_find_feature(
                self.face,
                TABLE_TAGS[table_index],
                self.script_index[table_index],
                self.language_index[table_index],
                tag,
                &mut feature_index,
            ) {
                return true;
            }
        }
        false
    }

    /// `add_lookups`
    #[allow(clippy::too_many_arguments)]
    fn add_lookups(
        &self,
        m: &mut HbOtMap,
        table_index: usize,
        feature_index: u32,
        variations_index: u32,
        mask: HbMask,
        auto_zwnj: bool,
        auto_zwj: bool,
        random: bool,
        per_syllable: bool,
        feature_tag: HbTag,
    ) {
        const LOOKUP_INDICES_LEN: u32 = 32;
        let mut offset = 0;

        let table_lookup_count =
            hb_ot_layout_table_get_lookup_count(self.face, TABLE_TAGS[table_index]);

        loop {
            let mut lookup_indices = Vec::new();
            hb_ot_layout_feature_with_variations_get_lookups(
                self.face,
                TABLE_TAGS[table_index],
                feature_index,
                variations_index,
                offset,
                LOOKUP_INDICES_LEN,
                &mut lookup_indices,
            );
            let len = lookup_indices.len() as u32;

            for &li in &lookup_indices {
                if li >= table_lookup_count {
                    continue;
                }
                m.lookups[table_index].push(LookupMap {
                    mask,
                    index: li as u16,
                    auto_zwnj,
                    auto_zwj,
                    random,
                    per_syllable,
                    feature_tag,
                });
            }

            offset += len;
            if len != LOOKUP_INDICES_LEN {
                break;
            }
        }
    }

    /// `add_pause`
    fn add_pause(&mut self, table_index: usize, pause_func: Option<PauseFunc>) {
        self.stages[table_index].push(StageInfo {
            index: self.current_stage[table_index],
            pause_func,
        });

        self.current_stage[table_index] += 1;
    }

    /// `compile`
    pub(crate) fn compile(&mut self, m: &mut HbOtMap, key: &HbOtShapePlanKey) {
        let global_bit_shift = 8 * 4 - 1;
        let global_bit_mask: u32 = 1 << global_bit_shift;

        m.global_mask = global_bit_mask;

        let mut required_feature_index = [0u32; 2];
        let mut required_feature_tag = [0 as HbTag; 2];
        /* We default to applying required feature in stage 0.  If the required
         * feature has a tag that is known to the shaper, we apply required feature
         * in the stage for that tag.
         */
        let mut required_feature_stage = [0u32; 2];

        for table_index in 0..2 {
            m.chosen_script[table_index] = self.chosen_script[table_index];
            m.found_script[table_index] = self.found_script[table_index];

            hb_ot_layout_language_get_required_feature(
                self.face,
                TABLE_TAGS[table_index],
                self.script_index[table_index],
                self.language_index[table_index],
                &mut required_feature_index[table_index],
                &mut required_feature_tag[table_index],
            );
        }

        /* Sort features and merge duplicates */
        if !self.feature_infos.is_empty() {
            if !self.is_simple {
                hb_qsort(&mut self.feature_infos, FeatureInfo::cmp);
            }
            let f = &mut self.feature_infos;
            let mut j = 0;
            let count = f.len();
            for i in 1..count {
                if f[i].tag != f[j].tag {
                    j += 1;
                    f[j] = f[i];
                } else {
                    if f[i].flags & F_GLOBAL != 0 {
                        f[j].flags |= F_GLOBAL;
                        f[j].max_value = f[i].max_value;
                        f[j].default_value = f[i].default_value;
                    } else {
                        if f[j].flags & F_GLOBAL != 0 {
                            f[j].flags ^= F_GLOBAL;
                        }
                        f[j].max_value = f[j].max_value.max(f[i].max_value);
                        /* Inherit default_value from j */
                    }
                    f[j].flags |= f[i].flags & F_HAS_FALLBACK;
                    f[j].stage[0] = f[j].stage[0].min(f[i].stage[0]);
                    f[j].stage[1] = f[j].stage[1].min(f[i].stage[1]);
                }
            }
            f.truncate(j + 1);
        }

        let mut feature_indices: [HashMap<HbTag, u32>; 2] = [HashMap::new(), HashMap::new()];
        for table_index in 0..2 {
            hb_ot_layout_collect_features_map(
                self.face,
                TABLE_TAGS[table_index],
                self.script_index[table_index],
                self.language_index[table_index],
                &mut feature_indices[table_index],
            );
        }

        /* Allocate bits now */
        let mut next_bit = HB_GLYPH_FLAG_DEFINED.count_ones() + 1;

        for info in &self.feature_infos {
            let bits_needed = if (info.flags & F_GLOBAL) != 0 && info.max_value == 1 {
                /* Uses the global bit */
                0
            } else {
                /* Limit bits per feature. */
                HB_OT_MAP_MAX_BITS.min(hb_bit_storage(info.max_value))
            };

            if info.max_value == 0 || next_bit + bits_needed >= global_bit_shift {
                continue; /* Feature disabled, or not enough bits. */
            }

            let mut found = false;
            let mut feature_index = [0u32; 2];
            for table_index in 0..2 {
                if required_feature_tag[table_index] == info.tag {
                    required_feature_stage[table_index] = info.stage[table_index];
                }

                if let Some(&index) = feature_indices[table_index].get(&info.tag) {
                    feature_index[table_index] = index;
                    found = true;
                } else {
                    feature_index[table_index] = HB_OT_LAYOUT_NO_FEATURE_INDEX;
                }
            }
            if !found && (info.flags & F_GLOBAL_SEARCH) != 0 {
                for table_index in 0..2 {
                    found |= hb_ot_layout_table_find_feature(
                        self.face,
                        TABLE_TAGS[table_index],
                        info.tag,
                        &mut feature_index[table_index],
                    );
                }
            }
            if !found && (info.flags & F_HAS_FALLBACK) == 0 {
                continue;
            }

            let mut map = FeatureMap {
                tag: info.tag,
                index: feature_index,
                stage: info.stage,
                auto_zwnj: (info.flags & F_MANUAL_ZWNJ) == 0,
                auto_zwj: (info.flags & F_MANUAL_ZWJ) == 0,
                random: (info.flags & F_RANDOM) != 0,
                per_syllable: (info.flags & F_PER_SYLLABLE) != 0,
                ..Default::default()
            };
            if (info.flags & F_GLOBAL) != 0 && info.max_value == 1 {
                /* Uses the global bit */
                map.shift = global_bit_shift;
                map.mask = global_bit_mask;
            } else {
                map.shift = next_bit;
                map.mask = (1u32 << (next_bit + bits_needed)).wrapping_sub(1u32 << next_bit);
                next_bit += bits_needed;
                m.global_mask |= (info.default_value << map.shift) & map.mask;
            }
            map._1_mask = (1u32 << map.shift) & map.mask;
            map.needs_fallback = !found;
            m.features.push(map);
        }
        //feature_infos.shrink (0); /* Done with these */
        if self.is_simple {
            hb_qsort(&mut m.features, |a: &FeatureMap, b: &FeatureMap| {
                if a.tag < b.tag {
                    -1
                } else if a.tag > b.tag {
                    1
                } else {
                    0
                }
            });
        }

        self.add_gsub_pause(None);
        self.add_gpos_pause(None);

        for table_index in 0..2 {
            /* Collect lookup indices for features */
            let mut stage_index = 0;
            let mut last_num_lookups = 0;
            for stage in 0..self.current_stage[table_index] {
                if required_feature_index[table_index] != HB_OT_LAYOUT_NO_FEATURE_INDEX
                    && required_feature_stage[table_index] == stage
                {
                    self.add_lookups(
                        m,
                        table_index,
                        required_feature_index[table_index],
                        key.variations_index[table_index],
                        global_bit_mask,
                        true,
                        true,
                        false,
                        false,
                        hb_tag(b' ', b' ', b' ', b' '),
                    );
                }

                for fi in 0..m.features.len() {
                    let feature = m.features[fi];
                    if feature.stage[table_index] == stage {
                        self.add_lookups(
                            m,
                            table_index,
                            feature.index[table_index],
                            key.variations_index[table_index],
                            feature.mask,
                            feature.auto_zwnj,
                            feature.auto_zwj,
                            feature.random,
                            feature.per_syllable,
                            feature.tag,
                        );
                    }
                }

                /* Sort lookups and merge duplicates */
                let lookups = &mut m.lookups[table_index];
                if last_num_lookups + 1 < lookups.len() {
                    hb_qsort(&mut lookups[last_num_lookups..], LookupMap::cmp);

                    let mut j = last_num_lookups;
                    for i in j + 1..lookups.len() {
                        if lookups[i].index != lookups[j].index {
                            j += 1;
                            lookups[j] = lookups[i];
                        } else {
                            lookups[j].mask |= lookups[i].mask;
                            lookups[j].auto_zwnj &= lookups[i].auto_zwnj;
                            lookups[j].auto_zwj &= lookups[i].auto_zwj;
                        }
                    }
                    lookups.truncate(j + 1);
                }

                last_num_lookups = lookups.len();

                if stage_index < self.stages[table_index].len()
                    && self.stages[table_index][stage_index].index == stage
                {
                    m.stages[table_index].push(StageMap {
                        last_lookup: last_num_lookups as u32,
                        pause_func: self.stages[table_index][stage_index].pause_func,
                    });
                    stage_index += 1;
                }
            }
        }
    }
}
