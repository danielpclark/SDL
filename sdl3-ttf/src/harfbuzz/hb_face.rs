// Rust translation of the parts of src/hb-face.hh, src/hb-face.cc,
// src/hb-static.cc (load_num_glyphs, load_upem) and src/hb-ot-face.hh that
// shaping uses, with the 'maxp', 'head' and 'OS/2' table fields it reads
// (src/hb-ot-maxp-table.hh, src/hb-ot-head-table.hh,
// src/hb-ot-os2-table.hh), from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2009  Red Hat, Inc.
// Copyright © 2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! `hb_face_t`: a font face, as a set of tables.
//!
//! Translation notes: a face is made from its tables' data, loaded when
//! the face is created (C's `reference_table` callback loads them on first
//! use; the data is the same). The tables HarfBuzz uses (`table.GDEF`,
//! `table.GSUB`, ...) are sanitized and accelerated lazily, on first use,
//! as in C.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::hb_common::*;
use super::hb_ot_kern_table::KernAccel;
use super::hb_ot_layout_gdef::GdefAccel;
use super::hb_ot_layout_gsubgpos::{GsubGposAccel, GsubGposKind};
use super::hb_sanitize::hb_sanitize_blob;

/// `HB_OT_TAG_GDEF`
pub const HB_OT_TAG_GDEF: HbTag = hb_tag(b'G', b'D', b'E', b'F');
/// `HB_OT_TAG_GSUB`
pub const HB_OT_TAG_GSUB: HbTag = hb_tag(b'G', b'S', b'U', b'B');
/// `HB_OT_TAG_GPOS`
pub const HB_OT_TAG_GPOS: HbTag = hb_tag(b'G', b'P', b'O', b'S');
/// `HB_OT_TAG_kern`
pub const HB_OT_TAG_KERN: HbTag = hb_tag(b'k', b'e', b'r', b'n');
/// `HB_OT_TAG_maxp`
pub const HB_OT_TAG_MAXP: HbTag = hb_tag(b'm', b'a', b'x', b'p');
/// `HB_OT_TAG_head`
pub const HB_OT_TAG_HEAD: HbTag = hb_tag(b'h', b'e', b'a', b'd');
/// `HB_OT_TAG_OS2`
pub const HB_OT_TAG_OS2: HbTag = hb_tag(b'O', b'S', b'/', b'2');
/// `HB_AAT_TAG_morx`
pub const HB_AAT_TAG_MORX: HbTag = hb_tag(b'm', b'o', b'r', b'x');
/// `HB_AAT_TAG_mort`
pub const HB_AAT_TAG_MORT: HbTag = hb_tag(b'm', b'o', b'r', b't');
/// `HB_AAT_TAG_kerx`
pub const HB_AAT_TAG_KERX: HbTag = hb_tag(b'k', b'e', b'r', b'x');
/// `HB_AAT_TAG_trak`
pub const HB_AAT_TAG_TRAK: HbTag = hb_tag(b't', b'r', b'a', b'k');

/// The tables a face loads for shaping (C loads them on demand).
pub(crate) const HB_FACE_TABLES: [HbTag; 11] = [
    HB_OT_TAG_GDEF,
    HB_OT_TAG_GSUB,
    HB_OT_TAG_GPOS,
    HB_OT_TAG_KERN,
    HB_OT_TAG_MAXP,
    HB_OT_TAG_HEAD,
    HB_OT_TAG_OS2,
    HB_AAT_TAG_MORX,
    HB_AAT_TAG_MORT,
    HB_AAT_TAG_KERX,
    HB_AAT_TAG_TRAK,
];

/// `hb_face_t`
pub struct HbFace {
    /// the tables' data (C's `reference_table_func` results; an absent
    /// table is the empty blob)
    tables: HashMap<HbTag, Vec<u8>>,
    pub(crate) index: u32,
    /// `upem` (0: not set yet)
    upem: OnceLock<u32>,
    /// `num_glyphs`
    num_glyphs: OnceLock<u32>,

    /* table */
    gdef: OnceLock<GdefAccel>,
    gsub: OnceLock<GsubGposAccel>,
    gpos: OnceLock<GsubGposAccel>,
    kern: OnceLock<KernAccel>,
    os2: OnceLock<Vec<u8>>,
}

impl std::fmt::Debug for HbFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HbFace")
            .field("index", &self.index)
            .finish()
    }
}

impl HbFace {
    /// `hb_face_create_for_tables`, with the tables' data (as
    /// `reference_table` returns it), and `hb_face_set_index` and
    /// `hb_face_set_upem` (0: unset).
    pub fn new_for_tables(tables: HashMap<HbTag, Vec<u8>>, index: u32, upem: u32) -> HbFace {
        let face = HbFace {
            tables,
            index,
            upem: OnceLock::new(),
            num_glyphs: OnceLock::new(),
            gdef: OnceLock::new(),
            gsub: OnceLock::new(),
            gpos: OnceLock::new(),
            kern: OnceLock::new(),
            os2: OnceLock::new(),
        };
        if upem != 0 {
            let _ = face.upem.set(upem);
        }
        face
    }

