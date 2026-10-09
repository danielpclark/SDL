// Rust translation of OT/Layout/GDEF/GDEF.hh (src/hb-ot-layout-gdef-table.hh)
// and GDEF::is_blocklisted (src/hb-ot-layout.cc) from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it), without the subsetter.
// Copyright © 2007,2008,2009  Red Hat, Inc.
// Copyright © 2010,2011,2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! GDEF -- Glyph Definition
//! <https://docs.microsoft.com/en-us/typography/opentype/spec/gdef>
//!
//! Translation notes: the glyph properties cache and the mark glyph set
//! digests of the accelerator are not translated (they only avoid
//! recomputing the same results).

use super::hb_common::*;
use super::hb_face::*;
use super::hb_open_type::*;
use super::hb_ot_layout::*;
use super::hb_ot_layout_common::*;
use super::hb_sanitize::{hb_sanitize_blob, HbSanitizeContext};
use super::hb_ucd_table::hb_codepoint_encode3;

/* GlyphClasses */
pub(crate) const GDEF_UNCLASSIFIED_GLYPH: u32 = 0;
pub(crate) const GDEF_BASE_GLYPH: u32 = 1;
pub(crate) const GDEF_LIGATURE_GLYPH: u32 = 2;
pub(crate) const GDEF_MARK_GLYPH: u32 = 3;
pub(crate) const GDEF_COMPONENT_GLYPH: u32 = 4;

/// `AttachList::sanitize`
fn attach_list_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    c.sanitize_offset16(p, p, Coverage::sanitize)
        && c.sanitize_array16_of_offset16(p + 2, p, |c, q| {
            /* AttachPoint: Array16Of<HBUINT16> */
            c.sanitize_array16_shallow(q, 2)
        })
}

/// `CaretValue::sanitize`
fn caret_value_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        /* CaretValueFormat1 */
        1 => c.check_struct(p, 4),
        /* CaretValueFormat2 */
        2 => c.check_struct(p, 4),
        /* CaretValueFormat3 */
        3 => c.check_struct(p, 6) && c.sanitize_offset16(p + 4, p, Device::sanitize),
        _ => true,
    }
}

/// `LigCaretList::sanitize`
fn lig_caret_list_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    c.sanitize_offset16(p, p, Coverage::sanitize)
        && c.sanitize_array16_of_offset16(p + 2, p, |c, q| {
            /* LigGlyph::sanitize: carets */
            c.sanitize_array16_of_offset16(q, q, caret_value_sanitize)
        })
}

/// `MarkGlyphSets::sanitize`
fn mark_glyph_sets_sanitize(c: &mut HbSanitizeContext, p: usize) -> bool {
    if !c.sanitize_u16(p) {
        return false;
    }
    match c.u16(p) {
        /* MarkGlyphSetsFormat1: coverage */
        1 => c.sanitize_array16_of_offset32(p + 2, p, Coverage::sanitize),
        _ => true,
    }
}

/// `GDEF`
#[derive(Clone, Copy)]
pub(crate) struct Gdef<'a>(pub(crate) &'a [u8]);

impl<'a> Gdef<'a> {
    /// `sanitize`
    pub(crate) fn sanitize(c: &mut HbSanitizeContext) -> bool {
        let p = 0;
        if !c.check_struct(p, 4) {
            return false;
        }
        match c.u16(p) {
            1 => {
                /* GDEFVersion1_2::sanitize */
                let version = c.u32(p);
                c.check_struct(p, 4)
                    && c.sanitize_offset16(p + 4, p, ClassDef::sanitize)
                    && c.sanitize_offset16(p + 6, p, attach_list_sanitize)
                    && c.sanitize_offset16(p + 8, p, lig_caret_list_sanitize)
                    && c.sanitize_offset16(p + 10, p, ClassDef::sanitize)
                    && (version < 0x00010002
                        || c.sanitize_offset16(p + 12, p, mark_glyph_sets_sanitize))
                    && (version < 0x00010003
                        || c.sanitize_offset32(p + 14, p, ItemVariationStore::sanitize))
            }
            _ => true,
        }
    }

    #[inline]
    fn version(&self) -> u32 {
        u32_at(self.0, 0)
    }
    #[inline]
    fn major(&self) -> u16 {
        u16_at(self.0, 0)
    }

