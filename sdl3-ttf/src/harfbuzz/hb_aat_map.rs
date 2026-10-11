// Rust translation of src/hb-aat-map.hh and src/hb-aat-map.cc from
// HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2018  Google, Inc.
// Copyright © 2009,2010  Red Hat, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! The AAT feature map: the `morx`/`mort` chains' subtable flags for the
//! features requested on each range of the text.

use std::cmp::Ordering;

use super::hb_aat_layout::*;
use super::hb_aat_layout_common::RangeFlags;
use super::hb_algs::hb_qsort;
use super::hb_common::*;
use super::hb_face::HbFace;

pub(crate) use super::hb_aat_layout::{
    HB_AAT_LAYOUT_FEATURE_SELECTOR_LOWER_CASE_SMALL_CAPS,
    HB_AAT_LAYOUT_FEATURE_SELECTOR_SMALL_CAPS, HB_AAT_LAYOUT_FEATURE_TYPE_LANGUAGE_TAG_TYPE,
    HB_AAT_LAYOUT_FEATURE_TYPE_LETTER_CASE, HB_AAT_LAYOUT_FEATURE_TYPE_LOWER_CASE,
};

/// `hb_aat_map_t`
#[derive(Debug, Default)]
pub(crate) struct HbAatMap {
    pub(crate) chain_flags: Vec<Vec<RangeFlags>>,
}

/// `hb_aat_map_builder_t::feature_info_t`
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FeatureInfo {
    pub(crate) type_: u32,
    pub(crate) setting: u32,
    pub(crate) is_exclusive: bool,
    pub(crate) seq: u32, /* For stable sorting only. */
}