    /// The empty face (`hb_face_get_empty`).
    pub fn empty() -> HbFace {
        HbFace::new_for_tables(HashMap::new(), 0, 0)
    }

    /// `reference_table`: the table's data (empty if absent).
    pub(crate) fn reference_table(&self, tag: HbTag) -> Vec<u8> {
        self.tables.get(&tag).cloned().unwrap_or_default()
    }

    /// `get_upem`
    pub(crate) fn get_upem(&self) -> u32 {
        *self.upem.get_or_init(|| self.load_upem())
    }

    /// `load_upem`: from the sanitized 'head' table.
    fn load_upem(&self) -> u32 {
        let head = hb_sanitize_blob(self.reference_table(HB_OT_TAG_HEAD), Some(0), |c| {
            /* head::sanitize */
            c.check_struct(0, 54) && c.u16(0) == 1 && c.u32(12) == 0x5F0F3CF5
        });
        /* head::get_upem */
        let upem = if head.len() >= 54 {
            u16::from_be_bytes([head[18], head[19]]) as u32
        } else {
            0
        };
        /* If no valid head table found, assume 1000, which matches typical Type1 usage. */
        if (16..=16384).contains(&upem) {
            upem
        } else {
            1000
        }
    }

    /// `get_num_glyphs` (`hb_face_get_glyph_count`)
    pub(crate) fn get_num_glyphs(&self) -> u32 {
        *self.num_glyphs.get_or_init(|| self.load_num_glyphs())
    }

    /// `load_num_glyphs` (from 'maxp': HB_NO_BEYOND_64K is defined)
    fn load_num_glyphs(&self) -> u32 {
        /* (core tables are sanitized with a glyph count of 0, so as not to
         * recurse) */
        let maxp = hb_sanitize_blob(self.reference_table(HB_OT_TAG_MAXP), Some(0), |c| {
            /* maxp::sanitize */
            if !c.check_struct(0, 6) {
                return false;
            }
            if c.u16(0) == 1 {
                /* maxpV1Tail::sanitize */
                return c.check_struct(6, 26);
            }
            c.u16(0) == 0 && c.u16(2) == 0x5000
        });
        /* maxp::get_num_glyphs */
        if maxp.len() >= 6 {
            u16::from_be_bytes([maxp[4], maxp[5]]) as u32
        } else {
            0
        }
    }

    /// `table.GDEF`
    pub(crate) fn gdef(&self) -> &GdefAccel {
        self.gdef.get_or_init(|| GdefAccel::new(self))
    }

    /// `table.GSUB`
    pub(crate) fn gsub(&self) -> &GsubGposAccel {
        self.gsub
            .get_or_init(|| GsubGposAccel::new(self, GsubGposKind::Gsub))
    }

    /// `table.GPOS`
    pub(crate) fn gpos(&self) -> &GsubGposAccel {
        self.gpos
            .get_or_init(|| GsubGposAccel::new(self, GsubGposKind::Gpos))
    }

    /// `table.kern`
    pub(crate) fn kern(&self) -> &KernAccel {
        self.kern.get_or_init(|| KernAccel::new(self))
    }

    /// `table.OS2->get_font_page ()`
    pub(crate) fn os2_get_font_page(&self) -> u32 {
        let os2 = self.os2.get_or_init(|| {
            hb_sanitize_blob(
                self.reference_table(HB_OT_TAG_OS2),
                Some(self.get_num_glyphs()),
                |c| {
                    /* OS2::sanitize */
                    if !c.check_struct(0, 78) {
                        return false;
                    }
                    let version = c.u16(0);
                    if version >= 1 && !c.check_struct(78, 8) {
                        return false;
                    }
                    if version >= 2 && !c.check_struct(86, 10) {
                        return false;
                    }
                    if version >= 5 && !c.check_struct(96, 4) {
                        return false;
                    }
                    true
                },
            )
        });
        if os2.len() < 78 {
            return 0;
        }
        let version = u16::from_be_bytes([os2[0], os2[1]]);
        let fs_selection = u16::from_be_bytes([os2[62], os2[63]]) as u32;
        /* OS2::get_font_page */
        if version == 0 {
            fs_selection & 0xFF00
        } else {
            0
        }
    }
}

/// `font_page_t`
pub(crate) const FONT_PAGE_NONE: u32 = 0;
pub(crate) const FONT_PAGE_SIMP_ARABIC: u32 = 0xB200; /* Simplified Arabic Windows 3.1 font page */
pub(crate) const FONT_PAGE_TRAD_ARABIC: u32 = 0xB300; /* Traditional Arabic Windows 3.1 font page */