    /// `has_glyph_classes`
    pub(crate) fn has_glyph_classes(&self) -> bool {
        match self.major() {
            1 => u16_at(self.0, 4) != 0,
            _ => false,
        }
    }
    /// `get_glyph_class_def`
    pub(crate) fn get_glyph_class_def(&self) -> ClassDef<'a> {
        match self.major() {
            1 => ClassDef(offset16_to(self.0, 4)),
            _ => ClassDef(NULL),
        }
    }
    /// `get_mark_attach_class_def`
    pub(crate) fn get_mark_attach_class_def(&self) -> ClassDef<'a> {
        match self.major() {
            1 => ClassDef(offset16_to(self.0, 10)),
            _ => ClassDef(NULL),
        }
    }
    /// `get_mark_glyph_sets`
    pub(crate) fn get_mark_glyph_sets(&self) -> &'a [u8] {
        match self.major() {
            1 if self.version() >= 0x00010002 => offset16_to(self.0, 12),
            _ => NULL,
        }
    }
    /// `get_var_store`
    pub(crate) fn get_var_store(&self) -> ItemVariationStore<'a> {
        match self.major() {
            1 if self.version() >= 0x00010003 => ItemVariationStore(offset32_to(self.0, 14)),
            _ => ItemVariationStore(NULL),
        }
    }
    /// `has_data`
    pub(crate) fn has_data(&self) -> bool {
        self.version() != 0
    }

    /// `get_glyph_class`
    pub(crate) fn get_glyph_class(&self, glyph: HbCodepoint) -> u32 {
        self.get_glyph_class_def().get_class(glyph)
    }
    /// `get_mark_attachment_type`
    pub(crate) fn get_mark_attachment_type(&self, glyph: HbCodepoint) -> u32 {
        self.get_mark_attach_class_def().get_class(glyph)
    }

    /// `mark_set_covers` (`MarkGlyphSets::covers`)
    pub(crate) fn mark_set_covers(&self, set_index: u32, glyph_id: HbCodepoint) -> bool {
        let sets = self.get_mark_glyph_sets();
        match u16_at(sets, 0) {
            1 => {
                /* MarkGlyphSetsFormat1::covers */
                let count = u16_at(sets, 2) as u32;
                let cov = if set_index < count {
                    offset32_to(sets, 4 + set_index as usize * 4)
                } else {
                    NULL
                };
                Coverage(cov).get_coverage(glyph_id) != NOT_COVERED
            }
            _ => false,
        }
    }

    /// `get_glyph_props`: glyph_props is a 16-bit integer where the lower
    /// 8-bit have bits representing glyph class and other bits, and high
    /// 8-bit the mark attachment type (if any). Not to be confused with
    /// lookup_props which is very similar.
    pub(crate) fn get_glyph_props(&self, glyph: HbCodepoint) -> u32 {
        let klass = self.get_glyph_class(glyph);

        match klass {
            GDEF_BASE_GLYPH => HB_OT_LAYOUT_GLYPH_PROPS_BASE_GLYPH,
            GDEF_LIGATURE_GLYPH => HB_OT_LAYOUT_GLYPH_PROPS_LIGATURE,
            GDEF_MARK_GLYPH => {
                let klass = self.get_mark_attachment_type(glyph);
                HB_OT_LAYOUT_GLYPH_PROPS_MARK | (klass << 8)
            }
            _ => HB_OT_LAYOUT_GLYPH_CLASS_UNCLASSIFIED,
        }
    }

    /// `is_blocklisted`: the ugly business of blocklisting individual
    /// fonts' tables happen here!  See this thread for why we finally had
    /// to bend in and do this:
    /// <https://lists.freedesktop.org/archives/harfbuzz/2016-February/005489.html>
    ///
    /// In certain versions of Times New Roman Italic and Bold Italic,
    /// ASCII double quotation mark U+0022 has wrong glyph class 3 (mark)
    /// in GDEF.  Many versions of Tahoma have bad GDEF tables that
    /// incorrectly classify some spacing marks such as certain IPA
    /// symbols as glyph class 3. So do older versions of Microsoft
    /// Himalaya, and the version of Cantarell shipped by Ubuntu 16.04.
    ///
    /// Nuke the GDEF tables of to avoid unwanted width-zeroing.
    ///
    /// See <https://bugzilla.mozilla.org/show_bug.cgi?id=1279925>,
    /// <https://bugzilla.mozilla.org/show_bug.cgi?id=1279693> and
    /// <https://bugzilla.mozilla.org/show_bug.cgi?id=1279875>.
    fn is_blocklisted(blob_length: u32, face: &HbFace) -> bool {
        const BLOCKLIST: &[(u32, u32, u32)] = &[
            /* sha1sum:c5ee92f0bca4bfb7d06c4d03e8cf9f9cf75d2e8a Windows 7? timesi.ttf */
            (442, 2874, 42038),
            /* sha1sum:37fc8c16a0894ab7b749e35579856c73c840867b Windows 7? timesbi.ttf */
            (430, 2874, 40662),
            /* sha1sum:19fc45110ea6cd3cdd0a5faca256a3797a069a80 Windows 7 timesi.ttf */
            (442, 2874, 39116),
            /* sha1sum:6d2d3c9ed5b7de87bc84eae0df95ee5232ecde26 Windows 7 timesbi.ttf */
            (430, 2874, 39374),
            /* sha1sum:8583225a8b49667c077b3525333f84af08c6bcd8 OS X 10.11.3 Times New Roman Italic.ttf */
            (490, 3046, 41638),
            /* sha1sum:ec0f5a8751845355b7c3271d11f9918a966cb8c9 OS X 10.11.3 Times New Roman Bold Italic.ttf */
            (478, 3046, 41902),
            /* sha1sum:96eda93f7d33e79962451c6c39a6b51ee893ce8c  tahoma.ttf from Windows 8 */
            (898, 12554, 46470),
            /* sha1sum:20928dc06014e0cd120b6fc942d0c3b1a46ac2bc  tahomabd.ttf from Windows 8 */
            (910, 12566, 47732),
            /* sha1sum:4f95b7e4878f60fa3a39ca269618dfde9721a79e  tahoma.ttf from Windows 8.1 */
            (928, 23298, 59332),
            /* sha1sum:6d400781948517c3c0441ba42acb309584b73033  tahomabd.ttf from Windows 8.1 */
            (940, 23310, 60732),
            /* tahoma.ttf v6.04 from Windows 8.1 x64, see https://bugzilla.mozilla.org/show_bug.cgi?id=1279925 */
            (964, 23836, 60072),
            /* tahomabd.ttf v6.04 from Windows 8.1 x64, see https://bugzilla.mozilla.org/show_bug.cgi?id=1279925 */
            (976, 23832, 61456),
            /* sha1sum:e55fa2dfe957a9f7ec26be516a0e30b0c925f846  tahoma.ttf from Windows 10 */
            (994, 24474, 60336),
            /* sha1sum:7199385abb4c2cc81c83a151a7599b6368e92343  tahomabd.ttf from Windows 10 */
            (1006, 24470, 61740),
            /* tahoma.ttf v6.91 from Windows 10 x64, see https://bugzilla.mozilla.org/show_bug.cgi?id=1279925 */
            (1006, 24576, 61346),
            /* tahomabd.ttf v6.91 from Windows 10 x64, see https://bugzilla.mozilla.org/show_bug.cgi?id=1279925 */
            (1018, 24572, 62828),
            /* sha1sum:b9c84d820c49850d3d27ec498be93955b82772b5  tahoma.ttf from Windows 10 AU */
            (1006, 24576, 61352),
            /* sha1sum:2bdfaab28174bdadd2f3d4200a30a7ae31db79d2  tahomabd.ttf from Windows 10 AU */
            (1018, 24572, 62834),
            /* sha1sum:b0d36cf5a2fbe746a3dd277bffc6756a820807a7  Tahoma.ttf from Mac OS X 10.9 */
            (832, 7324, 47162),
            /* sha1sum:12fc4538e84d461771b30c18b5eb6bd434e30fba  Tahoma Bold.ttf from Mac OS X 10.9 */
            (844, 7302, 45474),
            /* sha1sum:eb8afadd28e9cf963e886b23a30b44ab4fd83acc  himalaya.ttf from Windows 7 */
            (180, 13054, 7254),
            /* sha1sum:73da7f025b238a3f737aa1fde22577a6370f77b0  himalaya.ttf from Windows 8 */
            (192, 12638, 7254),
            /* sha1sum:6e80fd1c0b059bbee49272401583160dc1e6a427  himalaya.ttf from Windows 8.1 */
            (192, 12690, 7254),
            /* 8d9267aea9cd2c852ecfb9f12a6e834bfaeafe44  cantarell-fonts-0.0.21/otf/Cantarell-Regular.otf */
            /* 983988ff7b47439ab79aeaf9a45bd4a2c5b9d371  cantarell-fonts-0.0.21/otf/Cantarell-Oblique.otf */
            (188, 248, 3852),
            /* 2c0c90c6f6087ffbfea76589c93113a9cbb0e75f  cantarell-fonts-0.0.21/otf/Cantarell-Bold.otf */
            /* 55461f5b853c6da88069ffcdf7f4dd3f8d7e3e6b  cantarell-fonts-0.0.21/otf/Cantarell-Bold-Oblique.otf */
            (188, 264, 3426),
            /* d125afa82a77a6475ac0e74e7c207914af84b37a padauk-2.80/Padauk.ttf RHEL 7.2 */
            (1058, 47032, 11818),
            /* 0f7b80437227b90a577cc078c0216160ae61b031 padauk-2.80/Padauk-Bold.ttf RHEL 7.2*/
            (1046, 47030, 12600),
            /* d3dde9aa0a6b7f8f6a89ef1002e9aaa11b882290 padauk-2.80/Padauk.ttf Ubuntu 16.04 */
            (1058, 71796, 16770),
            /* 5f3c98ccccae8a953be2d122c1b3a77fd805093f padauk-2.80/Padauk-Bold.ttf Ubuntu 16.04 */
            (1046, 71790, 17862),
            /* 6c93b63b64e8b2c93f5e824e78caca555dc887c7 padauk-2.80/Padauk-book.ttf */
            (1046, 71788, 17112),
            /* d89b1664058359b8ec82e35d3531931125991fb9 padauk-2.80/Padauk-bookbold.ttf */
            (1058, 71794, 17514),
            /* 824cfd193aaf6234b2b4dc0cf3c6ef576c0d00ef padauk-3.0/Padauk-book.ttf */
            (1330, 109904, 57938),
            /* 91fcc10cf15e012d27571e075b3b4dfe31754a8a padauk-3.0/Padauk-bookbold.ttf */
            (1330, 109904, 58972),
            /* sha1sum: c26e41d567ed821bed997e937bc0c41435689e85  Padauk.ttf
             *  "Padauk Regular" "Version 2.5", see https://crbug.com/681813 */
            (1004, 59092, 14836),
        ];
        let key = hb_codepoint_encode3(
            blob_length,
            face.gsub().table.len() as u32,
            face.gpos().table.len() as u32,
        );
        BLOCKLIST
            .iter()
            .any(|&(a, b, c)| hb_codepoint_encode3(a, b, c) == key)
    }
}

/// `GDEF::accelerator_t`
#[derive(Debug)]
pub(crate) struct GdefAccel {
    pub(crate) table: Vec<u8>,
}

impl GdefAccel {
    pub(crate) fn new(face: &HbFace) -> GdefAccel {
        let mut table = hb_sanitize_blob(
            face.reference_table(HB_OT_TAG_GDEF),
            Some(face.get_num_glyphs()),
            Gdef::sanitize,
        );
        if Gdef::is_blocklisted(table.len() as u32, face) {
            table = Vec::new();
        }
        GdefAccel { table }
    }

    /// `table`
    #[inline]
    pub(crate) fn table(&self) -> Gdef<'_> {
        Gdef(&self.table)
    }

    /// `get_glyph_props`
    #[inline]
    pub(crate) fn get_glyph_props(&self, glyph: HbCodepoint) -> u32 {
        self.table().get_glyph_props(glyph)
    }

    /// `mark_set_covers`
    #[inline]
    pub(crate) fn mark_set_covers(&self, set_index: u32, glyph_id: HbCodepoint) -> bool {
        self.table().mark_set_covers(set_index, glyph_id)
    }
}