impl FeatureInfo {
    /// `cmp (const void *pa, const void *pb)`
    fn cmp_sort(a: &FeatureInfo, b: &FeatureInfo) -> Ordering {
        if a.type_ != b.type_ {
            return if a.type_ < b.type_ {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        if !a.is_exclusive && (a.setting & !1) != (b.setting & !1) {
            return if a.setting < b.setting {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        a.seq.cmp(&b.seq)
    }

    /// `cmp (const feature_info_t& f)`: compares type & setting only
    /// (`self` is the item, `f` the key).
    fn cmp(&self, f: &FeatureInfo) -> i32 {
        if f.type_ != self.type_ {
            if f.type_ < self.type_ {
                -1
            } else {
                1
            }
        } else if f.setting != self.setting {
            if f.setting < self.setting {
                -1
            } else {
                1
            }
        } else {
            0
        }
    }
}

/// `hb_aat_map_builder_t::feature_range_t`
#[derive(Debug, Clone, Copy)]
pub(crate) struct FeatureRange {
    pub(crate) info: FeatureInfo,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

/// `hb_aat_map_builder_t::feature_event_t`
#[derive(Debug, Clone, Copy)]
struct FeatureEvent {
    index: u32,
    start: bool,
    feature: FeatureInfo,
}

impl FeatureEvent {
    /// `cmp`
    fn cmp(a: &FeatureEvent, b: &FeatureEvent) -> Ordering {
        a.index
            .cmp(&b.index)
            .then(a.start.cmp(&b.start))
            .then_with(|| FeatureInfo::cmp_sort(&a.feature, &b.feature))
    }
}

/// `hb_aat_map_builder_t`
pub(crate) struct HbAatMapBuilder<'f> {
    pub(crate) face: &'f HbFace,
    pub(crate) props: HbSegmentProperties,

    pub(crate) features: Vec<FeatureRange>,
    pub(crate) current_features: Vec<FeatureInfo>,
    pub(crate) range_first: u32,
    pub(crate) range_last: u32,
}

impl<'f> HbAatMapBuilder<'f> {
    /// `hb_aat_map_builder_t (face, props)`
    pub(crate) fn new(face: &'f HbFace, props: HbSegmentProperties) -> HbAatMapBuilder<'f> {
        HbAatMapBuilder {
            face,
            props,
            features: Vec::new(),
            current_features: Vec::new(),
            range_first: HB_FEATURE_GLOBAL_START,
            range_last: HB_FEATURE_GLOBAL_END,
        }
    }

    /// `current_features.bsearch (info)`
    pub(crate) fn current_features_bsearch(&self, info: &FeatureInfo) -> bool {
        let features = &self.current_features;
        super::hb_open_type::hb_bsearch_impl(features.len(), |mid| features[mid].cmp(info)).is_ok()
    }

    /// `add_feature`
    pub(crate) fn add_feature(&mut self, feature: &HbFeature) {
        let feat = self.face.feat();
        if !feat.has_data() {
            return;
        }

        if feature.tag == hb_tag(b'a', b'a', b'l', b't') {
            if !feat.exposes_feature(HB_AAT_LAYOUT_FEATURE_TYPE_CHARACTER_ALTERNATIVES) {
                return;
            }
            self.features.push(FeatureRange {
                start: feature.start,
                end: feature.end,
                info: FeatureInfo {
                    type_: HB_AAT_LAYOUT_FEATURE_TYPE_CHARACTER_ALTERNATIVES,
                    setting: feature.value,
                    seq: self.features.len() as u32 + 1,
                    is_exclusive: true,
                },
            });
            return;
        }

        let Some(mapping) = hb_aat_layout_find_feature_mapping(feature.tag) else {
            return;
        };

        let mut feature_name = feat.get_feature(mapping.aat_feature_type);
        if !feature_name.has_data() {
            /* Special case: Chain::compile_flags will fall back to the deprecated version of
             * small-caps if necessary, so we need to check for that possibility.
             * https://github.com/harfbuzz/harfbuzz/issues/2307 */
            if mapping.aat_feature_type == HB_AAT_LAYOUT_FEATURE_TYPE_LOWER_CASE
                && mapping.selector_to_enable
                    == HB_AAT_LAYOUT_FEATURE_SELECTOR_LOWER_CASE_SMALL_CAPS
            {
                feature_name = feat.get_feature(HB_AAT_LAYOUT_FEATURE_TYPE_LETTER_CASE);
                if !feature_name.has_data() {
                    return;
                }
            } else {
                return;
            }
        }

        self.features.push(FeatureRange {
            start: feature.start,
            end: feature.end,
            info: FeatureInfo {
                type_: mapping.aat_feature_type,
                setting: if feature.value != 0 {
                    mapping.selector_to_enable
                } else {
                    mapping.selector_to_disable
                },
                seq: self.features.len() as u32 + 1,
                is_exclusive: feature_name.is_exclusive(),
            },
        });
    }

    /// `compile`
    pub(crate) fn compile(&mut self, m: &mut HbAatMap) {
        /* Compute active features per range, and compile each. */

        /* Sort features by start/end events. */
        let mut feature_events: Vec<FeatureEvent> = Vec::new();
        for feature in &self.features {
            if feature.start == feature.end {
                continue;
            }

            feature_events.push(FeatureEvent {
                index: feature.start,
                start: true,
                feature: feature.info,
            });

            feature_events.push(FeatureEvent {
                index: feature.end,
                start: false,
                feature: feature.info,
            });
        }
        hb_qsort(&mut feature_events, |a, b| FeatureEvent::cmp(a, b) as i32);
        /* Add a strategic final event. */
        {
            /* (C leaves the feature's other fields uninitialized; they are
             * only compared after the last range is compiled) */
            let feature = FeatureInfo {
                seq: self.features.len() as u32 + 1,
                ..Default::default()
            };

            feature_events.push(FeatureEvent {
                index: u32::MAX, /* This value does magic. */
                start: false,
                feature,
            });
        }

        /* Scan events and save features for each range. */
        let mut active_features: Vec<FeatureInfo> = Vec::new();
        let mut last_index = 0u32;
        for event in &feature_events {
            if event.index != last_index {
                /* Save a snapshot of active features and the range. */

                /* Sort features and merge duplicates */
                self.current_features = active_features.clone();
                self.range_first = last_index;
                self.range_last = event.index.wrapping_sub(1);
                if !self.current_features.is_empty() {
                    let current_features = &mut self.current_features;
                    hb_qsort(current_features, |a, b| FeatureInfo::cmp_sort(a, b) as i32);
                    let mut j = 0;
                    for i in 1..current_features.len() {
                        if current_features[i].type_ != current_features[j].type_ ||
                            /* Nonexclusive feature selectors come in even/odd pairs to turn a setting on/off
                             * respectively, so we mask out the low-order bit when checking for "duplicates"
                             * (selectors referring to the same feature setting) here. */
                            (!current_features[i].is_exclusive
                                && ((current_features[i].setting & !1)
                                    != (current_features[j].setting & !1)))
                        {
                            j += 1;
                            current_features[j] = current_features[i];
                        }
                    }
                    current_features.truncate(j + 1);
                }

                hb_aat_layout_compile_map(self, m);

                last_index = event.index;
            }

            if event.start {
                active_features.push(event.feature);
            } else if let Some(i) = active_features
                .iter()
                .position(|f| f.cmp(&event.feature) == 0)
            {
                active_features.remove(i);
            }
        }

        for chain_flags in &mut m.chain_flags {
            // With our above setup this value is one less than desired; adjust it.
            if let Some(tail) = chain_flags.last_mut() {
                tail.cluster_last = HB_FEATURE_GLOBAL_END;
            }
        }
    }
}
