// Rust translation of src/hb-aat-layout-feat-table.hh from HarfBuzz
// (8.5.0, as SDL_ttf's external/harfbuzz pins it).
// Copyright © 2018  Ebrahim Byagowi
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).

//! feat -- Feature Name
//! <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6feat.html>
//!
//! Translation notes: the selector queries of the public API
//! (`get_feature_types`, `get_selector_infos`, the name ids), which
//! SDL_ttf does not call, are not translated.

use super::hb_common::*;
use super::hb_open_type::*;
use super::hb_sanitize::HbSanitizeContext;

/// `HB_AAT_TAG_feat`
pub(crate) const HB_AAT_TAG_FEAT: HbTag = hb_tag(b'f', b'e', b'a', b't');

/* SettingName: setting (HBUINT16, the setting), nameIndex (NameID, the
 * name table index for the setting's name): 4 bytes. */

/// `FeatureName` (a slice of the table starting at it; the Null one is
/// empty) and its table (`base`).
#[derive(Clone, Copy)]
pub(crate) struct FeatureName<'a>(pub(crate) &'a [u8]);

/* FeatureName flags */
pub(crate) const FEATURE_NAME_EXCLUSIVE: u16 = 0x8000; /* If set, the feature settings are mutually exclusive. */
pub(crate) const FEATURE_NAME_NOT_DEFAULT: u16 = 0x4000; /* If clear, then the setting with an index of 0 in
                                                          * the setting name array for this feature should
                                                          * be taken as the default for the feature
                                                          * (if one is required). If set, then bits 0-15 of this
                                                          * featureFlags field contain the index of the setting
                                                          * which is to be taken as the default. */
pub(crate) const FEATURE_NAME_INDEX_MASK: u16 = 0x00FF; /* If bits 30 and 31 are set, then these sixteen bits
                                                         * indicate the index of the setting in the setting name
                                                         * array for this feature which should be taken
                                                         * as the default. */

impl FeatureName<'_> {
    /// `get_feature_type`
    pub(crate) fn get_feature_type(&self) -> u32 {
        u16_at(self.0, 0) as u32
    }

    /// `is_exclusive`
    pub(crate) fn is_exclusive(&self) -> bool {
        u16_at(self.0, 8) & FEATURE_NAME_EXCLUSIVE != 0
    }

    /* A FeatureName with no settings is meaningless */
    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        u16_at(self.0, 2) != 0
    }

    /// `sanitize (c, base)`: the feature name at `p` of the table at
    /// `base`.
    pub(crate) fn sanitize(c: &mut HbSanitizeContext, p: usize, base: usize) -> bool {
        c.check_struct(p, 12) && {
            let n_settings = c.u16(p + 2) as u32;
            let setting_table = base + c.u32(p + 4) as usize;
            /* (base+settingTableZ).sanitize (c, nSettings) */
            c.check_array(setting_table, n_settings, 4)
        }
    }
}

/// `feat` (the sanitized table)
#[derive(Clone, Copy)]
pub(crate) struct Feat<'a>(pub(crate) &'a [u8]);

impl<'a> Feat<'a> {
    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        /* version.to_int () */
        u32_at(self.0, 0) != 0
    }

    /// `exposes_feature`
    pub(crate) fn exposes_feature(&self, feature_type: u32) -> bool {
        self.get_feature(feature_type).has_data()
    }

    /// `get_feature`: `namesZ.bsearch (featureNameCount, feature_type)`
    pub(crate) fn get_feature(&self, feature_type: u32) -> FeatureName<'a> {
        let count = u16_at(self.0, 4) as usize;
        let names = struct_at(self.0, 12);
        match hb_bsearch_impl(count, |mid| {
            /* FeatureName::cmp: (int) key - (int) feature */
            (feature_type as i32).wrapping_sub(u16_at(names, mid * 12) as i32)
        }) {
            Ok(i) => FeatureName(struct_at(names, i as usize * 12)),
            Err(_) => FeatureName(NULL),
        }
    }

    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext) -> bool {
        c.check_struct(0, 12) && c.u16(0) == 1 /* version.major */ && {
            let count = c.u16(4) as u32;
            /* namesZ.sanitize (c, featureNameCount, this) */
            c.check_array(12, count, 12)
                && (0..count as usize).all(|i| FeatureName::sanitize(c, 12 + i * 12, 0))
        }
    }
}
